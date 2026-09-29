//! # nomos-substrate-linux
//!
//! **Driven adapter** for the `nomos-substrate` port. Linux, Debian reference.
//!
//! The first operation ([substrate-contract.md]): regular files beneath one
//! root directory, observed and replaced. Every path is resolved at use,
//! relative to an open descriptor of the root, with `openat2` and
//! `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, so no
//! symbolic link is followed and nothing resolves outside the root (ADR 0013
//! §2). Replacement is atomic: a temporary file in the target's directory,
//! written, synced, renamed over the target, and the directory synced (spec
//! §11). Native system calls only; no shell (AGENTS.md rule 5).
//!
//! Later resources: `directory`, `system_user`, `package`, `systemd_unit`
//! (via D-Bus), `sysctl`.
//!
//! [substrate-contract.md]: ../../../../docs/formal/substrate-contract.md

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use nomos_canon::sha256::Sha256;
use nomos_core::condition::{Content, FileCondition};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::{
    Collection, CollectionFailure, CollectorId, FileEvidence, Instant, Observation, Provenance,
    Window,
};
use nomos_core::resource::{Digest, ResourcePath};
use nomos_substrate::{Mutate, Observe};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;
use rustix::time::{ClockId, clock_gettime};

/// The resolve flags of every path resolution (ADR 0013 §2).
const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS);

/// The mode a replaced file is given.
const FILE_MODE: u32 = 0o644;

/// The Linux Substrate for regular files beneath `root`.
#[derive(Debug)]
pub struct LinuxHost {
    root: OwnedFd,
    collector: CollectorId,
    content: BTreeMap<Digest, Vec<u8>>,
    ledger: BTreeMap<EffectKey, Vec<Receipt>>,
    executions: usize,
}

/// Why a Linux host could not be opened.
#[derive(Debug)]
pub struct OpenError(pub std::io::Error);

impl LinuxHost {
    /// A host rooted at the directory `root`, which is `/` in production.
    pub fn open(root: &Path) -> Result<Self, OpenError> {
        let dir = rustix::fs::open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| OpenError(e.into()))?;
        let collector = CollectorId::new("linux")
            .ok_or_else(|| OpenError(std::io::Error::other("the collector name is empty")))?;
        Ok(LinuxHost {
            root: dir,
            collector,
            content: BTreeMap::new(),
            ledger: BTreeMap::new(),
            executions: 0,
        })
    }

    /// Adds `bytes` to the content store and returns their digest, which an
    /// exact requirement names.
    pub fn add_content(&mut self, bytes: Vec<u8>) -> Digest {
        let digest = digest_of(&bytes);
        self.content.insert(digest, bytes);
        digest
    }

    /// How many executions the host has started: requests neither refused
    /// nor answered from the ledger.
    pub fn executions(&self) -> usize {
        self.executions
    }

    fn collect(&self, path: &ResourcePath) -> Collection {
        let file = match rustix::fs::openat2(
            self.root.as_fd(),
            relative(path),
            // Nonblocking, so that opening a named pipe at the resource
            // returns at once instead of waiting for a writer.
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NOCTTY | OFlags::NONBLOCK,
            Mode::empty(),
            RESOLVE,
        ) {
            Ok(fd) => File::from(fd),
            Err(e) => return failure(e),
        };
        match read_regular(file) {
            Ok(Some((digest, size))) => {
                Collection::Collected(FileEvidence::Present { digest, size })
            }
            Ok(None) => Collection::Failed(CollectionFailure::Unsupported),
            Err(e) => failure(e),
        }
    }

    fn provenance(&self, start: Instant, end: Instant) -> Provenance {
        let window =
            Window::new(start, end.max(start)).expect("a window ending at or after its start");
        Provenance::new(self.collector.clone(), window)
    }

    fn execute(&self, request: &Apply) -> Receipt {
        let Operation::Replace(requirement) = &request.operation else {
            return Receipt::Refused;
        };
        let path = request.key.resource();
        let (parent, name) = match self.parent(path) {
            Ok(split) => split,
            Err(_) => return Receipt::Refused,
        };
        let before = match current(parent.as_fd(), &name) {
            Ok(before) => before,
            Err(_) => return Receipt::Refused,
        };
        match requirement {
            FileCondition::Absent => match before {
                None => Receipt::Completed { changed: false },
                Some(_) => {
                    match rustix::fs::unlinkat(parent.as_fd(), name.as_str(), AtFlags::empty()) {
                        Ok(()) => sync(parent.as_fd(), Receipt::Completed { changed: true }),
                        Err(_) => Receipt::Failed,
                    }
                }
            },
            FileCondition::Present {
                content: Content::Any,
            } => match before {
                Some(_) => Receipt::Completed { changed: false },
                None => write_atomically(parent.as_fd(), &name, &[], &request.key),
            },
            FileCondition::Present {
                content: Content::Exactly(digest),
            } => {
                let Some(bytes) = self.content.get(digest) else {
                    return Receipt::Refused;
                };
                if before == Some(*digest) {
                    return Receipt::Completed { changed: false };
                }
                write_atomically(parent.as_fd(), &name, bytes, &request.key)
            }
        }
    }

    /// The resource's parent directory, opened beneath the root, and its
    /// last component.
    fn parent(&self, path: &ResourcePath) -> Result<(OwnedFd, String), Errno> {
        let rel = relative(path);
        let (dir, name) = match rel.rsplit_once('/') {
            Some((dir, name)) => (dir, name),
            None => (".", rel),
        };
        let fd = rustix::fs::openat2(
            self.root.as_fd(),
            dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            RESOLVE,
        )?;
        Ok((fd, name.to_string()))
    }
}

