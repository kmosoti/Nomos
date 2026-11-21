//! Vault key initialization, TPM2 sealing policy enforcement, and recipient parsing.

use std::fs;
use std::path::Path;

use age::secrecy::ExposeSecret;
use nomos_cipher::{CipherError, CipherResult};
use zeroize::Zeroize;

use crate::pq_hybrid::{HybridIdentity, HybridRecipient};
use crate::sealer::{CredSealer, VaultCipherMode, VaultKeyStorage};
use crate::vault_io::{
    ENTRIES_DIR, ensure_private_dir, identity_path, index_path, recipient_path, with_locked_index,
    write_atomic,
};

/// Parsed vault decryption identity (hybrid ML-KEM-768+X25519 or classical X25519).
pub enum VaultIdentity {
    /// Post-quantum hybrid `ML-KEM-768 + X25519` identity.
    Hybrid(HybridIdentity),
    /// Classical `age` X25519 identity.
    Classical(age::x25519::Identity),
}

impl VaultIdentity {
    /// Derives the public recipient corresponding to this identity.
    pub fn to_public(&self) -> VaultRecipient {
        match self {
            VaultIdentity::Hybrid(id) => VaultRecipient::Hybrid(id.to_public()),
            VaultIdentity::Classical(id) => VaultRecipient::Classical(id.to_public()),
        }
    }
}

/// Parsed vault public encryption recipient.
#[derive(Clone, Debug)]
pub enum VaultRecipient {
    /// Post-quantum hybrid `ML-KEM-768 + X25519` recipient.
    Hybrid(HybridRecipient),
    /// Classical `age` X25519 recipient.
    Classical(age::x25519::Recipient),
}

impl VaultRecipient {
    /// Formats the recipient string written to `recipient.pub`.
    pub fn encode_public(&self) -> String {
        match self {
            VaultRecipient::Hybrid(r) => r.to_string(),
            VaultRecipient::Classical(r) => r.to_string(),
        }
    }

    /// Parses either a hybrid (`nomos1pq1:...`) or classical `age1...` recipient string.
    pub fn parse_public(text: &str) -> CipherResult<Self> {
        let trimmed = text.trim();
        if trimmed.starts_with("nomos1pq1:") {
            return Ok(VaultRecipient::Hybrid(trimmed.parse()?));
        }
        let classical = trimmed
            .parse::<age::x25519::Recipient>()
            .map_err(|e| CipherError::Crypto(format!("invalid age recipient: {e}")))?;
        Ok(VaultRecipient::Classical(classical))
    }
}

/// Returns `true` if raw `vault.key` bytes appear to be an unsealed plaintext key.
pub fn is_plaintext_vault_key(raw: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(raw) else {
        return false;
    };
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .is_some_and(|line| {
            line.starts_with("AGE-SECRET-KEY-")
                || line.starts_with("NOMOS-HYBRID-MLKEM768-X25519-V1:")
        })
}

fn parse_identity_bytes(raw: &mut Vec<u8>) -> CipherResult<VaultIdentity> {
    let res = (|| {
        let text = std::str::from_utf8(raw).map_err(|_| {
            CipherError::Crypto("decrypted vault identity is not valid UTF-8".into())
        })?;
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .ok_or_else(|| CipherError::Crypto("vault key file contains no identity".into()))?;
        if line.starts_with("NOMOS-HYBRID-MLKEM768-X25519-V1:") {
            Ok(VaultIdentity::Hybrid(HybridIdentity::parse_secret(line)?))
        } else {
            let parsed = line
                .parse::<age::x25519::Identity>()
                .map_err(|e| CipherError::Crypto(format!("invalid age identity: {e}")))?;
            Ok(VaultIdentity::Classical(parsed))
        }
    })();
    raw.zeroize();
    res
}

