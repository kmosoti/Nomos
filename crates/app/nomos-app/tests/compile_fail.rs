//! Type boundaries the application promises, each shown to fail to compile.
//! They establish API restrictions, not that an adapter tells the truth.

#[test]
fn the_type_boundaries_hold() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}
