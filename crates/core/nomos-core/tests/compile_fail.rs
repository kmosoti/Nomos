//! Type boundaries the kernel promises, each shown to fail to compile.
//!
//! These establish API restrictions: a value the constructor would reject
//! cannot be built around it, a contradiction the sum type excludes cannot
//! be written, and a Secret cannot be printed or serialized by the ordinary
//! paths. They say nothing about whether an Observation is truthful
//! (grounding plan, Three Boundaries).

#[test]
fn the_type_boundaries_hold() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}
