//! The kernel's inputs as bytes, for the Cell's durable journal (ADR 0017
//! §3).
//!
//! The journal holds inputs, not Events: `step` recomputes the Events and
//! the snapshot, and an Observation read back is verified again, so nothing
//! here has to rebuild a `Verified`. Each input is a value of the Canon IR's
//! data model, written in deterministic CBOR, inside a two-element array
//! whose first element is [`VERSION`]. The encoding is positional: every
//! structure is an array led by a text tag, and every set is written in its
//! order, so one input has one encoding. A byte string that does not decode
//! to an input, whatever the reason, decodes to `None`, and the journal's
//! reader fails loudly on it.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use nomos_canon::cbor;
use nomos_canon::value::{MAX_UINT, Value};
use nomos_core::condition::{
    AccountClass, Activity, Condition, Content, DirectoryCondition, Enablement, FileCondition,
    Metadata, PackageCondition, PackageVersion, Requirement, SysctlCondition, SysctlValue,
    UnitCondition, UserCondition,
};
use nomos_core::effect::{EffectKey, Receipt};
use nomos_core::observation::{
    Account, ActiveState, Collection, CollectionFailure, CollectorId, DirectoryEvidence, Evidence,
    FileEvidence, Instant, Observation, ObservedMetadata, PackageEvidence, Provenance,
    SysctlEvidence, UnitEvidence, UnitFileState, UserEvidence, Window,
};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{AccountName, Digest, Mode, ResourceKey, ResourcePath};
use nomos_warp::budget::{Budget, Node};
use nomos_warp::graph::{ConflictKey, Edge, EdgeKind};

use crate::kernel::{Canon, Input, Managed, Plan, Policy};

/// The version of the encoding. A journal written with another version is
/// not read.
pub const VERSION: u64 = 1;

/// `input` as bytes.
pub fn encode(input: &Input) -> Vec<u8> {
    cbor::encode(&Value::Array(vec![Value::Uint(VERSION), input.to_value()]))
}

/// How deep a journaled input nests: an input holds a Plan, which holds a
/// Canon, whose Conditions hold requirements with metadata.
const MAX_DEPTH: usize = 32;

/// The input `bytes` encode, or `None`.
pub fn decode(bytes: &[u8]) -> Option<Input> {
    let value = cbor::decode_nested(bytes, MAX_DEPTH).ok()?;
    let items = array(&value)?;
    match items {
        [Value::Uint(VERSION), input] => Input::from_value(input),
        _ => None,
    }
}

