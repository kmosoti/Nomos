//! Observations: evidence from Substrate about one resource (spec §3).
//!
//! An Observation says what was seen, by which collector, over which window,
//! and whether collection succeeded. Provenance and the collection outcome
//! are part of the evidence, not metadata beside it (ADR 0005). A collector
//! that could not observe returns an Observation whose [`Collection`]
//! failed, with the reason; it has no channel through which to say
//! "satisfied" or "mismatch" (ADR 0006 §1).
//!
//! Time here is data. An [`Instant`] is a count of nanoseconds on a clock the
//! collector names; the kernel compares instants it is given and never reads
//! a clock (ADR 0016).

use alloc::string::String;
use core::fmt;

use crate::condition::{PackageVersion, SysctlValue};
use crate::resource::{
    AccountName, Digest, Family, Mode, PackageName, ResourceKey, ResourcePath, SysctlKey, UnitName,
};

/// A point on a collector's clock, in nanoseconds. Only comparisons between
/// instants from the same collector are meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct Instant(pub u64);

/// The window over which evidence was collected: it is true of the resource
/// at some moment between `start` and `end`, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Window {
    start: Instant,
    end: Instant,
}

/// Why two instants do not make a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowError;

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("window ends before it starts")
    }
}

impl Window {
    /// A window from `start` to `end`; `end` may equal `start`.
    pub fn new(start: Instant, end: Instant) -> Result<Self, WindowError> {
        if end < start {
            return Err(WindowError);
        }
        Ok(Window { start, end })
    }

    /// When collection began.
    pub fn start(&self) -> Instant {
        self.start
    }

    /// When collection ended.
    pub fn end(&self) -> Instant {
        self.end
    }
}

/// Which collector produced an Observation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CollectorId(String);

impl CollectorId {
    /// A collector by name. Any non-empty text; naming is an adapter concern.
    pub fn new(name: &str) -> Option<Self> {
        (!name.is_empty()).then(|| CollectorId(String::from(name)))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who collected the evidence and when.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Provenance {
    collector: CollectorId,
    window: Window,
}

impl Provenance {
    /// Evidence from `collector` over `window`.
    pub fn new(collector: CollectorId, window: Window) -> Self {
        Provenance { collector, window }
    }

    /// The collector.
    pub fn collector(&self) -> &CollectorId {
        &self.collector
    }

    /// The collection window.
    pub fn window(&self) -> Window {
        self.window
    }
}

/// An owner or group as the host reports it: by name when the host names
/// the numeric ID, by number otherwise.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Account {
    /// The host names the ID.
    Named(AccountName),
    /// The host has no name for the ID.
    Id(u32),
}

/// The owner, group, and mode a collector saw.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObservedMetadata {
    /// The owner.
    pub owner: Account,
    /// The group.
    pub group: Account,
    /// The permission bits.
    pub mode: Mode,
}

impl ObservedMetadata {
    /// Owned by root, user and group, with mode `0644`: what a file created
    /// by root under the default umask has.
    pub fn root_default() -> Self {
        ObservedMetadata {
            owner: Account::Id(0),
            group: Account::Id(0),
            mode: Mode::DEFAULT_FILE,
        }
    }
}

/// What a collector saw of a file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileEvidence {
    /// No file exists at the path.
    Absent,
    /// A file exists with these bytes and this metadata.
    Present {
        /// The digest of the bytes.
        digest: Digest,
        /// The byte count.
        size: u64,
        /// The owner, group, and mode.
        metadata: ObservedMetadata,
    },
}

impl FileEvidence {
    /// A present file with this digest and size and root's default metadata.
    pub fn present(digest: Digest, size: u64) -> Self {
        FileEvidence::Present {
            digest,
            size,
            metadata: ObservedMetadata::root_default(),
        }
    }
}

/// What a collector saw of a directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectoryEvidence {
    /// No directory exists at the path.
    Absent,
    /// A directory exists with this metadata.
    Present {
        /// The owner, group, and mode.
        metadata: ObservedMetadata,
    },
}

/// A unit's active state, as systemd reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum ActiveState {
    /// Running.
    Active,
    /// Reloading its configuration; still running.
    Reloading,
    /// Not running.
    Inactive,
    /// Not running, after a failure.
    Failed,
    /// Starting.
    Activating,
    /// Stopping.
    Deactivating,
}

/// A unit file's state, as systemd reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum UnitFileState {
    /// Started at boot.
    Enabled,
    /// Not started at boot.
    Disabled,
    /// Has no install section; neither enabled nor disabled.
    Static,
    /// Masked: it cannot be started.
    Masked,
    /// Any other state systemd reports.
    Other,
}

/// What a collector saw of a unit systemd knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct UnitEvidence {
    /// Whether it runs.
    pub active: ActiveState,
    /// Whether it starts at boot.
    pub file_state: UnitFileState,
}

/// What a collector read of a kernel parameter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SysctlEvidence {
    /// The value, normalized.
    pub value: SysctlValue,
}

/// What a collector saw of an account.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UserEvidence {
    /// No account has the name.
    Absent,
    /// An account has the name.
    Present {
        /// Its numeric ID.
        uid: u32,
        /// Its primary group's numeric ID.
        gid: u32,
        /// Its home directory, as recorded.
        home: String,
        /// Its login shell, as recorded.
        shell: String,
    },
}

/// What a collector saw of a package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageEvidence {
    /// Not installed; its configuration files may remain.
    NotInstalled,
    /// Installed at this version.
    Installed {
        /// The installed version.
        version: PackageVersion,
    },
    /// Half-installed, unpacked, or failed: dpkg holds it in no settled state.
    Broken,
}

