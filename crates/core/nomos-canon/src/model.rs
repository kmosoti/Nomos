//! The validated Canon, its untrusted data-transfer object, and the one
//! validator between them ([canon-ir.md](../../../../docs/formal/canon-ir.md)).
//!
//! [`RawCanon`] has public fields and holds anything. [`Canon`] has private
//! fields, and its only constructor is `Canon::try_from(RawCanon)`: the
//! authoring API builds a `RawCanon` and calls it, the decoder builds a
//! `RawCanon` from bytes and calls it, and a migration builds one from an
//! older schema and calls it. One validator, three paths, and no way around
//! it that compiles (`tests/compile-fail/`).
//!
//! A `Canon` is normalized by construction: resources are a map by path,
//! relations, keys, and nodes are sets. Two equivalent Canons are equal
//! values, and [`Canon::to_raw`] gives the one normalized DTO.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use nomos_core::condition::{Content, FileCondition};
use nomos_core::resource::{Digest, ResourcePath};

/// The longest resource path the IR carries, in bytes.
pub const MAX_PATH: usize = 4096;

// ---------------------------------------------------------------------------
// The untrusted form

/// A Canon as data, unvalidated. Anything can be written here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawCanon {
    /// The Canon's name.
    pub name: String,
    /// The managed resources.
    pub resources: Vec<RawResource>,
    /// The relations between them.
    pub relations: Vec<RawRelation>,
}

/// A resource as data, unvalidated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawResource {
    /// The resource path.
    pub path: String,
    /// What is required of it.
    pub spec: RawSpec,
    /// Its Action's conflict keys.
    pub keys: Vec<String>,
    /// The nodes its Action would disrupt.
    pub disrupts: Vec<String>,
}

/// A requirement as data, unvalidated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawSpec {
    /// `file` or `service`.
    pub kind: String,
    /// `absent`, `present-any`, `present-exact`, `running`, or `loaded`.
    pub state: String,
    /// The digest, for `present-exact` and `loaded`.
    pub digest: Option<String>,
}

/// A relation as data, unvalidated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawRelation {
    /// The constraining resource.
    pub source: String,
    /// The constrained resource.
    pub target: String,
    /// `requires`, `after`, or `on_change`.
    pub kind: String,
}

// ---------------------------------------------------------------------------
// The validated form

/// A name: of a Canon, a conflict key, or a node.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({})", self.0)
    }
}

fn canon_name(text: &str) -> Option<Name> {
    let ok = (1..=64).contains(&text.len())
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    ok.then(|| Name(String::from(text)))
}

fn label(text: &str) -> Option<Name> {
    let ok = (1..=128).contains(&text.len())
        && text.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | ':' | '/' | '-')
        });
    ok.then(|| Name(String::from(text)))
}

/// What a service must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceRequirement {
    /// Running.
    Running,
    /// Running the configuration with this digest.
    Loaded(Digest),
}

/// What a resource must be. A file that is absent with contents, or an
/// absent service, has no representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Requirement {
    /// A file requirement.
    File(FileCondition),
    /// A service requirement.
    Service(ServiceRequirement),
}

/// A resource kind: the capability a reader needs to execute it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Files.
    File,
    /// Services.
    Service,
}

impl Requirement {
    /// The requirement's kind.
    pub fn kind(&self) -> Kind {
        match self {
            Requirement::File(_) => Kind::File,
            Requirement::Service(_) => Kind::Service,
        }
    }
}

/// A managed resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    requirement: Requirement,
    keys: BTreeSet<Name>,
    disrupts: BTreeSet<Name>,
}

impl Resource {
    /// What is required of it.
    pub fn requirement(&self) -> &Requirement {
        &self.requirement
    }

    /// Its Action's conflict keys.
    pub fn keys(&self) -> &BTreeSet<Name> {
        &self.keys
    }

    /// The nodes its Action would disrupt.
    pub fn disrupts(&self) -> &BTreeSet<Name> {
        &self.disrupts
    }
}

/// How one resource constrains another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationKind {
    /// The target starts only after the source succeeded.
    Requires,
    /// The target starts only after the source ended.
    After,
    /// The target runs when the source changed.
    OnChange,
}

