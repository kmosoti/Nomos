//! Atomic filesystem primitives and path safety for the `age` vault directory.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use age::secrecy::ExposeSecret;
use nomos_cipher::{CipherError, CipherResult, SecretMetadata, validate_secret_key};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

const IDENTITY_FILE: &str = "vault.key";
const RECIPIENT_FILE: &str = "recipient.pub";
const INDEX_FILE: &str = "metadata.json";
const ENTRIES_DIR: &str = "entries";

/// On-disk metadata index storing non-secret entry digests and lengths.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VaultMetadataIndex {
    /// Entry metadata keyed by validated logical secret key.
    pub entries: BTreeMap<String, SecretMetadata>,
}

/// Validates and joins the vault entry ciphertext path for `key`.
pub fn entry_ciphertext_path(vault_dir: &Path, key: &str) -> CipherResult<PathBuf> {
    validate_secret_key(key)?;
    Ok(vault_dir.join(ENTRIES_DIR).join(format!("{key}.age")))
}

/// Path to the vault's private X25519 identity file (`0o600`).
pub fn identity_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(IDENTITY_FILE)
}

/// Path to the vault's public X25519 recipient string (`0o644`).
pub fn recipient_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(RECIPIENT_FILE)
}

/// Path to the vault's non-secret metadata index (`0o600`).
pub fn index_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(INDEX_FILE)
}

/// Returns current Unix timestamp in seconds.
pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Atomically writes `bytes` to `dest` via a sibling temporary file with `mode`.
pub fn write_atomic(dest: &Path, bytes: &[u8], mode: u32) -> CipherResult<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| CipherError::Io(format!("path `{}` has no parent", dest.display())))?;
    fs::create_dir_all(parent)
        .map_err(|e| CipherError::Io(format!("mkdir `{}`: {e}", parent.display())))?;

    let tmp_name = format!(
        ".{}.{}.tmp",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("vault"),
        std::process::id()
    );
    let tmp_path = parent.join(tmp_name);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .map_err(|e| CipherError::Io(format!("open temp `{}`: {e}", tmp_path.display())))?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|e| CipherError::Io(format!("chmod `{}`: {e}", tmp_path.display())))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            CipherError::Io(format!("write temp `{}`: {e}", tmp_path.display()))
        })?;
    drop(file);

    fs::rename(&tmp_path, dest).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        CipherError::Io(format!("rename to `{}`: {e}", dest.display()))
    })?;
    Ok(())
}

/// Ensures directory exists with restricted permissions (`0o700`).
pub fn ensure_private_dir(dir: &Path) -> CipherResult<()> {
    fs::create_dir_all(dir)
        .map_err(|e| CipherError::Io(format!("create `{}`: {e}", dir.display())))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| CipherError::Io(format!("chmod `{}`: {e}", dir.display())))?;
    Ok(())
}

/// Initializes an uninitialized vault directory with a fresh `age::x25519::Identity`.
pub fn init_vault_dir(vault_dir: &Path) -> CipherResult<age::x25519::Recipient> {
    ensure_private_dir(vault_dir)?;
    ensure_private_dir(&vault_dir.join(ENTRIES_DIR))?;

    let id_file = identity_path(vault_dir);
    if id_file.exists() {
        return load_recipient(vault_dir);
    }

    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public();
    let encoded_secret = identity.to_string();
    let mut secret_bytes = format!("{}\n", encoded_secret.expose_secret()).into_bytes();
    let write_res = write_atomic(&id_file, &secret_bytes, 0o600);
    secret_bytes.zeroize();
    write_res?;

    write_atomic(
        &recipient_path(vault_dir),
        format!("{recipient}\n").as_bytes(),
        0o644,
    )?;
    if !index_path(vault_dir).exists() {
        save_index(vault_dir, &VaultMetadataIndex::default())?;
    }
    Ok(recipient)
}

/// Loads the private X25519 identity from `vault_dir`, scrubbing temporary plaintext bytes.
pub fn load_identity(vault_dir: &Path) -> CipherResult<age::x25519::Identity> {
    let path = identity_path(vault_dir);
    if !path.exists() {
        return Err(CipherError::NotInitialized(vault_dir.display().to_string()));
    }
    let mut raw = fs::read_to_string(&path)
        .map_err(|e| CipherError::Io(format!("read `{}`: {e}", path.display())))?;
    let trimmed = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| CipherError::Crypto("vault key file contains no identity".into()))?;
    let parsed = trimmed
        .parse::<age::x25519::Identity>()
        .map_err(|e| CipherError::Crypto(format!("invalid age identity: {e}")));
    raw.zeroize();
    parsed
}

/// Loads the public recipient from `vault_dir`.
pub fn load_recipient(vault_dir: &Path) -> CipherResult<age::x25519::Recipient> {
    let path = recipient_path(vault_dir);
    if path.exists() {
        let text = fs::read_to_string(&path)
            .map_err(|e| CipherError::Io(format!("read `{}`: {e}", path.display())))?;
        let trimmed = text.trim();
        return trimmed
            .parse::<age::x25519::Recipient>()
            .map_err(|e| CipherError::Crypto(format!("invalid age recipient: {e}")));
    }
    Ok(load_identity(vault_dir)?.to_public())
}

/// Loads the metadata index from `vault_dir`, returning empty if absent.
pub fn load_index(vault_dir: &Path) -> CipherResult<VaultMetadataIndex> {
    let path = index_path(vault_dir);
    if !path.exists() {
        return Ok(VaultMetadataIndex::default());
    }
    let file = File::open(&path)
        .map_err(|e| CipherError::Io(format!("open `{}`: {e}", path.display())))?;
    serde_json::from_reader(file)
        .map_err(|e| CipherError::Io(format!("parse `{}`: {e}", path.display())))
}

/// Persists the non-secret metadata index atomically with `0o600` permissions.
pub fn save_index(vault_dir: &Path, index: &VaultMetadataIndex) -> CipherResult<()> {
    let json = serde_json::to_vec_pretty(index)
        .map_err(|e| CipherError::Io(format!("serialize metadata index: {e}")))?;
    write_atomic(&index_path(vault_dir), &json, 0o600)
}
