//! The Cell's commands (Phase 1 plan, `14-cell-commands`;
//! `docs/formal/cell-commands.md`), run in process through
//! `nomos_cell::cli::run`, which is all the binary's `main` calls.
//!
//! The tests that change the host are ignored on an ordinary run and run
//! in a disposable Debian container through `cargo xtask debian`: a Canon
//! of a directory, a configuration file with its content in a bundle, a
//! kernel parameter, a system account, and a service restarted when its
//! configuration changes. The before-and-after projection that shows
//! `trace` changes nothing is read with the host's own tools, never
//! through the adapter.

mod scratch;
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use nomos_app::kernel::{Canon, KernelSnapshot};
use nomos_canon::artifact::{Profile, encode};
use nomos_canon::model::{CanonBuilder, RelationKind};
use nomos_cell::cli;
use nomos_core::condition::{
    AccountClass, Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata,
    UnitCondition, UserCondition,
};
use nomos_core::observation::Instant;
use nomos_core::resource::ResourcePath;

fn run(args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = cli::run(&args, &mut out, &mut err);
    (
        status,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn scratch(name: &str) -> PathBuf {
    scratch::dir("cmd", name)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `canon` as an artifact, and `contents` as a bundle beside it.
fn artifact(
    dir: &Path,
    canon: &nomos_canon::model::Canon,
    contents: &[&[u8]],
) -> (PathBuf, PathBuf) {
    let file = dir.join("canon.cbor");
    std::fs::write(&file, encode(canon, Profile::Cbor)).unwrap();
    let bundle = dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    for c in contents {
        std::fs::write(bundle.join(hex(&nomos_canon::sha256::digest(c))), c).unwrap();
    }
    (file, bundle)
}

fn digest(bytes: &[u8]) -> nomos_core::resource::Digest {
    nomos_core::resource::Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

// ---------------------------------------------------------------------------
// Here

#[test]
fn a_command_line_that_is_not_one_exits_with_an_error() {
    for args in [
        &[][..],
        &["frobnicate"],
        &["trace"],
        &["enforce", "--canon"],
        &["import"],
    ] {
        let (status, _, err) = run(args);
        assert_eq!(status, 1, "{args:?}");
        assert!(err.contains("usage: nomos-cell"), "{args:?}: {err}");
    }
    let dir = scratch("bad-artifact");
    std::fs::write(dir.join("bad.cbor"), b"not an artifact").unwrap();
    let canon = dir.join("bad.cbor");
    let (status, _, err) = run(&["trace", "--canon", canon.to_str().unwrap()]);
    assert_eq!(status, 1);
    assert!(err.contains("bad.cbor"), "{err}");
}

#[test]
fn traits_reports_the_host_with_stability() {
    let (status, out, _) = run(&["traits"]);
    assert_eq!(status, 0);
    assert!(out.contains("architecture = "), "{out}");
    assert!(out.contains("kernel.release = "), "{out}");
    assert!(
        out.contains("(dynamic, /proc/sys/kernel/osrelease)"),
        "{out}"
    );
}

/// Trace on the mock, through the pipeline `enforce` uses: a missing file
/// is a Variance with an Action, a denied read is Indeterminate with none,
/// and nothing is executed (N1, N13).
#[test]
fn trace_plans_nothing_for_an_indeterminate_resource_and_executes_nothing() {
    let wanted = digest(b"wanted");
    let validated = CanonBuilder::new("trace")
        .file(
            "/etc/app.conf",
            FileCondition::present(Content::Exactly(wanted)),
        )
        .file(
            "/etc/secret.conf",
            FileCondition::present(Content::Exactly(wanted)),
        )
        .build()
        .unwrap();
    let canon = Canon::try_from(&validated).unwrap();
    let mut host = nomos_substrate_mock::MockHost::new();
    host.write(&support::p("/etc/secret.conf"), support::d(1));
    host.deny(&support::p("/etc/secret.conf"), true);
    let before = host.clone();
    let (report, actions) =
        cli::trace_with(&mut host, KernelSnapshot::new(), false, canon, Instant(0)).unwrap();
    let (text, status) = nomos_cell::render::trace(&report, &actions);
    assert_eq!(status, 2, "{text}");
    assert!(
        text.contains("file:/etc/app.conf\n  VARIANCE missing\n  action: converge"),
        "{text}"
    );
    assert!(
        text.contains(
            "file:/etc/secret.conf\n  INDETERMINATE collection-failed: permission-denied\n  action: none"
        ),
        "{text}"
    );
    assert_eq!(host, before, "trace changed the host");
    assert!(host.executions().is_empty());
}

// ---------------------------------------------------------------------------
// On Debian

const DIR: &str = "/etc/nomos-cmd";
const CONF: &str = "/etc/nomos-cmd/app.conf";
const SERVICE: &str = "nomos-cmd.service";
const LOADED: &str = "/run/nomos-cmd.loaded";
const ACCOUNT: &str = "nomos-cmd";

/// One test at a time: they share the host.
static HOST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    HOST.lock().unwrap_or_else(|e| e.into_inner())
}

fn sh(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Removes everything the demonstration Canon manages, and writes the
/// service's unit file, which the Canon does not manage.
fn reset(service: &str) {
    sh("systemctl", &["stop", SERVICE]);
    sh("systemctl", &["disable", SERVICE]);
    sh("systemctl", &["reset-failed", SERVICE]);
    sh("userdel", &[ACCOUNT]);
    let _ = std::fs::remove_dir_all(DIR);
    let _ = std::fs::remove_file(LOADED);
    std::fs::write(
        format!("/etc/systemd/system/{SERVICE}"),
        format!(
            "[Unit]\nDescription=Nomos command test\n[Service]\n{service}\
             [Install]\nWantedBy=multi-user.target\n"
        ),
    )
    .unwrap();
    sh("systemctl", &["daemon-reload"]);
    std::fs::write("/proc/sys/kernel/domainname", "(none)\n").unwrap();
}

/// What the host holds of the Canon's resources, read without the
/// adapter: the directory's mode, the file's bytes, the service's state,
/// the parameter, and the account.
fn projection() -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;
    vec![
        std::fs::metadata(DIR)
            .map(|m| format!("{:o}", m.permissions().mode()))
            .unwrap_or_default(),
        std::fs::read(CONF).map(|b| hex(&b)).unwrap_or_default(),
        sh(
            "systemctl",
            &[
                "show",
                "-p",
                "ActiveState,UnitFileState,ExecMainPID",
                SERVICE,
            ],
        ),
        std::fs::read_to_string("/proc/sys/kernel/domainname").unwrap(),
        sh("getent", &["passwd", ACCOUNT]),
        std::fs::read_to_string(LOADED).unwrap_or_default(),
    ]
}

fn demonstration(config: &[u8]) -> nomos_canon::model::Canon {
    CanonBuilder::new("cell-commands")
        .directory(
            DIR,
            DirectoryCondition::Present {
                metadata: Metadata {
                    owner: None,
                    group: None,
                    mode: Some(nomos_core::resource::Mode::from_octal("0755").unwrap()),
                },
            },
        )
        .file(
            CONF,
            FileCondition::present(Content::Exactly(digest(config))),
        )
        .sysctl("kernel.domainname", "nomos-cmd.example")
        .user(
            ACCOUNT,
            UserCondition::Present {
                class: AccountClass::System,
                home: Some(ResourcePath::new("/var/lib/nomos-cmd").unwrap()),
                shell: Some(ResourcePath::new("/usr/sbin/nologin").unwrap()),
            },
        )
        .unit(
            SERVICE,
            UnitCondition {
                activity: Activity::Active,
                enablement: Enablement::Enabled,
            },
        )
        .relate(
            &format!("directory:{DIR}"),
            RelationKind::Requires,
            &format!("file:{CONF}"),
        )
        .relate(
            &format!("file:{CONF}"),
            RelationKind::OnChange,
            &format!("unit:{SERVICE}"),
        )
        .build()
        .unwrap()
}

/// The milestone's exit, and the fixed point: `trace` shows the Variance
/// and changes nothing (N1); `enforce` converges; a second `enforce`
/// executes nothing (spec §39, N3); `trace` then finds every Condition
/// Satisfied; `events` shows the journal's Event Log.
#[test]
#[ignore = "changes the host; run by `cargo xtask debian`"]
fn trace_changes_nothing_and_enforce_converges_to_a_fixed_point() {
    let _host = exclusive();
    let config: &[u8] = b"setting = on\n";
    reset(&format!(
        "ExecStartPre=/bin/cp {CONF} {LOADED}\nExecStart=/bin/sleep infinity\n"
    ));
    let dir = scratch("fixed-point");
    let (canon, bundle) = artifact(&dir, &demonstration(config), &[config]);
    let state = dir.join("state");
    let (canon, bundle, state) = (
        canon.to_str().unwrap(),
        bundle.to_str().unwrap(),
        state.to_str().unwrap(),
    );

    let before = projection();
    let (status, out, err) = run(&["--state", state, "trace", "--canon", canon]);
    println!("trace before:\n{out}");
    assert_eq!(status, 3, "{out}{err}");
    assert!(
        out.contains(&format!(
            "file:{CONF}\n  VARIANCE missing\n  action: converge"
        )),
        "{out}"
    );
    assert_eq!(projection(), before, "trace changed the host");

    let (status, out, err) = run(&[
        "--state", state, "enforce", "--canon", canon, "--bundle", bundle,
    ]);
    println!("enforce:\n{out}");
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("outcome: converged"), "{out}");
    assert_eq!(
        std::fs::read(LOADED).unwrap(),
        config,
        "the service did not load the configuration"
    );

    let (status, out, err) = run(&["--state", state, "enforce", "--canon", canon]);
    println!("enforce again:\n{out}");
    assert_eq!(status, 0, "{out}{err}");
    assert!(
        out.contains("executions: 0"),
        "a converged host was changed again: {out}"
    );

    let (status, out, _) = run(&["--state", state, "trace", "--canon", canon]);
    println!("trace after:\n{out}");
    assert_eq!(status, 0, "{out}");
    assert!(
        !out.contains("VARIANCE") && !out.contains("INDETERMINATE"),
        "{out}"
    );

    let (status, out, _) = run(&["--state", state, "events"]);
    println!("events:\n{out}");
    assert_eq!(status, 0);
    assert!(out.contains("plan-accepted cell generation 1"), "{out}");
    assert!(out.contains("plan-accepted cell generation 2"), "{out}");
    assert!(out.contains("outcome: converged"), "{out}");
}

/// Removes the capabilities to override file permissions from this
/// thread, so that a file of mode 0000 is unreadable to root here as it
/// is to anyone else.
fn drop_permission_override() {
    use rustix::thread::{CapabilitySet, capabilities, set_capabilities};
    let mut sets = capabilities(None).unwrap();
    sets.effective
        .remove(CapabilitySet::DAC_OVERRIDE | CapabilitySet::DAC_READ_SEARCH);
    set_capabilities(None, sets).unwrap();
}

/// A denied read: `trace` prints the Indeterminate Assessment and plans
/// nothing for it, and `enforce` ends Indeterminate having changed
/// nothing it could not see.
#[test]
#[ignore = "changes the host; run by `cargo xtask debian`"]
fn a_denied_read_is_indeterminate_and_plans_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let _host = exclusive();
    let secret = "/etc/nomos-cmd-secret.conf";
    std::fs::write(secret, b"old").unwrap();
    std::fs::set_permissions(secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    let dir = scratch("denied");
    let validated = CanonBuilder::new("denied")
        .file(
            secret,
            FileCondition::present(Content::Exactly(digest(b"new"))),
        )
        .build()
        .unwrap();
    let (canon, bundle) = artifact(&dir, &validated, &[b"new"]);
    let state = dir.join("state");
    let (canon, bundle, state) = (
        canon.to_str().unwrap(),
        bundle.to_str().unwrap(),
        state.to_str().unwrap(),
    );
    drop_permission_override();

    let (status, out, err) = run(&["--state", state, "trace", "--canon", canon]);
    assert_eq!(status, 2, "{out}{err}");
    assert!(
        out.contains(&format!(
            "file:{secret}\n  INDETERMINATE collection-failed: permission-denied\n  action: none"
        )),
        "{out}"
    );
    let (status, out, err) = run(&[
        "--state", state, "enforce", "--canon", canon, "--bundle", bundle,
    ]);
    assert_eq!(status, 2, "{out}{err}");
    assert!(out.contains("outcome: indeterminate"), "{out}");
    assert!(out.contains("executions: 0"), "{out}");
    std::fs::set_permissions(secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(std::fs::read(secret).unwrap(), b"old");
    std::fs::remove_file(secret).unwrap();
}

/// A service whose start fails: `enforce` ends Failed, status 4.
#[test]
#[ignore = "changes the host; run by `cargo xtask debian`"]
fn a_failed_start_ends_enforce_failed() {
    let _host = exclusive();
    reset("ExecStartPre=/bin/false\nExecStart=/bin/sleep infinity\n");
    let dir = scratch("failed");
    let validated = CanonBuilder::new("failed")
        .unit(
            SERVICE,
            UnitCondition {
                activity: Activity::Active,
                enablement: Enablement::Any,
            },
        )
        .build()
        .unwrap();
    let (canon, _) = artifact(&dir, &validated, &[]);
    let state = dir.join("state");
    let (status, out, err) = run(&[
        "--state",
        state.to_str().unwrap(),
        "enforce",
        "--canon",
        canon.to_str().unwrap(),
    ]);
    assert_eq!(status, 4, "{out}{err}");
    assert!(
        out.contains(&format!("outcome: failed\n  failed: unit:{SERVICE}")),
        "{out}"
    );
}
