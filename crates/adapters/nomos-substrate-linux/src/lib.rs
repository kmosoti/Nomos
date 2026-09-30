//! # nomos-substrate-linux
//!
//! **Driven adapter** for the `nomos-substrate` port. Linux, Debian reference.
//!
//! Files and directories beneath one root directory ([substrate-contract.md],
//! The Linux Adapter's First Operation and Files and Directories on Linux):
//! observed, replaced, created, and removed, with their owner, group, and
//! mode. Every path is resolved at use,
//! relative to an open descriptor of the root, with `openat2` and
//! `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, so no
//! symbolic link is followed and nothing resolves outside the root (ADR 0013
//! §2). Replacement is atomic: a temporary file in the target's directory,
//! written, synced, renamed over the target, and the directory synced (spec
//! §11). Native system calls only; no shell (AGENTS.md rule 5).
//!
//! It serves the `file` and `directory` families, and the `unit` family
//! when it is given a connection to systemd ([`units`], Units on Linux); a
//! key of another family is observed as a failed collection
//! (`Unsupported`), and an operation on one is refused. Owners and groups
//! are resolved from the user and group databases beneath the root
//! ([`accounts`]). Content comes from a [`ContentSource`], or from bytes
//! added directly. Later families: `sysctl`, `user`, `package`.
//!
//! [substrate-contract.md]: ../../../../docs/formal/substrate-contract.md

pub mod accounts;
pub mod units;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use nomos_canon::sha256::Sha256;
use nomos_core::condition::{Content, DirectoryCondition, FileCondition, Metadata, Requirement};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::{
    Account, Collection, CollectionFailure, CollectorId, DirectoryEvidence, Evidence, FileEvidence,
    Instant, Observation, ObservedMetadata, Provenance, Window,
};
use nomos_core::resource::{Digest, ResourceKey, ResourcePath};
use nomos_substrate::{ContentSource, Mutate, Observe};
use rustix::fs::{AtFlags, FileType, Gid, Mode, OFlags, ResolveFlags, Stat, Uid};
use rustix::io::Errno;
use rustix::time::{ClockId, clock_gettime};

use accounts::Accounts;
use units::Systemd;

/// The resolve flags of every path resolution (ADR 0013 §2).
const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS);

/// The mode a new file gets when its requirement states none.
const FILE_MODE: u16 = 0o644;
/// The mode a new directory gets when its requirement states none.
const DIRECTORY_MODE: u16 = 0o755;

/// The Linux Substrate for files and directories beneath `root`.
pub struct LinuxHost {
    root: OwnedFd,
    collector: CollectorId,
    content: BTreeMap<Digest, Vec<u8>>,
    source: Option<Box<dyn ContentSource>>,
    systemd: Option<Systemd>,
    ledger: BTreeMap<EffectKey, Vec<Receipt>>,
    executions: usize,
}

impl std::fmt::Debug for LinuxHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxHost")
            .field("executions", &self.executions)
            .finish_non_exhaustive()
    }
}

/// Why a Linux host could not be opened.
#[derive(Debug)]
pub struct OpenError(pub std::io::Error);

/// What is at a resource's path, examined without following a link.
enum Leaf {
    /// Nothing, or a parent that is missing or not a directory.
    None,
    /// A regular file; its status is read again through a descriptor.
    File,
    /// A directory.
    Directory(Stat),
    /// A symbolic link, a device, a socket, or a pipe.
    Other,
}

