//! # nomos-store-fs
//!
//! **Driven adapter** for the `nomos-store` port, on the Cell's local disk
//! ([ADR 0017](../../../../docs/adr/0017-local-store.md)).
//!
//! [`FsContentStore`] keeps immutable blobs addressed by their SHA-256
//! digest beneath one directory, `content/sha256/<first two hex
//! digits>/<digest>`. A blob is written through a temporary file in its
//! directory, synced, and renamed, and the directory synced, so a crash
//! leaves the blob whole or absent. Every read checks the digest, and a
//! mismatch is reported as corruption, never returned as content. A bundle,
//! a directory of files named by their digests, is imported whole or not at
//! all. Paths beneath the store are opened relative to its directory
//! without following a symbolic link (ADR 0013 §2).
//!
//! [`FileLog`] is the Cell's durable log: one append-only file of
//! checksummed records, a torn final record truncated on open and anything
//! else malformed refused (ADR 0017 §3). [`FileLog::inspect`] reads it
//! without writing, for the commands that must change nothing.
//!
//! [`StateLease`] is mutation authority over the state directory, and every
//! object under it is trusted only if it is private to the Cell's user
//! (cell-commands.md, State Directory).

mod log;
#[cfg(test)]
mod scratch;
mod state;

pub use log::{FileLog, Inspection, LogError, Recovery, Replayed};
pub use state::{StateError, StateLease, inspect as inspect_state};

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;

use nomos_core::resource::Digest;
use nomos_store::{ContentError, ContentStore};
use rustix::fs::{AtFlags, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;

const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS);

fn io(e: impl std::fmt::Display) -> ContentError {
    ContentError::Io(e.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest of `bytes`, as the store addresses them.
pub fn digest_of(bytes: &[u8]) -> Digest {
    Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

/// A content store beneath one directory.
#[derive(Debug)]
pub struct FsContentStore {
    root: OwnedFd,
}

/// Why a bundle was refused. Nothing of it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// An entry is not a regular file named by 64 lowercase hexadecimal
    /// digits.
    Misnamed(String),
    /// An entry's bytes do not have the digest its name says.
    Mismatch(String),
    /// The bundle or the store could not be read or written.
    Io(String),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Misnamed(n) => write!(f, "bundle entry {n:?} is not named by a digest"),
            BundleError::Mismatch(n) => write!(f, "bundle entry {n} does not have its digest"),
            BundleError::Io(e) => write!(f, "bundle: {e}"),
        }
    }
}

impl std::error::Error for BundleError {}

impl FsContentStore {
    /// The store beneath `dir`, which is created, with mode `0700`, if it
    /// does not exist, and refused unless it is private to the Cell's user.
    pub fn open(dir: &Path) -> Result<Self, ContentError> {
        let root = state::open_root(dir, true)
            .map_err(|e| ContentError::Io(e.to_string()))?
            .ok_or_else(|| io("absent after creation"))?;
        Ok(FsContentStore { root })
    }

    fn relative(digest: &Digest) -> (String, String) {
        let h = hex(digest.as_bytes());
        (format!("content/sha256/{}", &h[..2]), h)
    }

    /// The directory for a digest's prefix, created if needed.
    fn directory(&self, dir: &str) -> Result<OwnedFd, ContentError> {
        let mut at = String::new();
        for part in dir.split('/') {
            if !at.is_empty() {
                at.push('/');
            }
            at.push_str(part);
            match rustix::fs::mkdirat(self.root.as_fd(), at.as_str(), Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(e) => return Err(io(e)),
            }
        }
        let fd = rustix::fs::openat2(
            self.root.as_fd(),
            dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            RESOLVE,
        )
        .map_err(io)?;
        state::check(&fd, Path::new(dir), state::Kind::Directory)
            .map_err(|e| ContentError::Io(e.to_string()))?;
        Ok(fd)
    }

