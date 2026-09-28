//! A relation cannot be written as a literal, so none can skip the checks
//! for dangling ends, self relations, and cycles.
use nomos_canon::model::{Relation, RelationKind};
use nomos_core::resource::ResourcePath;

fn main() {
    let p = ResourcePath::new("/a").unwrap();
    let _ = Relation {
        source: p.clone(),
        target: p,
        kind: RelationKind::Requires,
    };
}
