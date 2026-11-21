//! Atomic filesystem primitives, CSPRNG temp naming, and `flock` index locking.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nomos_cipher::{CipherError, CipherResult, SecretMetadata, validate_secret_key};
use ring::rand::{SecureRandom, SystemRandom};
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};

pub(crate) const IDENTITY_FILE: &str = "vault.key";
pub(crate) const RECIPIENT_FILE: &str = "recipient.pub";
const INDEX_FILE: &str = "metadata.json";
const INDEX_LOCK_FILE: &str = ".metadata.lock";
pub(crate) const ENTRIES_DIR: &str = "entries";

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

/// Path to the vault's private identity file (`0o600`).
pub fn identity_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(IDENTITY_FILE)
}

/// Path to the vault's public recipient string (`0o644`).
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

/// Generates a 16-byte CSPRNG lowercase hex suffix for race-free temp files.
pub fn csprng_hex_suffix() -> CipherResult<String> {
    let mut raw = [0u8; 16];
    SystemRandom::new()
        .fill(&mut raw)
        .map_err(|_| CipherError::Crypto("system CSPRNG failure".into()))?;
    Ok(hex::encode(raw))
}

/// Atomically writes `bytes` to `dest` via a CSPRNG-named sibling temp file with `mode`.
pub fn write_atomic(dest: &Path, bytes: &[u8], mode: u32) -> CipherResult<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| CipherError::Io(format!("path `{}` has no parent", dest.display())))?;
    fs::create_dir_all(parent)
        .map_err(|e| CipherError::Io(format!("mkdir `{}`: {e}", parent.display())))?;

    let suffix = csprng_hex_suffix()?;
    let tmp_name = format!(
        ".{}.{}.{}.tmp",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("vault"),
        std::process::id(),
        suffix,
    );
    let tmp_path = parent.join(tmp_name);

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
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

fn acquire_index_lock(vault_dir: &Path) -> CipherResult<File> {
    ensure_private_dir(vault_dir)?;
    let lock_path = vault_dir.join(INDEX_LOCK_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .map_err(|e| CipherError::Io(format!("open index lock `{}`: {e}", lock_path.display())))?;
    flock(&file, FlockOperation::LockExclusive)
        .map_err(|e| CipherError::Io(format!("flock `{}`: {e}", lock_path.display())))?;
    Ok(file)
}

fn load_index_unlocked(vault_dir: &Path) -> CipherResult<VaultMetadataIndex> {
    let path = index_path(vault_dir);
    if !path.exists() {
        return Ok(VaultMetadataIndex::default());
    }
    let file = File::open(&path)
        .map_err(|e| CipherError::Io(format!("open `{}`: {e}", path.display())))?;
    serde_json::from_reader(file)
        .map_err(|e| CipherError::Io(format!("parse `{}`: {e}", path.display())))
}

/// Loads the metadata index from `vault_dir` under an exclusive `flock`.
pub fn load_index(vault_dir: &Path) -> CipherResult<VaultMetadataIndex> {
    let _lock = acquire_index_lock(vault_dir)?;
    load_index_unlocked(vault_dir)
}

/// Persists the non-secret metadata index atomically under `flock` with `0o600` permissions.
pub fn save_index(vault_dir: &Path, index: &VaultMetadataIndex) -> CipherResult<()> {
    let _lock = acquire_index_lock(vault_dir)?;
    let json = serde_json::to_vec_pretty(index)
        .map_err(|e| CipherError::Io(format!("serialize metadata index: {e}")))?;
    write_atomic(&index_path(vault_dir), &json, 0o600)
}

/// Runs an atomic read-modify-write closure on `metadata.json` guarded by `flock`.
pub fn with_locked_index<F>(vault_dir: &Path, mutate: F) -> CipherResult<()>
where
    F: FnOnce(&mut VaultMetadataIndex) -> CipherResult<bool>,
{
    let _lock = acquire_index_lock(vault_dir)?;
    let mut index = load_index_unlocked(vault_dir)?;
    if mutate(&mut index)? {
        let json = serde_json::to_vec_pretty(&index)
            .map_err(|e| CipherError::Io(format!("serialize metadata index: {e}")))?;
        write_atomic(&index_path(vault_dir), &json, 0o600)?;
    }
    Ok(())
}
