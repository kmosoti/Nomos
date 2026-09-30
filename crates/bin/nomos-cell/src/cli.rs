//! The Cell's commands ([cell-commands.md]): `traits`, `trace`, `enforce`,
//! `import`, and `events`. This is wiring: every Assessment is core's,
//! every Plan the transition kernel's, and every Event is recomputed from
//! the journal.
//!
//! [cell-commands.md]: ../../../../docs/formal/cell-commands.md

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nomos_app::driver::JournaledCell;
use nomos_app::kernel::{
    ActionRecord, Canon, Input, KernelSnapshot, Plan, Policy, RunOutcome, step,
};
use nomos_canon::artifact::{Profile, Reader, decode};
use nomos_core::assessment::Report;
use nomos_core::effect::EffectRequest;
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::ResourceKey;
use nomos_store::EventLog;
use nomos_store_fs::{FileLog, FsContentStore};
use nomos_substrate::Observe;
use nomos_substrate_linux::LinuxHost;
use nomos_substrate_linux::units::Systemd;

use crate::StoreSource;
use crate::render::{self, ERROR};

/// The state directory when none is named.
pub const STATE: &str = "/var/lib/nomos";
/// The Plan's bound, in iterations.
pub const BOUND: u32 = 8;
/// Actions at once.
pub const CAPACITY: usize = 8;
/// Receipt, verification, and settlement timeouts, in nanoseconds.
pub const TIMEOUT: u64 = 300_000_000_000;

const USAGE: &str = "usage: nomos-cell [--state <dir>] <command>
  traits
  trace   --canon <artifact.cbor>
  enforce --canon <artifact.cbor> [--bundle <dir>]
  import  --bundle <dir>
  events
";

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The command.
    pub command: String,
    /// The state directory.
    pub state: PathBuf,
    /// `--canon`.
    pub canon: Option<PathBuf>,
    /// `--bundle`.
    pub bundle: Option<PathBuf>,
}

/// Parses the arguments after the program's name.
pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut out = Args {
        command: String::new(),
        state: PathBuf::from(STATE),
        canon: None,
        bundle: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--state" => out.state = value("--state")?,
            "--canon" => out.canon = Some(value("--canon")?),
            "--bundle" => out.bundle = Some(value("--bundle")?),
            "-h" | "--help" => return Err(String::new()),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other if out.command.is_empty() => out.command = other.to_string(),
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    let needs = |flag: &Option<PathBuf>, name: &str| {
        if flag.is_none() {
            Err(format!("{} needs {name}", out.command))
        } else {
            Ok(())
        }
    };
    match out.command.as_str() {
        "traits" | "events" => {}
        "trace" | "enforce" => needs(&out.canon, "--canon")?,
        "import" => needs(&out.bundle, "--bundle")?,
        "" => return Err(String::new()),
        other => return Err(format!("unknown command {other}")),
    }
    Ok(out)
}

/// Runs the command line and returns the exit status.
pub fn run(args: &[String], out: &mut impl Write, err: &mut impl Write) -> i32 {
    let args = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                let _ = writeln!(err, "nomos-cell: {e}");
            }
            let _ = write!(err, "{USAGE}");
            return ERROR;
        }
    };
    let result = match args.command.as_str() {
        "traits" => traits(out, err),
        "trace" => trace(&args, out, err),
        "enforce" => enforce(&args, out, err),
        "import" => import(&args, out),
        "events" => events(&args, out),
        _ => Err("unreachable".into()),
    };
    match result {
        Ok(status) => status,
        Err(e) => {
            let _ = writeln!(err, "nomos-cell: {e}");
            ERROR
        }
    }
}

/// The artifact at `path`, decoded with the host reader.
pub fn load(path: &Path) -> Result<Canon, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let decoded = decode(&bytes, Profile::Cbor, &Reader::host())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Canon::try_from(&decoded.canon).map_err(|_| format!("{}: an empty label", path.display()))
}