/// A type that is a value of the IR's data model and back.
trait Codec: Sized {
    fn to_value(&self) -> Value;
    fn from_value(value: &Value) -> Option<Self>;
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn tagged(tag: &str, mut fields: Vec<Value>) -> Value {
    fields.insert(0, text(tag));
    Value::Array(fields)
}

fn array(value: &Value) -> Option<&[Value]> {
    match value {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

fn as_text(value: &Value) -> Option<&str> {
    match value {
        Value::Text(s) => Some(s),
        _ => None,
    }
}

/// A 64-bit integer. The IR's data model holds integers up to
/// [`MAX_UINT`], so a larger one, such as a monotonic instant after about
/// 104 days of uptime, is written as its decimal text; a value has one
/// encoding either way.
fn uint(n: u64) -> Value {
    if n <= MAX_UINT {
        Value::Uint(n)
    } else {
        Value::Text(n.to_string())
    }
}

fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Uint(n) => Some(*n),
        Value::Text(t) => {
            let n: u64 = t.parse().ok()?;
            (n > MAX_UINT && n.to_string() == *t).then_some(n)
        }
        _ => None,
    }
}

/// The tag and fields of a tagged array.
fn untag(value: &Value) -> Option<(&str, &[Value])> {
    let (tag, fields) = array(value)?.split_first()?;
    Some((as_text(tag)?, fields))
}

fn list<T: Codec>(items: impl IntoIterator<Item = T>) -> Value {
    Value::Array(items.into_iter().map(|i| i.to_value()).collect())
}

fn from_list<T: Codec>(value: &Value) -> Option<Vec<T>> {
    array(value)?.iter().map(T::from_value).collect()
}

fn from_set<T: Codec + Ord>(value: &Value) -> Option<BTreeSet<T>> {
    Some(from_list::<T>(value)?.into_iter().collect())
}

fn option<T: Codec>(value: &Option<T>) -> Value {
    match value {
        None => Value::Array(Vec::new()),
        Some(v) => Value::Array(vec![v.to_value()]),
    }
}

fn from_option<T: Codec>(value: &Value) -> Option<Option<T>> {
    match array(value)? {
        [] => Some(None),
        [v] => Some(Some(T::from_value(v)?)),
        _ => None,
    }
}

fn boolean(b: bool) -> Value {
    Value::Uint(u64::from(b))
}

fn from_boolean(value: &Value) -> Option<bool> {
    match as_u64(value)? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

impl Codec for u64 {
    fn to_value(&self) -> Value {
        uint(*self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        as_u64(value)
    }
}

impl Codec for u32 {
    fn to_value(&self) -> Value {
        uint(u64::from(*self))
    }
    fn from_value(value: &Value) -> Option<Self> {
        u32::try_from(as_u64(value)?).ok()
    }
}

impl Codec for usize {
    fn to_value(&self) -> Value {
        uint(*self as u64)
    }
    fn from_value(value: &Value) -> Option<Self> {
        usize::try_from(as_u64(value)?).ok()
    }
}

impl Codec for Instant {
    fn to_value(&self) -> Value {
        uint(self.0)
    }
    fn from_value(value: &Value) -> Option<Self> {
        Some(Instant(as_u64(value)?))
    }
}

impl Codec for Digest {
    fn to_value(&self) -> Value {
        Value::Text(
            self.as_bytes()
                .iter()
                .map(|b| alloc::format!("{b:02x}"))
                .collect(),
        )
    }
    fn from_value(value: &Value) -> Option<Self> {
        Digest::from_hex(as_text(value)?).ok()
    }
}

macro_rules! textual {
    ($ty:ty, $new:expr) => {
        impl Codec for $ty {
            fn to_value(&self) -> Value {
                text(self.as_str())
            }
            fn from_value(value: &Value) -> Option<Self> {
                $new(as_text(value)?)
            }
        }
    };
}

textual!(ResourcePath, |t| ResourcePath::new(t).ok());
textual!(AccountName, |t| AccountName::new(t).ok());
textual!(PackageVersion, PackageVersion::new);
textual!(PlanId, PlanId::new);
textual!(CollectorId, CollectorId::new);
textual!(ConflictKey, ConflictKey::new);
textual!(Node, Node::new);

impl Codec for SysctlValue {
    fn to_value(&self) -> Value {
        text(self.as_str())
    }
    fn from_value(value: &Value) -> Option<Self> {
        let t = as_text(value)?;
        SysctlValue::is_normal(t).then(|| SysctlValue::normalized(t))
    }
}

impl Codec for Mode {
    fn to_value(&self) -> Value {
        Value::Uint(u64::from(self.bits()))
    }
    fn from_value(value: &Value) -> Option<Self> {
        Mode::new(u16::try_from(as_u64(value)?).ok()?)
    }
}

impl Codec for ResourceKey {
    fn to_value(&self) -> Value {
        text(&alloc::format!("{self}"))
    }
    fn from_value(value: &Value) -> Option<Self> {
        ResourceKey::parse(as_text(value)?).ok()
    }
}

/// A closed set of unit variants, by name.
macro_rules! named {
    ($ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl Codec for $ty {
            fn to_value(&self) -> Value {
                text(match self { $(<$ty>::$variant => $name),+ })
            }
            fn from_value(value: &Value) -> Option<Self> {
                match as_text(value)? { $($name => Some(<$ty>::$variant),)+ _ => None }
            }
        }
    };
}

named!(Activity { Active => "active", Inactive => "inactive", Any => "any" });
named!(Enablement { Enabled => "enabled", Disabled => "disabled", Any => "any" });
named!(AccountClass { System => "system", Regular => "regular" });
named!(EdgeKind { Requires => "requires", After => "after", OnChange => "on_change" });
named!(ActiveState {
    Active => "active",
    Reloading => "reloading",
    Inactive => "inactive",
    Failed => "failed",
    Activating => "activating",
    Deactivating => "deactivating",
});
named!(UnitFileState {
    Enabled => "enabled",
    Disabled => "disabled",
    Static => "static",
    Masked => "masked",
    Other => "other",
});
named!(CollectionFailure {
    PermissionDenied => "permission-denied",
    TimedOut => "timed-out",
    Unsupported => "unsupported",
    Io => "io",
    Unavailable => "unavailable",
});

impl Codec for Metadata {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            option(&self.owner),
            option(&self.group),
            option(&self.mode),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [o, g, m] => Some(Metadata {
                owner: from_option(o)?,
                group: from_option(g)?,
                mode: from_option(m)?,
            }),
            _ => None,
        }
    }
}

impl Codec for Content {
    fn to_value(&self) -> Value {
        match self {
            Content::Any => tagged("any", vec![]),
            Content::Exactly(d) => tagged("exactly", vec![d.to_value()]),
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("any", []) => Some(Content::Any),
            ("exactly", [d]) => Some(Content::Exactly(Digest::from_value(d)?)),
            _ => None,
        }
    }
}

impl Codec for FileCondition {
    fn to_value(&self) -> Value {
        match self {
            FileCondition::Absent => tagged("absent", vec![]),
            FileCondition::Present { content, metadata } => {
                tagged("present", vec![content.to_value(), metadata.to_value()])
            }
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("absent", []) => Some(FileCondition::Absent),
            ("present", [c, m]) => Some(FileCondition::Present {
                content: Content::from_value(c)?,
                metadata: Metadata::from_value(m)?,
            }),
            _ => None,
        }
    }
}

impl Codec for Requirement {
    fn to_value(&self) -> Value {
        match self {
            Requirement::File(c) => tagged("file", vec![c.to_value()]),
            Requirement::Service(c) => tagged("service", vec![c.to_value()]),
            Requirement::Directory(DirectoryCondition::Absent) => {
                tagged("directory", vec![tagged("absent", vec![])])
            }
            Requirement::Directory(DirectoryCondition::Present { metadata }) => tagged(
                "directory",
                vec![tagged("present", vec![metadata.to_value()])],
            ),
            Requirement::Unit(u) => {
                tagged("unit", vec![u.activity.to_value(), u.enablement.to_value()])
            }
            Requirement::Sysctl(s) => tagged("sysctl", vec![s.value.to_value()]),
            Requirement::User(UserCondition::Absent) => {
                tagged("user", vec![tagged("absent", vec![])])
            }
            Requirement::User(UserCondition::Present { class, home, shell }) => tagged(
                "user",
                vec![tagged(
                    "present",
                    vec![class.to_value(), option(home), option(shell)],
                )],
            ),
            Requirement::Package(PackageCondition::Absent) => {
                tagged("package", vec![tagged("absent", vec![])])
            }
            Requirement::Package(PackageCondition::Installed { version }) => {
                tagged("package", vec![tagged("installed", vec![option(version)])])
            }
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        Some(match untag(value)? {
            ("file", [c]) => Requirement::File(FileCondition::from_value(c)?),
            ("service", [c]) => Requirement::Service(FileCondition::from_value(c)?),
            ("directory", [d]) => Requirement::Directory(match untag(d)? {
                ("absent", []) => DirectoryCondition::Absent,
                ("present", [m]) => DirectoryCondition::Present {
                    metadata: Metadata::from_value(m)?,
                },
                _ => return None,
            }),
            ("unit", [a, e]) => Requirement::Unit(UnitCondition {
                activity: Activity::from_value(a)?,
                enablement: Enablement::from_value(e)?,
            }),
            ("sysctl", [v]) => Requirement::Sysctl(SysctlCondition {
                value: SysctlValue::from_value(v)?,
            }),
            ("user", [u]) => Requirement::User(match untag(u)? {
                ("absent", []) => UserCondition::Absent,
                ("present", [c, h, s]) => UserCondition::Present {
                    class: AccountClass::from_value(c)?,
                    home: from_option(h)?,
                    shell: from_option(s)?,
                },
                _ => return None,
            }),
            ("package", [p]) => Requirement::Package(match untag(p)? {
                ("absent", []) => PackageCondition::Absent,
                ("installed", [v]) => PackageCondition::Installed {
                    version: from_option(v)?,
                },
                _ => return None,
            }),
            _ => return None,
        })
    }
}

impl Codec for Managed {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.condition.key().to_value(),
            self.condition.requirement().to_value(),
            list(self.keys.iter().cloned()),
            list(self.disrupts.iter().cloned()),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [k, r, keys, nodes] => Some(Managed {
                condition: Condition::new(
                    ResourceKey::from_value(k)?,
                    Requirement::from_value(r)?,
                )?,
                keys: from_set(keys)?,
                disrupts: from_set(nodes)?,
            }),
            _ => None,
        }
    }
}

