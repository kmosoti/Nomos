//! Experiment `substrate-contract` (grounding plan, `07-substrate-conformance`):
//! does the Linux adapter honor the `nomos-substrate` contract the mock
//! honors?
//!
//! One suite, `conformance`, runs against both adapters. Its negative
//! controls, a mutating Trace and a denied read reported as absence, must
//! fail it, on each adapter. Then the production driver enforces a Canon on
//! each, and the Linux adapter meets what a real file system can do to it.
//! The Linux tests run on the machine that runs the tests, beneath a
//! scratch root; the record names the host.

mod conformance;
mod support;

use conformance::*;
use nomos_app::driver::Cell;
use nomos_app::kernel::{Canon, Input, Managed, RunOutcome};
use nomos_core::assessment::{Assessment, Reason, assess};
use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::effect::Receipt;
use nomos_core::observation::{Collection, CollectionFailure, Evidence, FileEvidence};
use nomos_substrate::{Mutate, Observe};
use std::collections::BTreeSet;

fn passes(results: Vec<(&'static str, Result<(), String>)>) {
    let failures: Vec<String> = results
        .into_iter()
        .filter_map(|(k, r)| r.err().map(|e| format!("{k}: {e}")))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------------
// The suite

#[test]
fn the_mock_passes_the_suite() {
    passes(all(&mut MockSubject::new()));
}

#[test]
fn the_linux_adapter_passes_the_suite() {
    passes(all(&mut LinuxSubject::new("suite")));
}

/// Milestone `08-resource-families`: the mock passes every clause for each
/// of the six Phase 1 families.
#[test]
fn the_mock_passes_the_suite_for_every_family() {
    let mut s = MockSubject::new();
    for family in families::World::families(&s) {
        let results = families::all(&mut s, family);
        println!("{family:?}: {:?}", report(&results));
        passes(results);
    }
}

/// Milestone `09-file-and-directory`: the Linux adapter passes every
/// clause for files and directories, with owners, groups, and modes, on the
/// host that runs the tests.
///
/// The suite sets owners and groups, which only root may, so it runs as
/// root: in CI, and on Debian through `cargo xtask debian`. As another user
/// it fails, saying so, rather than pass on less.
#[test]
fn the_linux_adapter_passes_the_suite_for_files_and_directories() {
    assert!(
        rustix::process::geteuid().is_root(),
        "the Linux suite for files and directories sets owners and needs root; \
         run it as root or through `cargo xtask debian`"
    );
    use nomos_core::resource::Family;
    let mut s = LinuxSubject::new("families");
    for family in [Family::Directory, Family::File] {
        let results = families::all(&mut s, family);
        println!("{family:?}: {:?}", report(&results));
        passes(results);
    }
}

/// The per-family suite for kernel parameters and users on Linux, each
/// against a scratch root of its own: `proc/sys` as a tree of regular
/// files, which is how the adapter reads it, and the user database
/// changed through the distribution's tools with `--prefix`. Root only,
/// as above.
#[test]
fn the_linux_adapter_passes_the_suite_for_kernel_parameters_and_users() {
    use nomos_core::resource::Family;
    assert!(
        rustix::process::geteuid().is_root(),
        "the Linux suite for users runs the account tools and needs root; \
         run it as root or through `cargo xtask debian`"
    );
    for family in [Family::Sysctl, Family::User] {
        let mut s = LinuxSubject::new(&format!("families-{family:?}"));
        let results = families::all(&mut s, family);
        println!("{family:?}: {:?}", report(&results));
        passes(results);
    }
}

/// A family the Linux adapter does not serve is a failed collection when
/// observed, never absence, and refused when changed (S7): units, for a
/// host given no connection to systemd.
#[test]
fn the_linux_adapter_refuses_the_families_it_does_not_serve() {
    use nomos_core::resource::Family;
    let mut s = LinuxSubject::new("unserved");
    for family in families::PHASE_1 {
        // Packages are served; their suite runs on Debian (debian_host).
        if families::World::families(&s).contains(&family) || family == Family::Package {
            continue;
        }
        let key = families::resource(family, "unserved", 0);
        let observed = s.observe(std::slice::from_ref(&key));
        assert_eq!(
            observed
                .iter()
                .map(|o| o.collection().clone())
                .collect::<Vec<_>>(),
            vec![Collection::Failed(CollectionFailure::Unsupported)],
            "{key}"
        );
        for (i, requirement) in families::requirements(family, &families::World::contents(&mut s))
            .into_iter()
            .enumerate()
        {
            let receipts = s.apply(&families::converge(&key, i as u32, requirement));
            assert_eq!(receipts, vec![Receipt::Refused], "{key}");
        }
    }
    assert_eq!(s.executions(), 0);
}

// ---------------------------------------------------------------------------
// The negative controls

#[test]
fn a_mutating_trace_fails_the_suite_on_the_mock() {
    let mut s = MutatingTrace(MockSubject::new(), 0);
    assert!(s4_observation_does_not_mutate(&mut s).is_err());
}

#[test]
fn a_mutating_trace_fails_the_suite_on_linux() {
    let mut s = MutatingTrace(LinuxSubject::new("mutating-trace"), 0);
    assert!(s4_observation_does_not_mutate(&mut s).is_err());
}

#[test]
fn a_denied_read_reported_as_absence_fails_the_suite_on_the_mock() {
    let mut s = DeniedAsAbsent(MockSubject::new());
    assert!(s2_truthful_absence(&mut s).is_err());
}

#[test]
fn a_denied_read_reported_as_absence_fails_the_suite_on_linux() {
    let mut s = DeniedAsAbsent(LinuxSubject::new("denied-as-absent"));
    assert!(s2_truthful_absence(&mut s).is_err());
}

// ---------------------------------------------------------------------------
// End to end, through the production driver

/// One file that must hold `digest`.
fn one_file(path: &str, digest: nomos_core::resource::Digest) -> Canon {
    Canon::new(
        vec![Managed {
            condition: Condition::file(p(path), FileCondition::present(Content::Exactly(digest))),
            keys: BTreeSet::new(),
            disrupts: BTreeSet::new(),
        }],
        vec![],
    )
}

/// The driver converges the file, a second Enforce executes nothing (N3),
/// and a denied read ends Indeterminate with no execution (N13).
fn end_to_end(s: &mut impl Subject) {
    assert_eq!(s.executions(), 0);
    s.put(&p("/etc/app/app.conf"), b"old");
    let wanted = s.content(b"new configuration");
    let canon = one_file("/etc/app/app.conf", wanted);

    let mut cell = Cell::open(support::MemLog::default());
    cell.settle(Input::Enforce(support::plan("p1", 1, canon.clone(), 3)), s)
        .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert_eq!(s.truth(&p("/etc/app/app.conf")), Truth::File(wanted));
    let executions = s.executions();
    assert_eq!(executions, 1);

    cell.settle(Input::Enforce(support::plan("p2", 2, canon.clone(), 3)), s)
        .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert_eq!(
        s.executions(),
        executions,
        "re-enforcing a converged Canon executed"
    );

    s.put(&p("/etc/app/app.conf"), b"drifted");
    s.deny(&p("/etc/app/app.conf"));
    cell.settle(Input::Enforce(support::plan("p3", 3, canon, 3)), s)
        .unwrap();
    assert_eq!(
        cell.snapshot().outcome(),
        Some(&RunOutcome::Indeterminate(vec![(
            f("/etc/app/app.conf"),
            Reason::CollectionFailed(CollectionFailure::PermissionDenied)
        )]))
    );
    assert_eq!(s.executions(), executions, "an unknown drove a mutation");
}

#[test]
fn the_driver_converges_the_mock() {
    end_to_end(&mut MockSubject::new());
}

#[test]
fn the_driver_converges_linux() {
    end_to_end(&mut LinuxSubject::new("end-to-end"));
}

/// An adapter whose receipts never settle: the execution is accepted and
/// nothing more is heard. The kernel keeps the reservation, and the run
/// does not end Converged (N10).
struct Unsettled<S>(S);

impl<S: Subject> nomos_substrate::Observe for Unsettled<S> {
    fn observe(
        &mut self,
        resources: &[nomos_core::resource::ResourceKey],
    ) -> Vec<nomos_core::observation::Observation> {
        self.0.observe(resources)
    }
}

impl<S: Subject> nomos_substrate::Mutate for Unsettled<S> {
    fn apply(&mut self, request: &nomos_core::effect::Apply) -> Vec<Receipt> {
        let mut receipts = self.0.apply(request);
        receipts.retain(|r| !r.settles());
        receipts
    }
}

fn an_unresolved_effect_keeps_its_reservation(s: impl Subject) {
    let mut s = Unsettled(s);
    s.0.put(&p("/etc/app/app.conf"), b"old");
    let wanted = s.0.content(b"new configuration");
    let mut cell = Cell::open(support::MemLog::default());
    cell.settle(
        Input::Enforce(support::plan(
            "p1",
            1,
            one_file("/etc/app/app.conf", wanted),
            3,
        )),
        &mut s,
    )
    .unwrap();
    assert_ne!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    assert!(
        !cell.snapshot().effects().is_empty(),
        "an unsettled execution released its reservation"
    );
}

#[test]
fn an_unresolved_effect_keeps_its_reservation_on_the_mock() {
    an_unresolved_effect_keeps_its_reservation(MockSubject::new());
}

#[test]
fn an_unresolved_effect_keeps_its_reservation_on_linux() {
    an_unresolved_effect_keeps_its_reservation(LinuxSubject::new("unsettled"));
}

// ---------------------------------------------------------------------------
// What a real file system does (Linux only)

fn observe_one(s: &mut LinuxSubject, path: &str) -> Collection {
    s.observe(&[f(path)])[0].collection().clone()
}

/// A symbolic link at the resource is not followed: it is unsupported, not
/// the file it points at and not absent; and replacing it is refused.
#[test]
fn a_symbolic_link_at_the_resource_is_not_followed() {
    let mut s = LinuxSubject::new("leaf-link");
    s.put(&p("/etc/target"), b"secret");
    std::os::unix::fs::symlink(s.host_path(&p("/etc/target")), s.host_path(&p("/etc/link")))
        .unwrap();
    assert_eq!(
        observe_one(&mut s, "/etc/link"),
        Collection::Failed(CollectionFailure::Unsupported)
    );
    let wanted = s.content(b"overwrite");
    let receipts = s.apply(&replace(
        "/etc/link",
        0,
        FileCondition::present(Content::Exactly(wanted)),
    ));
    assert_eq!(receipts, vec![Receipt::Refused]);
    assert_eq!(s.truth(&p("/etc/target")), Truth::File(sha(b"secret")));
}

/// A symbolic link in a parent directory is not followed either, even when
/// it points inside the root. The link is relative, so resolution beneath
/// the root alone would follow it: only `RESOLVE_NO_SYMLINKS` refuses it.
/// (A first version used an absolute target, which the beneath rule
/// already refuses, and did not catch `SM-SUBSTRATE-002`.)
#[test]
fn a_symbolic_link_in_a_parent_is_not_followed() {
    let mut s = LinuxSubject::new("parent-link");
    s.put(&p("/real/f"), b"inside");
    std::os::unix::fs::symlink("real", s.host_path(&p("/alias"))).unwrap();
    assert_eq!(
        std::fs::read(s.host_path(&p("/alias/f"))).unwrap(),
        b"inside",
        "the link resolves inside the root when followed"
    );
    assert_eq!(
        observe_one(&mut s, "/alias/f"),
        Collection::Failed(CollectionFailure::Unsupported)
    );
    let receipts = s.apply(&replace("/alias/f", 0, FileCondition::Absent));
    assert_eq!(receipts, vec![Receipt::Refused]);
    assert_eq!(s.truth(&p("/real/f")), Truth::File(sha(b"inside")));
}

/// A link that points outside the root is refused the same way: nothing
/// outside the root is read or changed.
#[test]
fn nothing_outside_the_root_is_reached() {
    let mut s = LinuxSubject::new("escape");
    let outside = std::env::temp_dir().join(format!("nomos-outside-{}", std::process::id()));
    std::fs::write(&outside, b"outside").unwrap();
    std::fs::create_dir_all(s.host_path(&p("/etc"))).unwrap();
    std::os::unix::fs::symlink(&outside, s.host_path(&p("/etc/out"))).unwrap();
    assert_eq!(
        observe_one(&mut s, "/etc/out"),
        Collection::Failed(CollectionFailure::Unsupported)
    );
    assert_eq!(
        s.apply(&replace("/etc/out", 0, FileCondition::Absent)),
        vec![Receipt::Refused]
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    std::fs::remove_file(outside).unwrap();
}

/// A directory where a file is expected is unsupported, never absent.
#[test]
fn a_directory_where_a_file_is_expected_is_unsupported() {
    let mut s = LinuxSubject::new("directory");
    std::fs::create_dir_all(s.host_path(&p("/etc/conf.d"))).unwrap();
    assert_eq!(
        observe_one(&mut s, "/etc/conf.d"),
        Collection::Failed(CollectionFailure::Unsupported)
    );
}

/// A path under a regular file cannot exist: absent, truthfully.
#[test]
fn a_path_under_a_file_is_absent() {
    let mut s = LinuxSubject::new("under-file");
    s.put(&p("/etc/plain"), b"x");
    assert_eq!(
        observe_one(&mut s, "/etc/plain/child"),
        Collection::Collected(Evidence::File(FileEvidence::Absent))
    );
}

/// A foreign writer changes the file after the execution and before
/// verification: core judges the file as it is, not as the receipt said.
#[test]
fn verification_sees_a_foreign_write() {
    let mut s = LinuxSubject::new("foreign");
    s.put(&p("/etc/app.conf"), b"old");
    let wanted = s.content(b"new");
    let requirement = FileCondition::present(Content::Exactly(wanted));
    let receipts = s.apply(&replace("/etc/app.conf", 0, requirement.clone()));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    s.put(&p("/etc/app.conf"), b"foreign");
    let assessed = assess(
        &Condition::file(p("/etc/app.conf"), requirement),
        &s.observe(&[f("/etc/app.conf")]),
    );
    assert!(matches!(assessed, Assessment::Variance(_)), "{assessed:?}");
}

/// Replacement leaves no temporary file behind, succeeds or not.
#[test]
fn replacement_leaves_no_temporary_file() {
    let mut s = LinuxSubject::new("temporary");
    s.put(&p("/srv/x"), b"old");
    let wanted = s.content(b"new");
    s.apply(&replace(
        "/srv/x",
        0,
        FileCondition::present(Content::Exactly(wanted)),
    ));
    let names: Vec<String> = std::fs::read_dir(s.host_path(&p("/srv")))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["x".to_string()]);
}

/// A hard link planted at the temporary file's name is not written
/// through: the execution fails, and the file it links to is unchanged.
#[test]
fn a_planted_temporary_file_is_not_written_through() {
    let mut s = LinuxSubject::new("planted");
    s.put(&p("/etc/shadow"), b"precious");
    s.put(&p("/etc/app.conf"), b"old");
    let wanted = s.content(b"new");
    let request = replace(
        "/etc/app.conf",
        0,
        FileCondition::present(Content::Exactly(wanted)),
    );
    let temp = nomos_substrate_linux::temporary_name("app.conf", &request.key);
    std::fs::hard_link(
        s.host_path(&p("/etc/shadow")),
        s.host_path(&p("/etc")).join(temp),
    )
    .unwrap();
    assert_eq!(s.apply(&request).last(), Some(&Receipt::Failed));
    assert_eq!(s.truth(&p("/etc/shadow")), Truth::File(sha(b"precious")));
    assert_eq!(s.truth(&p("/etc/app.conf")), Truth::File(sha(b"old")));
}

/// A temporary file left behind by another execution, as a crash would
/// leave it, does not block this one: each execution has its own name.
#[test]
fn a_stale_temporary_file_does_not_block_a_later_execution() {
    let mut s = LinuxSubject::new("stale");
    s.put(&p("/etc/app.conf"), b"old");
    let wanted = s.content(b"new");
    let requirement = FileCondition::present(Content::Exactly(wanted));
    let crashed = replace("/etc/app.conf", 0, requirement.clone());
    let stale = nomos_substrate_linux::temporary_name("app.conf", &crashed.key);
    std::fs::write(s.host_path(&p("/etc")).join(stale), b"half written").unwrap();
    let receipts = s.apply(&replace("/etc/app.conf", 1, requirement));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(s.truth(&p("/etc/app.conf")), Truth::File(wanted));
}

/// A named pipe at the resource: observation returns at once, unsupported,
/// and replacing it is refused.
#[test]
fn a_named_pipe_is_unsupported_and_does_not_block() {
    let mut s = LinuxSubject::new("fifo");
    std::fs::create_dir_all(s.host_path(&p("/etc"))).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        s.host_path(&p("/etc/pipe")),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    assert_eq!(
        observe_one(&mut s, "/etc/pipe"),
        Collection::Failed(CollectionFailure::Unsupported)
    );
    assert_eq!(
        s.apply(&replace("/etc/pipe", 0, FileCondition::Absent)),
        vec![Receipt::Refused]
    );
}

/// A root that is not a directory is refused when the host is opened,
/// rather than making every path under it look absent.
#[test]
fn a_root_that_is_not_a_directory_is_refused() {
    let s = LinuxSubject::new("file-root");
    let file = s.root.join("plain");
    std::fs::write(&file, b"x").unwrap();
    assert!(nomos_substrate_linux::LinuxHost::open(&file).is_err());
}

/// The denied read the suite uses is a real denial: this thread cannot
/// read the file through the standard library either.
#[test]
fn the_denial_is_real() {
    let mut s = LinuxSubject::new("denial");
    s.put(&p("/etc/secret"), b"x");
    s.deny(&p("/etc/secret"));
    assert_eq!(
        std::fs::read(s.host_path(&p("/etc/secret"))).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::PermissionDenied)
    );
}

// ---------------------------------------------------------------------------
// Failure injection for files and directories (`09-file-and-directory`)

fn dir_key(path: &str) -> nomos_core::resource::ResourceKey {
    nomos_core::resource::ResourceKey::Directory(p(path))
}

fn mode(bits: &str) -> nomos_core::resource::Mode {
    nomos_core::resource::Mode::from_octal(bits).unwrap()
}

fn directory(bits: &str) -> nomos_core::condition::Requirement {
    nomos_core::condition::Requirement::Directory(
        nomos_core::condition::DirectoryCondition::Present {
            metadata: nomos_core::condition::Metadata {
                owner: None,
                group: None,
                mode: Some(mode(bits)),
            },
        },
    )
}

fn collected_once(s: &mut LinuxSubject, key: nomos_core::resource::ResourceKey) -> Collection {
    s.observe(&[key])[0].collection().clone()
}

/// A directory replaced by a file between runs: the directory is observed
/// as unsupported, never absent, and neither creating nor removing it
/// touches the file.
#[test]
fn a_directory_replaced_by_a_file_is_not_touched() {
    let mut s = LinuxSubject::new("dir-to-file");
    std::fs::create_dir_all(s.host_path(&p("/srv"))).unwrap();
    let key = dir_key("/srv/data");
    let made = s.apply(&families::converge(&key, 0, directory("0750")));
    assert_eq!(made.last(), Some(&Receipt::Completed { changed: true }));
    std::fs::remove_dir(s.host_path(&p("/srv/data"))).unwrap();
    s.put(&p("/srv/data"), b"a file now");
    assert_eq!(
        collected_once(&mut s, key.clone()),
        Collection::Failed(CollectionFailure::Unsupported)
    );
    for (i, requirement) in [
        directory("0750"),
        nomos_core::condition::Requirement::Directory(
            nomos_core::condition::DirectoryCondition::Absent,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let receipts = s.apply(&families::converge(&key, 1 + i as u32, requirement));
        assert_eq!(receipts, vec![Receipt::Refused]);
    }
    assert_eq!(s.truth(&p("/srv/data")), Truth::File(sha(b"a file now")));
}

/// A symbolic link in place of a directory is not followed: it is observed
/// as unsupported, and no operation reaches the directory it points to.
#[test]
fn a_symbolic_link_in_place_of_a_directory_is_not_followed() {
    use std::os::unix::fs::PermissionsExt;
    let mut s = LinuxSubject::new("dir-link");
    let real = s.host_path(&p("/srv/real"));
    std::fs::create_dir_all(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&real, s.host_path(&p("/srv/link"))).unwrap();
    let key = dir_key("/srv/link");
    assert_eq!(
        collected_once(&mut s, key.clone()),
        Collection::Failed(CollectionFailure::Unsupported)
    );
    let receipts = s.apply(&families::converge(&key, 0, directory("0700")));
    assert_eq!(receipts, vec![Receipt::Refused]);
    let receipts = s.apply(&families::converge(
        &key,
        1,
        nomos_core::condition::Requirement::Directory(
            nomos_core::condition::DirectoryCondition::Absent,
        ),
    ));
    assert_eq!(receipts, vec![Receipt::Refused]);
    let bits = std::fs::metadata(&real).unwrap().permissions().mode() & 0o7777;
    assert_eq!(bits, 0o755, "the linked directory was changed");
    assert!(
        std::fs::symlink_metadata(s.host_path(&p("/srv/link")))
            .unwrap()
            .is_symlink()
    );
}

/// An adapter that changes a file's mode behind Nomos right after each
/// execution, before anything verifies it.
struct ForeignMode(LinuxSubject);

impl Observe for ForeignMode {
    fn observe(
        &mut self,
        resources: &[nomos_core::resource::ResourceKey],
    ) -> Vec<nomos_core::observation::Observation> {
        self.0.observe(resources)
    }
}

impl Mutate for ForeignMode {
    fn apply(&mut self, request: &nomos_core::effect::Apply) -> Vec<Receipt> {
        use std::os::unix::fs::PermissionsExt;
        let receipts = self.0.apply(request);
        if let Some(path) = request.key.resource().path() {
            let at = self.0.host_path(path);
            let _ = std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o666));
        }
        receipts
    }
}

