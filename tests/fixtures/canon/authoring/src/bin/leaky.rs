//! The negative control: a generator that reads every undeclared input the
//! experiment varies, and lets each reach the IR. The harness must see each
//! one, as a denial, a recorded access, or a difference in the output.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nomos_canon::model::CanonBuilder;
use nomos_core::condition::FileCondition;

/// Keeps what a label may hold: `a-z`, `0-9`, and `-`.
fn label(text: &str) -> String {
    let kept: String = text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .take(40)
        .collect();
    if kept.is_empty() {
        String::from("none")
    } else {
        kept
    }
}

fn main() -> std::process::ExitCode {
    // The clock.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() % 1_000_000)
        .unwrap_or(0);
    // The environment: locale, time zone, temporary directory.
    let locale = std::env::var("LC_ALL").unwrap_or_default();
    let zone = std::env::var("TZ").unwrap_or_default();
    let tmp = std::env::temp_dir();
    // The working directory.
    let cwd = std::env::current_dir()
        .map(|d| d.as_os_str().len())
        .unwrap_or(0);
    // A file outside the declared inputs.
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    // The network: a documentation address, which answers nothing.
    let reached = TcpStream::connect_timeout(
        &SocketAddr::from(([192, 0, 2, 1], 80)),
        Duration::from_millis(200),
    )
    .is_ok();
    // A hash seed: the first key the iteration yields.
    let seeded: HashMap<&str, ()> = ["alpha", "bravo", "charlie", "delta"]
        .into_iter()
        .map(|k| (k, ()))
        .collect();
    let first = seeded.keys().next().copied().unwrap_or("none");
    let canon = CanonBuilder::new(&format!("leaky-{now}"))
        .resource(
            "/etc/leaky",
            nomos_canon::model::Requirement::File(FileCondition::Absent),
            &[
                &format!("locale:{}", label(&locale)),
                &format!("tz:{}", label(&zone)),
                &format!("tmp:{}", tmp.as_os_str().len()),
                &format!("cwd:{cwd}"),
                &format!("host:{}", label(&host)),
                &format!("net:{reached}"),
                &format!("seed:{first}"),
            ],
            &[],
        )
        .build();
    match canon {
        Ok(c) => canon_authoring::emit(&c),
        Err(_) => std::process::ExitCode::FAILURE,
    }
}