/// The owner, group, and mode an operation must leave, as numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    uid: Option<u32>,
    gid: Option<u32>,
    mode: u16,
}

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
            source: None,
            systemd: None,
            ledger: BTreeMap::new(),
            executions: 0,
        })
    }

    /// The same host, reading content it was not given directly from
    /// `source`.
    pub fn with_source(mut self, source: Box<dyn ContentSource>) -> Self {
        self.source = Some(source);
        self
    }

    /// The same host, serving units through `systemd`.
    pub fn with_systemd(mut self, systemd: Systemd) -> Self {
        self.systemd = Some(systemd);
        self
    }

    /// Adds `bytes` to the host's own content and returns their digest,
    /// which an exact requirement names.
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

    /// The bytes named by `digest`, checked against it.
    fn bytes(&self, digest: &Digest) -> Option<Vec<u8>> {
        let bytes = match self.content.get(digest) {
            Some(b) => b.clone(),
            None => self.source.as_ref()?.bytes(digest)?,
        };
        (digest_of(&bytes) == *digest).then_some(bytes)
    }

    fn accounts(&self) -> Result<Accounts, Errno> {
        Accounts::load(self.root.as_fd())
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

    fn collect(&self, key: &ResourceKey) -> Collection {
        let path = match (key, &self.systemd) {
            (ResourceKey::File(p) | ResourceKey::Directory(p), _) => p,
            (ResourceKey::Unit(name), Some(systemd)) => {
                return match systemd.examine(name) {
                    Collection::Collected(e) => Collection::Collected(Evidence::Unit(e)),
                    Collection::Failed(f) => Collection::Failed(f),
                };
            }
            _ => return Collection::Failed(CollectionFailure::Unsupported),
        };
        let absent = || match key {
            ResourceKey::Directory(_) => Evidence::Directory(DirectoryEvidence::Absent),
            _ => Evidence::File(FileEvidence::Absent),
        };
        let (parent, name) = match self.parent(path) {
            Ok(split) => split,
            Err(Errno::NOENT | Errno::NOTDIR) => return Collection::Collected(absent()),
            Err(e) => return failure(e),
        };
        let leaf = match examine(parent.as_fd(), &name) {
            Ok(leaf) => leaf,
            Err(e) => return failure(e),
        };
        let accounts = match self.accounts() {
            Ok(a) => a,
            Err(e) => return failure(e),
        };
        match (key, leaf) {
            (_, Leaf::None) => Collection::Collected(absent()),
            (ResourceKey::File(_), Leaf::File) => {
                match open_leaf(parent.as_fd(), &name).and_then(read_regular) {
                    Ok(Some((digest, size, stat))) => {
                        Collection::Collected(Evidence::File(FileEvidence::Present {
                            digest,
                            size,
                            metadata: observed(&accounts, &stat),
                        }))
                    }
                    Ok(None) => Collection::Failed(CollectionFailure::Unsupported),
                    Err(e) => failure(e),
                }
            }
            (ResourceKey::Directory(_), Leaf::Directory(stat)) => {
                Collection::Collected(Evidence::Directory(DirectoryEvidence::Present {
                    metadata: observed(&accounts, &stat),
                }))
            }
            _ => Collection::Failed(CollectionFailure::Unsupported),
        }
    }

    fn provenance(&self, start: Instant, end: Instant) -> Provenance {
        let window =
            Window::new(start, end.max(start)).expect("a window ending at or after its start");
        Provenance::new(self.collector.clone(), window)
    }

    /// The numeric owner, group, and mode `metadata` asks for, keeping an
    /// unstated field from `before`, or the default for a new resource;
    /// `None` when it names an account the databases do not have.
    fn target(
        accounts: &Accounts,
        metadata: &Metadata,
        before: Option<&Stat>,
        default_mode: u16,
    ) -> Option<Target> {
        let uid = match &metadata.owner {
            Some(name) => Some(accounts.uid(name.as_str())?),
            None => before.map(|s| s.st_uid),
        };
        let gid = match &metadata.group {
            Some(name) => Some(accounts.gid(name.as_str())?),
            None => before.map(|s| s.st_gid),
        };
        let mode = match metadata.mode {
            Some(m) => m.bits(),
            None => before.map_or(default_mode, mode_of),
        };
        Some(Target { uid, gid, mode })
    }

    fn execute(&self, request: &Apply) -> Receipt {
        let Operation::Converge(requirement) = &request.operation else {
            return Receipt::Refused;
        };
        let (path, requirement) = match (request.key.resource(), requirement) {
            (ResourceKey::File(p), Requirement::File(r)) => (p, Want::File(r)),
            (ResourceKey::Directory(p), Requirement::Directory(r)) => (p, Want::Directory(r)),
            _ => return Receipt::Refused,
        };
        let Ok(accounts) = self.accounts() else {
            return Receipt::Refused;
        };
        let Ok((parent, name)) = self.parent(path) else {
            return Receipt::Refused;
        };
        let Ok(leaf) = examine(parent.as_fd(), &name) else {
            return Receipt::Refused;
        };
        let dir = parent.as_fd();
        match requirement {
            Want::File(FileCondition::Absent) => match leaf {
                Leaf::None => Receipt::Completed { changed: false },
                Leaf::File => match rustix::fs::unlinkat(dir, name.as_str(), AtFlags::empty()) {
                    Ok(()) => sync(dir, Receipt::Completed { changed: true }),
                    Err(_) => Receipt::Failed,
                },
                Leaf::Directory(_) | Leaf::Other => Receipt::Refused,
            },
            Want::File(FileCondition::Present { content, metadata }) => {
                let before = match leaf {
                    Leaf::None => None,
                    Leaf::File => match open_leaf(dir, &name).and_then(read_regular) {
                        Ok(Some((digest, _, stat))) => Some((digest, stat)),
                        _ => return Receipt::Refused,
                    },
                    Leaf::Directory(_) | Leaf::Other => return Receipt::Refused,
                };
                let Some(target) = Self::target(
                    &accounts,
                    metadata,
                    before.as_ref().map(|b| &b.1),
                    FILE_MODE,
                ) else {
                    return Receipt::Refused;
                };
                let write = match (content, &before) {
                    (Content::Exactly(d), Some((digest, _))) if d == digest => None,
                    (Content::Exactly(d), _) => match self.bytes(d) {
                        Some(bytes) => Some(bytes),
                        None => return Receipt::Refused,
                    },
                    (Content::Any, Some(_)) => None,
                    (Content::Any, None) => Some(Vec::new()),
                };
                match (write, before) {
                    (Some(bytes), _) => write_atomically(dir, &name, &bytes, &request.key, target),
                    (None, Some((_, stat))) => {
                        if Self::holds(&stat, target) {
                            Receipt::Completed { changed: false }
                        } else {
                            set_metadata(open_leaf(dir, &name), target)
                        }
                    }
                    (None, None) => Receipt::Failed,
                }
            }
            Want::Directory(DirectoryCondition::Absent) => match leaf {
                Leaf::None => Receipt::Completed { changed: false },
                Leaf::Directory(_) => match has_entries(dir, &name) {
                    Ok(false) => {
                        match rustix::fs::unlinkat(dir, name.as_str(), AtFlags::REMOVEDIR) {
                            Ok(()) => sync(dir, Receipt::Completed { changed: true }),
                            Err(_) => Receipt::Failed,
                        }
                    }
                    Ok(true) | Err(_) => Receipt::Refused,
                },
                Leaf::File | Leaf::Other => Receipt::Refused,
            },
            Want::Directory(DirectoryCondition::Present { metadata }) => {
                let before = match leaf {
                    Leaf::None => None,
                    Leaf::Directory(stat) => Some(stat),
                    Leaf::File | Leaf::Other => return Receipt::Refused,
                };
                let Some(target) =
                    Self::target(&accounts, metadata, before.as_ref(), DIRECTORY_MODE)
                else {
                    return Receipt::Refused;
                };
                match before {
                    Some(stat) if Self::holds(&stat, target) => {
                        Receipt::Completed { changed: false }
                    }
                    Some(_) => set_metadata(open_directory(dir, &name), target),
                    None => {
                        if rustix::fs::mkdirat(dir, name.as_str(), Mode::from_raw_mode(0o700))
                            .is_err()
                        {
                            return Receipt::Failed;
                        }
                        match set_metadata(open_directory(dir, &name), target) {
                            Receipt::Completed { .. } => {
                                sync(dir, Receipt::Completed { changed: true })
                            }
                            other => other,
                        }
                    }
                }
            }
        }
    }

    /// Whether `stat` already has the owner, group, and mode of `target`.
    fn holds(stat: &Stat, target: Target) -> bool {
        target.uid.is_none_or(|u| u == stat.st_uid)
            && target.gid.is_none_or(|g| g == stat.st_gid)
            && target.mode == mode_of(stat)
    }
}

