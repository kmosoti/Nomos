//! The Cell's state directory (`docs/formal/cell-commands.md`, State
//! Directory; alpha.2 issues #30, #36, and #37): who may mutate it, what a
//! read-only command may do to it, and what makes it safe to trust.
//!
//! **What these tests establish.** Real operating-system processes contend
//! for the lease: one child process holds a state directory, and the
//! contenders are this test process and the installed binary. That is
//! process concurrency on the host's `flock`, which is not the simulator's
//! interleaving of kernel inputs (`bounded_convergence`,
//! `refresh_recovery`): the simulator shows the kernel's decisions do not
//! depend on delivery order, and says nothing of two Cells at once.
//!
//! The oracle is the specification's State Directory section. Every test
//! that judges a rule has a semantic mutant in the corpus that breaks the
//! rule, named beside it.

mod scratch;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nomos_app::kernel::Input;
use nomos_canon::artifact::{Profile, encode};
use nomos_canon::model::CanonBuilder;
use nomos_cell::cli;
use nomos_core::condition::{DirectoryCondition, Metadata};
use nomos_store::EventLog;
use nomos_store_fs::{FileLog, StateError, StateLease};

/// The tests of this file run one at a time. A lock belongs to an open file
/// description, and a process forked by one test's thread shares every
/// descriptor of the test binary, the lock file of another test's lease
/// among them, until it execs; a lease just released could still look held.
/// That is how `flock` works, and it cannot hurt the Cell, whose lease file
/// is close-on-exec; it would only make these tests flaky.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

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

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// A Canon of `names.len()` directories beneath `dir`, as an artifact.
fn canon(dir: &Path, names: &[&str]) -> PathBuf {
    let mut builder = CanonBuilder::new("state-ownership");
    for name in names {
        builder = builder.directory(
            dir.join(name).to_str().unwrap(),
            DirectoryCondition::Present {
                metadata: Metadata::any(),
            },
        );
    }
    let file = dir.join("canon.cbor");
    std::fs::write(&file, encode(&builder.build().unwrap(), Profile::Cbor)).unwrap();
    file
}

// ---------------------------------------------------------------------------
// A second process, holding a state directory

