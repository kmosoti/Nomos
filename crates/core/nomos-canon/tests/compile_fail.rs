//! The type boundary of `typed-validation`: a validated Canon, or any part
//! of one, cannot be built around the validator. Each case fails to
//! compile. Deserialization is not a way in either: nothing in the crate
//! derives it, and the derive a caller might add is the experiment's
//! negative control (`tests/validation.rs`).

#[test]
fn a_canon_cannot_be_built_around_the_validator() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile-fail/*.rs");
}
