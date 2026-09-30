//! Traits: facts a host reports about itself, each with its provenance,
//! the instant it was observed, and how long it can be expected to hold
//! (spec §4). A Trait is evidence about the host, never a Condition on it.

use alloc::string::String;

use crate::observation::Instant;

/// How long a Trait can be expected to keep its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stability {
    /// Names the host; it changes only when the host is rebuilt.
    Identity,
    /// Changes when the host is reconfigured: an upgrade, a new name.
    Stable,
    /// Changes on its own over days: a kernel release after a reboot.
    Dynamic,
    /// Changes from one reading to the next.
    Ephemeral,
}

impl Stability {
    /// The stability's name in text.
    pub fn as_str(self) -> &'static str {
        match self {
            Stability::Identity => "identity",
            Stability::Stable => "stable",
            Stability::Dynamic => "dynamic",
            Stability::Ephemeral => "ephemeral",
        }
    }
}

/// One fact about the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trait {
    /// What the fact is, for example `os.id`.
    pub key: String,
    /// Its value, as read.
    pub value: String,
    /// Where it was read from.
    pub source: String,
    /// When it was read, on the collector's clock.
    pub observed_at: Instant,
    /// How long it can be expected to hold.
    pub stability: Stability,
}
