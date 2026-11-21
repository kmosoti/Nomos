//! Nomos Loom: headless IPv6 fleet coordinator, lease broker, and PSI governor.
//!
//! Wires `nomos-app` across concrete cipher (`age-vtpm`, `vault`), mesh
//! (`wireguard`, `headscale`), control protocol (`quic`), and cloud (`gcp`)
//! adapters while serving HTTP/SSE JSON routes and optional prebuilt assets
//! via `--web-assets-dir` with zero bundled WebAssembly artifacts.

pub mod cli;
pub mod governor_svc;
pub mod lease;
pub mod quic_svc;
pub mod server;
pub mod telemetry;
