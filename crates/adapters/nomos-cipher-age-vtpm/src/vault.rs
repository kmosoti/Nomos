//! Hybrid ML-KEM-768+X25519 and classical `age` X25519 vault implementation.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomos_cipher::{
    CipherError, CipherPort, CipherRef, CipherResult, CipherVault, NodeTarget, SecretMetadata,
    SecretPolicy, SecretsPosture, SyncPlan, TargetTransport, VaultStatus,
};
use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroize;

use crate::pq_hybrid::{decrypt_hybrid, encrypt_hybrid, is_hybrid_ciphertext};
use crate::sealer::{CredSealer, SystemdCredsSealer, VaultCipherMode, VaultKeyStorage};
use crate::sync_plan::plan_and_execute_sync_with_sealer;
use crate::vault_io::{
    entry_ciphertext_path, identity_path, load_index, unix_now_secs, with_locked_index,
    write_atomic,
};
use crate::vault_keys::{
    VaultIdentity, VaultRecipient, init_vault_dir, load_identity, load_recipient,
};

/// Encrypted vault adapter supporting hybrid `ML-KEM-768 + X25519` and classical
/// `age` X25519 recipients with TPM2-sealed `vault.key` protection.
#[derive(Clone, Debug)]
pub struct AgeVtpmVault {
    vault_dir: PathBuf,
    key_storage: VaultKeyStorage,
    cipher_mode: VaultCipherMode,
    sealer: Arc<dyn CredSealer>,
}

impl AgeVtpmVault {
    /// Creates a production-default vault handle (`VaultKeyStorage::Tpm2Sealed`,
    /// `VaultCipherMode::HybridMlKem768X25519`) without creating files on disk yet.
    pub fn new(vault_dir: impl AsRef<Path>) -> Self {
        Self {
            vault_dir: vault_dir.as_ref().to_path_buf(),
            key_storage: VaultKeyStorage::Tpm2Sealed,
            cipher_mode: VaultCipherMode::HybridMlKem768X25519,
            sealer: Arc::new(SystemdCredsSealer),
        }
    }

    /// Configures how `vault.key` is sealed on disk (`Tpm2Sealed` vs `PlaintextDev`).
    pub fn with_key_storage(mut self, storage: VaultKeyStorage) -> Self {
        self.key_storage = storage;
        self
    }

    /// Toggles explicit `--allow-plaintext-key` opt-in for local development/CI.
    pub fn allow_plaintext_key(mut self, allow: bool) -> Self {
        self.key_storage = if allow {
            VaultKeyStorage::PlaintextDev
        } else {
            VaultKeyStorage::Tpm2Sealed
        };
        self
    }

    /// Configures the recipient encryption mode (`HybridMlKem768X25519` or `ClassicalX25519`).
    pub fn with_cipher_mode(mut self, mode: VaultCipherMode) -> Self {
        self.cipher_mode = mode;
        self
    }

    /// Injects a custom [`CredSealer`] (defaults to [`SystemdCredsSealer`]).
    pub fn with_cred_sealer(mut self, sealer: Arc<dyn CredSealer>) -> Self {
        self.sealer = sealer;
        self
    }

    /// Configured vault directory path.
    pub fn vault_dir(&self) -> &Path {
        &self.vault_dir
    }

    /// Configured key storage policy.
    pub fn key_storage(&self) -> VaultKeyStorage {
        self.key_storage
    }

    /// Configured cipher mode.
    pub fn cipher_mode(&self) -> VaultCipherMode {
        self.cipher_mode
    }

    /// Generates cryptographically random token bytes and stores hex-encoded value under `key`.
    pub fn rotate_random_hex(&self, key: &str, byte_len: usize) -> CipherResult<SecretMetadata> {
        if byte_len == 0 || byte_len > 4096 {
            return Err(CipherError::InvalidPolicy(
                "random secret length must be in 1..=4096 bytes".into(),
            ));
        }
        let mut raw = vec![0u8; byte_len];
        SystemRandom::new()
            .fill(&mut raw)
            .map_err(|_| CipherError::Crypto("system CSPRNG failure".into()))?;
        let mut encoded = hex::encode(&raw).into_bytes();
        raw.zeroize();

        let wrapped = Secret::new(std::mem::take(&mut encoded));
        let reference = CipherRef::new(key)?;
        self.set(&reference, &wrapped)
    }
}

/// Computes a 32-byte BLAKE3 digest for arbitrary secret bytes.
pub fn blake3_digest(bytes: &[u8]) -> Digest {
    let hash = blake3::hash(bytes);
    Digest::from_bytes(*hash.as_bytes())
}

