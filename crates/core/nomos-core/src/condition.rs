//! Conditions: propositions reality is expected to satisfy (spec §3), one
//! requirement type per resource family ([resource-families.md]).
//!
//! Each requirement is a sum type, so the contradictions a product type
//! would admit cannot be written: an absent file has no content requirement,
//! because `Absent` has no field for one (ADR 0004 §4, finding
//! `sum-not-product`). A [`Condition`] pairs a [`ResourceKey`] with a
//! [`Requirement`] of the same family; its fields are private and its
//! constructors take a key and a requirement of one family together, so a
//! Condition whose requirement belongs to another family than its resource
//! cannot be built.
//!
//! [resource-families.md]: ../../../../docs/formal/resource-families.md

use alloc::string::String;
use alloc::vec::Vec;

use crate::resource::{
    AccountName, Digest, Family, Mode, PackageName, ResourceKey, ResourcePath, SysctlKey, UnitName,
};

/// What the bytes of a present file must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Content {
    /// Any bytes at all; presence is the whole requirement.
    Any,
    /// Bytes whose digest is exactly this one.
    Exactly(Digest),
}

/// The owner, group, and mode a present file or directory must have. A
/// field left `None` is not required, and never a Variance.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Metadata {
    /// The owning account, by name.
    pub owner: Option<AccountName>,
    /// The owning group, by name.
    pub group: Option<AccountName>,
    /// The permission bits.
    pub mode: Option<Mode>,
}

impl Metadata {
    /// No metadata requirement at all.
    pub const fn any() -> Self {
        Metadata {
            owner: None,
            group: None,
            mode: None,
        }
    }
}

/// The requirement on one file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileCondition {
    /// No file exists at the path.
    Absent,
    /// A file exists at the path with the given content and metadata.
    Present {
        /// The content requirement.
        content: Content,
        /// The metadata requirement.
        metadata: Metadata,
    },
}

impl FileCondition {
    /// A present file with this content requirement and no metadata one.
    pub const fn present(content: Content) -> Self {
        FileCondition::Present {
            content,
            metadata: Metadata::any(),
        }
    }
}

/// The requirement on one directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectoryCondition {
    /// No directory exists at the path.
    Absent,
    /// A directory exists at the path with the given metadata.
    Present {
        /// The metadata requirement.
        metadata: Metadata,
    },
}

/// Whether a unit must be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Activity {
    /// Active.
    Active,
    /// Inactive (a failed unit is inactive).
    Inactive,
    /// Either.
    Any,
}

/// Whether a unit must start at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Enablement {
    /// Enabled.
    Enabled,
    /// Disabled.
    Disabled,
    /// Either.
    Any,
}

/// The requirement on one systemd unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct UnitCondition {
    /// Whether it must be running.
    pub activity: Activity,
    /// Whether it must start at boot.
    pub enablement: Enablement,
}

/// A kernel parameter's value, normalized: whitespace-separated tokens
/// joined by one space. The empty value is a value the kernel may report;
/// the IR does not let a Canon require it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SysctlValue(String);

impl SysctlValue {
    /// `text`, normalized.
    pub fn normalized(text: &str) -> Self {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        SysctlValue(tokens.join(" "))
    }

    /// The normalized text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `text` is already in normal form.
    pub fn is_normal(text: &str) -> bool {
        SysctlValue::normalized(text).0 == text
    }
}

/// The requirement on one kernel parameter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SysctlCondition {
    /// The value it must have.
    pub value: SysctlValue,
}

/// Which range an account's numeric ID is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum AccountClass {
    /// A system account: numeric ID 0 to [`AccountClass::SYSTEM_MAX`].
    System,
    /// A regular account: every other numeric ID.
    Regular,
}

impl AccountClass {
    /// The largest system ID, Debian's `SYS_UID_MAX`.
    pub const SYSTEM_MAX: u32 = 999;

    /// The class of a numeric ID.
    pub fn of(id: u32) -> AccountClass {
        if id <= AccountClass::SYSTEM_MAX {
            AccountClass::System
        } else {
            AccountClass::Regular
        }
    }
}

