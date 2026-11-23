//! Regression tests for Linux strict deadline refusal (#8) and cross-reboot clock continuity (#10).

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nomos_core::condition::{Content, FileCondition, Metadata, Requirement};
use nomos_core::effect::{Apply, EffectKey, Operation, Receipt};
use nomos_core::observation::Instant;
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::{ResourceKey, ResourcePath};
use nomos_substrate::Mutate;
use nomos_substrate_linux::LinuxHost;
use nomos_substrate_linux::boot_clock::{
    BootEpochRecord, advance_for_state, read_boot_clock_offset, reconcile_boot_clock,
};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "nomos-linux-sec-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("etc")).unwrap();
        fs::write(dir.join("etc/passwd"), "root:x:0:0:root:/root:/bin/sh\n").unwrap();
        fs::write(dir.join("etc/group"), "root:x:0:\n").unwrap();
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn strict_deadline_refuses_expired_linux_mutation_before_touching_filesystem() {
    let tmp = TempRoot::new("deadline");
    let mut host = LinuxHost::open(&tmp.0).unwrap().with_strict_deadline(true);
    let now = host.host_now();

    let path = ResourcePath::new("/etc/should-not-exist").unwrap();
    let req = Apply {
        key: EffectKey::new(
            PlanId::new("p").unwrap(),
            Generation(1),
            0,
            ResourceKey::File(path),
        ),
        operation: Operation::Converge(Requirement::File(FileCondition::Present {
            content: Content::Any,
            metadata: Metadata::any(),
        })),
        // Expired before execution starts.
        settle_by: Instant(now.0.saturating_sub(1)),
    };

    let receipts = host.apply(&req);
    assert_eq!(receipts, vec![Receipt::Refused]);
    assert_eq!(host.executions(), 0);
    assert!(!tmp.0.join("etc/should-not-exist").exists());
}

#[test]
fn boot_clock_preserves_remaining_cooldown_across_quick_reboot_and_expires_after_long_reboot() {
    let boot_1 = BootEpochRecord {
        boot_id: "11111111-1111-1111-1111-111111111111".to_owned(),
        last_mono_ns: 1_000_000_000_000, // 1000s
        last_wall_ns: 1_700_000_000_000_000_000,
        offset_ns: 0,
    };
    let cooldown_deadline = 1_005_000_000_000u64; // +5s cooldown

    // Quick reboot: kernel monotonic resets to 1s; 2s wall elapsed.
    let quick = advance_for_state(
        "22222222-2222-2222-2222-222222222222",
        1_000_000_000,
        boot_1.last_wall_ns + 2_000_000_000,
        Some(&boot_1),
        boot_1.last_mono_ns,
    );
    let effective_quick = quick.last_mono_ns + quick.offset_ns;
    assert_eq!(effective_quick, 1_002_000_000_000);
    assert!(
        effective_quick < cooldown_deadline,
        "2s quick reboot must preserve remaining 3s cooldown instead of jumping past or resetting to zero"
    );

    // Slow reboot: 30s wall elapsed.
    let slow = advance_for_state(
        "22222222-2222-2222-2222-222222222222",
        1_000_000_000,
        boot_1.last_wall_ns + 30_000_000_000,
        Some(&boot_1),
        boot_1.last_mono_ns,
    );
    let effective_slow = slow.last_mono_ns + slow.offset_ns;
    assert_eq!(effective_slow, 1_030_000_000_000);
    assert!(
        effective_slow > cooldown_deadline,
        "30s reboot must clear expired 5s deadline"
    );

    // Backward NTP clock jump across reboot MUST still move forward by >= 1s.
    let ntp_skew = advance_for_state(
        "22222222-2222-2222-2222-222222222222",
        500_000_000,
        boot_1.last_wall_ns.saturating_sub(60_000_000_000),
        Some(&boot_1),
        boot_1.last_mono_ns,
    );
    assert!(ntp_skew.last_mono_ns + ntp_skew.offset_ns > boot_1.last_mono_ns);
}

#[test]
fn boot_clock_persists_state_dir_offset_and_trace_never_writes() {
    let tmp = TempRoot::new("boot-persist");
    let state_dir = tmp.0.join("state");
    fs::create_dir_all(&state_dir).unwrap();

    // Pre-seed stale high persisted journal instant far ahead of current raw monotonic.
    let fake_future_ns = u64::MAX / 4;
    let trace_offset = read_boot_clock_offset(&state_dir, fake_future_ns);
    assert!(trace_offset > 0);
    assert!(
        !state_dir.join("boot-epoch").exists(),
        "read-only trace offset helper must not mutate state_dir"
    );

    let enforce_offset = reconcile_boot_clock(&state_dir, fake_future_ns).unwrap();
    assert!(enforce_offset > 0);
    assert!(state_dir.join("boot-epoch").exists());
    let host = LinuxHost::open(&tmp.0)
        .unwrap()
        .with_clock_offset_ns(enforce_offset);
    assert!(host.host_now().0 >= fake_future_ns);
}