impl Codec for Edge {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.source().to_value(),
            self.target().to_value(),
            self.kind().to_value(),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [s, t, k] => Some(Edge::new(
                ResourceKey::from_value(s)?,
                ResourceKey::from_value(t)?,
                EdgeKind::from_value(k)?,
            )),
            _ => None,
        }
    }
}

impl Codec for Canon {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            list(self.resources().values().cloned()),
            list(self.edges().iter().cloned()),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [r, e] => Some(Canon::new(from_list(r)?, from_list(e)?)),
            _ => None,
        }
    }
}

impl Codec for Budget {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            list(self.members().iter().cloned()),
            self.limit().to_value(),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [m, l] => Some(Budget::new(from_set(m)?, usize::from_value(l)?)),
            _ => None,
        }
    }
}

impl Codec for Policy {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.capacity.to_value(),
            self.receipt_timeout.to_value(),
            self.verify_timeout.to_value(),
            self.settle_after.to_value(),
            list(self.budgets.iter().cloned()),
            list(self.unavailable.iter().cloned()),
            self.budget_observed_at.to_value(),
            self.budget_max_age.to_value(),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [c, r, v, s, b, u, o, a] => Some(Policy {
                capacity: usize::from_value(c)?,
                receipt_timeout: u64::from_value(r)?,
                verify_timeout: u64::from_value(v)?,
                settle_after: u64::from_value(s)?,
                budgets: from_list(b)?,
                unavailable: from_set(u)?,
                budget_observed_at: Instant::from_value(o)?,
                budget_max_age: u64::from_value(a)?,
            }),
            _ => None,
        }
    }
}

