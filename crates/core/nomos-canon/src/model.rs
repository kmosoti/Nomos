//! The validated Canon, its untrusted data-transfer objects, and the one
//! validator behind them ([canon-ir.md](../../../../docs/formal/canon-ir.md),
//! [resource-families.md](../../../../docs/formal/resource-families.md)).
//!
//! [`RawCanon3`] is the schema-3 data-transfer object and [`RawCanon`] the
//! schema-2 one; both have public fields and hold anything. [`Canon`] has
//! private fields, and its only constructors are `Canon::try_from` of either:
//! the authoring API builds a `RawCanon3`, the decoder builds one from bytes,
//! and a schema-2 or schema-1 artifact arrives as a `RawCanon`. Each schema
//! has its own reading of a resource's `spec`, with its own errors, and every
//! Canon then passes the same structural validation: labels, one resource
//! per key, no two resources writing one property, relations, and cycles. No
//! way around it compiles (`tests/compile-fail/`).
//!
//! A `Canon` is normalized by construction: resources are a map by key,
//! relations, keys, and nodes are sets. Two equivalent Canons are equal
//! values, and [`Canon::to_raw`] gives the one normalized schema-3 DTO.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use nomos_core::condition::{
    AccountClass, Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata,
    PackageCondition, PackageVersion, SysctlCondition, SysctlValue, UnitCondition, UserCondition,
};
use nomos_core::footprint::Property;
use nomos_core::resource::{AccountName, Digest, Family, Mode, ResourceKey, ResourcePath};

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

/// A schema-3 Canon as data, unvalidated. Anything can be written here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawCanon3 {
    /// The Canon's name.
    pub name: String,
    /// The managed resources.
    pub resources: Vec<RawResource3>,
    /// The relations between them; each end is a key's text form.
    pub relations: Vec<RawRelation>,
}

/// A schema-3 resource as data, unvalidated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawResource3 {
    /// The family.
    pub kind: String,
    /// The name within the family.
    pub name: String,
    /// The requirement's fields, by name.
    pub spec: BTreeMap<String, String>,
    /// Its Action's conflict keys.
    pub keys: Vec<String>,
    /// The nodes its Action would disrupt.
    pub disrupts: Vec<String>,
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

/// What a resource must be, of one family ([resource-families.md]). A file
/// that is absent with contents, or an absent service, has no
/// representation.
///
/// [resource-families.md]: ../../../../docs/formal/resource-families.md
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Requirement {
    /// A directory requirement.
    Directory(DirectoryCondition),
    /// A file requirement.
    File(FileCondition),
    /// A package requirement.
    Package(PackageCondition),
    /// A service requirement.
    Service(ServiceRequirement),
    /// A kernel parameter requirement.
    Sysctl(SysctlCondition),
    /// A unit requirement.
    Unit(UnitCondition),
    /// An account requirement.
    User(UserCondition),
}

/// A resource kind: the capability a reader needs to execute it. One per
/// resource family.
pub type Kind = Family;

