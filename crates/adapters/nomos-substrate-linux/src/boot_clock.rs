//! Cross-reboot monotonic clock continuity bound to `/proc/sys/kernel/random/boot_id` and wall time.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::fd::AsFd as _;
use std::path::Path;

use nomos_core::observation::Instant;
use rustix::fs::{AtFlags, Mode, OFlags};
use rustix::time::{ClockId, clock_gettime};

const BOOT_EPOCH_FILE: &str = "boot-epoch";
const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
/// Minimum positive advance (1s) applied across a reboot even if NTP stepped wall clock backwards.
const REBOOT_MIN_ADVANCE_NS: u64 = 1_000_000_000;

/// Persisted monotonic epoch anchor binding kernel instants across Linux reboots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootEpochRecord {
    /// Kernel boot UUID read from `/proc/sys/kernel/random/boot_id`.
    pub boot_id: String,
    /// Last raw `CLOCK_MONOTONIC` nanosecond reading observed under `boot_id`.
    pub last_mono_ns: u64,
    /// Last `CLOCK_REALTIME` nanosecond reading observed alongside `last_mono_ns`.
    pub last_wall_ns: u64,
    /// Monotonic offset added to raw `CLOCK_MONOTONIC` on this boot.
    pub offset_ns: u64,
}

impl BootEpochRecord {
    /// Serializes the 4-line epoch record.
    pub fn encode(&self) -> String {
        format!(
            "boot_id={}\nlast_mono_ns={}\nlast_wall_ns={}\noffset_ns={}\n",
            self.boot_id, self.last_mono_ns, self.last_wall_ns, self.offset_ns
        )
    }

    /// Parses a persisted epoch record, rejecting malformed fields.
    pub fn decode(raw: &str) -> Option<Self> {
        let mut boot_id = None;
        let mut last_mono_ns = None;
        let mut last_wall_ns = None;
        let mut offset_ns = None;
        for line in raw.lines() {
            if let Some(v) = line.strip_prefix("boot_id=") {
                let clean = v.trim();
                if !clean.is_empty() && clean.bytes().all(|b| b.is_ascii_graphic()) {
                    boot_id = Some(clean.to_owned());
                }
            } else if let Some(v) = line.strip_prefix("last_mono_ns=") {
                last_mono_ns = v.trim().parse::<u64>().ok();
            } else if let Some(v) = line.strip_prefix("last_wall_ns=") {
                last_wall_ns = v.trim().parse::<u64>().ok();
            } else if let Some(v) = line.strip_prefix("offset_ns=") {
                offset_ns = v.trim().parse::<u64>().ok();
            }
        }
        Some(Self {
            boot_id: boot_id?,
            last_mono_ns: last_mono_ns?,
            last_wall_ns: last_wall_ns?,
            offset_ns: offset_ns?,
        })
    }
}

/// Computes an updated [`BootEpochRecord`] ensuring post-reboot instants advance across
/// `/proc/sys/kernel/random/boot_id` changes without discarding wall-clock elapsed time
/// or shrinking below `min_persisted_instant_ns`.
pub fn advance_for_state(
    current_boot_id: &str,
    cur_mono_ns: u64,
    cur_wall_ns: u64,
    prev: Option<&BootEpochRecord>,
    min_persisted_instant_ns: u64,
) -> BootEpochRecord {
    let offset_ns = match prev {
        Some(p) if p.boot_id == current_boot_id => {
            let floor_offset = min_persisted_instant_ns.saturating_sub(cur_mono_ns);
            let prev_high = p.offset_ns.saturating_add(p.last_mono_ns);
            let mono_floor = prev_high.saturating_sub(cur_mono_ns);
            p.offset_ns.max(floor_offset).max(mono_floor)
        }
        Some(p) => {
            let prev_high = p
                .offset_ns
                .saturating_add(p.last_mono_ns)
                .max(min_persisted_instant_ns);
            let wall_elapsed = cur_wall_ns
                .saturating_sub(p.last_wall_ns)
                .max(REBOOT_MIN_ADVANCE_NS);
            let target_now = prev_high.saturating_add(wall_elapsed);
            target_now.saturating_sub(cur_mono_ns)
        }
        None if min_persisted_instant_ns > cur_mono_ns => min_persisted_instant_ns
            .saturating_add(REBOOT_MIN_ADVANCE_NS)
            .saturating_sub(cur_mono_ns),
        None => 0,
    };

    BootEpochRecord {
        boot_id: current_boot_id.to_owned(),
        last_mono_ns: cur_mono_ns,
        last_wall_ns: cur_wall_ns,
        offset_ns,
    }
}