/// A foreign change of mode between execution and verification: the
/// Action is judged on the file as it is, so the run does not end
/// Converged on the receipt's word, and a later run repairs the mode.
#[test]
fn a_foreign_mode_change_before_verification_is_seen() {
    let mut s = ForeignMode(LinuxSubject::new("foreign-mode"));
    s.0.put(&p("/srv/app.conf"), b"old");
    let wanted = s.0.content(b"new");
    let requirement = FileCondition::Present {
        content: Content::Exactly(wanted),
        metadata: nomos_core::condition::Metadata {
            owner: None,
            group: None,
            mode: Some(mode("0600")),
        },
    };
    let canon = Canon::new(
        vec![Managed {
            condition: Condition::file(p("/srv/app.conf"), requirement.clone()),
            keys: BTreeSet::new(),
            disrupts: BTreeSet::new(),
        }],
        vec![],
    );
    let mut cell = Cell::open(support::MemLog::default());
    cell.settle(
        Input::Enforce(support::plan("p1", 1, canon.clone(), 2)),
        &mut s,
    )
    .unwrap();
    assert_ne!(
        cell.snapshot().outcome(),
        Some(&RunOutcome::Converged),
        "converged on a mode that a foreign writer changed"
    );
    // Without the foreign writer, the next run repairs the mode alone.
    let mut plain = s.0;
    cell.settle(Input::Enforce(support::plan("p2", 2, canon, 2)), &mut plain)
        .unwrap();
    assert_eq!(cell.snapshot().outcome(), Some(&RunOutcome::Converged));
    let assessed = assess(
        &Condition::file(p("/srv/app.conf"), requirement),
        &plain.observe(&[f("/srv/app.conf")]),
    );
    assert_eq!(assessed, Assessment::Satisfied);
}

