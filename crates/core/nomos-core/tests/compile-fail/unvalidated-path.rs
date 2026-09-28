//! A ResourcePath cannot be built from unvalidated text.
use nomos_core::resource::ResourcePath;

fn main() {
    let _ = ResourcePath(String::from("relative/path"));
}
