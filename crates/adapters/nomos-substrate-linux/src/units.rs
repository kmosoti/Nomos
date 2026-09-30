//! Units through systemd's D-Bus API ([substrate-contract.md], Units on
//! Linux): the manager `org.freedesktop.systemd1` on the system bus, over
//! `zbus`'s blocking client. Never `systemctl` (AGENTS.md rule 5).
//!
//! A unit is examined with `GetUnit` and, when systemd has unloaded it,
//! `GetUnitFileState`. A convergence or refresh sets the unit-file state
//! with `EnableUnitFiles` or `DisableUnitFiles` and `Reload`, then starts,
//! stops, or restarts the unit and waits for the job: its completion is the
//! settlement evidence. A job still pending at the settle-by instant is
//! cancelled, and the execution ends unsettled.
//!
//! [substrate-contract.md]: ../../../../docs/formal/substrate-contract.md

use std::time::Duration;

use nomos_core::condition::{Activity, Enablement, Requirement, UnitCondition};
use nomos_core::effect::{Operation, Receipt};
use nomos_core::observation::{
    ActiveState, Collection, CollectionFailure, UnitEvidence, UnitFileState,
};
use nomos_core::resource::UnitName;
use zbus::blocking::proxy::Builder;
use zbus::blocking::{Connection, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

use crate::OpenError;

const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT: &str = "org.freedesktop.systemd1.Unit";
const JOB: &str = "org.freedesktop.systemd1.Job";

/// The longest the adapter waits for a job, whatever the settle-by instant.
const JOB_LIMIT: Duration = Duration::from_secs(90);
/// How often a pending job is looked at.
const POLL: Duration = Duration::from_millis(20);

/// A connection to systemd on the system bus.
pub struct Systemd {
    bus: Connection,
}

impl std::fmt::Debug for Systemd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Systemd").finish_non_exhaustive()
    }
}

/// The name of a D-Bus error, when the error is one a peer sent.
fn error_name(e: &zbus::Error) -> Option<String> {
    match e {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str().to_string()),
        zbus::Error::FDO(fdo) => {
            use zbus::DBusError;
            Some(fdo.name().as_str().to_string())
        }
        _ => None,
    }
}

/// The failed collection a D-Bus error maps to (Units on Linux).
fn failure(e: &zbus::Error) -> CollectionFailure {
    match error_name(e).as_deref() {
        Some("org.freedesktop.DBus.Error.AccessDenied") => CollectionFailure::PermissionDenied,
        Some("org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout") => {
            CollectionFailure::TimedOut
        }
        _ => CollectionFailure::Io,
    }
}

/// Whether `e` says systemd has no such unit or unit file.
fn unknown(e: &zbus::Error) -> bool {
    matches!(
        error_name(e).as_deref(),
        Some("org.freedesktop.systemd1.NoSuchUnit" | "org.freedesktop.DBus.Error.FileNotFound")
    )
}

/// Whether `e` says the object asked about no longer exists.
fn gone(e: &zbus::Error) -> bool {
    matches!(
        error_name(e).as_deref(),
        Some(
            "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        )
    )
}

/// An `ActiveState` of the family, or `None` for one outside it.
pub fn active_state(text: &str) -> Option<ActiveState> {
    Some(match text {
        "active" => ActiveState::Active,
        "reloading" => ActiveState::Reloading,
        "inactive" => ActiveState::Inactive,
        "failed" => ActiveState::Failed,
        "activating" => ActiveState::Activating,
        "deactivating" => ActiveState::Deactivating,
        _ => return None,
    })
}

/// The family's reading of a `UnitFileState`.
pub fn file_state(text: &str) -> UnitFileState {
    match text {
        "enabled" => UnitFileState::Enabled,
        "disabled" => UnitFileState::Disabled,
        "static" => UnitFileState::Static,
        "masked" | "masked-runtime" => UnitFileState::Masked,
        _ => UnitFileState::Other,
    }
}

/// Whether a unit counts as running.
fn running(active: ActiveState) -> bool {
    matches!(active, ActiveState::Active | ActiveState::Reloading)
}

/// Whether a unit counts as stopped.
fn stopped(active: ActiveState) -> bool {
    matches!(active, ActiveState::Inactive | ActiveState::Failed)
}

/// The unit-file state `enablement` asks for, or `None` when it asks for
/// none.
fn asked(enablement: Enablement) -> Option<UnitFileState> {
    match enablement {
        Enablement::Enabled => Some(UnitFileState::Enabled),
        Enablement::Disabled => Some(UnitFileState::Disabled),
        Enablement::Any => None,
    }
}

/// What an operation does to a unit's activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Start,
    Stop,
    Restart,
}

