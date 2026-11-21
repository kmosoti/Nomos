//! Domain types and zero-alloc fixed-array budgets for the `#![no_std]` PSI/cgroup governor.

/// Basis-point scale (`10_000 == 100.00%`).
pub const BPS_SCALE: u32 = 10_000;

/// Fixed 16-sample OLS regression window matching `green-infra` governor math.
pub const GOVERNOR_WINDOW: usize = 16;

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
    /// Holt-Winters trend or 16-sample OLS runway predicted threshold crossing.
    PredictiveSlope,
    /// Hysteresis cooldown held the elevated tier against premature drop.
    CooldownHold,
}

/// Snapshot of kernel PSI (`some`/`full` `avg10` in basis points) and memory usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// Per-class systemd/cgroup v2 resource budget computed from [`PressureTier`] and Gradient2.
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
    /// Dynamic concurrent tasks or jobs permitted for this class (`0` = shed/paused).
    pub max_concurrency: u16,
}

/// Tunable integer thresholds and Holt-Winters / OLS / Gradient2 parameters.
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
    /// Holt-Winters level smoothing factor alpha in basis points (`1..=10_000`).
    pub ewma_alpha_bps: u16,
    /// Holt-Winters trend smoothing factor beta in basis points (`1..=10_000`).
    pub trend_beta_bps: u16,
    /// Minimum dwell time (ns) before stepping down a pressure tier.
    pub cooldown_ns: u64,
    /// Horizon (ns) for Holt-Winters double-exponential slope projection.
    pub forecast_horizon_ns: u64,
    /// Memory OLS runway warning horizon (seconds) triggering `Elevated`.
    pub runway_warn_secs: u32,
    /// Memory OLS runway critical horizon (seconds) triggering `Critical`.
    pub runway_crit_secs: u32,
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
            ewma_alpha_bps: 3_500,
            trend_beta_bps: 2_500,
            cooldown_ns: 15_000_000_000,
            forecast_horizon_ns: 10_000_000_000,
            runway_warn_secs: 120,
            runway_crit_secs: 30,
        }
    }
}

/// Pure deterministic telemetry forecast emitted by [`crate::governor::ServiceGovernor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovernorForecast {
    /// Active pressure tier after hysteresis.
    pub tier: PressureTier,
    /// Dominant reason code explaining the active tier.
    pub reason: GovernorReason,
    /// Smoothed Holt-Winters CPU PSI level (bps).
    pub ewma_cpu_bps: u16,
    /// Smoothed Holt-Winters memory PSI level (bps).
    pub ewma_mem_bps: u16,
    /// Smoothed I/O PSI level (bps).
    pub ewma_io_bps: u16,
    /// Holt-Winters projected CPU PSI at `forecast_horizon_ns` (bps).
    pub predicted_cpu_bps: u16,
    /// Holt-Winters projected memory PSI at `forecast_horizon_ns` (bps).
    pub predicted_mem_bps: u16,
    /// Active memory utilization ratio (bps).
    pub memory_ratio_bps: u16,
    /// 16-sample OLS memory growth slope (bytes/sec).
    pub mem_slope_bytes_per_sec: i64,
    /// Estimated seconds until memory high watermark (`None` if non-increasing).
    pub eta_mem_high_secs: Option<u32>,
    /// Estimated seconds until OOM / total limit (`None` if non-increasing).
    pub eta_oom_secs: Option<u32>,
    /// Gradient2 / Little's Law dynamic concurrency multiplier (`5_000..=10_000` bps).
    pub concurrency_scale_bps: u16,
}

/// Actionable zero-alloc admission and cgroup directive derived from [`GovernorForecast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovernorDirective {
    /// Active pressure tier.
    pub tier: PressureTier,
    /// Dominant reason code.
    pub reason: GovernorReason,
    /// Monotonic transition sequence counter.
    pub generation: u64,
    /// True if new batch leases/jobs may be admitted.
    pub admit_batch: bool,
    /// True if background tasks should be shed or suspended.
    pub shed_background: bool,
    /// Zero-alloc fixed per-class cgroup v2 and Gradient2 concurrency budgets.
    pub budgets: [ClassBudget; 4],
}