/// A requirement the adapter serves, with its family.
enum Want<'a> {
    File(&'a FileCondition),
    Directory(&'a DirectoryCondition),
}

fn mode_of(stat: &Stat) -> u16 {
    u16::try_from(stat.st_mode & 0o7777).unwrap_or(0)
}

/// Observed metadata from `stat`, an ID named when the databases name it.
fn observed(accounts: &Accounts, stat: &Stat) -> ObservedMetadata {
    let name = |n: Option<&str>, id: u32| {
        n.and_then(|n| nomos_core::resource::AccountName::new(n).ok())
            .map_or(Account::Id(id), Account::Named)
    };
    ObservedMetadata {
        owner: name(accounts.user_name(stat.st_uid), stat.st_uid),
        group: name(accounts.group_name(stat.st_gid), stat.st_gid),
        mode: nomos_core::resource::Mode::new(mode_of(stat))
            .unwrap_or(nomos_core::resource::Mode::DEFAULT_FILE),
    }
}

/// What is at `name` in `dir`, without following a link.
fn examine(dir: BorrowedFd<'_>, name: &str) -> Result<Leaf, Errno> {
    match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile => Leaf::File,
            FileType::Directory => Leaf::Directory(stat),
            _ => Leaf::Other,
        }),
        Err(Errno::NOENT | Errno::NOTDIR) => Ok(Leaf::None),
        Err(e) => Err(e),
    }
}

