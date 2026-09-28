//! A Secret has no Serialize, so it cannot reach an Event, a Plan, or a wire through serde.
use nomos_core::cipher::Secret;

fn main() {
    let secret = Secret::new("hunter2");
    let _ = serde_json::to_string(&secret);
}