/// Computes lowercase hex BLAKE3 digest string for metadata indices.
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl CipherVault for AgeVtpmVault {
    fn init(&self) -> CipherResult<VaultStatus> {
        let _ = init_vault_dir(
            &self.vault_dir,
            self.cipher_mode,
            self.key_storage,
            self.sealer.as_ref(),
        )?;
        self.status(None)
    }

    fn status(&self, policy: Option<&SecretPolicy>) -> CipherResult<VaultStatus> {
        let initialized = identity_path(&self.vault_dir).exists();
        if !initialized {
            let missing = policy
                .map(|p| {
                    p.entries
                        .iter()
                        .filter(|e| e.required)
                        .map(|e| e.key.clone())
                        .collect()
                })
                .unwrap_or_default();
            return Ok(VaultStatus {
                vault_path: self.vault_dir.display().to_string(),
                initialized: false,
                recipient_public: None,
                entry_count: 0,
                entries: Vec::new(),
                policy_missing_keys: missing,
            });
        }

        let recipient = load_recipient(&self.vault_dir, self.key_storage, self.sealer.as_ref())?;
        let index = load_index(&self.vault_dir)?;
        let entries: Vec<SecretMetadata> = index.entries.values().cloned().collect();
        let policy_missing_keys = policy
            .map(|p| {
                p.entries
                    .iter()
                    .filter(|e| !index.entries.contains_key(&e.key))
                    .map(|e| e.key.clone())
                    .collect()
            })
            .unwrap_or_default();

        Ok(VaultStatus {
            vault_path: self.vault_dir.display().to_string(),
            initialized: true,
            recipient_public: Some(recipient.encode_public()),
            entry_count: entries.len(),
            entries,
            policy_missing_keys,
        })
    }

    fn get(&self, reference: &CipherRef) -> CipherResult<Secret<Vec<u8>>> {
        let identity = load_identity(&self.vault_dir, self.key_storage, self.sealer.as_ref())?;
        let ciphertext_path = entry_ciphertext_path(&self.vault_dir, reference.key())?;
        if !ciphertext_path.exists() {
            return Err(CipherError::NotFound(reference.key().to_owned()));
        }
        let ciphertext = fs::read(&ciphertext_path)
            .map_err(|e| CipherError::Io(format!("read `{}`: {e}", ciphertext_path.display())))?;

        let mut plaintext = match (&identity, is_hybrid_ciphertext(&ciphertext)) {
            (VaultIdentity::Hybrid(hybrid_id), true) => decrypt_hybrid(hybrid_id, &ciphertext)?,
            (VaultIdentity::Classical(age_id), false) => age::decrypt(age_id, &ciphertext)
                .map_err(|e| CipherError::Crypto(format!("decrypt `{}`: {e}", reference.key())))?,
            _ => {
                return Err(CipherError::Crypto(format!(
                    "ciphertext algorithm mismatch for secret `{}`",
                    reference.key()
                )));
            }
        };

        if let Some(expected_hex) = reference.expected_blake3_hex() {
            let expected = blake3::Hash::from_hex(expected_hex).map_err(|_| {
                CipherError::InvalidKey(format!(
                    "malformed expected digest for `{}`",
                    reference.key()
                ))
            })?;
            let actual = blake3::hash(&plaintext);
            if actual != expected {
                plaintext.zeroize();
                return Err(CipherError::Crypto(format!(
                    "integrity digest mismatch for secret `{}`",
                    reference.key()
                )));
            }
        }

        Ok(Secret::new(plaintext))
    }

    fn set(
        &self,
        reference: &CipherRef,
        plaintext: &Secret<Vec<u8>>,
    ) -> CipherResult<SecretMetadata> {
        let recipient = load_recipient(&self.vault_dir, self.key_storage, self.sealer.as_ref())?;
        let ciphertext_path = entry_ciphertext_path(&self.vault_dir, reference.key())?;
        let bytes = plaintext.expose();
        let ciphertext = match &recipient {
            VaultRecipient::Hybrid(pq_recipient) => encrypt_hybrid(pq_recipient, bytes)?,
            VaultRecipient::Classical(age_recipient) => age::encrypt(age_recipient, bytes)
                .map_err(|e| CipherError::Crypto(format!("encrypt `{}`: {e}", reference.key())))?,
        };
        write_atomic(&ciphertext_path, &ciphertext, 0o600)?;

        let meta = SecretMetadata {
            key: reference.key().to_owned(),
            blake3_hex: blake3_hex(bytes),
            byte_len: bytes.len(),
            updated_unix_secs: unix_now_secs(),
        };
        let meta_clone = meta.clone();
        with_locked_index(&self.vault_dir, |index| {
            index.entries.insert(meta_clone.key.clone(), meta_clone);
            Ok(true)
        })?;
        Ok(meta)
    }

    fn delete(&self, reference: &CipherRef) -> CipherResult<()> {
        let _ = load_identity(&self.vault_dir, self.key_storage, self.sealer.as_ref())?;
        let ciphertext_path = entry_ciphertext_path(&self.vault_dir, reference.key())?;
        if !ciphertext_path.exists() {
            return Err(CipherError::NotFound(reference.key().to_owned()));
        }
        fs::remove_file(&ciphertext_path)
            .map_err(|e| CipherError::Io(format!("remove `{}`: {e}", ciphertext_path.display())))?;
        let key = reference.key().to_owned();
        with_locked_index(&self.vault_dir, |index| {
            index.entries.remove(&key);
            Ok(true)
        })?;
        Ok(())
    }

    fn digest_secret(&self, plaintext: &Secret<Vec<u8>>) -> Digest {
        blake3_digest(plaintext.expose())
    }
}

impl CipherPort for AgeVtpmVault {
    fn sync_policy(
        &self,
        policy: &SecretPolicy,
        transport: &dyn TargetTransport,
        target_filter: Option<&str>,
        confirm_apply: bool,
    ) -> CipherResult<SyncPlan> {
        plan_and_execute_sync_with_sealer(
            self,
            policy,
            transport,
            target_filter,
            confirm_apply,
            self.sealer.as_ref(),
        )
    }

    fn posture(
        &self,
        policy: &SecretPolicy,
        transport: &dyn TargetTransport,
        target: Option<&NodeTarget>,
    ) -> CipherResult<SecretsPosture> {
        policy.validate()?;
        let status = self.status(Some(policy))?;
        let vtpm = Some(transport.probe_vtpm(target)?);
        Ok(SecretsPosture::from_status(
            &status,
            policy.entries.len(),
            vtpm,
        ))
    }
}
