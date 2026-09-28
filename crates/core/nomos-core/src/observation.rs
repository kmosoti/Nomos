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

use crate::resource::{Digest, ResourcePath};

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

/// What a collector saw of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum FileEvidence {
    /// No file exists at the path.
    Absent,
    /// A file exists with these bytes.
    Present {
        /// The digest of the bytes.
        digest: Digest,
        /// The byte count.
        size: u64,
    },
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
}

/// The outcome of one collection attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Collection {
    /// Evidence was obtained.
    Collected(FileEvidence),
    /// No evidence was obtained, for this reason.
    Failed(CollectionFailure),
}

/// Evidence about one resource, with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Observation {
    path: ResourcePath,
    collection: Collection,
    provenance: Provenance,
}

impl Observation {
    /// An Observation of the file at `path`.
    pub fn file(path: ResourcePath, collection: Collection, provenance: Provenance) -> Self {
        Observation {
            path,
            collection,
            provenance,
        }
    }

    /// The resource observed.
    pub fn path(&self) -> &ResourcePath {
        &self.path
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
        assert_eq!(
            Window::new(Instant(6), Instant(5)).unwrap_err(),
            WindowError
        );
    }

    #[test]
    fn a_collector_has_a_name() {
        assert!(CollectorId::new("").is_none());
        assert_eq!(CollectorId::new("mock").unwrap().as_str(), "mock");
    }
}
