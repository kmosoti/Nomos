//! Domain types and budgets for the pure `#![no_std]` PSI/cgroup resource governor.

use alloc::vec::Vec;

/// Basis-point scale (`10_000 == 100.00%`).
pub const BPS_SCALE: u32 = 10_000;

/// Workload scheduling tier protected or throttled by the governor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceClass {
    /// Control plane, mesh keepalives, and health endpoints (never shed).
    Control,
    /// Latency-sensitive interactive requests and user-facing handlers.
    Interactive,
    /// Throughput-oriented batch compilations or Spot jobs.
    Batch,
    /// Best-effort background maintenance and compaction.
    Background,
}

impl ServiceClass {
    /// All four service classes in descending priority order.
    pub const ALL: [Self; 4] = [
        Self::Control,
        Self::Interactive,
        Self::Batch,
        Self::Background,
    ];
}

/// Discrete pressure classification driving admission control and cgroup weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PressureTier {
    /// Headroom abundant; all service classes admitted normally.
    Normal,
    /// Moderate contention; background work throttled.
    Elevated,
    /// High contention; new batch admission paused and background shed.
    High,
    /// Severe saturation; aggressive protection reserved for `Control`.
    Critical,
}

/// Primary signal or state-machine cause behind a [`PressureTier`] transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GovernorReason {
    /// Pressure signals and memory ratio sit below elevated thresholds.
    Healthy,
    /// CPU PSI exceeded policy threshold.
    CpuPsiHigh,
    /// Memory PSI exceeded policy threshold.
    MemoryPsiHigh,
    /// Block I/O PSI exceeded policy threshold.
    IoPsiHigh,
    /// Active memory usage fraction crossed headroom limit.
    MemoryHeadroomLow,
    /// Linear slope forecast predicted threshold crossing within horizon.
    PredictiveSlope,
    /// Hysteresis cooldown held the elevated tier against premature drop.
    CooldownHold,
}

/// Snapshot of kernel PSI (`some`/`full` `avg10` in basis points) and memory usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceSample {
    /// Monotonic timestamp in nanoseconds.
    pub timestamp_ns: u64,
    /// CPU PSI `some avg10` in basis points (`0..=10_000`).
    pub cpu_some_avg10_bps: u16,
    /// CPU PSI `full avg10` in basis points (`0..=10_000`).
    pub cpu_full_avg10_bps: u16,
    /// Memory PSI `some avg10` in basis points (`0..=10_000`).
    pub mem_some_avg10_bps: u16,
    /// Memory PSI `full avg10` in basis points (`0..=10_000`).
    pub mem_full_avg10_bps: u16,
    /// I/O PSI `some avg10` in basis points (`0..=10_000`).
    pub io_some_avg10_bps: u16,
    /// I/O PSI `full avg10` in basis points (`0..=10_000`).
    pub io_full_avg10_bps: u16,
    /// Current cgroup/host memory consumption in bytes.
    pub memory_used_bytes: u64,
    /// Total memory budget in bytes (`> 0` for ratio calculation).
    pub memory_total_bytes: u64,
}

/// Per-class systemd/cgroup v2 resource budget computed from [`PressureTier`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassBudget {
    /// Target workload class.
    pub service_class: ServiceClass,
    /// cgroup v2 `CPUWeight` (`1..=10000`).
    pub cpu_weight: u16,
    /// Relative `CPUQuota` percentage (`1..=100`).
    pub cpu_quota_pct: u16,
    /// `MemoryHigh` target as percentage of node memory budget.
    pub memory_high_pct: u16,
    /// cgroup v2 `IOWeight` (`1..=10000`).
    pub io_weight: u16,
    /// Maximum concurrent tasks or jobs permitted for this class (`0` = shed/paused).
    pub max_concurrency: u16,
}

/// Tunable integer thresholds and hysteresis configuration for [`crate::governor::ServiceGovernor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovernorConfig {
    /// PSI threshold for [`PressureTier::Elevated`] (bps).
    pub elevated_psi_bps: u16,
    /// PSI threshold for [`PressureTier::High`] (bps).
    pub high_psi_bps: u16,
    /// PSI threshold for [`PressureTier::Critical`] (bps).
    pub critical_psi_bps: u16,
    /// Memory usage ratio triggering [`PressureTier::Elevated`] (bps).
    pub elevated_mem_bps: u16,
    /// Memory usage ratio triggering [`PressureTier::High`] (bps).
    pub high_mem_bps: u16,
    /// Memory usage ratio triggering [`PressureTier::Critical`] (bps).
    pub critical_mem_bps: u16,
    /// EWMA smoothing factor in basis points (`1..=10_000`).
    pub ewma_alpha_bps: u16,
    /// Minimum dwell time (ns) before stepping down a pressure tier.
    pub cooldown_ns: u64,
    /// Prediction horizon in seconds for slope projection.
    pub horizon_secs: u16,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            elevated_psi_bps: 1_500,
            high_psi_bps: 3_500,
            critical_psi_bps: 6_500,
            elevated_mem_bps: 7_500,
            high_mem_bps: 8_500,
            critical_mem_bps: 9_300,
            ewma_alpha_bps: 4_000,
            cooldown_ns: 15_000_000_000,
            horizon_secs: 10,
        }
    }
}

/// Smoothed pressure observations and short-horizon predictions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovernorForecast {
    /// Current active pressure tier after hysteresis.
    pub tier: PressureTier,
    /// Dominant reason for the active tier.
    pub reason: GovernorReason,
    /// Smoothed CPU pressure (bps).
    pub ewma_cpu_bps: u16,
    /// Smoothed memory pressure (bps).
    pub ewma_mem_bps: u16,
    /// Smoothed I/O pressure (bps).
    pub ewma_io_bps: u16,
    /// Horizon-projected CPU pressure (bps).
    pub predicted_cpu_bps: u16,
    /// Horizon-projected memory pressure (bps).
    pub predicted_mem_bps: u16,
    /// Instantaneous memory utilization ratio (bps).
    pub memory_ratio_bps: u16,
}

/// Control directive emitted per sample for cgroup and lease admission enforcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GovernorDirective {
    /// Monotonic transition counter.
    pub generation: u64,
    /// Active pressure tier.
    pub tier: PressureTier,
    /// Reason for tier.
    pub reason: GovernorReason,
    /// True if new `Batch` jobs/leases may be admitted.
    pub admit_batch: bool,
    /// True if `Background` tasks should be paused or shed.
    pub shed_background: bool,
    /// Computed per-class cgroup budgets.
    pub budgets: Vec<ClassBudget>,
}