impl RelationKind {
    /// The kind's name in the IR.
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationKind::Requires => "requires",
            RelationKind::After => "after",
            RelationKind::OnChange => "on_change",
        }
    }
}

/// A relation between two resources of the Canon.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Relation {
    source: ResourcePath,
    target: ResourcePath,
    kind: RelationKind,
}

impl Relation {
    /// The constraining resource.
    pub fn source(&self) -> &ResourcePath {
        &self.source
    }

    /// The constrained resource.
    pub fn target(&self) -> &ResourcePath {
        &self.target
    }

    /// How.
    pub fn kind(&self) -> RelationKind {
        self.kind
    }
}

/// A validated, normalized Canon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canon {
    name: Name,
    resources: BTreeMap<ResourcePath, Resource>,
    relations: BTreeSet<Relation>,
}

impl Canon {
    /// The name.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// The resources, by path.
    pub fn resources(&self) -> &BTreeMap<ResourcePath, Resource> {
        &self.resources
    }

    /// The relations, in order.
    pub fn relations(&self) -> &BTreeSet<Relation> {
        &self.relations
    }

    /// The resource kinds the Canon uses.
    pub fn kinds(&self) -> BTreeSet<Kind> {
        self.resources
            .values()
            .map(|r| r.requirement.kind())
            .collect()
    }

    /// The normalized data-transfer object: every set sorted, every field
    /// present. Validating it gives this Canon back.
    pub fn to_raw(&self) -> RawCanon {
        RawCanon {
            name: String::from(self.name.as_str()),
            resources: self
                .resources
                .iter()
                .map(|(path, r)| RawResource {
                    path: String::from(path.as_str()),
                    spec: raw_spec(&r.requirement),
                    keys: r.keys.iter().map(|k| String::from(k.as_str())).collect(),
                    disrupts: r
                        .disrupts
                        .iter()
                        .map(|k| String::from(k.as_str()))
                        .collect(),
                })
                .collect(),
            relations: self
                .relations
                .iter()
                .map(|rel| RawRelation {
                    source: String::from(rel.source.as_str()),
                    target: String::from(rel.target.as_str()),
                    kind: String::from(rel.kind.as_str()),
                })
                .collect(),
        }
    }
}

fn hex(d: &Digest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for b in d.as_bytes() {
        s.push(char::from(HEX[usize::from(b >> 4)]));
        s.push(char::from(HEX[usize::from(b & 0xf)]));
    }
    s
}

fn raw_spec(r: &Requirement) -> RawSpec {
    let (kind, state, digest) = match r {
        Requirement::File(FileCondition::Absent) => ("file", "absent", None),
        Requirement::File(FileCondition::Present {
            content: Content::Any,
        }) => ("file", "present-any", None),
        Requirement::File(FileCondition::Present {
            content: Content::Exactly(d),
        }) => ("file", "present-exact", Some(hex(d))),
        Requirement::Service(ServiceRequirement::Running) => ("service", "running", None),
        Requirement::Service(ServiceRequirement::Loaded(d)) => ("service", "loaded", Some(hex(d))),
    };
    RawSpec {
        kind: String::from(kind),
        state: String::from(state),
        digest,
    }
}

// ---------------------------------------------------------------------------
// The validator

/// Why a `RawCanon` is not a Canon. Errors say where, by position in the
/// input, and never repeat the rejected text, which may be anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonError {
    /// The name is not 1 to 64 of `a-z`, `0-9`, `-`, starting with a letter.
    InvalidName,
    /// A resource's path is not a valid resource path of at most 4,096 bytes.
    InvalidPath {
        /// The resource's position.
        resource: usize,
    },
    /// A resource's kind or state is not one the IR defines.
    UnknownRequirement {
        /// The resource's position.
        resource: usize,
    },
    /// A digest is missing, present where none belongs, or not 64 lowercase
    /// hexadecimal characters.
    InvalidDigest {
        /// The resource's position.
        resource: usize,
    },
    /// A conflict key or node is not 1 to 128 of `a-z`, `0-9`, `._:/-`.
    InvalidLabel {
        /// The resource's position.
        resource: usize,
    },
    /// Two resources have one path.
    DuplicatePath {
        /// The second resource's position.
        resource: usize,
    },
    /// A relation's kind is not `requires`, `after`, or `on_change`.
    UnknownRelation {
        /// The relation's position.
        relation: usize,
    },
    /// A relation names a resource the Canon does not have.
    DanglingRelation {
        /// The relation's position.
        relation: usize,
    },
    /// A relation from a resource to itself.
    SelfRelation {
        /// The relation's position.
        relation: usize,
    },
    /// The relations form a cycle.
    Cycle,
}