/// Initializes an uninitialized vault directory using the configured cipher and key storage mode.
pub fn init_vault_dir(
    vault_dir: &Path,
    mode: VaultCipherMode,
    storage: VaultKeyStorage,
    sealer: &dyn CredSealer,
) -> CipherResult<VaultRecipient> {
    ensure_private_dir(vault_dir)?;
    ensure_private_dir(&vault_dir.join(ENTRIES_DIR))?;

    let id_file = identity_path(vault_dir);
    if id_file.exists() {
        let _ = load_identity(vault_dir, storage, sealer)?;
        return load_recipient(vault_dir, storage, sealer);
    }

    let (recipient, mut secret_bytes) = match mode {
        VaultCipherMode::HybridMlKem768X25519 => {
            let identity = HybridIdentity::generate()?;
            let recipient = VaultRecipient::Hybrid(identity.to_public());
            let encoded = format!("{}\n", identity.encode_secret());
            (recipient, encoded.into_bytes())
        }
        VaultCipherMode::ClassicalX25519 => {
            let identity = age::x25519::Identity::generate();
            let recipient = VaultRecipient::Classical(identity.to_public());
            let encoded = identity.to_string();
            let bytes = format!("{}\n", encoded.expose_secret()).into_bytes();
            (recipient, bytes)
        }
    };

    let mut to_write = match storage {
        VaultKeyStorage::Tpm2Sealed => {
            let sealed = sealer.seal_tpm2(&secret_bytes);
            secret_bytes.zeroize();
            sealed?
        }
        VaultKeyStorage::PlaintextDev => {
            let out = secret_bytes.clone();
            secret_bytes.zeroize();
            out
        }
    };
    let write_res = write_atomic(&id_file, &to_write, 0o600);
    to_write.zeroize();
    write_res?;

    write_atomic(
        &recipient_path(vault_dir),
        format!("{}\n", recipient.encode_public()).as_bytes(),
        0o644,
    )?;
    if !index_path(vault_dir).exists() {
        with_locked_index(vault_dir, |_index| Ok(true))?;
    }
    Ok(recipient)
}

/// Loads the private identity from `vault_dir`, enforcing [`VaultKeyStorage`] policy.
pub fn load_identity(
    vault_dir: &Path,
    storage: VaultKeyStorage,
    sealer: &dyn CredSealer,
) -> CipherResult<VaultIdentity> {
    let path = identity_path(vault_dir);
    if !path.exists() {
        return Err(CipherError::NotInitialized(vault_dir.display().to_string()));
    }
    let mut raw =
        fs::read(&path).map_err(|e| CipherError::Io(format!("read `{}`: {e}", path.display())))?;

    match storage {
        VaultKeyStorage::Tpm2Sealed => {
            if is_plaintext_vault_key(&raw) {
                raw.zeroize();
                return Err(CipherError::Crypto(
                    "refusing unsealed plaintext vault.key in Tpm2Sealed production mode; seal with TPM2 or pass --allow-plaintext-key".into(),
                ));
            }
            let unsealed = sealer.unseal_tpm2(&raw);
            raw.zeroize();
            let mut plain = unsealed?;
            parse_identity_bytes(&mut plain)
        }
        VaultKeyStorage::PlaintextDev => {
            if !is_plaintext_vault_key(&raw) {
                raw.zeroize();
                return Err(CipherError::Crypto(
                    "vault.key does not contain a plaintext dev key (may be TPM2-sealed)".into(),
                ));
            }
            parse_identity_bytes(&mut raw)
        }
    }
}

/// Loads the public recipient from `vault_dir`.
pub fn load_recipient(
    vault_dir: &Path,
    storage: VaultKeyStorage,
    sealer: &dyn CredSealer,
) -> CipherResult<VaultRecipient> {
    let path = recipient_path(vault_dir);
    if path.exists() {
        let text = fs::read_to_string(&path)
            .map_err(|e| CipherError::Io(format!("read `{}`: {e}", path.display())))?;
        return VaultRecipient::parse_public(&text);
    }
    Ok(load_identity(vault_dir, storage, sealer)?.to_public())
}
