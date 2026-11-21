//! Stateful governor orchestration, `/proc/pressure` reader, and SSE event broadcasting.

use std::sync::{Arc, Mutex};

use nomos_app::fleet::FleetCoordinatorApp;
use nomos_core::governor::{GovernorConfig, ResourceSample};
use tokio::sync::broadcast;

use crate::lease::unix_now_secs;
use crate::telemetry::{GovernorStatusDto, LoomEvent, SampleInputDto};

/// Thread-safe governor coordinator tracking pressure forecasts and emitting SSE events.
#[derive(Clone, Debug)]
pub struct GovernorService {
    app: Arc<Mutex<FleetCoordinatorApp>>,
    latest: Arc<Mutex<GovernorStatusDto>>,
    events: broadcast::Sender<LoomEvent>,
}

impl GovernorService {
    /// Initializes the governor service with an initial normal baseline directive.
    pub fn new(config: GovernorConfig) -> Self {
        let mut app = FleetCoordinatorApp::new(config);
        let initial = ResourceSample {
            timestamp_ns: 0,
            cpu_some_avg10_bps: 0,
            cpu_full_avg10_bps: 0,
            mem_some_avg10_bps: 0,
            mem_full_avg10_bps: 0,
            io_some_avg10_bps: 0,
            io_full_avg10_bps: 0,
            memory_used_bytes: 0,
            memory_total_bytes: 1,
        };
        let (forecast, directive) = app.observe_pressure(&initial);
        let dto = GovernorStatusDto::from_core(&forecast, &directive);
        let (events, _) = broadcast::channel(64);
        Self {
            app: Arc::new(Mutex::new(app)),
            latest: Arc::new(Mutex::new(dto)),
            events,
        }
    }

    /// Evaluates an incoming sample, updates current directive, and notifies SSE subscribers.
    pub fn observe(&self, sample: &ResourceSample) -> Result<GovernorStatusDto, String> {
        let dto = {
            let mut app = self.app.lock().map_err(|_| "governor mutex poisoned")?;
            let (forecast, directive) = app.observe_pressure(sample);
            GovernorStatusDto::from_core(&forecast, &directive)
        };
        if let Ok(mut guard) = self.latest.lock() {
            *guard = dto.clone();
        }
        let _ = self.events.send(LoomEvent {
            kind: "governor_update".to_owned(),
            detail: format!(
                "tier={} reason={} admit_batch={} shed_bg={}",
                dto.tier, dto.reason, dto.admit_batch, dto.shed_background
            ),
            unix_secs: unix_now_secs(),
        });
        Ok(dto)
    }

    /// Evaluates a JSON sample payload submitted over HTTP.
    pub fn observe_dto(&self, input: &SampleInputDto) -> Result<GovernorStatusDto, String> {
        let sample = ResourceSample {
            timestamp_ns: input.timestamp_ns,
            cpu_some_avg10_bps: input.cpu_some_avg10_bps,
            cpu_full_avg10_bps: input.cpu_full_avg10_bps,
            mem_some_avg10_bps: input.mem_some_avg10_bps,
            mem_full_avg10_bps: input.mem_full_avg10_bps,
            io_some_avg10_bps: input.io_some_avg10_bps,
            io_full_avg10_bps: input.io_full_avg10_bps,
            memory_used_bytes: input.memory_used_bytes,
            memory_total_bytes: input.memory_total_bytes,
        };
        self.observe(&sample)
    }

    /// Snapshot of latest computed governor posture.
    pub fn status(&self) -> GovernorStatusDto {
        self.latest
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Current authority generation watermark.
    pub fn generation(&self) -> u64 {
        self.app.lock().map(|g| g.generation().0).unwrap_or(1)
    }

    /// Publishes an arbitrary non-secret event to SSE clients.
    pub fn emit_event(&self, kind: impl Into<String>, detail: impl Into<String>) {
        let _ = self.events.send(LoomEvent {
            kind: kind.into(),
            detail: detail.into(),
            unix_secs: unix_now_secs(),
        });
    }

    /// Subscribes to the live SSE broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<LoomEvent> {
        self.events.subscribe()
    }
}

/// Parses Linux `/proc/pressure/{cpu,memory,io}` `some avg10=X.YZ` into basis points (`0..=10_000`).
pub fn parse_psi_avg10_bps(text: &str, prefix: &str) -> Option<u16> {
    let line = text.lines().find(|l| l.starts_with(prefix))?;
    let token = line.split_whitespace().find(|t| t.starts_with("avg10="))?;
    let num_str = token.strip_prefix("avg10=")?;
    let (whole_str, frac_str) = num_str.split_once('.').unwrap_or((num_str, "00"));
    let whole: u32 = whole_str.parse().ok()?;
    let mut padded = frac_str.as_bytes().to_vec();
    while padded.len() < 2 {
        padded.push(b'0');
    }
    let tenths = padded[0].checked_sub(b'0')? as u32;
    let hundredths = padded[1].checked_sub(b'0')? as u32;
    if tenths > 9 || hundredths > 9 {
        return None;
    }
    Some((whole * 100 + tenths * 10 + hundredths).min(10_000) as u16)
}