impl Codec for Plan {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.id.to_value(),
            self.generation.0.to_value(),
            self.canon.to_value(),
            self.bound.to_value(),
            self.policy.to_value(),
            option(&self.expires),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [i, g, c, b, p, e] => Some(Plan {
                id: PlanId::from_value(i)?,
                generation: Generation(u64::from_value(g)?),
                canon: Canon::from_value(c)?,
                bound: u32::from_value(b)?,
                policy: Policy::from_value(p)?,
                expires: from_option(e)?,
            }),
            _ => None,
        }
    }
}

impl Codec for Account {
    fn to_value(&self) -> Value {
        match self {
            Account::Named(n) => tagged("named", vec![n.to_value()]),
            Account::Id(id) => tagged("id", vec![id.to_value()]),
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("named", [n]) => Some(Account::Named(AccountName::from_value(n)?)),
            ("id", [i]) => Some(Account::Id(u32::from_value(i)?)),
            _ => None,
        }
    }
}

impl Codec for ObservedMetadata {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.owner.to_value(),
            self.group.to_value(),
            self.mode.to_value(),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [o, g, m] => Some(ObservedMetadata {
                owner: Account::from_value(o)?,
                group: Account::from_value(g)?,
                mode: Mode::from_value(m)?,
            }),
            _ => None,
        }
    }
}

impl Codec for FileEvidence {
    fn to_value(&self) -> Value {
        match self {
            FileEvidence::Absent => tagged("absent", vec![]),
            FileEvidence::Present {
                digest,
                size,
                metadata,
            } => tagged(
                "present",
                vec![digest.to_value(), size.to_value(), metadata.to_value()],
            ),
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("absent", []) => Some(FileEvidence::Absent),
            ("present", [d, s, m]) => Some(FileEvidence::Present {
                digest: Digest::from_value(d)?,
                size: u64::from_value(s)?,
                metadata: ObservedMetadata::from_value(m)?,
            }),
            _ => None,
        }
    }
}