/// The child half: when the environment names a state directory, take the
/// lease, say so, and hold it until killed. Without it, a test that passes.
#[test]
fn holder_child() {
    let Ok(dir) = std::env::var("NOMOS_TEST_HOLD") else {
        return;
    };
    let _lease = StateLease::acquire(Path::new(&dir)).unwrap();
    println!("HELD");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// A process of this test binary that holds `state`, until it is killed or
/// dropped.
struct Holder(Child);

impl Holder {
    fn spawn(state: &Path) -> Holder {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "holder_child", "--nocapture", "--test-threads=1"])
            .env("NOMOS_TEST_HOLD", state)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let held = BufReader::new(out)
            .lines()
            .map_while(Result::ok)
            .any(|line| line.ends_with("HELD"));
        assert!(held, "the holder process did not take the state directory");
        Holder(child)
    }

    /// Kills the holder the hard way and waits for the system to reap it.
    fn kill(mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ---------------------------------------------------------------------------
// Ownership (#30)

/// SM-CELL-005: `enforce` does not take the lease. A second `enforce` of a
/// state directory another process owns does nothing at all: no journal, no
/// content, no effect on the host.
#[test]
fn enforce_is_refused_while_another_process_holds_the_state() {
    let _one = serial();
    let dir = scratch::dir("own", "enforce");
    let state = dir.join("state");
    let artifact = canon(&dir, &["target"]);
    let holder = Holder::spawn(&state);

    let (status, out, err) = run(&["--state", s(&state), "enforce", "--canon", s(&artifact)]);
    assert_eq!(status, 1, "{out}{err}");
    assert!(
        err.contains("already in use"),
        "the error names the state directory as in use: {err}"
    );
    assert!(err.contains(s(&state)), "{err}");
    assert!(!dir.join("target").exists(), "an effect was issued");
    assert!(!state.join("journal").exists(), "the journal was opened");
    assert!(!state.join("content").exists(), "content was imported");
    drop(holder);
}

/// The service and a command run by hand are the same process image taking
/// the same lease: the installed binary is refused as the library is.
#[test]
fn the_binary_is_refused_while_another_process_holds_the_state() {
    let _one = serial();
    let dir = scratch::dir("own", "binary");
    let state = dir.join("state");
    let artifact = canon(&dir, &["target"]);
    let holder = Holder::spawn(&state);

    let out = Command::new(env!("CARGO_BIN_EXE_nomos-cell"))
        .args(["--state", s(&state), "enforce", "--canon", s(&artifact)])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already in use"));
    assert!(!dir.join("target").exists());
    assert!(!state.join("journal").exists());
    drop(holder);
}

/// `import` is mutation too: it cannot write the content store of a
/// directory another process owns.
#[test]
fn import_is_refused_while_another_process_holds_the_state() {
    let _one = serial();
    let dir = scratch::dir("own", "import");
    let state = dir.join("state");
    let bundle = dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let blob = b"content";
    let name: String = nomos_canon::sha256::digest(blob)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(bundle.join(name), blob).unwrap();
    let holder = Holder::spawn(&state);

    let (status, out, err) = run(&["--state", s(&state), "import", "--bundle", s(&bundle)]);
    assert_eq!(status, 1, "{out}{err}");
    assert!(err.contains("already in use"), "{err}");
    assert!(!state.join("content").exists(), "content was imported");
    drop(holder);
}

/// SM-CELL-006: the lease is dropped as soon as it is taken. While a run
/// mutates, no other process can take the state directory, so the journal
/// has not changed since any moment another process held the lease.
#[test]
fn the_lease_is_held_for_the_whole_run() {
    let _one = serial();
    let dir = scratch::dir("own", "whole-run");
    let state = dir.join("state");
    let names: Vec<String> = (0..30).map(|i| format!("d{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let artifact = canon(&dir, &refs);

    let journal = state.join("journal");
    let done = Arc::new(AtomicBool::new(false));
    let taken: Arc<Mutex<Vec<u64>>> = Arc::default();
    let watcher = {
        let (state, journal, done, taken) =
            (state.clone(), journal.clone(), done.clone(), taken.clone());
        std::thread::spawn(move || {
            // From the moment the run has begun, until it has ended.
            while !journal.exists() && !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_micros(200));
            }
            while !done.load(Ordering::SeqCst) {
                if let Ok(lease) = StateLease::acquire(&state) {
                    let length = std::fs::metadata(&journal).map_or(0, |m| m.len());
                    taken.lock().unwrap().push(length);
                    drop(lease);
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        })
    };
    let (status, out, err) = run(&["--state", s(&state), "enforce", "--canon", s(&artifact)]);
    done.store(true, Ordering::SeqCst);
    watcher.join().unwrap();

    assert_eq!(status, 0, "{out}{err}");
    let last = std::fs::metadata(&journal).unwrap().len();
    for length in taken.lock().unwrap().iter() {
        assert_eq!(
            *length, last,
            "another process took the state directory while the run was still writing"
        );
    }
}

/// A holder that dies, by any means, gives up the state directory with no
/// cleanup by hand, even to a contender that is waiting for it.
#[test]
fn a_killed_holder_releases_the_state_to_a_waiting_contender() {
    let _one = serial();
    let dir = scratch::dir("own", "killed");
    let state = dir.join("state");
    let holder = Holder::spawn(&state);
    assert_eq!(
        StateLease::acquire(&state).unwrap_err(),
        StateError::Held(state.clone())
    );

    let contender = {
        let state = state.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            loop {
                match StateLease::acquire(&state) {
                    Err(StateError::Held(_)) if start.elapsed() < Duration::from_secs(30) => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    other => return other.map(drop),
                }
            }
        })
    };
    std::thread::sleep(Duration::from_millis(300));
    assert!(!contender.is_finished(), "the contender got in early");
    holder.kill();
    assert_eq!(contender.join().unwrap(), Ok(()));
    assert!(
        state.join("lock").exists(),
        "the lock file stays, and is not what excludes"
    );
}

/// Different state directories are independent: one held, the other usable,
/// by a command as by a lease.
#[test]
fn different_state_directories_proceed_independently() {
    let _one = serial();
    let dir = scratch::dir("own", "independent");
    let (held, free) = (dir.join("held"), dir.join("free"));
    let bundle = dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let blob = b"independent";
    let name: String = nomos_canon::sha256::digest(blob)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(bundle.join(name), blob).unwrap();
    let holder = Holder::spawn(&held);

    let (status, out, err) = run(&["--state", s(&free), "import", "--bundle", s(&bundle)]);
    assert_eq!(status, 0, "{out}{err}");
    assert!(out.contains("imported 1 blobs"), "{out}");
    drop(StateLease::acquire(&dir.join("another")).unwrap());
    drop(holder);
}

// ---------------------------------------------------------------------------
// Read-only commands (#36)

/// A journal of `ticks` batches, one input each, in a state directory the
/// Cell would have made.
fn journal_of(dir: &Path, ticks: &[u64]) -> PathBuf {
    let state = dir.join("state");
    drop(StateLease::acquire(&state).unwrap());
    let (mut log, _) = FileLog::<Input>::open(&state.join("journal")).unwrap();
    for t in ticks {
        log.append(&[Input::Tick(nomos_core::observation::Instant(*t))])
            .unwrap();
    }
    state
}

/// Every file under `dir` with its bytes and its modification time.
fn fingerprint(dir: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
                out.push((path.clone(), std::fs::read(&path).unwrap(), modified));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn trace_and_events_leave_a_normal_journal_as_they_found_it() {
    let _one = serial();
    let dir = scratch::dir("own", "normal");
    let state = journal_of(&dir, &[1, 2, 3]);
    let artifact = canon(&dir, &["target"]);
    let before = fingerprint(&state);

    let (status, out, err) = run(&["--state", s(&state), "trace", "--canon", s(&artifact)]);
    assert_eq!(status, 3, "a directory to make is a Variance: {out}{err}");
    assert!(!dir.join("target").exists(), "trace made it");
    let (status, out, _) = run(&["--state", s(&state), "events"]);
    assert_eq!(status, 0);
    assert!(out.contains("clock 1"), "{out}");
    assert_eq!(fingerprint(&state), before, "bytes or times changed");
}

#[test]
fn trace_and_events_create_nothing_when_there_is_no_journal() {
    let _one = serial();
    let dir = scratch::dir("own", "absent");
    let artifact = canon(&dir, &["target"]);

    let absent = dir.join("absent");
    let (status, out, err) = run(&["--state", s(&absent), "trace", "--canon", s(&artifact)]);
    assert_eq!(status, 3, "{out}{err}");
    let (status, out, _) = run(&["--state", s(&absent), "events"]);
    assert_eq!((status, out.as_str()), (0, ""));
    assert!(!absent.exists(), "a read-only command made the directory");

    // A state directory with no journal gets none, and no lock file.
    let empty = dir.join("empty");
    std::fs::create_dir(&empty).unwrap();
    std::fs::set_permissions(&empty, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let (status, ..) = run(&["--state", s(&empty), "trace", "--canon", s(&artifact)]);
    assert_eq!(status, 3);
    let (status, ..) = run(&["--state", s(&empty), "events"]);
    assert_eq!(status, 0);
    assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);
}

/// SM-CELL-007: `trace` opens the journal as `enforce` does, so a torn
/// final record is truncated. It is reported, and left where it is.
#[test]
fn a_torn_journal_is_not_repaired_by_trace_or_events() {
    let _one = serial();
    let dir = scratch::dir("own", "torn");
    let state = journal_of(&dir, &[1]);
    let journal = state.join("journal");
    let whole = std::fs::read(&journal).unwrap();
    let mut torn = whole.clone();
    torn.extend_from_slice(&whole[..whole.len() - 5]);
    std::fs::write(&journal, &torn).unwrap();
    let artifact = canon(&dir, &["target"]);
    let before = fingerprint(&state);

    let (status, _, err) = run(&["--state", s(&state), "trace", "--canon", s(&artifact)]);
    assert_eq!(status, 3, "{err}");
    assert!(err.contains("cut short"), "trace says so: {err}");
    assert!(err.contains("left in place"), "{err}");
    let (status, out, err) = run(&["--state", s(&state), "events"]);
    assert_eq!(status, 0);
    assert!(err.contains("cut short"), "{err}");
    assert!(
        out.contains("clock 1"),
        "the whole record before it is read: {out}"
    );
    assert_eq!(std::fs::read(&journal).unwrap(), torn, "the tail was cut");
    assert_eq!(fingerprint(&state), before);
}

/// Journal damage is an error, never a Variance, and is not touched.
#[test]
fn a_corrupted_journal_is_an_error_to_trace_and_events_and_is_not_touched() {
    let _one = serial();
    let dir = scratch::dir("own", "corrupt");
    let state = journal_of(&dir, &[1, 2]);
    let journal = state.join("journal");
    let mut bytes = std::fs::read(&journal).unwrap();
    bytes[10] ^= 0xff;
    std::fs::write(&journal, &bytes).unwrap();
    let artifact = canon(&dir, &["target"]);
    let before = fingerprint(&state);

    let (status, _, err) = run(&["--state", s(&state), "trace", "--canon", s(&artifact)]);
    assert_eq!(status, 1, "{err}");
    assert!(err.contains("corrupt"), "{err}");
    let (status, out, err) = run(&["--state", s(&state), "events"]);
    assert_eq!((status, out.as_str()), (1, ""), "{err}");
    assert!(err.contains("corrupt"), "{err}");
    assert_eq!(fingerprint(&state), before);
}

/// A read-only command takes no lease: it reads a directory another process
/// owns, as the specification says.
#[test]
fn trace_and_events_read_a_state_directory_another_process_owns() {
    let _one = serial();
    let dir = scratch::dir("own", "reader");
    let state = dir.join("state");
    let artifact = canon(&dir, &["target"]);
    let holder = Holder::spawn(&state);

    let (status, out, err) = run(&["--state", s(&state), "trace", "--canon", s(&artifact)]);
    assert_eq!(status, 3, "{out}{err}");
    let (status, ..) = run(&["--state", s(&state), "events"]);
    assert_eq!(status, 0);
    drop(holder);
}

// ---------------------------------------------------------------------------
// Safety (#37)

fn mode(path: &Path, bits: u32) {
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(bits)).unwrap();
}

/// A state directory the Cell did not make and cannot trust is refused by
/// every command, before the journal is read and before anything is written.
#[test]
fn an_unsafe_state_directory_is_refused_by_every_command() {
    let _one = serial();
    let dir = scratch::dir("own", "unsafe");
    let artifact = canon(&dir, &["target"]);
    let bundle = dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();

    let open = dir.join("open");
    std::fs::create_dir(&open).unwrap();
    mode(&open, 0o755);
    let real = dir.join("real");
    std::fs::create_dir(&real).unwrap();
    mode(&real, 0o700);
    let link = dir.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    for (name, state) in [("open to others", &open), ("a link", &link)] {
        for args in [
            vec!["enforce", "--canon", s(&artifact)],
            vec!["import", "--bundle", s(&bundle)],
            vec!["trace", "--canon", s(&artifact)],
            vec!["events"],
        ] {
            let mut full = vec!["--state", s(state)];
            full.extend(args.iter().copied());
            let (status, out, err) = run(&full);
            assert_eq!(status, 1, "{name}, {args:?}: {out}{err}");
            assert!(err.contains("unsafe state"), "{name}, {args:?}: {err}");
        }
    }
    assert!(!dir.join("target").exists(), "an effect was issued");
    assert_eq!(std::fs::read_dir(&open).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&real).unwrap().count(), 0);
}