/// The host, with systemd when a bus answers.
fn host(store: Option<FsContentStore>) -> Result<LinuxHost, String> {
    let mut host = LinuxHost::open(Path::new("/")).map_err(|e| format!("/: {}", e.0))?;
    if let Some(store) = store {
        host = host.with_source(Box::new(StoreSource(store)));
    }
    Ok(match Systemd::system() {
        Ok(systemd) => host.with_systemd(systemd),
        Err(_) => host,
    })
}

fn content(state: &Path) -> Result<FsContentStore, String> {
    let dir = state.join("content");
    FsContentStore::open(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn journal(state: &Path) -> Result<(FileLog<Input>, u64), String> {
    std::fs::create_dir_all(state).map_err(|e| format!("{}: {e}", state.display()))?;
    let path = state.join("journal");
    let (log, recovery) = FileLog::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((log, recovery.truncated))
}

/// The Plan the Cell enforces after `snapshot` (cell-commands.md, Enforce).
pub fn plan(snapshot: &KernelSnapshot, canon: Canon) -> Result<Plan, String> {
    let generation = snapshot.fence().accepted().map_or(1, |(g, _)| g.0 + 1);
    Ok(Plan {
        id: PlanId::new("cell").ok_or("the plan id")?,
        generation: Generation(generation),
        canon,
        bound: BOUND,
        policy: Policy::new(CAPACITY, TIMEOUT, TIMEOUT, TIMEOUT),
        expires: None,
    })
}

fn traits(out: &mut impl Write, err: &mut impl Write) -> Result<i32, String> {
    let (found, missing) = host(None)?.traits();
    let _ = write!(out, "{}", render::traits(&found));
    for key in missing {
        let _ = writeln!(err, "nomos-cell: {key}: its source could not be read");
    }
    Ok(0)
}

/// Trace: enforce's pipeline, stopped before execution and never
/// journaled (cell-commands.md, Trace). `observe` is the only capability
/// it is handed.
pub fn trace_with(
    observe: &mut impl Observe,
    start: KernelSnapshot,
    recovered: bool,
    canon: Canon,
    now: nomos_core::observation::Instant,
) -> Result<(Report, BTreeMap<ResourceKey, ActionRecord>), String> {
    let conditions = canon.conditions();
    let mut snapshot = start;
    let mut queue = VecDeque::new();
    if recovered {
        queue.push_back(Input::Recovered);
    }
    queue.push_back(Input::Tick(now));
    queue.push_back(Input::Enforce(plan(&snapshot, canon)?));
    let mut last = Vec::new();
    // A Decision asks for Observations, then plans; a kernel that kept
    // asking would be a bug, and trace stops rather than loop.
    let mut steps = 0;
    while let Some(input) = queue.pop_front() {
        steps += 1;
        if steps > 16 {
            return Err("the kernel asked for Observations without planning".into());
        }
        let decision = step(&snapshot, input);
        snapshot = decision.snapshot;
        let mut applies = false;
        for effect in decision.effects {
            match effect {
                EffectRequest::Observe(keys) => {
                    let observed = observe.observe(&keys);
                    last = observed.clone();
                    queue.push_back(Input::Observed(observed));
                }
                EffectRequest::Apply(_) => applies = true,
            }
        }
        if applies {
            break;
        }
    }
    let actions = snapshot
        .round()
        .map(|r| r.actions.clone())
        .unwrap_or_default();
    Ok((Report::assess(&conditions, &last), actions))
}

fn trace(args: &Args, out: &mut impl Write, _err: &mut impl Write) -> Result<i32, String> {
    let canon = load(args.canon.as_deref().ok_or("--canon")?)?;
    // The journal is read, never appended to: a trace records nothing.
    let path = args.state.join("journal");
    let (start, recovered) = if path.exists() {
        let (log, _) =
            FileLog::<Input>::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let recovered = !log.events().is_empty();
        (JournaledCell::open(log).snapshot().clone(), recovered)
    } else {
        (KernelSnapshot::new(), false)
    };
    let mut host = host(None)?;
    let (report, actions) = trace_with(
        &mut host,
        start,
        recovered,
        canon,
        nomos_substrate_linux::now(),
    )?;
    let (text, status) = render::trace(&report, &actions);
    let _ = write!(out, "{text}");
    Ok(status)
}

fn import(args: &Args, out: &mut impl Write) -> Result<i32, String> {
    let dir = args.bundle.as_deref().ok_or("--bundle")?;
    let mut store = content(&args.state)?;
    let n = store
        .import(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let _ = writeln!(out, "imported {n} blobs from {}", dir.display());
    Ok(0)
}

fn enforce(args: &Args, out: &mut impl Write, err: &mut impl Write) -> Result<i32, String> {
    let canon = load(args.canon.as_deref().ok_or("--canon")?)?;
    if args.bundle.is_some() {
        import(args, out)?;
    }
    let store = content(&args.state)?;
    let (log, truncated) = journal(&args.state)?;
    if truncated > 0 {
        let _ = writeln!(
            err,
            "nomos-cell: the journal's last record was cut short; {truncated} bytes truncated"
        );
    }
    let recovered = !log.events().is_empty();
    let mut host = host(Some(store))?;
    let mut cell = JournaledCell::open(log);
    let full = |_| "the journal refused an append".to_string();
    if recovered {
        cell.settle(Input::Recovered, &mut host).map_err(full)?;
    }
    cell.settle(Input::Tick(nomos_substrate_linux::now()), &mut host)
        .map_err(full)?;
    let plan = plan(cell.snapshot(), canon)?;
    let generation = plan.generation;
    cell.settle(Input::Enforce(plan), &mut host).map_err(full)?;
    let limit = std::time::Instant::now() + Duration::from_nanos(3 * TIMEOUT);
    let outcome: RunOutcome = loop {
        let current = cell
            .snapshot()
            .run()
            .filter(|r| r.plan.generation == generation)
            .and_then(|_| cell.snapshot().outcome().cloned());
        if let Some(o) = current {
            break o;
        }
        if cell
            .snapshot()
            .fence()
            .accepted()
            .is_none_or(|(g, _)| g != generation)
        {
            return Err("the kernel did not accept the Plan".into());
        }
        if std::time::Instant::now() >= limit {
            return Err("the run did not end before the Plan's timeouts".into());
        }
        std::thread::sleep(Duration::from_secs(1));
        cell.settle(Input::Tick(nomos_substrate_linux::now()), &mut host)
            .map_err(full)?;
    };
    let (text, status) = render::outcome(&outcome);
    let _ = write!(out, "{text}");
    let _ = writeln!(out, "executions: {}", host.executions());
    Ok(status)
}

fn events(args: &Args, out: &mut impl Write) -> Result<i32, String> {
    let path = args.state.join("journal");
    if !path.exists() {
        return Ok(0);
    }
    let (log, _) = FileLog::<Input>::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    for (i, e) in JournaledCell::open(log).events().iter().enumerate() {
        let _ = writeln!(out, "{} {}", i + 1, render::event(e));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn the_command_line_reads_as_the_usage_says() {
        let a = args(&[
            "--state", "/tmp/s", "enforce", "--canon", "c.cbor", "--bundle", "b",
        ])
        .unwrap();
        assert_eq!(a.command, "enforce");
        assert_eq!(a.state, PathBuf::from("/tmp/s"));
        assert_eq!(a.canon, Some(PathBuf::from("c.cbor")));
        assert_eq!(a.bundle, Some(PathBuf::from("b")));
        assert_eq!(args(&["traits"]).unwrap().state, PathBuf::from(STATE));
        assert!(args(&["trace"]).is_err(), "trace needs --canon");
        assert!(args(&["import"]).is_err(), "import needs --bundle");
        assert!(args(&["enforce", "--canon"]).is_err());
        assert!(args(&["frobnicate"]).is_err());
        assert!(args(&["events", "extra"]).is_err());
        assert!(args(&[]).is_err());
    }
}