impl Requirement {
    /// The requirement's kind.
    pub fn kind(&self) -> Kind {
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
    source: ResourceKey,
    target: ResourceKey,
    kind: RelationKind,
}

impl Relation {
    /// The constraining resource.
    pub fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The constrained resource.
    pub fn target(&self) -> &ResourceKey {
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
    resources: BTreeMap<ResourceKey, Resource>,
    relations: BTreeSet<Relation>,
}

impl Canon {
    /// The name.
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// The resources, by key.
    pub fn resources(&self) -> &BTreeMap<ResourceKey, Resource> {
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

    /// The normalized schema-3 data-transfer object: every set sorted,
    /// every stated field present. Validating it gives this Canon back.
    pub fn to_raw(&self) -> RawCanon3 {
        RawCanon3 {
            name: String::from(self.name.as_str()),
            resources: self
                .resources
                .iter()
                .map(|(key, r)| RawResource3 {
                    kind: String::from(key.family().as_str()),
                    name: String::from(key.name()),
                    spec: spec_fields(&r.requirement),
                    keys: labels_text(&r.keys),
                    disrupts: labels_text(&r.disrupts),
                })
                .collect(),
            relations: self
                .relations
                .iter()
                .map(|rel| RawRelation {
                    source: format!("{}", rel.source),
                    target: format!("{}", rel.target),
                    kind: String::from(rel.kind.as_str()),
                })
                .collect(),
        }
    }

    /// The normalized schema-2 data-transfer object, when schema 2 can say
    /// this Canon: files and services only, and no file metadata.
    pub fn to_raw_v2(&self) -> Option<RawCanon> {
        let mut resources = Vec::new();
        for (key, r) in &self.resources {
            let path = key.path()?;
            resources.push(RawResource {
                path: String::from(path.as_str()),
                spec: raw_spec_v2(&r.requirement)?,
                keys: labels_text(&r.keys),
                disrupts: labels_text(&r.disrupts),
            });
        }
        // Schema 2 orders by path, then by the relation kind's order, as the
        // Canon of `06-canon-artifact` held them; a key orders by family
        // first, so the order is restated here rather than inherited.
        let mut ends = Vec::new();
        for rel in &self.relations {
            ends.push((rel.source.path()?, rel.target.path()?, rel.kind));
        }
        ends.sort();
        let relations = ends
            .into_iter()
            .map(|(source, target, kind)| RawRelation {
                source: String::from(source.as_str()),
                target: String::from(target.as_str()),
                kind: String::from(kind.as_str()),
            })
            .collect();
        resources.sort_by(|a, b| a.path.cmp(&b.path));
        Some(RawCanon {
            name: String::from(self.name.as_str()),
            resources,
            relations,
        })
    }
}

fn labels_text(set: &BTreeSet<Name>) -> Vec<String> {
    set.iter().map(|k| String::from(k.as_str())).collect()
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

fn raw_spec_v2(r: &Requirement) -> Option<RawSpec> {
    let (kind, state, digest) = match r {
        Requirement::File(FileCondition::Absent) => ("file", "absent", None),
        Requirement::File(FileCondition::Present { content, metadata }) => {
            if *metadata != Metadata::any() {
                return None;
            }
            match content {
                Content::Any => ("file", "present-any", None),
                Content::Exactly(d) => ("file", "present-exact", Some(hex(d))),
            }
        }
        Requirement::Service(ServiceRequirement::Running) => ("service", "running", None),
        Requirement::Service(ServiceRequirement::Loaded(d)) => ("service", "loaded", Some(hex(d))),
        _ => return None,
    };
    Some(RawSpec {
        kind: String::from(kind),
        state: String::from(state),
        digest,
    })
}

fn put(map: &mut BTreeMap<String, String>, key: &str, value: &str) {
    map.insert(String::from(key), String::from(value));
}

fn metadata_fields(map: &mut BTreeMap<String, String>, m: &Metadata) {
    if let Some(owner) = &m.owner {
        put(map, "owner", owner.as_str());
    }
    if let Some(group) = &m.group {
        put(map, "group", group.as_str());
    }
    if let Some(mode) = m.mode {
        put(map, "mode", &format!("{mode}"));
    }
}

/// The schema-3 `spec` fields of a requirement: a field is present exactly
/// when the requirement states it (canon-ir.md, Schema Version 3).
fn spec_fields(r: &Requirement) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    match r {
        Requirement::File(FileCondition::Absent)
        | Requirement::Directory(DirectoryCondition::Absent)
        | Requirement::User(UserCondition::Absent)
        | Requirement::Package(PackageCondition::Absent) => put(&mut m, "state", "absent"),
        Requirement::File(FileCondition::Present { content, metadata }) => {
            put(&mut m, "state", "present");
            match content {
                Content::Any => put(&mut m, "content", "any"),
                Content::Exactly(d) => put(&mut m, "content", &hex(d)),
            }
            metadata_fields(&mut m, metadata);
        }
        Requirement::Directory(DirectoryCondition::Present { metadata }) => {
            put(&mut m, "state", "present");
            metadata_fields(&mut m, metadata);
        }
        Requirement::Service(ServiceRequirement::Running) => put(&mut m, "state", "running"),
        Requirement::Service(ServiceRequirement::Loaded(d)) => {
            put(&mut m, "state", "loaded");
            put(&mut m, "digest", &hex(d));
        }
        Requirement::Unit(u) => {
            put(
                &mut m,
                "active",
                match u.activity {
                    Activity::Active => "active",
                    Activity::Inactive => "inactive",
                    Activity::Any => "any",
                },
            );
            put(
                &mut m,
                "enabled",
                match u.enablement {
                    Enablement::Enabled => "enabled",
                    Enablement::Disabled => "disabled",
                    Enablement::Any => "any",
                },
            );
        }
        Requirement::Sysctl(c) => put(&mut m, "value", c.value.as_str()),
        Requirement::User(UserCondition::Present { class, home, shell }) => {
            put(&mut m, "state", "present");
            put(
                &mut m,
                "class",
                match class {
                    AccountClass::System => "system",
                    AccountClass::Regular => "regular",
                },
            );
            if let Some(h) = home {
                put(&mut m, "home", h.as_str());
            }
            if let Some(sh) = shell {
                put(&mut m, "shell", sh.as_str());
            }
        }
        Requirement::Package(PackageCondition::Installed { version }) => {
            put(&mut m, "state", "installed");
            if let Some(v) = version {
                put(&mut m, "version", v.as_str());
            }
        }
    }
    m
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
    /// Two resources have one key; in schema 2, one path.
    DuplicatePath {
        /// The second resource's position.
        resource: usize,
    },
    /// A resource's name is not valid for its family.
    InvalidResourceName {
        /// The resource's position.
        resource: usize,
    },
    /// A `spec` field has a value its family does not allow, or a field the
    /// family requires is missing.
    InvalidField {
        /// The resource's position.
        resource: usize,
    },
    /// A resource writes a property an earlier resource writes: a file and a
    /// directory at one path, for example (resource-families.md, Admission).
    Conflict {
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
                write!(f, "resource {resource}: a second resource with one key")
            }
            CanonError::InvalidResourceName { resource } => {
                write!(f, "resource {resource}: invalid name for its family")
            }
            CanonError::InvalidField { resource } => {
                write!(f, "resource {resource}: invalid or missing spec field")
            }
            CanonError::Conflict { resource } => {
                write!(
                    f,
                    "resource {resource}: writes a property another resource writes"
                )
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
        ("file", "present-any") => {
            no_digest(Requirement::File(FileCondition::present(Content::Any)))
        }
        ("file", "present-exact") => Ok(Requirement::File(FileCondition::present(
            Content::Exactly(digest(&spec.digest, resource)?),
        ))),
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

/// One resource after its schema's reading of `spec`, before the structural
/// checks every schema shares.
struct Item<'a> {
    key: ResourceKey,
    requirement: Requirement,
    keys: &'a [String],
    disrupts: &'a [String],
}

/// The structural validation every Canon passes, whatever schema it came
/// from: labels, one resource per key, no two resources writing one
/// property, relations that name resources of the Canon, no self relation,
/// and no cycle. `end` resolves a relation's end text to a key.
fn assemble(
    name: &str,
    items: Vec<Item<'_>>,
    relations: &[RawRelation],
    end: impl Fn(&str, &BTreeMap<ResourceKey, Resource>) -> Option<ResourceKey>,
) -> Result<Canon, CanonError> {
    let name = canon_name(name).ok_or(CanonError::InvalidName)?;
    let mut resources = BTreeMap::new();
    let mut written: BTreeSet<Property> = BTreeSet::new();
    for (i, item) in items.into_iter().enumerate() {
        let labels = |list: &[String]| -> Result<BTreeSet<Name>, CanonError> {
            list.iter()
                .map(|k| label(k).ok_or(CanonError::InvalidLabel { resource: i }))
                .collect()
        };
        let resource = Resource {
            requirement: item.requirement,
            keys: labels(item.keys)?,
            disrupts: labels(item.disrupts)?,
        };
        if resources.contains_key(&item.key) {
            return Err(CanonError::DuplicatePath { resource: i });
        }
        if !written.insert(Property::written(&item.key)) {
            return Err(CanonError::Conflict { resource: i });
        }
        resources.insert(item.key, resource);
    }
    let mut set = BTreeSet::new();
    for (i, rel) in relations.iter().enumerate() {
        let kind = relation_kind(&rel.kind).ok_or(CanonError::UnknownRelation { relation: i })?;
        let dangling = CanonError::DanglingRelation { relation: i };
        let source = end(&rel.source, &resources).ok_or(dangling)?;
        let target = end(&rel.target, &resources).ok_or(dangling)?;
        if source == target {
            return Err(CanonError::SelfRelation { relation: i });
        }
        set.insert(Relation {
            source,
            target,
            kind,
        });
    }
    if has_cycle(&resources, &set) {
        return Err(CanonError::Cycle);
    }
    Ok(Canon {
        name,
        resources,
        relations: set,
    })
}

impl TryFrom<RawCanon> for Canon {
    type Error = CanonError;

    /// Schema 2's reading: a path and a `spec` of `kind`, `state`, and
    /// `digest`; then the shared structural validation. A relation's ends
    /// are paths, each naming the one resource at that path.
    fn try_from(raw: RawCanon) -> Result<Self, CanonError> {
        canon_name(&raw.name).ok_or(CanonError::InvalidName)?;
        let mut items = Vec::new();
        let mut paths = BTreeSet::new();
        for (i, r) in raw.resources.iter().enumerate() {
            if r.path.len() > MAX_PATH {
                return Err(CanonError::InvalidPath { resource: i });
            }
            let path =
                ResourcePath::new(&r.path).map_err(|_| CanonError::InvalidPath { resource: i })?;
            let requirement = requirement(&r.spec, i)?;
            if !paths.insert(path.clone()) {
                return Err(CanonError::DuplicatePath { resource: i });
            }
            let key = match requirement {
                Requirement::Service(_) => ResourceKey::Service(path),
                _ => ResourceKey::File(path),
            };
            items.push(Item {
                key,
                requirement,
                keys: &r.keys,
                disrupts: &r.disrupts,
            });
        }
        assemble(&raw.name, items, &raw.relations, |text, resources| {
            let path = ResourcePath::new(text).ok()?;
            resources.keys().find(|k| k.path() == Some(&path)).cloned()
        })
    }
}

impl TryFrom<RawCanon3> for Canon {
    type Error = CanonError;

    /// Schema 3's reading: a family, a name valid for it, and the family's
    /// `spec` fields; then the shared structural validation. A relation's
    /// ends are keys in their text form.
    fn try_from(raw: RawCanon3) -> Result<Self, CanonError> {
        canon_name(&raw.name).ok_or(CanonError::InvalidName)?;
        let mut items = Vec::new();
        for (i, r) in raw.resources.iter().enumerate() {
            let family =
                Family::from_name(&r.kind).ok_or(CanonError::UnknownRequirement { resource: i })?;
            if family.is_path() && r.name.len() > MAX_PATH {
                return Err(CanonError::InvalidPath { resource: i });
            }
            let key = ResourceKey::from_parts(family, &r.name).map_err(|_| {
                if family.is_path() {
                    CanonError::InvalidPath { resource: i }
                } else {
                    CanonError::InvalidResourceName { resource: i }
                }
            })?;
            if family == Family::Directory && r.name == "/" {
                return Err(CanonError::InvalidPath { resource: i });
            }
            let requirement = requirement_v3(family, &r.spec, i)?;
            items.push(Item {
                key,
                requirement,
                keys: &r.keys,
                disrupts: &r.disrupts,
            });
        }
        assemble(&raw.name, items, &raw.relations, |text, resources| {
            ResourceKey::parse(text)
                .ok()
                .filter(|k| resources.contains_key(k))
        })
    }
}

/// Reads one field set. `allowed` are the fields the family defines for the
/// state; any other field is an unknown requirement.
struct Spec<'a> {
    fields: &'a BTreeMap<String, String>,
    resource: usize,
}

impl<'a> Spec<'a> {
    fn only(&self, allowed: &[&str]) -> Result<(), CanonError> {
        if self.fields.keys().all(|k| allowed.contains(&k.as_str())) {
            Ok(())
        } else {
            Err(CanonError::UnknownRequirement {
                resource: self.resource,
            })
        }
    }

