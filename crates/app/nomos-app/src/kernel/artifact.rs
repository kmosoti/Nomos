//! The kernel's Canon from a decoded artifact (`06-canon-artifact`).
//!
//! The artifact is validated by `nomos-canon` before it gets here, so this
//! is a change of representation and nothing else: every resource, key,
//! node, and relation carries over, and nothing is added or dropped. The
//! conversion is fallible only because Warp's key and node types refuse
//! empty text, and a core Condition refuses a requirement of another family
//! than its key, neither of which a validated Canon holds; it reports those
//! cases rather than dropping what it cannot convert.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use nomos_canon::model::{Canon as Validated, Name, RelationKind, Requirement, ServiceRequirement};
use nomos_core::condition::{Condition, Content, FileCondition, Requirement as Core};
use nomos_warp::budget::Node;
use nomos_warp::graph::{ConflictKey, Edge, EdgeKind};

use super::{Canon, Managed};

/// Why a validated Canon has no kernel form. Unreachable from a Canon
/// `nomos-canon` accepted; present so that the conversion cannot drop a
/// label, or a resource whose requirement is of another family than its
/// key, silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyLabel;

fn labels<T>(names: &BTreeSet<Name>, make: fn(&str) -> Option<T>) -> Result<BTreeSet<T>, EmptyLabel>
where
    T: Ord,
{
    names
        .iter()
        .map(|n| make(n.as_str()).ok_or(EmptyLabel))
        .collect()
}

impl TryFrom<&Validated> for Canon {
    type Error = EmptyLabel;

    fn try_from(canon: &Validated) -> Result<Self, EmptyLabel> {
        let mut resources = Vec::new();
        for (key, r) in canon.resources() {
            let requirement = match r.requirement() {
                Requirement::Directory(c) => Core::Directory(c.clone()),
                Requirement::File(c) => Core::File(c.clone()),
                Requirement::Package(c) => Core::Package(c.clone()),
                // A service is observed through its status path: running is
                // a status that exists, loaded is one that names a revision.
                Requirement::Service(ServiceRequirement::Running) => {
                    Core::Service(FileCondition::present(Content::Any))
                }
                Requirement::Service(ServiceRequirement::Loaded(d)) => {
                    Core::Service(FileCondition::present(Content::Exactly(*d)))
                }
                Requirement::Sysctl(c) => Core::Sysctl(c.clone()),
                Requirement::Unit(c) => Core::Unit(*c),
                Requirement::User(c) => Core::User(c.clone()),
            };
            let condition = Condition::new(key.clone(), requirement).ok_or(EmptyLabel)?;
            resources.push(Managed {
                condition,
                keys: labels(r.keys(), ConflictKey::new)?,
                disrupts: labels(r.disrupts(), Node::new)?,
            });
        }
        let edges = canon
            .relations()
            .iter()
            .map(|rel| {
                let kind = match rel.kind() {
                    RelationKind::Requires => EdgeKind::Requires,
                    RelationKind::After => EdgeKind::After,
                    RelationKind::OnChange => EdgeKind::OnChange,
                };
                Edge::new(rel.source().clone(), rel.target().clone(), kind)
            })
            .collect();
        Ok(Canon::new(resources, edges))
    }
}