impl Job {
    fn method(self) -> &'static str {
        match self {
            Job::Start => "StartUnit",
            Job::Stop => "StopUnit",
            Job::Restart => "RestartUnit",
        }
    }

    /// Whether the unit ended where the job was to take it.
    fn succeeded(self, active: ActiveState) -> bool {
        match self {
            Job::Start | Job::Restart => running(active),
            Job::Stop => stopped(active),
        }
    }
}

/// The job, if any, that makes a unit with `evidence` meet `condition`.
fn job(refresh: bool, condition: &UnitCondition, evidence: &UnitEvidence) -> Option<Job> {
    match (refresh, condition.activity) {
        (_, Activity::Inactive) => (!stopped(evidence.active)).then_some(Job::Stop),
        (true, _) => Some(Job::Restart),
        (false, Activity::Active) => (!running(evidence.active)).then_some(Job::Start),
        (false, Activity::Any) => None,
    }
}

impl Systemd {
    /// A connection to the system bus.
    pub fn system() -> Result<Self, OpenError> {
        let bus = Connection::system().map_err(|e| OpenError(std::io::Error::other(e)))?;
        Ok(Systemd { bus })
    }

    fn proxy(&self, path: OwnedObjectPath, interface: &'static str) -> zbus::Result<Proxy<'_>> {
        Builder::new(&self.bus)
            .destination(DESTINATION)?
            .path(path)?
            .interface(interface)?
            .cache_properties(CacheProperties::No)
            .build()
    }

    fn manager(&self) -> zbus::Result<Proxy<'_>> {
        let path = OwnedObjectPath::try_from(MANAGER_PATH).map_err(zbus::Error::from)?;
        self.proxy(path, MANAGER)
    }

    /// The unit's object path, when systemd has it loaded.
    fn loaded(&self, name: &UnitName) -> zbus::Result<OwnedObjectPath> {
        self.manager()?.call("GetUnit", &(name.as_str(),))
    }

    /// What systemd reports of `name` (Units on Linux, Observation).
    pub fn examine(&self, name: &UnitName) -> Collection<UnitEvidence> {
        match self.examine_or_error(name) {
            Ok(collection) => collection,
            Err(e) => Collection::Failed(failure(&e)),
        }
    }

    fn examine_or_error(&self, name: &UnitName) -> zbus::Result<Collection<UnitEvidence>> {
        let path = match self.loaded(name) {
            Ok(path) => path,
            Err(e) if unknown(&e) => {
                let state: zbus::Result<String> =
                    self.manager()?.call("GetUnitFileState", &(name.as_str(),));
                return match state {
                    Ok(state) => Ok(Collection::Collected(UnitEvidence {
                        active: ActiveState::Inactive,
                        file_state: file_state(&state),
                    })),
                    Err(e) if unknown(&e) => Ok(Collection::Failed(CollectionFailure::Unavailable)),
                    Err(e) => Err(e),
                };
            }
            Err(e) => return Err(e),
        };
        let unit = self.proxy(path, UNIT)?;
        let load: String = unit.get_property("LoadState")?;
        if !matches!(load.as_str(), "loaded" | "masked") {
            return Ok(Collection::Failed(CollectionFailure::Unavailable));
        }
        let active: String = unit.get_property("ActiveState")?;
        let state: String = unit.get_property("UnitFileState")?;
        Ok(match active_state(&active) {
            Some(active) => Collection::Collected(UnitEvidence {
                active,
                file_state: file_state(&state),
            }),
            None => Collection::Failed(CollectionFailure::Unsupported),
        })
    }

    /// Performs `operation` on `name`: `None` when a job was still pending
    /// at the deadline and was cancelled, so the outcome is unknown.
    pub fn execute(
        &self,
        name: &UnitName,
        operation: &Operation,
        deadline: impl Fn() -> bool,
    ) -> Option<Receipt> {
        let (refresh, condition) = match operation {
            Operation::Converge(Requirement::Unit(c)) => (false, c),
            Operation::Refresh(Requirement::Unit(c)) => (true, c),
            _ => return Some(Receipt::Refused),
        };
        let Collection::Collected(before) = self.examine(name) else {
            return Some(Receipt::Refused);
        };
        let wanted = asked(condition.enablement);
        let enable = wanted.filter(|w| *w != before.file_state);
        if enable.is_some()
            && !matches!(
                before.file_state,
                UnitFileState::Enabled | UnitFileState::Disabled
            )
        {
            return Some(Receipt::Refused);
        }
        if let Some(state) = enable
            && self.set_enablement(name, state).is_err()
        {
            return Some(Receipt::Failed);
        }
        let job = job(refresh, condition, &before);
        if let Some(job) = job {
            match self.run_job(name, job, &deadline) {
                Ok(true) => {}
                Ok(false) => return None,
                Err(_) => return Some(Receipt::Failed),
            }
        }
        let Collection::Collected(after) = self.examine(name) else {
            return Some(Receipt::Failed);
        };
        let done = wanted.is_none_or(|w| w == after.file_state)
            && job.is_none_or(|j| j.succeeded(after.active));
        Some(if !done {
            Receipt::Failed
        } else if refresh {
            Receipt::Completed { changed: true }
        } else {
            Receipt::Completed {
                changed: after != before,
            }
        })
    }