    fn get(&self, field: &str) -> Option<&'a str> {
        self.fields.get(field).map(String::as_str)
    }

    fn required(&self, field: &str) -> Result<&'a str, CanonError> {
        self.get(field).ok_or(CanonError::InvalidField {
            resource: self.resource,
        })
    }

    fn invalid(&self) -> CanonError {
        CanonError::InvalidField {
            resource: self.resource,
        }
    }

    fn account(&self, field: &str) -> Result<Option<AccountName>, CanonError> {
        self.get(field)
            .map(|t| AccountName::new(t).map_err(|_| self.invalid()))
            .transpose()
    }

    fn path(&self, field: &str) -> Result<Option<ResourcePath>, CanonError> {
        self.get(field)
            .map(|t| {
                if t.len() > MAX_PATH {
                    return Err(self.invalid());
                }
                ResourcePath::new(t).map_err(|_| self.invalid())
            })
            .transpose()
    }

    fn metadata(&self) -> Result<Metadata, CanonError> {
        Ok(Metadata {
            owner: self.account("owner")?,
            group: self.account("group")?,
            mode: self
                .get("mode")
                .map(|t| Mode::from_octal(t).ok_or(self.invalid()))
                .transpose()?,
        })
    }
}

fn requirement_v3(
    family: Family,
    fields: &BTreeMap<String, String>,
    resource: usize,
) -> Result<Requirement, CanonError> {
    let spec = Spec { fields, resource };
    let unknown = CanonError::UnknownRequirement { resource };
    let state = || spec.get("state").ok_or(unknown);
    Ok(match family {
        Family::File => match state()? {
            "absent" => {
                spec.only(&["state"])?;
                Requirement::File(FileCondition::Absent)
            }
            "present" => {
                spec.only(&["state", "content", "owner", "group", "mode"])?;
                let content = match spec.required("content")? {
                    "any" => Content::Any,
                    text => Content::Exactly(digest(&Some(String::from(text)), resource)?),
                };
                Requirement::File(FileCondition::Present {
                    content,
                    metadata: spec.metadata()?,
                })
            }
            _ => return Err(unknown),
        },
        Family::Directory => match state()? {
            "absent" => {
                spec.only(&["state"])?;
                Requirement::Directory(DirectoryCondition::Absent)
            }
            "present" => {
                spec.only(&["state", "owner", "group", "mode"])?;
                Requirement::Directory(DirectoryCondition::Present {
                    metadata: spec.metadata()?,
                })
            }
            _ => return Err(unknown),
        },
        Family::Service => match state()? {
            "running" => {
                spec.only(&["state"])?;
                Requirement::Service(ServiceRequirement::Running)
            }
            "loaded" => {
                spec.only(&["state", "digest"])?;
                Requirement::Service(ServiceRequirement::Loaded(digest(
                    &spec.get("digest").map(String::from),
                    resource,
                )?))
            }
            _ => return Err(unknown),
        },
        Family::Unit => {
            spec.only(&["active", "enabled"])?;
            let activity = match spec.required("active")? {
                "active" => Activity::Active,
                "inactive" => Activity::Inactive,
                "any" => Activity::Any,
                _ => return Err(spec.invalid()),
            };
            let enablement = match spec.required("enabled")? {
                "enabled" => Enablement::Enabled,
                "disabled" => Enablement::Disabled,
                "any" => Enablement::Any,
                _ => return Err(spec.invalid()),
            };
            Requirement::Unit(UnitCondition {
                activity,
                enablement,
            })
        }
        Family::Sysctl => {
            spec.only(&["value"])?;
            let value = spec.required("value")?;
            if value.is_empty() || !SysctlValue::is_normal(value) {
                return Err(spec.invalid());
            }
            Requirement::Sysctl(SysctlCondition {
                value: SysctlValue::normalized(value),
            })
        }
        Family::User => match state()? {
            "absent" => {
                spec.only(&["state"])?;
                Requirement::User(UserCondition::Absent)
            }
            "present" => {
                spec.only(&["state", "class", "home", "shell"])?;
                let class = match spec.required("class")? {
                    "system" => AccountClass::System,
                    "regular" => AccountClass::Regular,
                    _ => return Err(spec.invalid()),
                };
                Requirement::User(UserCondition::Present {
                    class,
                    home: spec.path("home")?,
                    shell: spec.path("shell")?,
                })
            }
            _ => return Err(unknown),
        },
        Family::Package => match state()? {
            "absent" => {
                spec.only(&["state"])?;
                Requirement::Package(PackageCondition::Absent)
            }
            "installed" => {
                spec.only(&["state", "version"])?;
                let version = spec
                    .get("version")
                    .map(|t| PackageVersion::new(t).ok_or(spec.invalid()))
                    .transpose()?;
                Requirement::Package(PackageCondition::Installed { version })
            }
            _ => return Err(unknown),
        },
    })
}

/// Kahn's algorithm over every relation kind: a cycle leaves vertices
/// with remaining in-degree.
fn has_cycle(resources: &BTreeMap<ResourceKey, Resource>, relations: &BTreeSet<Relation>) -> bool {
    let mut indegree: BTreeMap<&ResourceKey, usize> = resources.keys().map(|p| (p, 0)).collect();
    for r in relations {
        if let Some(d) = indegree.get_mut(&r.target) {
            *d += 1;
        }
    }
    let mut ready: Vec<&ResourceKey> = indegree
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

/// Builds a Canon from typed parts. It writes a [`RawCanon3`] and validates
/// it with the same validation the decoder uses, so the two cannot disagree
/// about what a Canon is.
#[derive(Debug, Clone, Default)]
pub struct CanonBuilder {
    raw: RawCanon3,
}

impl CanonBuilder {
    /// A Canon named `name`.
    pub fn new(name: &str) -> Self {
        CanonBuilder {
            raw: RawCanon3 {
                name: String::from(name),
                ..RawCanon3::default()
            },
        }
    }

    /// Adds the resource named `name` in `requirement`'s family, with
    /// conflict `keys` and disrupted `nodes`.
    pub fn resource(
        mut self,
        name: &str,
        requirement: Requirement,
        keys: &[&str],
        nodes: &[&str],
    ) -> Self {
        self.raw.resources.push(RawResource3 {
            kind: String::from(requirement.kind().as_str()),
            name: String::from(name),
            spec: spec_fields(&requirement),
            keys: keys.iter().map(|k| String::from(*k)).collect(),
            disrupts: nodes.iter().map(|k| String::from(*k)).collect(),
        });
        self
    }

    /// Adds a file with no conflict keys and no disruption.
    pub fn file(self, path: &str, requirement: FileCondition) -> Self {
        self.resource(path, Requirement::File(requirement), &[], &[])
    }

    /// Adds a directory with no conflict keys and no disruption.
    pub fn directory(self, path: &str, requirement: DirectoryCondition) -> Self {
        self.resource(path, Requirement::Directory(requirement), &[], &[])
    }

    /// Adds a service with no conflict keys and no disruption.
    pub fn service(self, path: &str, requirement: ServiceRequirement) -> Self {
        self.resource(path, Requirement::Service(requirement), &[], &[])
    }

    /// Adds a unit with no conflict keys and no disruption.
    pub fn unit(self, name: &str, requirement: UnitCondition) -> Self {
        self.resource(name, Requirement::Unit(requirement), &[], &[])
    }

    /// Adds a kernel parameter with this value, normalized.
    pub fn sysctl(self, name: &str, value: &str) -> Self {
        let requirement = Requirement::Sysctl(SysctlCondition {
            value: SysctlValue::normalized(value),
        });
        self.resource(name, requirement, &[], &[])
    }

    /// Adds an account with no conflict keys and no disruption.
    pub fn user(self, name: &str, requirement: UserCondition) -> Self {
        self.resource(name, Requirement::User(requirement), &[], &[])
    }

    /// Adds a package with no conflict keys and no disruption.
    pub fn package(self, name: &str, requirement: PackageCondition) -> Self {
        self.resource(name, Requirement::Package(requirement), &[], &[])
    }

    /// Relates `source` to `target`. Each end is a key's text form,
    /// `<family>:<name>`, or a path, which names the resource already added
    /// at that path.
    pub fn relate(mut self, source: &str, kind: RelationKind, target: &str) -> Self {
        let end = |text: &str| -> String {
            if text.starts_with('/') {
                let found = self.raw.resources.iter().find(|r| {
                    r.name == text && Family::from_name(&r.kind).is_some_and(|f| f.is_path())
                });
                match found {
                    Some(r) => format!("{}:{}", r.kind, r.name),
                    None => format!("file:{text}"),
                }
            } else {
                String::from(text)
            }
        };
        let relation = RawRelation {
            source: end(source),
            target: end(target),
            kind: String::from(kind.as_str()),
        };
        self.raw.relations.push(relation);
        self
    }

    /// Validates and returns the Canon.
    pub fn build(self) -> Result<Canon, CanonError> {
        Canon::try_from(self.raw)
    }
}