    /// Imports every blob of the bundle at `dir`: each entry a regular file
    /// named by the hexadecimal digest of its bytes. Every entry is checked
    /// before anything is stored, so a bundle with one bad entry stores
    /// nothing. Returns how many blobs the bundle held.
    pub fn import(&mut self, dir: &Path) -> Result<usize, BundleError> {
        let mut blobs = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| BundleError::Io(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| BundleError::Io(e.to_string()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let named = name.len() == 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            let kind = entry
                .file_type()
                .map_err(|e| BundleError::Io(e.to_string()))?;
            if !named || !kind.is_file() {
                return Err(BundleError::Misnamed(name));
            }
            let bytes = std::fs::read(entry.path()).map_err(|e| BundleError::Io(e.to_string()))?;
            if hex(digest_of(&bytes).as_bytes()) != name {
                return Err(BundleError::Mismatch(name));
            }
            blobs.push(bytes);
        }
        blobs.sort();
        for bytes in &blobs {
            self.put(bytes)
                .map_err(|e| BundleError::Io(e.to_string()))?;
        }
        Ok(blobs.len())
    }
}

impl ContentStore for FsContentStore {
    fn put(&mut self, bytes: &[u8]) -> Result<Digest, ContentError> {
        let digest = digest_of(bytes);
        if self.get(&digest)?.is_some() {
            return Ok(digest);
        }
        let (dir, name) = Self::relative(&digest);
        let dir = self.directory(&dir)?;
        let temp = format!(".{name}.tmp");
        let _ = rustix::fs::unlinkat(dir.as_fd(), temp.as_str(), AtFlags::empty());
        let fd = rustix::fs::openat(
            dir.as_fd(),
            temp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(0o600),
        )
        .map_err(io)?;
        let mut file = File::from(fd);
        let written = file
            .write_all(bytes)
            .map_err(io)
            .and_then(|()| rustix::fs::fsync(file.as_fd()).map_err(io));
        drop(file);
        let done = written.and_then(|()| {
            rustix::fs::renameat(dir.as_fd(), temp.as_str(), dir.as_fd(), name.as_str()).map_err(io)
        });
        if done.is_err() {
            let _ = rustix::fs::unlinkat(dir.as_fd(), temp.as_str(), AtFlags::empty());
        }
        done?;
        rustix::fs::fsync(dir.as_fd()).map_err(io)?;
        Ok(digest)
    }