impl Codec for Evidence {
    fn to_value(&self) -> Value {
        match self {
            Evidence::File(f) => tagged("file", vec![f.to_value()]),
            Evidence::Service(f) => tagged("service", vec![f.to_value()]),
            Evidence::Directory(DirectoryEvidence::Absent) => {
                tagged("directory", vec![tagged("absent", vec![])])
            }
            Evidence::Directory(DirectoryEvidence::Present { metadata }) => tagged(
                "directory",
                vec![tagged("present", vec![metadata.to_value()])],
            ),
            Evidence::Unit(u) => tagged("unit", vec![u.active.to_value(), u.file_state.to_value()]),
            Evidence::Sysctl(s) => tagged("sysctl", vec![s.value.to_value()]),
            Evidence::User(UserEvidence::Absent) => tagged("user", vec![tagged("absent", vec![])]),
            Evidence::User(UserEvidence::Present {
                uid,
                gid,
                home,
                shell,
            }) => tagged(
                "user",
                vec![tagged(
                    "present",
                    vec![uid.to_value(), gid.to_value(), text(home), text(shell)],
                )],
            ),
            Evidence::Package(PackageEvidence::NotInstalled) => {
                tagged("package", vec![tagged("not-installed", vec![])])
            }
            Evidence::Package(PackageEvidence::Installed { version }) => tagged(
                "package",
                vec![tagged("installed", vec![version.to_value()])],
            ),
            Evidence::Package(PackageEvidence::Broken) => {
                tagged("package", vec![tagged("broken", vec![])])
            }
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        Some(match untag(value)? {
            ("file", [f]) => Evidence::File(FileEvidence::from_value(f)?),
            ("service", [f]) => Evidence::Service(FileEvidence::from_value(f)?),
            ("directory", [d]) => Evidence::Directory(match untag(d)? {
                ("absent", []) => DirectoryEvidence::Absent,
                ("present", [m]) => DirectoryEvidence::Present {
                    metadata: ObservedMetadata::from_value(m)?,
                },
                _ => return None,
            }),
            ("unit", [a, f]) => Evidence::Unit(UnitEvidence {
                active: ActiveState::from_value(a)?,
                file_state: UnitFileState::from_value(f)?,
            }),
            ("sysctl", [v]) => Evidence::Sysctl(SysctlEvidence {
                value: SysctlValue::from_value(v)?,
            }),
            ("user", [u]) => Evidence::User(match untag(u)? {
                ("absent", []) => UserEvidence::Absent,
                ("present", [uid, gid, h, s]) => UserEvidence::Present {
                    uid: u32::from_value(uid)?,
                    gid: u32::from_value(gid)?,
                    home: String::from(as_text(h)?),
                    shell: String::from(as_text(s)?),
                },
                _ => return None,
            }),
            ("package", [p]) => Evidence::Package(match untag(p)? {
                ("not-installed", []) => PackageEvidence::NotInstalled,
                ("installed", [v]) => PackageEvidence::Installed {
                    version: PackageVersion::from_value(v)?,
                },
                ("broken", []) => PackageEvidence::Broken,
                _ => return None,
            }),
            _ => return None,
        })
    }
}

impl Codec for Collection {
    fn to_value(&self) -> Value {
        match self {
            Collection::Collected(e) => tagged("collected", vec![e.to_value()]),
            Collection::Failed(f) => tagged("failed", vec![f.to_value()]),
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("collected", [e]) => Some(Collection::Collected(Evidence::from_value(e)?)),
            ("failed", [f]) => Some(Collection::Failed(CollectionFailure::from_value(f)?)),
            _ => None,
        }
    }
}

impl Codec for Observation {
    fn to_value(&self) -> Value {
        let p = self.provenance();
        Value::Array(vec![
            self.key().to_value(),
            self.collection().to_value(),
            p.collector().to_value(),
            p.window().start().to_value(),
            p.window().end().to_value(),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [k, c, who, start, end] => {
                let window =
                    Window::new(Instant::from_value(start)?, Instant::from_value(end)?).ok()?;
                Observation::new(
                    ResourceKey::from_value(k)?,
                    Collection::from_value(c)?,
                    Provenance::new(CollectorId::from_value(who)?, window),
                )
            }
            _ => None,
        }
    }
}

impl Codec for EffectKey {
    fn to_value(&self) -> Value {
        Value::Array(vec![
            self.plan().to_value(),
            self.generation().0.to_value(),
            self.iteration().to_value(),
            self.resource().to_value(),
        ])
    }
    fn from_value(value: &Value) -> Option<Self> {
        match array(value)? {
            [p, g, i, r] => Some(EffectKey::new(
                PlanId::from_value(p)?,
                Generation(u64::from_value(g)?),
                u32::from_value(i)?,
                ResourceKey::from_value(r)?,
            )),
            _ => None,
        }
    }
}

impl Codec for Receipt {
    fn to_value(&self) -> Value {
        match self {
            Receipt::Accepted => tagged("accepted", vec![]),
            Receipt::Started => tagged("started", vec![]),
            Receipt::Completed { changed } => tagged("completed", vec![boolean(*changed)]),
            Receipt::Failed => tagged("failed", vec![]),
            Receipt::Refused => tagged("refused", vec![]),
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("accepted", []) => Some(Receipt::Accepted),
            ("started", []) => Some(Receipt::Started),
            ("completed", [c]) => Some(Receipt::Completed {
                changed: from_boolean(c)?,
            }),
            ("failed", []) => Some(Receipt::Failed),
            ("refused", []) => Some(Receipt::Refused),
            _ => None,
        }
    }
}

impl Codec for Input {
    fn to_value(&self) -> Value {
        match self {
            Input::Enforce(plan) => tagged("enforce", vec![plan.to_value()]),
            Input::Observed(observations) => {
                tagged("observed", vec![list(observations.iter().cloned())])
            }
            Input::Receipt(key, receipt) => {
                tagged("receipt", vec![key.to_value(), receipt.to_value()])
            }
            Input::Tick(at) => tagged("tick", vec![at.to_value()]),
            Input::Recovered => tagged("recovered", vec![]),
        }
    }
    fn from_value(value: &Value) -> Option<Self> {
        match untag(value)? {
            ("enforce", [p]) => Some(Input::Enforce(Plan::from_value(p)?)),
            ("observed", [o]) => Some(Input::Observed(from_list(o)?)),
            ("receipt", [k, r]) => Some(Input::Receipt(
                EffectKey::from_value(k)?,
                Receipt::from_value(r)?,
            )),
            ("tick", [t]) => Some(Input::Tick(Instant::from_value(t)?)),
            ("recovered", []) => Some(Input::Recovered),
            _ => None,
        }
    }
}

impl nomos_store::Record for Input {
    fn encode(&self) -> Vec<u8> {
        encode(self)
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        decode(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integers across the IR's limit read back as themselves, and a
    /// non-canonical text form is not an integer.
    #[test]
    fn every_u64_reads_back_as_itself() {
        for n in [0, 1, MAX_UINT, MAX_UINT + 1, u64::MAX] {
            let bytes = cbor::encode(&n.to_value());
            let back = cbor::decode(&bytes).ok().and_then(|v| u64::from_value(&v));
            assert_eq!(back, Some(n));
        }
        assert_eq!(u64::from_value(&text("7")), None);
        assert_eq!(u64::from_value(&text("09007199254740993")), None);
    }

    fn path(t: &str) -> ResourcePath {
        ResourcePath::new(t).unwrap()
    }

    /// Every family's requirements and evidence, every failure, every
    /// receipt, and large instants read back as themselves; so does the
    /// Plan and its policy.
    #[test]
    fn every_input_shape_reads_back_as_itself() {
        use crate::kernel::{Canon, Managed, Plan, Policy};
        let d = Digest::from_bytes([7; 32]);
        let name = |t: &str| AccountName::new(t).unwrap();
        let meta = Metadata {
            owner: Some(name("app")),
            group: None,
            mode: Mode::from_octal("0640"),
        };
        let requirements: Vec<(ResourceKey, Requirement)> = vec![
            (
                ResourceKey::File(path("/a")),
                Requirement::File(FileCondition::Absent),
            ),
            (
                ResourceKey::File(path("/b")),
                Requirement::File(FileCondition::Present {
                    content: Content::Exactly(d),
                    metadata: meta.clone(),
                }),
            ),
            (
                ResourceKey::Directory(path("/c")),
                Requirement::Directory(DirectoryCondition::Present { metadata: meta }),
            ),
            (
                ResourceKey::Service(path("/run/s")),
                Requirement::Service(FileCondition::present(Content::Any)),
            ),
            (
                ResourceKey::parse("unit:a.service").unwrap(),
                Requirement::Unit(UnitCondition {
                    activity: Activity::Active,
                    enablement: Enablement::Disabled,
                }),
            ),
            (
                ResourceKey::parse("sysctl:net.ipv4.ip_forward").unwrap(),
                Requirement::Sysctl(SysctlCondition {
                    value: SysctlValue::normalized("1"),
                }),
            ),
            (
                ResourceKey::parse("user:app").unwrap(),
                Requirement::User(UserCondition::Present {
                    class: AccountClass::System,
                    home: Some(path("/var/lib/app")),
                    shell: None,
                }),
            ),
            (
                ResourceKey::parse("package:nginx").unwrap(),
                Requirement::Package(PackageCondition::Installed {
                    version: PackageVersion::new("1.2-3"),
                }),
            ),
        ];
        let managed = requirements
            .iter()
            .map(|(k, r)| Managed {
                condition: Condition::new(k.clone(), r.clone()).unwrap(),
                keys: [ConflictKey::new("k").unwrap()].into(),
                disrupts: [Node::new("n").unwrap()].into(),
            })
            .collect();
        let mut policy = Policy::new(2, 3, 4, 5);
        policy.budgets = vec![Budget::new([Node::new("n").unwrap()].into(), 1)];
        policy.budget_observed_at = Instant(u64::MAX - 1);
        let plan = Plan {
            id: PlanId::new("p").unwrap(),
            generation: Generation(9),
            canon: Canon::new(
                managed,
                vec![Edge::new(
                    requirements[1].0.clone(),
                    requirements[4].0.clone(),
                    EdgeKind::OnChange,
                )],
            ),
            bound: 5,
            policy,
            expires: Some(Instant(MAX_UINT + 7)),
        };
        let window = Window::new(Instant(1), Instant(MAX_UINT + 1)).unwrap();
        let prov = Provenance::new(CollectorId::new("linux").unwrap(), window);
        let observed = ObservedMetadata {
            owner: Account::Named(name("root")),
            group: Account::Id(4242),
            mode: Mode::DEFAULT_FILE,
        };
        let evidence = vec![
            (
                ResourceKey::File(path("/b")),
                Evidence::File(FileEvidence::Present {
                    digest: d,
                    size: 3,
                    metadata: observed.clone(),
                }),
            ),
            (
                ResourceKey::Directory(path("/c")),
                Evidence::Directory(DirectoryEvidence::Present { metadata: observed }),
            ),
            (
                ResourceKey::parse("unit:a.service").unwrap(),
                Evidence::Unit(UnitEvidence {
                    active: ActiveState::Reloading,
                    file_state: UnitFileState::Masked,
                }),
            ),
            (
                ResourceKey::parse("sysctl:net.ipv4.ip_forward").unwrap(),
                Evidence::Sysctl(SysctlEvidence {
                    value: SysctlValue::normalized("0"),
                }),
            ),
            (
                ResourceKey::parse("user:app").unwrap(),
                Evidence::User(UserEvidence::Present {
                    uid: 999,
                    gid: 999,
                    home: "/var/lib/app".into(),
                    shell: "/usr/sbin/nologin".into(),
                }),
            ),
            (
                ResourceKey::parse("package:nginx").unwrap(),
                Evidence::Package(PackageEvidence::Broken),
            ),
            (
                ResourceKey::parse("package:nginx").unwrap(),
                Evidence::Package(PackageEvidence::NotInstalled),
            ),
        ];
        let mut observations: Vec<Observation> = evidence
            .into_iter()
            .map(|(k, e)| Observation::new(k, Collection::Collected(e), prov.clone()).unwrap())
            .collect();
        observations.push(
            Observation::new(
                ResourceKey::parse("unit:b.service").unwrap(),
                Collection::Failed(CollectionFailure::Unavailable),
                prov,
            )
            .unwrap(),
        );
        let key = EffectKey::new(
            PlanId::new("p").unwrap(),
            Generation(9),
            2,
            requirements[0].0.clone(),
        );
        let inputs = vec![
            Input::Enforce(plan),
            Input::Observed(observations),
            Input::Receipt(key.clone(), Receipt::Completed { changed: true }),
            Input::Receipt(key, Receipt::Refused),
            Input::Tick(Instant(u64::MAX)),
            Input::Recovered,
        ];
        for input in inputs {
            let bytes = encode(&input);
            assert_eq!(decode(&bytes), Some(input.clone()), "{input:?}");
            assert_eq!(encode(&decode(&bytes).unwrap()), bytes, "one encoding");
        }
    }

    /// Bytes that are not an input decode to nothing, and never panic.
    #[test]
    fn anything_else_decodes_to_nothing() {
        let good = encode(&Input::Tick(Instant(5)));
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&good[..good.len() - 1]), None);
        let other_version = cbor::encode(&Value::Array(vec![
            Value::Uint(VERSION + 1),
            Input::Recovered.to_value(),
        ]));
        assert_eq!(decode(&other_version), None);
        for at in 0..good.len() {
            for bit in 0..8 {
                let mut b = good.clone();
                b[at] ^= 1 << bit;
                let _ = decode(&b);
            }
        }
    }
}