/// Computes read-only monotonic offset for `trace` without writing to `state_dir`.
pub fn read_boot_clock_offset(state_dir: &Path, min_persisted_instant_ns: u64) -> u64 {
    let cur_boot_id = current_boot_id();
    let cur_mono_ns = crate::now().0;
    let cur_wall_ns = current_wall_ns();
    let prev = read_record(&state_dir.join(BOOT_EPOCH_FILE));
    advance_for_state(
        &cur_boot_id,
        cur_mono_ns,
        cur_wall_ns,
        prev.as_ref(),
        min_persisted_instant_ns,
    )
    .offset_ns
}

/// Reconciles and atomically persists the boot epoch anchor under `state_dir`, returning
/// the `offset_ns` to configure on [`crate::LinuxHost::with_clock_offset_ns`].
pub fn reconcile_boot_clock(
    state_dir: &Path,
    min_persisted_instant_ns: u64,
) -> Result<u64, String> {
    let cur_boot_id = current_boot_id();
    let cur_mono_ns = crate::now().0;
    let cur_wall_ns = current_wall_ns();
    let path = state_dir.join(BOOT_EPOCH_FILE);
    let prev = read_record(&path);
    let next = advance_for_state(
        &cur_boot_id,
        cur_mono_ns,
        cur_wall_ns,
        prev.as_ref(),
        min_persisted_instant_ns,
    );
    write_record_atomically(state_dir, &next)?;
    Ok(next.offset_ns)
}

/// Returns current effective `Instant` given `offset_ns`.
pub fn instant_with_offset(offset_ns: u64) -> Instant {
    Instant(crate::now().0.saturating_add(offset_ns))
}

fn current_boot_id() -> String {
    if let Ok(mut f) = File::open(BOOT_ID_PATH) {
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    "unknown-boot".to_owned()
}

fn current_wall_ns() -> u64 {
    let t = clock_gettime(ClockId::Realtime);
    u64::try_from(t.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::try_from(t.tv_nsec).unwrap_or(0))
}

fn read_record(path: &Path) -> Option<BootEpochRecord> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let file = File::from(fd);
    let mut text = String::new();
    file.take(4096).read_to_string(&mut text).ok()?;
    BootEpochRecord::decode(&text)
}

fn write_record_atomically(state_dir: &Path, record: &BootEpochRecord) -> Result<(), String> {
    let dir_fd = rustix::fs::open(
        state_dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| format!("open state dir `{}`: {e}", state_dir.display()))?;
    let temp = format!(".{BOOT_EPOCH_FILE}.tmp.{}", std::process::id());
    let _ = rustix::fs::unlinkat(dir_fd.as_fd(), temp.as_str(), AtFlags::empty());
    let fd = rustix::fs::openat(
        dir_fd.as_fd(),
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| format!("create temp boot-epoch: {e}"))?;
    let mut file = File::from(fd);
    let write_res = file
        .write_all(record.encode().as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = write_res {
        let _ = rustix::fs::unlinkat(dir_fd.as_fd(), temp.as_str(), AtFlags::empty());
        return Err(format!("write boot-epoch: {err}"));
    }
    rustix::fs::renameat(
        dir_fd.as_fd(),
        temp.as_str(),
        dir_fd.as_fd(),
        BOOT_EPOCH_FILE,
    )
    .map_err(|e| {
        let _ = rustix::fs::unlinkat(dir_fd.as_fd(), temp.as_str(), AtFlags::empty());
        format!("rename boot-epoch: {e}")
    })?;
    let _ = rustix::fs::fsync(dir_fd.as_fd());
    Ok(())
}
