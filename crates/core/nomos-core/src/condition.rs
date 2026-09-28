//! Conditions: propositions reality is expected to satisfy (spec §3).
//!
//! The first resource family is files. A [`FileCondition`] is a sum type, so
//! the contradictions a product type would admit cannot be written: an
//! absent file has no content requirement, because `Absent` has no field for
//! one (ADR 0004 §4, finding `sum-not-product`). A [`Condition`] pairs the
//! requirement with the resource it is about; its fields are private, so a
//! `Condition` exists only through [`Condition::file`], which is the one
//! place validation happens.

use crate::resource::{Digest, ResourcePath};

/// What the bytes of a present file must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Content {
    /// Any bytes at all; presence is the whole requirement.
    Any,
    /// Bytes whose digest is exactly this one.
    Exactly(Digest),
}

/// The requirement on one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum FileCondition {
    /// No file exists at the path.
    Absent,
    /// A file exists at the path with the given content.
    Present {
        /// The content requirement.
        content: Content,
    },
}

/// One Condition of a Canon: a requirement on one resource.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Condition {
    path: ResourcePath,
    requirement: FileCondition,
}

impl Condition {
    /// A Condition on the file at `path`.
    pub fn file(path: ResourcePath, requirement: FileCondition) -> Self {
        Condition { path, requirement }
    }

    /// The resource the Condition is about.
    pub fn path(&self) -> &ResourcePath {
        &self.path
    }

    /// The requirement on it.
    pub fn requirement(&self) -> &FileCondition {
        &self.requirement
    }
}