impl fmt::Display for CanonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonError::InvalidName => f.write_str("invalid Canon name"),
            CanonError::InvalidPath { resource } => write!(f, "resource {resource}: invalid path"),
            CanonError::UnknownRequirement { resource } => {
                write!(f, "resource {resource}: unknown kind or state")
            }
            CanonError::InvalidDigest { resource } => {
                write!(f, "resource {resource}: invalid or misplaced digest")
            }
            CanonError::InvalidLabel { resource } => {
                write!(f, "resource {resource}: invalid conflict key or node")
            }
            CanonError::DuplicatePath { resource } => {
                write!(f, "resource {resource}: a second resource at one path")
            }
            CanonError::UnknownRelation { relation } => {
                write!(f, "relation {relation}: unknown kind")
            }
            CanonError::DanglingRelation { relation } => {
                write!(f, "relation {relation}: names a resource the Canon lacks")
            }
            CanonError::SelfRelation { relation } => {
                write!(f, "relation {relation}: relates a resource to itself")
            }
            CanonError::Cycle => f.write_str("the relations form a cycle"),
        }
    }
}

fn digest(text: &Option<String>, resource: usize) -> Result<Digest, CanonError> {
    let err = CanonError::InvalidDigest { resource };
    let t = text.as_deref().ok_or(err)?;
    if t.len() != 64
        || !t
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(err);
    }
    Digest::from_hex(t).map_err(|_| err)
}

fn requirement(spec: &RawSpec, resource: usize) -> Result<Requirement, CanonError> {
    let unknown = CanonError::UnknownRequirement { resource };
    let no_digest = |r: Requirement| {
        if spec.digest.is_some() {
            Err(CanonError::InvalidDigest { resource })
        } else {
            Ok(r)
        }
    };
    match (spec.kind.as_str(), spec.state.as_str()) {
        ("file", "absent") => no_digest(Requirement::File(FileCondition::Absent)),
        ("file", "present-any") => no_digest(Requirement::File(FileCondition::Present {
            content: Content::Any,
        })),
        ("file", "present-exact") => Ok(Requirement::File(FileCondition::Present {
            content: Content::Exactly(digest(&spec.digest, resource)?),
        })),
        ("service", "running") => no_digest(Requirement::Service(ServiceRequirement::Running)),
        ("service", "loaded") => Ok(Requirement::Service(ServiceRequirement::Loaded(digest(
            &spec.digest,
            resource,
        )?))),
        _ => Err(unknown),
    }
}

fn relation_kind(text: &str) -> Option<RelationKind> {
    match text {
        "requires" => Some(RelationKind::Requires),
        "after" => Some(RelationKind::After),
        "on_change" => Some(RelationKind::OnChange),
        _ => None,
    }
}

impl TryFrom<RawCanon> for Canon {
    type Error = CanonError;

