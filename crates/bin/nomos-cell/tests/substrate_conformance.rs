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
use nomos_app::kernel::{Canon, Input, Kind, Managed, RunOutcome};
use nomos_core::assessment::{Assessment, Reason, assess};
use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::effect::Receipt;
use nomos_core::observation::{Collection, CollectionFailure, FileEvidence};
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
            condition: Condition::file(
                p(path),
                FileCondition::Present {
                    content: Content::Exactly(digest),
                },
            ),
            kind: Kind::File,
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
            p("/etc/app/app.conf"),
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
        resources: &[nomos_core::resource::ResourcePath],
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
    *s.observe(&[p(path)])[0].collection()
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
        FileCondition::Present {
            content: Content::Exactly(wanted),
        },
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
        Collection::Collected(FileEvidence::Absent)
    );
}

/// A foreign writer changes the file after the execution and before
/// verification: core judges the file as it is, not as the receipt said.
#[test]
fn verification_sees_a_foreign_write() {
    let mut s = LinuxSubject::new("foreign");
    s.put(&p("/etc/app.conf"), b"old");
    let wanted = s.content(b"new");
    let requirement = FileCondition::Present {
        content: Content::Exactly(wanted),
    };
    let receipts = s.apply(&replace("/etc/app.conf", 0, requirement));
    assert_eq!(receipts.last(), Some(&Receipt::Completed { changed: true }));
    s.put(&p("/etc/app.conf"), b"foreign");
    let assessed = assess(
        &Condition::file(p("/etc/app.conf"), requirement),
        &s.observe(&[p("/etc/app.conf")]),
    );
    assert!(matches!(assessed, Assessment::Variance(_)), "{assessed:?}");
}

/// Replacement leaves no temporary file behind, succeeds or not.
#[test]
fn replacement_leaves_no_temporary_file() {
    let mut s = LinuxSubject::new("temporary");
    s.put(&p("/etc/x"), b"old");
    let wanted = s.content(b"new");
    s.apply(&replace(
        "/etc/x",
        0,
        FileCondition::Present {
            content: Content::Exactly(wanted),
        },
    ));
    let names: Vec<String> = std::fs::read_dir(s.host_path(&p("/etc")))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["x".to_string()]);
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
