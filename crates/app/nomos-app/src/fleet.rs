//! Application-layer fleet coordination, PSI governor orchestration, and telemetry redaction.
//!
//! Depends strictly on `nomos-core` and port crates (`nomos-cipher`,
//! `nomos-mesh`, `nomos-protocol`, `nomos-cloud`) — never on concrete adapters.

use nomos_cipher::SecretsPosture;
use nomos_cloud::AuthPosture;
use nomos_core::governor::{
    GovernorConfig, GovernorDirective, GovernorForecast, ResourceSample, ServiceGovernor,
};
use nomos_core::plan::{Generation, PlanId};
use nomos_core::resource::Digest;
use nomos_mesh::MeshPosture;
use nomos_protocol::{ControlEnvelope, ControlOp, QuicStats};

/// Combined non-secret fleet coordinator snapshot aggregated across ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FleetPostureSnapshot {
    /// Current authority generation watermark.
    pub generation: Generation,
    /// Metadata-only secrets posture (never contains secret values; N8).
    pub secrets: Option<SecretsPosture>,
    /// Mesh tunnel posture and peer telemetry.
    pub mesh: Option<MeshPosture>,
    /// QUIC control transport statistics.
    pub quic: QuicStats,
    /// Cloud credential readiness metadata (never contains bearer tokens; N8).
    pub cloud_auth: Option<AuthPosture>,
    /// Latest PSI/cgroup governor forecast.
    pub governor: Option<GovernorForecast>,
}

/// Application use-case orchestrator wrapping [`ServiceGovernor`] and port envelopes.
#[derive(Clone, Debug)]
pub struct FleetCoordinatorApp {
    governor: ServiceGovernor,
    last_forecast: Option<GovernorForecast>,
    last_directive: Option<GovernorDirective>,
    generation: Generation,
}

impl Default for FleetCoordinatorApp {
    fn default() -> Self {
        Self::new(GovernorConfig::default())
    }
}

impl FleetCoordinatorApp {
    /// Creates an application orchestrator with the supplied governor thresholds.
    pub fn new(config: GovernorConfig) -> Self {
        Self {
            governor: ServiceGovernor::new(config),
            last_forecast: None,
            last_directive: None,
            generation: Generation(1),
        }
    }

    /// Evaluates one PSI/cgroup [`ResourceSample`] and returns forecast + directive.
    pub fn observe_pressure(
        &mut self,
        sample: &ResourceSample,
    ) -> (GovernorForecast, GovernorDirective) {
        let (forecast, directive) = self.governor.observe(sample);
        self.last_forecast = Some(forecast);
        self.last_directive = Some(directive.clone());
        (forecast, directive)
    }

    /// Latest forecast produced by [`Self::observe_pressure`].
    pub const fn last_forecast(&self) -> Option<GovernorForecast> {
        self.last_forecast
    }

    /// Latest cgroup/admission directive produced by [`Self::observe_pressure`].
    pub fn last_directive(&self) -> Option<&GovernorDirective> {
        self.last_directive.as_ref()
    }

    /// Current coordinator generation fence.
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Advances coordinator generation watermark monotonically.
    pub fn bump_generation(&mut self) -> Generation {
        self.generation = Generation(self.generation.0.saturating_add(1));
        self.generation
    }

    /// Builds a typed control envelope referencing `plan_id`, current generation, and `canon_digest`.
    pub fn build_control_envelope(
        &self,
        op: ControlOp,
        plan_id: Option<&PlanId>,
        canon_digest: &Digest,
        payload_json: impl Into<String>,
    ) -> ControlEnvelope {
        ControlEnvelope::new(op, plan_id, self.generation, canon_digest, payload_json)
    }

    /// Assembles a redacted [`FleetPostureSnapshot`] across all driven ports.
    pub fn snapshot(
        &self,
        secrets: Option<SecretsPosture>,
        mesh: Option<MeshPosture>,
        quic: QuicStats,
        cloud_auth: Option<AuthPosture>,
    ) -> FleetPostureSnapshot {
        FleetPostureSnapshot {
            generation: self.generation,
            secrets,
            mesh,
            quic,
            cloud_auth,
            governor: self.last_forecast,
        }
    }
}
