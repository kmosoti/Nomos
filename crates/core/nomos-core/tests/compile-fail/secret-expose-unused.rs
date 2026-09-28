//! Exposing a secret and dropping the plaintext is a warning the crate denies.
#![deny(unused_must_use)]
use nomos_core::cipher::Secret;

fn main() {
    let secret = Secret::new("hunter2");
    secret.expose();
}