/// ADR 0017 acceptance on Linux: content reaches a file from the Cell's
/// store through the composition root, and a blob whose bytes changed on
/// disk is refused, not written.
#[test]
fn content_comes_from_the_store_and_a_changed_blob_is_refused() {
    use nomos_cell::StoreSource;
    use nomos_store::ContentStore;
    use nomos_store_fs::FsContentStore;
    let scratch = std::env::temp_dir().join(format!("nomos-store-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let mut store = FsContentStore::open(&scratch.join("store")).unwrap();
    let wanted = store.put(b"from the store").unwrap();
    let tampered = store.put(b"will be changed").unwrap();
    let blob = |d: &nomos_core::resource::Digest| {
        let h: String = d.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        scratch.join(format!("store/content/sha256/{}/{h}", &h[..2]))
    };
    std::fs::write(blob(&tampered), b"changed on disk").unwrap();
    let mut s = LinuxSubject::new("store-source");
    s.host = nomos_substrate_linux::LinuxHost::open(&s.root)
        .unwrap()
        .with_source(Box::new(StoreSource(store)));
    s.put(&p("/srv/keep"), b"");
    let done = s.apply(&replace(
        "/srv/a.conf",
        0,
        FileCondition::present(Content::Exactly(wanted)),
    ));
    assert_eq!(done.last(), Some(&Receipt::Completed { changed: true }));
    assert_eq!(
        s.truth(&p("/srv/a.conf")),
        Truth::File(sha(b"from the store"))
    );
    let refused = s.apply(&replace(
        "/srv/b.conf",
        1,
        FileCondition::present(Content::Exactly(tampered)),
    ));
    assert_eq!(refused, vec![Receipt::Refused]);
    assert_eq!(s.truth(&p("/srv/b.conf")), Truth::Absent);
    let _ = std::fs::remove_dir_all(&scratch);
}

// ---------------------------------------------------------------------------
// Packages beneath a scratch root

/// A lock another package manager holds on dpkg's database, as the
/// kernel's lock table shows it, makes a package's collection time out,
/// never read; once released, the package reads as the database says. A
/// package operation beneath a scratch root is refused (Packages on
/// Linux).
#[test]
fn a_held_dpkg_lock_times_the_collection_out() {
    use nomos_core::condition::{PackageCondition, Requirement};
    use nomos_core::observation::PackageEvidence;
    use nomos_core::resource::{PackageName, ResourceKey};
    use std::os::unix::fs::MetadataExt;
    let mut s = LinuxSubject::new("dpkg-lock");
    let dpkg = s.root.join("var/lib/dpkg");
    std::fs::create_dir_all(&dpkg).unwrap();
    std::fs::create_dir_all(s.root.join("proc")).unwrap();
    std::fs::write(
        dpkg.join("status"),
        "Package: hello\nStatus: install ok installed\nArchitecture: all\nVersion: 2.10-3\n",
    )
    .unwrap();
    std::fs::write(dpkg.join("lock-frontend"), b"").unwrap();
    let lock = std::fs::metadata(dpkg.join("lock-frontend")).unwrap();
    let dev = lock.dev();
    let entry = format!(
        "1: POSIX  ADVISORY  WRITE 4242 {:02x}:{:02x}:{} 0 EOF\n",
        rustix::fs::major(dev),
        rustix::fs::minor(dev),
        lock.ino()
    );
    std::fs::write(s.root.join("proc/locks"), &entry).unwrap();
    let key = ResourceKey::Package(PackageName::new("hello").unwrap());

    let started = std::time::Instant::now();
    let seen = s.observe(std::slice::from_ref(&key));
    assert_eq!(
        seen[0].collection(),
        &Collection::Failed(CollectionFailure::TimedOut)
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));

    std::fs::write(s.root.join("proc/locks"), b"").unwrap();
    let seen = s.observe(std::slice::from_ref(&key));
    assert_eq!(
        seen[0].collection(),
        &Collection::Collected(Evidence::Package(PackageEvidence::Installed {
            version: nomos_core::condition::PackageVersion::new("2.10-3").unwrap(),
        }))
    );
    let receipts = s.apply(&families::converge(
        &key,
        1,
        Requirement::Package(PackageCondition::Absent),
    ));
    assert_eq!(receipts, vec![Receipt::Refused]);
}