/// The requirement on one account.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UserCondition {
    /// No account has the name.
    Absent,
    /// An account has the name, in the class, with the stated fields.
    Present {
        /// Its class.
        class: AccountClass,
        /// Its home directory, when required.
        home: Option<ResourcePath>,
        /// Its login shell, when required.
        shell: Option<ResourcePath>,
    },
}

/// A Debian version string: 1 to 128 bytes of `[A-Za-z0-9.+~:-]`, starting
/// with a digit or an epoch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// Validates `text` as a version.
    pub fn new(text: &str) -> Option<Self> {
        let ok = !text.is_empty()
            && text.len() <= 128
            && text.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && text.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'~' | b':' | b'-')
            });
        ok.then(|| PackageVersion(String::from(text)))
    }

    /// The version as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The requirement on one package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageCondition {
    /// The package is not installed.
    Absent,
    /// The package is installed, at the version when one is stated.
    Installed {
        /// The exact version, when required.
        version: Option<PackageVersion>,
    },
}

/// A requirement of any family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Requirement {
    /// On a directory.
    Directory(DirectoryCondition),
    /// On a file.
    File(FileCondition),
    /// On a package.
    Package(PackageCondition),
    /// On the abstract service, whose evidence is a file's.
    Service(FileCondition),
    /// On a kernel parameter.
    Sysctl(SysctlCondition),
    /// On a unit.
    Unit(UnitCondition),
    /// On an account.
    User(UserCondition),
}

impl Requirement {
    /// The requirement's family.
    pub fn family(&self) -> Family {
        match self {
            Requirement::Directory(_) => Family::Directory,
            Requirement::File(_) => Family::File,
            Requirement::Package(_) => Family::Package,
            Requirement::Service(_) => Family::Service,
            Requirement::Sysctl(_) => Family::Sysctl,
            Requirement::Unit(_) => Family::Unit,
            Requirement::User(_) => Family::User,
        }
    }
}

/// One Condition of a Canon: a requirement on one resource, of its family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Condition {
    key: ResourceKey,
    requirement: Requirement,
}

impl Condition {
    /// A Condition on the file at `path`.
    pub fn file(path: ResourcePath, requirement: FileCondition) -> Self {
        Condition {
            key: ResourceKey::File(path),
            requirement: Requirement::File(requirement),
        }
    }

    /// A Condition on the directory at `path`.
    pub fn directory(path: ResourcePath, requirement: DirectoryCondition) -> Self {
        Condition {
            key: ResourceKey::Directory(path),
            requirement: Requirement::Directory(requirement),
        }
    }

    /// A Condition on the abstract service at `path`.
    pub fn service(path: ResourcePath, requirement: FileCondition) -> Self {
        Condition {
            key: ResourceKey::Service(path),
            requirement: Requirement::Service(requirement),
        }
    }

    /// A Condition on the unit `name`.
    pub fn unit(name: UnitName, requirement: UnitCondition) -> Self {
        Condition {
            key: ResourceKey::Unit(name),
            requirement: Requirement::Unit(requirement),
        }
    }

    /// A Condition on the kernel parameter `name`.
    pub fn sysctl(name: SysctlKey, requirement: SysctlCondition) -> Self {
        Condition {
            key: ResourceKey::Sysctl(name),
            requirement: Requirement::Sysctl(requirement),
        }
    }

    /// A Condition on the account `name`.
    pub fn user(name: AccountName, requirement: UserCondition) -> Self {
        Condition {
            key: ResourceKey::User(name),
            requirement: Requirement::User(requirement),
        }
    }

    /// A Condition on the package `name`.
    pub fn package(name: PackageName, requirement: PackageCondition) -> Self {
        Condition {
            key: ResourceKey::Package(name),
            requirement: Requirement::Package(requirement),
        }
    }

    /// A Condition from a key and a requirement, when they are of one family.
    pub fn new(key: ResourceKey, requirement: Requirement) -> Option<Self> {
        (key.family() == requirement.family()).then_some(Condition { key, requirement })
    }

    /// The resource the Condition is about.
    pub fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// The requirement on it.
    pub fn requirement(&self) -> &Requirement {
        &self.requirement
    }
}