/// The regular file `name` in `dir`, opened without following a link.
/// Nonblocking, so that a named pipe put there after the examination
/// returns at once instead of waiting for a writer.
fn open_leaf(dir: BorrowedFd<'_>, name: &str) -> Result<File, Errno> {
    rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NOCTTY | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
}

/// The directory `name` in `dir`, opened without following a link.
fn open_directory(dir: BorrowedFd<'_>, name: &str) -> Result<File, Errno> {
    rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
}

/// Whether the directory `name` in `dir` has any entry.
fn has_entries(dir: BorrowedFd<'_>, name: &str) -> Result<bool, Errno> {
    let fd = open_directory(dir, name)?;
    for entry in rustix::fs::Dir::read_from(fd.as_fd())? {
        let entry = entry?;
        let n = entry.file_name().to_bytes();
        if n != b"." && n != b".." {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Sets the owner, group, and mode of the open resource, then syncs it.
fn set_metadata(opened: Result<File, Errno>, target: Target) -> Receipt {
    let Ok(file) = opened else {
        return Receipt::Failed;
    };
    match apply_metadata(file.as_fd(), target).and_then(|()| rustix::fs::fsync(file.as_fd())) {
        Ok(()) => Receipt::Completed { changed: true },
        Err(_) => Receipt::Failed,
    }
}

fn apply_metadata(fd: BorrowedFd<'_>, target: Target) -> Result<(), Errno> {
    if target.uid.is_some() || target.gid.is_some() {
        rustix::fs::fchown(
            fd,
            target.uid.map(Uid::from_raw),
            target.gid.map(Gid::from_raw),
        )?;
    }
    rustix::fs::fchmod(fd, Mode::from_raw_mode(u32::from(target.mode)))
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
        Errno::NOENT | Errno::NOTDIR => Collection::Collected(Evidence::File(FileEvidence::Absent)),
        Errno::ACCESS | Errno::PERM => Collection::Failed(CollectionFailure::PermissionDenied),
        Errno::LOOP | Errno::XDEV => Collection::Failed(CollectionFailure::Unsupported),
        _ => Collection::Failed(CollectionFailure::Io),
    }
}

/// The digest, size, and metadata of an open file, or `None` if it is not
/// a regular file. Owner and group are reported by number; naming them is
/// `09-file-and-directory`'s.
fn read_regular(mut file: File) -> Result<Option<(Digest, u64, Stat)>, Errno> {
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
    Ok(Some((Digest::from_bytes(hash.finish()), size, stat)))
}

fn errno(e: std::io::Error) -> Errno {
    e.raw_os_error().map_or(Errno::IO, Errno::from_raw_os_error)
}

fn sync(dir: BorrowedFd<'_>, receipt: Receipt) -> Receipt {
    match rustix::fs::fsync(dir) {
        Ok(()) => receipt,
        Err(_) => Receipt::Failed,
    }
}

/// The name of the temporary file an execution writes before renaming it
/// over `name`: in the same directory, and named for the execution.
pub fn temporary_name(name: &str, key: &EffectKey) -> String {
    let mut h = Sha256::new();
    h.update(format!("{key:?}").as_bytes());
    let tag: String = h.finish()[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!(".{name}.nomos-{tag}.tmp")
}

/// Writes `bytes` to `name` in `dir` through a temporary file and a rename,
/// with the owner, group, and mode of `target` set on the temporary file
/// before the rename (spec §11). The temporary file is created exclusively
/// and private: whatever is already at its name, a planted hard link
/// included, fails the execution rather than being written through.
fn write_atomically(
    dir: BorrowedFd<'_>,
    name: &str,
    bytes: &[u8],
    key: &EffectKey,
    target: Target,
) -> Receipt {
    let temp = temporary_name(name, key);
    let fd = match rustix::fs::openat(
        dir,
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    ) {
        Ok(fd) => fd,
        Err(_) => return Receipt::Failed,
    };
    let mut file = File::from(fd);
    let written = file
        .write_all(bytes)
        .map_err(errno)
        .and_then(|()| apply_metadata(file.as_fd(), target))
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
    fn observe(&mut self, resources: &[ResourceKey]) -> Vec<Observation> {
        resources
            .iter()
            .filter_map(|key| {
                let start = now();
                let collection = self.collect(key);
                // The monotonic clock does not go backwards; `max` makes the
                // window valid by construction rather than by that promise.
                let end = now().max(start);
                Observation::new(key.clone(), collection, self.provenance(start, end))
            })
            .collect()
    }
}

impl Mutate for LinuxHost {
    fn apply(&mut self, request: &Apply) -> Vec<Receipt> {
        if let Some(receipts) = self.ledger.get(&request.key) {
            return receipts.clone();
        }
        let done = match (request.key.resource(), &self.systemd) {
            (ResourceKey::Unit(name), Some(systemd)) => {
                let settle_by = request.settle_by;
                systemd.execute(name, &request.operation, || now() >= settle_by)
            }
            _ => Some(self.execute(request)),
        };
        let receipts = match done {
            Some(Receipt::Refused) => vec![Receipt::Refused],
            Some(done) => {
                self.executions += 1;
                vec![Receipt::Accepted, Receipt::Started, done]
            }
            // A job cancelled at the deadline: whether it changed the unit
            // is unknown, and no receipt settles it (N10).
            None => {
                self.executions += 1;
                vec![Receipt::Accepted, Receipt::Started]
            }
        };
        self.ledger.insert(request.key.clone(), receipts.clone());
        receipts
    }
}