/// A journal or content directory that is not what the Cell made is refused
/// when the state is opened, and the command stops.
#[test]
fn an_unsafe_journal_or_content_store_is_refused_before_replay() {
    let _one = serial();
    let artifact_dir = scratch::dir("own", "components");
    let artifact = canon(&artifact_dir, &["target"]);
    let bundle = artifact_dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let blob = b"component";
    let name: String = nomos_canon::sha256::digest(blob)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(bundle.join(name), blob).unwrap();

    // A journal open to others.
    let state = journal_of(&artifact_dir, &[1]);
    mode(&state.join("journal"), 0o644);
    let (status, _, err) = run(&["--state", s(&state), "enforce", "--canon", s(&artifact)]);
    assert_eq!(status, 1, "{err}");
    assert!(err.contains("unsafe state"), "{err}");
    let (status, _, err) = run(&["--state", s(&state), "events"]);
    assert_eq!(status, 1, "{err}");
    assert!(err.contains("unsafe state"), "{err}");
    assert!(!artifact_dir.join("target").exists());

    // A journal that is a link.
    let linked = artifact_dir.join("linked");
    drop(StateLease::acquire(&linked).unwrap());
    std::os::unix::fs::symlink("/dev/null", linked.join("journal")).unwrap();
    let (status, _, err) = run(&["--state", s(&linked), "enforce", "--canon", s(&artifact)]);
    assert_eq!(status, 1, "{err}");
    assert!(err.contains("unsafe state"), "{err}");

    // A content directory that is a link.
    let content = artifact_dir.join("content-link");
    drop(StateLease::acquire(&content).unwrap());
    std::os::unix::fs::symlink(&artifact_dir, content.join("content")).unwrap();
    let (status, _, err) = run(&["--state", s(&content), "import", "--bundle", s(&bundle)]);
    assert_eq!(status, 1, "{err}");
    assert!(err.contains("unsafe state"), "{err}");
}

/// A custom state directory made by the Cell is private, and works as the
/// packaged one does: a run, then a second run, then a reader.
#[test]
fn a_custom_state_directory_the_cell_made_is_private_and_works() {
    let _one = serial();
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch::dir("own", "custom");
    let state = dir.join("a").join("b").join("state");
    let artifact = canon(&dir, &["target"]);

    for _ in 0..2 {
        let (status, out, err) = run(&["--state", s(&state), "enforce", "--canon", s(&artifact)]);
        assert_eq!(status, 0, "{out}{err}");
    }
    let m = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(m(state.clone()), 0o700);
    assert_eq!(m(state.join("journal")), 0o600);
    assert_eq!(m(state.join("lock")), 0o600);
    let (status, out, _) = run(&["--state", s(&state), "events"]);
    assert_eq!(status, 0);
    assert!(out.contains("plan-accepted cell generation 2"), "{out}");
}