    fn set_enablement(&self, name: &UnitName, state: UnitFileState) -> zbus::Result<()> {
        let manager = self.manager()?;
        let files = [name.as_str()];
        if state == UnitFileState::Enabled {
            let _: (bool, Vec<(String, String, String)>) =
                manager.call("EnableUnitFiles", &(&files[..], false, false))?;
        } else {
            let _: Vec<(String, String, String)> =
                manager.call("DisableUnitFiles", &(&files[..], false))?;
        }
        manager.call::<_, _, ()>("Reload", &())
    }

    /// Starts `job` and waits for it: `Ok(true)` once it is no longer
    /// pending, `Ok(false)` when it still was at the deadline and was
    /// cancelled.
    fn run_job(
        &self,
        name: &UnitName,
        job: Job,
        deadline: &impl Fn() -> bool,
    ) -> zbus::Result<bool> {
        let manager = self.manager()?;
        let path: OwnedObjectPath = manager.call(job.method(), &(name.as_str(), "replace"))?;
        // The job's own object exists while the job is pending. The unit's
        // is no witness: systemd may unload a unit the moment its job ends.
        let pending = self.proxy(path, JOB)?;
        let limit = std::time::Instant::now() + JOB_LIMIT;
        loop {
            match pending.get_property::<String>("State") {
                Ok(_) => {}
                Err(e) if gone(&e) => return Ok(true),
                Err(e) => return Err(e),
            }
            if deadline() || std::time::Instant::now() >= limit {
                let _: zbus::Result<()> = pending.call("Cancel", &());
                return Ok(false);
            }
            std::thread::sleep(POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The family's six active states and nothing else, and every
    /// unit-file state systemd documents read as the family reads it.
    #[test]
    fn systemd_states_read_as_the_family_reads_them() {
        for (text, state) in [
            ("active", ActiveState::Active),
            ("reloading", ActiveState::Reloading),
            ("inactive", ActiveState::Inactive),
            ("failed", ActiveState::Failed),
            ("activating", ActiveState::Activating),
            ("deactivating", ActiveState::Deactivating),
        ] {
            assert_eq!(active_state(text), Some(state));
        }
        for text in ["maintenance", "refreshing", "", "Active"] {
            assert_eq!(active_state(text), None, "{text}");
        }
        assert_eq!(file_state("enabled"), UnitFileState::Enabled);
        assert_eq!(file_state("disabled"), UnitFileState::Disabled);
        assert_eq!(file_state("static"), UnitFileState::Static);
        assert_eq!(file_state("masked"), UnitFileState::Masked);
        assert_eq!(file_state("masked-runtime"), UnitFileState::Masked);
        for text in [
            "enabled-runtime",
            "linked",
            "linked-runtime",
            "alias",
            "indirect",
            "generated",
            "transient",
            "bad",
            "",
        ] {
            assert_eq!(file_state(text), UnitFileState::Other, "{text}");
        }
    }

    /// The job table of Units on Linux, against every activity and active
    /// state.
    #[test]
    fn the_job_follows_the_table() {
        let states = [
            ActiveState::Active,
            ActiveState::Reloading,
            ActiveState::Inactive,
            ActiveState::Failed,
            ActiveState::Activating,
            ActiveState::Deactivating,
        ];
        for refresh in [false, true] {
            for activity in [Activity::Active, Activity::Inactive, Activity::Any] {
                for active in states {
                    let c = UnitCondition {
                        activity,
                        enablement: Enablement::Any,
                    };
                    let e = UnitEvidence {
                        active,
                        file_state: UnitFileState::Enabled,
                    };
                    let want = match (refresh, activity) {
                        (_, Activity::Inactive) if stopped(active) => None,
                        (_, Activity::Inactive) => Some(Job::Stop),
                        (true, _) => Some(Job::Restart),
                        (false, Activity::Active) if running(active) => None,
                        (false, Activity::Active) => Some(Job::Start),
                        (false, Activity::Any) => None,
                    };
                    assert_eq!(
                        job(refresh, &c, &e),
                        want,
                        "{refresh} {activity:?} {active:?}"
                    );
                }
            }
        }
    }
}
