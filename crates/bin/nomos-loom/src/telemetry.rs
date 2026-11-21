//! Serializable JSON telemetry, governor, and SSE event DTOs for `nomos-loom`.

use nomos_cipher::SecretsPosture;
use nomos_cloud::AuthPosture;
use nomos_core::governor::{
    ClassBudget, GovernorDirective, GovernorForecast, GovernorReason, PressureTier, ServiceClass,
};
use nomos_mesh::MeshPosture;
use nomos_protocol::QuicStats;
use serde::{Deserialize, Serialize};

/// JSON representation of a systemd/cgroup class budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassBudgetDto {
    /// Service class name (`control`, `interactive`, `batch`, `background`).
    pub service_class: String,
    /// cgroup v2 CPU weight.
    pub cpu_weight: u16,
    /// cgroup v2 CPU quota percentage.
    pub cpu_quota_pct: u16,
    /// Memory high percentage.
    pub memory_high_pct: u16,
    /// IO weight.
    pub io_weight: u16,
    /// Maximum concurrent tasks or leases permitted.
    pub max_concurrency: u16,
}

impl From<&ClassBudget> for ClassBudgetDto {
    fn from(b: &ClassBudget) -> Self {
        Self {
            service_class: match b.service_class {
                ServiceClass::Control => "control",
                ServiceClass::Interactive => "interactive",
                ServiceClass::Batch => "batch",
                ServiceClass::Background => "background",
            }
            .to_owned(),
            cpu_weight: b.cpu_weight,
            cpu_quota_pct: b.cpu_quota_pct,
            memory_high_pct: b.memory_high_pct,
            io_weight: b.io_weight,
            max_concurrency: b.max_concurrency,
        }
    }
}

/// JSON summary returned by `/v1/governor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernorStatusDto {
    /// Active pressure tier (`normal`, `elevated`, `high`, `critical`).
    pub tier: String,
    /// Dominant reason code.
    pub reason: String,
    /// Governor transition generation counter.
    pub generation: u64,
    /// True if batch admission is enabled.
    pub admit_batch: bool,
    /// True if background tasks should be shed.
    pub shed_background: bool,
    /// Smoothed CPU PSI basis points.
    pub ewma_cpu_bps: u16,
    /// Smoothed memory PSI basis points.
    pub ewma_mem_bps: u16,
    /// Smoothed I/O PSI basis points.
    pub ewma_io_bps: u16,
    /// Predicted horizon CPU PSI basis points.
    pub predicted_cpu_bps: u16,
    /// Predicted horizon memory PSI basis points.
    pub predicted_mem_bps: u16,
    /// Memory utilization ratio basis points.
    pub memory_ratio_bps: u16,
    /// Computed class budgets.
    pub budgets: Vec<ClassBudgetDto>,
}

impl GovernorStatusDto {
    /// Converts pure domain forecast + directive into JSON DTO.
    pub fn from_core(forecast: &GovernorForecast, directive: &GovernorDirective) -> Self {
        Self {
            tier: tier_str(forecast.tier).to_owned(),
            reason: reason_str(forecast.reason).to_owned(),
            generation: directive.generation,
            admit_batch: directive.admit_batch,
            shed_background: directive.shed_background,
            ewma_cpu_bps: forecast.ewma_cpu_bps,
            ewma_mem_bps: forecast.ewma_mem_bps,
            ewma_io_bps: forecast.ewma_io_bps,
            predicted_cpu_bps: forecast.predicted_cpu_bps,
            predicted_mem_bps: forecast.predicted_mem_bps,
            memory_ratio_bps: forecast.memory_ratio_bps,
            budgets: directive.budgets.iter().map(ClassBudgetDto::from).collect(),
        }
    }
}

/// Request payload for `POST /v1/governor/sample`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SampleInputDto {
    /// Monotonic sample timestamp (nanoseconds).
    pub timestamp_ns: u64,
    /// CPU PSI `some avg10` (bps).
    pub cpu_some_avg10_bps: u16,
    /// CPU PSI `full avg10` (bps).
    #[serde(default)]
    pub cpu_full_avg10_bps: u16,
    /// Memory PSI `some avg10` (bps).
    #[serde(default)]
    pub mem_some_avg10_bps: u16,
    /// Memory PSI `full avg10` (bps).
    #[serde(default)]
    pub mem_full_avg10_bps: u16,
    /// I/O PSI `some avg10` (bps).
    #[serde(default)]
    pub io_some_avg10_bps: u16,
    /// I/O PSI `full avg10` (bps).
    #[serde(default)]
    pub io_full_avg10_bps: u16,
    /// Current memory bytes used.
    pub memory_used_bytes: u64,
    /// Total memory budget bytes.
    pub memory_total_bytes: u64,
}

/// Combined metadata-only telemetry returned by `GET /v1/telemetry`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelemetryResponse {
    /// Coordinator logical node name.
    pub node: String,
    /// Authority generation watermark.
    pub generation: u64,
    /// Count of active non-revoked credential/workload leases.
    pub active_leases: usize,
    /// Optional metadata-only secrets posture (zero plaintext; N8).
    pub secrets: Option<SecretsPosture>,
    /// Optional mesh tunnel posture.
    pub mesh: Option<MeshPosture>,
    /// QUIC control transport counters.
    pub quic: QuicStats,
    /// Optional metadata-only cloud auth posture.
    pub cloud_auth: Option<AuthPosture>,
    /// Current governor posture and class budgets.
    pub governor: GovernorStatusDto,
    /// Count of credential files staged under `$CREDENTIALS_DIRECTORY` (names only).
    pub staged_credentials: Vec<String>,
}

/// Real-time SSE event pushed to `/v1/events` subscribers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoomEvent {
    /// Event category (`governor_update`, `lease_issued`, `lease_revoked`, `quic_datagram`).
    pub kind: String,
    /// Summary detail without secret values.
    pub detail: String,
    /// Event timestamp (Unix seconds).
    pub unix_secs: u64,
}

fn tier_str(tier: PressureTier) -> &'static str {
    match tier {
        PressureTier::Normal => "normal",
        PressureTier::Elevated => "elevated",
        PressureTier::High => "high",
        PressureTier::Critical => "critical",
    }
}

fn reason_str(reason: GovernorReason) -> &'static str {
    match reason {
        GovernorReason::Healthy => "healthy",
        GovernorReason::CpuPsiHigh => "cpu_psi_high",
        GovernorReason::MemoryPsiHigh => "memory_psi_high",
        GovernorReason::IoPsiHigh => "io_psi_high",
        GovernorReason::MemoryHeadroomLow => "memory_headroom_low",
        GovernorReason::PredictiveSlope => "predictive_slope",
        GovernorReason::CooldownHold => "cooldown_hold",
    }
}