/// The digest of `bytes`, as the adapter reports it.
pub fn digest_of(bytes: &[u8]) -> Digest {
    Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

fn relative(path: &ResourcePath) -> &str {
    path.as_str().trim_start_matches('/')
}

fn now() -> Instant {
    let t = clock_gettime(ClockId::Monotonic);
    let nanos = u64::try_from(t.tv_sec).unwrap_or(0) * 1_000_000_000
        + u64::try_from(t.tv_nsec).unwrap_or(0);
    Instant(nanos)
}

/// The collection a failed open or read maps to (substrate-contract.md).
fn failure(e: Errno) -> Collection {
    match e {
        Errno::NOENT | Errno::NOTDIR => Collection::Collected(FileEvidence::Absent),
        Errno::ACCESS | Errno::PERM => Collection::Failed(CollectionFailure::PermissionDenied),
        Errno::LOOP | Errno::XDEV => Collection::Failed(CollectionFailure::Unsupported),
        _ => Collection::Failed(CollectionFailure::Io),
    }
}

/// The digest and size of an open file, or `None` if it is not a regular
/// file.
fn read_regular(mut file: File) -> Result<Option<(Digest, u64)>, Errno> {
    let stat = rustix::fs::fstat(file.as_fd())?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Ok(None);
    }
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(errno)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        size += n as u64;
    }
    Ok(Some((Digest::from_bytes(hash.finish()), size)))
}

fn errno(e: std::io::Error) -> Errno {
    e.raw_os_error().map_or(Errno::IO, Errno::from_raw_os_error)
}

/// The digest of the regular file `name` in `dir`, `None` if nothing is
/// there, or an error if something other than a regular file is there.
fn current(dir: BorrowedFd<'_>, name: &str) -> Result<Option<Digest>, Errno> {
    match rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NOCTTY | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => match read_regular(File::from(fd))? {
            Some((digest, _)) => Ok(Some(digest)),
            None => Err(Errno::ISDIR),
        },
        Err(Errno::NOENT) => Ok(None),
        Err(e) => Err(e),
    }
}

fn sync(dir: BorrowedFd<'_>, receipt: Receipt) -> Receipt {
    match rustix::fs::fsync(dir) {
        Ok(()) => receipt,
        Err(_) => Receipt::Failed,
    }
}

/// Writes `bytes` to `name` in `dir` through a temporary file and a rename.
fn write_atomically(dir: BorrowedFd<'_>, name: &str, bytes: &[u8], key: &EffectKey) -> Receipt {
    let mut h = Sha256::new();
    h.update(format!("{key:?}").as_bytes());
    let tag: String = h.finish()[..8].iter().map(|b| format!("{b:02x}")).collect();
    let temp = format!(".{name}.nomos-{tag}.tmp");
    let fd = match rustix::fs::openat(
        dir,
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(FILE_MODE),
    ) {
        Ok(fd) => fd,
        Err(_) => return Receipt::Failed,
    };
    let mut file = File::from(fd);
    let written = file
        .write_all(bytes)
        .map_err(errno)
        .and_then(|()| rustix::fs::fchmod(file.as_fd(), Mode::from_raw_mode(FILE_MODE)))
        .and_then(|()| rustix::fs::fsync(file.as_fd()));
    drop(file);
    let renamed = written.and_then(|()| rustix::fs::renameat(dir, temp.as_str(), dir, name));
    match renamed {
        Ok(()) => sync(dir, Receipt::Completed { changed: true }),
        Err(_) => {
            let _ = rustix::fs::unlinkat(dir, temp.as_str(), AtFlags::empty());
            Receipt::Failed
        }
    }
}

impl Observe for LinuxHost {
    fn observe(&mut self, resources: &[ResourcePath]) -> Vec<Observation> {
        resources
            .iter()
            .map(|path| {
                let start = now();
                let collection = self.collect(path);
                // The monotonic clock does not go backwards; `max` makes the
                // window valid by construction rather than by that promise.
                let end = now().max(start);
                Observation::file(path.clone(), collection, self.provenance(start, end))
            })
            .collect()
    }
}

impl Mutate for LinuxHost {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        if let Some(receipts) = self.ledger.get(&request.key) {
            return receipts.clone();
        }
        let receipts = match self.execute(request) {
            Receipt::Refused => vec![Receipt::Refused],
            done => {
                self.executions += 1;
                vec![Receipt::Accepted, Receipt::Started, done]
            }
        };
        self.ledger.insert(request.key.clone(), receipts.clone());
        receipts
    }
}
