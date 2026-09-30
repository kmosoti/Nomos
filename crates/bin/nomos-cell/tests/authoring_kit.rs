//! The Canon authoring kit (`kit/canon`, Phase 1 plan, `16-alpha-release`),
//! built and run as the operator guide says, here, where the repository
//! and Cargo are.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = nomos_cell::cli::run(&args, &mut out, &mut err);
    (
        status,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nomos-kit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The authoring kit, as the operator guide tells an operator to run it,
/// writes an artifact the Cell reads and the bundle its file names: `trace`
/// reads it and reports, rather than refusing it (status 1).
#[test]
fn the_authoring_kit_writes_an_artifact_the_cell_reads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let out = scratch("kit");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let built = Command::new(cargo)
        .args(["run", "--quiet", "--offline", "--locked", "--manifest-path"])
        .arg(root.join("kit/canon/Cargo.toml"))
        .arg("--")
        .arg(&out)
        .env("CARGO_TARGET_DIR", root.join("target/kit"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let canon = out.join("canon.cbor");
    let motd = std::fs::read(root.join("kit/canon/files/motd")).unwrap();
    assert!(
        out.join("bundle")
            .join(hex(&nomos_canon::sha256::digest(&motd)))
            .exists()
    );
    let state = out.join("state");
    let (status, text, err) = run(&[
        "--state",
        state.to_str().unwrap(),
        "trace",
        "--canon",
        canon.to_str().unwrap(),
    ]);
    assert!(matches!(status, 0 | 2 | 3), "{status}: {text}{err}");
    assert!(text.contains("file:/etc/motd"), "{text}");
}