/// Evidence of any family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Evidence {
    /// Of a directory.
    Directory(DirectoryEvidence),
    /// Of a file.
    File(FileEvidence),
    /// Of a package.
    Package(PackageEvidence),
    /// Of the abstract service, as a file's.
    Service(FileEvidence),
    /// Of a kernel parameter.
    Sysctl(SysctlEvidence),
    /// Of a unit.
    Unit(UnitEvidence),
    /// Of an account.
    User(UserEvidence),
}

impl Evidence {
    /// The evidence's family.
    pub fn family(&self) -> Family {
        match self {
            Evidence::Directory(_) => Family::Directory,
            Evidence::File(_) => Family::File,
            Evidence::Package(_) => Family::Package,
            Evidence::Service(_) => Family::Service,
            Evidence::Sysctl(_) => Family::Sysctl,
            Evidence::Unit(_) => Family::Unit,
            Evidence::User(_) => Family::User,
        }
    }
}

/// Why a collector could not produce evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum CollectionFailure {
    /// The collector was not permitted to read the resource.
    PermissionDenied,
    /// The collector gave up before the resource answered.
    TimedOut,
    /// The resource exists in a form the collector cannot describe, for
    /// example a socket where a file was expected.
    Unsupported,
    /// The operating system reported an error the collector does not classify.
    Io,
    /// The resource's manager does not know it: a unit systemd cannot load,
    /// a parameter the running kernel does not have. Not absence.
    Unavailable,
}

/// The outcome of one collection attempt, over the evidence type `E`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Collection<E = Evidence> {
    /// Evidence was obtained.
    Collected(E),
    /// No evidence was obtained, for this reason.
    Failed(CollectionFailure),
}

impl<E> Collection<E> {
    fn wrap(self, family: impl FnOnce(E) -> Evidence) -> Collection {
        match self {
            Collection::Collected(e) => Collection::Collected(family(e)),
            Collection::Failed(f) => Collection::Failed(f),
        }
    }
}

/// Evidence about one resource, with its provenance. The constructors take
/// a key and a collection of one family together, so the evidence of an
/// Observation is always of its resource's family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Observation {
    key: ResourceKey,
    collection: Collection,
    provenance: Provenance,
}

impl Observation {
    fn with(key: ResourceKey, collection: Collection, provenance: Provenance) -> Self {
        Observation {
            key,
            collection,
            provenance,
        }
    }

    /// An Observation of the file at `path`.
    pub fn file(
        path: ResourcePath,
        collection: Collection<FileEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::File(path),
            collection.wrap(Evidence::File),
            provenance,
        )
    }

    /// An Observation of the directory at `path`.
    pub fn directory(
        path: ResourcePath,
        collection: Collection<DirectoryEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::Directory(path),
            collection.wrap(Evidence::Directory),
            provenance,
        )
    }

    /// An Observation of the abstract service at `path`.
    pub fn service(
        path: ResourcePath,
        collection: Collection<FileEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::Service(path),
            collection.wrap(Evidence::Service),
            provenance,
        )
    }

    /// An Observation of the unit `name`.
    pub fn unit(
        name: UnitName,
        collection: Collection<UnitEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::Unit(name),
            collection.wrap(Evidence::Unit),
            provenance,
        )
    }

    /// An Observation of the kernel parameter `name`.
    pub fn sysctl(
        name: SysctlKey,
        collection: Collection<SysctlEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::Sysctl(name),
            collection.wrap(Evidence::Sysctl),
            provenance,
        )
    }

    /// An Observation of the account `name`.
    pub fn user(
        name: AccountName,
        collection: Collection<UserEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::User(name),
            collection.wrap(Evidence::User),
            provenance,
        )
    }

    /// An Observation of the package `name`.
    pub fn package(
        name: PackageName,
        collection: Collection<PackageEvidence>,
        provenance: Provenance,
    ) -> Self {
        Observation::with(
            ResourceKey::Package(name),
            collection.wrap(Evidence::Package),
            provenance,
        )
    }

    /// An Observation of `key` whose collection failed.
    pub fn failed(key: ResourceKey, failure: CollectionFailure, provenance: Provenance) -> Self {
        Observation::with(key, Collection::Failed(failure), provenance)
    }

    /// An Observation from a key and a collection, when the collection's
    /// evidence is of the key's family.
    pub fn new(key: ResourceKey, collection: Collection, provenance: Provenance) -> Option<Self> {
        let matches = match &collection {
            Collection::Collected(evidence) => evidence.family() == key.family(),
            Collection::Failed(_) => true,
        };
        matches.then(|| Observation::with(key, collection, provenance))
    }

    /// The resource observed.
    pub fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// What collection produced.
    pub fn collection(&self) -> &Collection {
        &self.collection
    }

    /// Who collected it and when.
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

#[cfg(test)]
mod tests {
    use super::{CollectorId, Instant, Window, WindowError};

    #[test]
    fn a_window_never_ends_before_it_starts() {
        assert!(Window::new(Instant(5), Instant(5)).is_ok());
        assert!(Window::new(Instant(5), Instant(6)).is_ok());
        let error = Window::new(Instant(6), Instant(5)).unwrap_err();
        assert_eq!(error, WindowError);
        assert!(alloc::format!("{error}").contains("window"));
    }

    #[test]
    fn a_collector_has_a_name() {
        assert!(CollectorId::new("").is_none());
        assert_eq!(CollectorId::new("mock").unwrap().as_str(), "mock");
    }
}
