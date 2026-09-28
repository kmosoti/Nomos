//! An absent file cannot carry a content requirement.
use nomos_core::condition::{Content, FileCondition};

fn main() {
    let _ = FileCondition::Absent { content: Content::Any };
}
