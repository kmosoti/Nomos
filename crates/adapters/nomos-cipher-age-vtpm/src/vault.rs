//! Core `age` X25519 vault implementation backed by local disk encryption.

use std::fs;
use std::path::{Path, PathBuf};

use nomos_cipher::{
    CipherError, CipherPort, CipherRef, CipherResult, CipherVault, NodeTarget, SecretMetadata,
    SecretPolicy, SecretsPosture, SyncPlan, TargetTransport, VaultStatus,
};
use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroize;

use crate::sync_plan::plan_and_execute_sync;
use crate::vault_io::{
    entry_ciphertext_path, identity_path, init_vault_dir, load_identity, load_index,
    load_recipient, save_index, unix_now_secs, write_atomic,
};

/// Encrypted `age` X25519 vault adapter storing per-entry ciphertexts on disk.
#[derive(Clone, Debug)]
pub struct AgeVtpmVault {
    vault_dir: PathBuf,
}

impl AgeVtpmVault {
    /// Creates a handle targeting `vault_dir` without creating files on disk yet.
    pub fn new(vault_dir: impl AsRef<Path>) -> Self {
        Self {
            vault_dir: vault_dir.as_ref().to_path_buf(),
        }
    }

    /// Configured vault directory path.
    pub fn vault_dir(&self) -> &Path {
        &self.vault_dir
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
        let _ = init_vault_dir(&self.vault_dir)?;
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

        let recipient = load_recipient(&self.vault_dir)?;
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
            recipient_public: Some(recipient.to_string()),
            entry_count: entries.len(),
            entries,
            policy_missing_keys,
        })
    }

    fn get(&self, reference: &CipherRef) -> CipherResult<Secret<Vec<u8>>> {
        let identity = load_identity(&self.vault_dir)?;
        let ciphertext_path = entry_ciphertext_path(&self.vault_dir, reference.key())?;
        if !ciphertext_path.exists() {
            return Err(CipherError::NotFound(reference.key().to_owned()));
        }
        let ciphertext = fs::read(&ciphertext_path)
            .map_err(|e| CipherError::Io(format!("read `{}`: {e}", ciphertext_path.display())))?;
        let mut plaintext = age::decrypt(&identity, &ciphertext)
            .map_err(|e| CipherError::Crypto(format!("decrypt `{}`: {e}", reference.key())))?;

        if let Some(expected_hex) = reference.expected_blake3_hex() {
            let expected = blake3::Hash::from_hex(expected_hex).map_err(|_| {
                CipherError::InvalidKey(format!(
                    "malformed expected digest for `{}`",
                    reference.key()
                ))
            })?;
            let actual = blake3::hash(&plaintext);
            // `blake3::Hash::eq` uses `constant_time_eq`.
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
        let recipient = load_recipient(&self.vault_dir)?;
        let ciphertext_path = entry_ciphertext_path(&self.vault_dir, reference.key())?;
        let bytes = plaintext.expose();
        let ciphertext = age::encrypt(&recipient, bytes)
            .map_err(|e| CipherError::Crypto(format!("encrypt `{}`: {e}", reference.key())))?;
        write_atomic(&ciphertext_path, &ciphertext, 0o600)?;

        let meta = SecretMetadata {
            key: reference.key().to_owned(),
            blake3_hex: blake3_hex(bytes),
            byte_len: bytes.len(),
            updated_unix_secs: unix_now_secs(),
        };
        let mut index = load_index(&self.vault_dir)?;
        index.entries.insert(meta.key.clone(), meta.clone());
        save_index(&self.vault_dir, &index)?;
        Ok(meta)
    }

    fn delete(&self, reference: &CipherRef) -> CipherResult<()> {
        let _ = load_identity(&self.vault_dir)?;
        let ciphertext_path = entry_ciphertext_path(&self.vault_dir, reference.key())?;
        if !ciphertext_path.exists() {
            return Err(CipherError::NotFound(reference.key().to_owned()));
        }
        fs::remove_file(&ciphertext_path)
            .map_err(|e| CipherError::Io(format!("remove `{}`: {e}", ciphertext_path.display())))?;
        let mut index = load_index(&self.vault_dir)?;
        index.entries.remove(reference.key());
        save_index(&self.vault_dir, &index)?;
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
        plan_and_execute_sync(self, policy, transport, target_filter, confirm_apply)
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