    fn get(&self, digest: &Digest) -> Result<Option<Vec<u8>>, ContentError> {
        let (dir, name) = Self::relative(digest);
        let fd = match rustix::fs::openat2(
            self.root.as_fd(),
            format!("{dir}/{name}").as_str(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            RESOLVE,
        ) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        let blob = format!("{dir}/{name}");
        state::check(&fd, Path::new(&blob), state::Kind::File)
            .map_err(|e| ContentError::Io(e.to_string()))?;
        let mut bytes = Vec::new();
        File::from(fd).read_to_end(&mut bytes).map_err(io)?;
        if digest_of(&bytes) == *digest {
            Ok(Some(bytes))
        } else {
            Err(ContentError::Corrupt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        crate::scratch::path("store-fs", name)
    }

    /// A blob put is got back by its digest, and putting it again changes
    /// nothing.
    #[test]
    fn a_blob_is_stored_by_its_digest() {
        let dir = scratch("roundtrip");
        let mut store = FsContentStore::open(&dir).unwrap();
        let d = store.put(b"hello").unwrap();
        assert_eq!(d, digest_of(b"hello"));
        assert_eq!(store.put(b"hello").unwrap(), d);
        assert_eq!(store.get(&d).unwrap(), Some(b"hello".to_vec()));
        assert_eq!(store.get(&digest_of(b"other")).unwrap(), None);
        let h = hex(d.as_bytes());
        assert!(
            dir.join(format!("content/sha256/{}/{h}", &h[..2]))
                .is_file()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A store is private to the Cell's user: made `0700`, with blobs
    /// `0600`, and refused when a directory or blob is open to others, or
    /// is a link (cell-commands.md, Safety).
    #[test]
    fn a_store_open_to_others_or_through_a_link_is_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let mode = |p: &std::path::Path, bits| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(bits)).unwrap()
        };
        let dir = scratch("private");
        let mut store = FsContentStore::open(&dir).unwrap();
        let d = store.put(b"private").unwrap();
        let h = hex(d.as_bytes());
        let blob = dir.join(format!("content/sha256/{}/{h}", &h[..2]));
        let bits = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(bits(&dir), 0o700);
        assert_eq!(bits(&blob), 0o600);
        assert_eq!(bits(blob.parent().unwrap()), 0o700);

        mode(&blob, 0o644);
        assert!(matches!(store.get(&d), Err(ContentError::Io(e)) if e.contains("unsafe state")));
        mode(&blob, 0o600);
        assert_eq!(store.get(&d).unwrap(), Some(b"private".to_vec()));

        // A directory open to others is refused by a store that puts into it.
        mode(blob.parent().unwrap(), 0o755);
        let in_same_dir = (0u32..)
            .map(|i| format!("p{i}").into_bytes())
            .find(|b| hex(digest_of(b).as_bytes())[..2] == h[..2])
            .unwrap();
        assert!(matches!(
            store.put(&in_same_dir),
            Err(ContentError::Io(e)) if e.contains("unsafe state")
        ));
        mode(blob.parent().unwrap(), 0o700);

        // A store opened beneath a root open to others, or a link, is refused.
        mode(&dir, 0o750);
        assert!(FsContentStore::open(&dir).is_err());
        mode(&dir, 0o700);
        let link = scratch("private-link");
        symlink(&dir, &link).unwrap();
        assert!(FsContentStore::open(&link).is_err());
    }

    /// ADR 0017 acceptance: a blob whose bytes changed on disk is reported
    /// as corruption, never returned.
    #[test]
    fn a_changed_blob_is_corruption() {
        let dir = scratch("corrupt");
        let mut store = FsContentStore::open(&dir).unwrap();
        let d = store.put(b"original").unwrap();
        let h = hex(d.as_bytes());
        std::fs::write(
            dir.join(format!("content/sha256/{}/{h}", &h[..2])),
            b"tampered",
        )
        .unwrap();
        assert_eq!(store.get(&d), Err(ContentError::Corrupt));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A blob that is a symbolic link is not followed.
    #[test]
    fn a_linked_blob_is_not_followed() {
        let dir = scratch("link");
        let store = FsContentStore::open(&dir).unwrap();
        let d = digest_of(b"target");
        let h = hex(d.as_bytes());
        let at = dir.join(format!("content/sha256/{}", &h[..2]));
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(dir.join("elsewhere"), b"target").unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), at.join(&h)).unwrap();
        assert!(matches!(store.get(&d), Err(ContentError::Io(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ADR 0017 acceptance: a bundle with a misnamed or mismatched entry is
    /// refused whole; a good one is imported.
    #[test]
    fn a_bundle_is_imported_whole_or_not_at_all() {
        let base = scratch("bundle");
        let bundle = base.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let good = |b: &[u8]| hex(digest_of(b).as_bytes());
        std::fs::write(bundle.join(good(b"a")), b"a").unwrap();
        std::fs::write(bundle.join(good(b"b")), b"not b").unwrap();
        let mut store = FsContentStore::open(&base.join("store")).unwrap();
        assert_eq!(
            store.import(&bundle),
            Err(BundleError::Mismatch(good(b"b")))
        );
        assert_eq!(
            store.get(&digest_of(b"a")).unwrap(),
            None,
            "a partial import"
        );
        std::fs::remove_file(bundle.join(good(b"b"))).unwrap();
        std::fs::write(bundle.join("notes.txt"), b"x").unwrap();
        assert_eq!(
            store.import(&bundle),
            Err(BundleError::Misnamed("notes.txt".into()))
        );
        std::fs::remove_file(bundle.join("notes.txt")).unwrap();
        assert_eq!(store.import(&bundle), Ok(1));
        assert_eq!(store.get(&digest_of(b"a")).unwrap(), Some(b"a".to_vec()));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
