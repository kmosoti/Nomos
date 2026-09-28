//! A Secret has no Display, so it cannot reach a log line or an error message by formatting.
use nomos_core::cipher::Secret;

fn main() {
    let secret = Secret::new("hunter2");
    let _ = format!("{}", secret);
}