    /// The validator: the only way to a `Canon`.
    fn try_from(raw: RawCanon) -> Result<Self, CanonError> {
        let name = canon_name(&raw.name).ok_or(CanonError::InvalidName)?;
        let mut resources = BTreeMap::new();
        for (i, r) in raw.resources.iter().enumerate() {
            if r.path.len() > MAX_PATH {
                return Err(CanonError::InvalidPath { resource: i });
            }
            let path =
                ResourcePath::new(&r.path).map_err(|_| CanonError::InvalidPath { resource: i })?;
            let requirement = requirement(&r.spec, i)?;
            let labels = |list: &[String]| -> Result<BTreeSet<Name>, CanonError> {
                list.iter()
                    .map(|k| label(k).ok_or(CanonError::InvalidLabel { resource: i }))
                    .collect()
            };
            let resource = Resource {
                requirement,
                keys: labels(&r.keys)?,
                disrupts: labels(&r.disrupts)?,
            };
            if resources.insert(path, resource).is_some() {
                return Err(CanonError::DuplicatePath { resource: i });
            }
        }
        let mut relations = BTreeSet::new();
        for (i, rel) in raw.relations.iter().enumerate() {
            let kind =
                relation_kind(&rel.kind).ok_or(CanonError::UnknownRelation { relation: i })?;
            let end = |text: &str| {
                ResourcePath::new(text)
                    .ok()
                    .filter(|p| resources.contains_key(p))
                    .ok_or(CanonError::DanglingRelation { relation: i })
            };
            let source = end(&rel.source)?;
            let target = end(&rel.target)?;
            if source == target {
                return Err(CanonError::SelfRelation { relation: i });
            }
            relations.insert(Relation {
                source,
                target,
                kind,
            });
        }
        if has_cycle(&resources, &relations) {
            return Err(CanonError::Cycle);
        }
        Ok(Canon {
            name,
            resources,
            relations,
        })
    }
}

/// Kahn's algorithm over every relation kind: a cycle leaves vertices
/// with remaining in-degree.
fn has_cycle(resources: &BTreeMap<ResourcePath, Resource>, relations: &BTreeSet<Relation>) -> bool {
    let mut indegree: BTreeMap<&ResourcePath, usize> = resources.keys().map(|p| (p, 0)).collect();
    for r in relations {
        if let Some(d) = indegree.get_mut(&r.target) {
            *d += 1;
        }
    }
    let mut ready: Vec<&ResourcePath> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(p, _)| *p)
        .collect();
    let mut seen = 0;
    while let Some(p) = ready.pop() {
        seen += 1;
        for r in relations.iter().filter(|r| &r.source == p) {
            if let Some(d) = indegree.get_mut(&r.target) {
                *d -= 1;
                if *d == 0 {
                    ready.push(&r.target);
                }
            }
        }
    }
    seen != resources.len()
}

// ---------------------------------------------------------------------------
// The authoring API

/// Builds a Canon from typed parts. It writes a [`RawCanon`] and validates
/// it with the same validator the decoder uses, so the two cannot disagree
/// about what a Canon is.
#[derive(Debug, Clone, Default)]
pub struct CanonBuilder {
    raw: RawCanon,
}

impl CanonBuilder {
    /// A Canon named `name`.
    pub fn new(name: &str) -> Self {
        CanonBuilder {
            raw: RawCanon {
                name: String::from(name),
                ..RawCanon::default()
            },
        }
    }

    /// Adds the resource at `path` with `requirement`, conflict `keys`, and
    /// disrupted `nodes`.
    pub fn resource(
        mut self,
        path: &str,
        requirement: Requirement,
        keys: &[&str],
        nodes: &[&str],
    ) -> Self {
        self.raw.resources.push(RawResource {
            path: String::from(path),
            spec: raw_spec(&requirement),
            keys: keys.iter().map(|k| String::from(*k)).collect(),
            disrupts: nodes.iter().map(|k| String::from(*k)).collect(),
        });
        self
    }

    /// Adds a file with no conflict keys and no disruption.
    pub fn file(self, path: &str, requirement: FileCondition) -> Self {
        self.resource(path, Requirement::File(requirement), &[], &[])
    }

    /// Adds a service with no conflict keys and no disruption.
    pub fn service(self, path: &str, requirement: ServiceRequirement) -> Self {
        self.resource(path, Requirement::Service(requirement), &[], &[])
    }

    /// Relates `source` to `target`.
    pub fn relate(mut self, source: &str, kind: RelationKind, target: &str) -> Self {
        self.raw.relations.push(RawRelation {
            source: String::from(source),
            target: String::from(target),
            kind: String::from(kind.as_str()),
        });
        self
    }

    /// Validates and returns the Canon.
    pub fn build(self) -> Result<Canon, CanonError> {
        Canon::try_from(self.raw)
    }
}
