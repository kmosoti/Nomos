//! A Condition cannot be fabricated around its constructor.
use nomos_core::condition::{Condition, FileCondition, Requirement};
use nomos_core::resource::{ResourceKey, ResourcePath};

fn main() {
    let _ = Condition {
        key: ResourceKey::File(ResourcePath::new("/etc/hosts").unwrap()),
        requirement: Requirement::File(FileCondition::Absent),
    };
}
