//! Pure `#![no_std]` deterministic PSI and cgroup resource governor.
//!
//! Uses injected monotonic `timestamp_ns` only (invariant N12) with zero
//! external dependencies (`crates/core/PURITY.toml`).

pub mod types;

use alloc::vec::Vec;

pub use types::{
    BPS_SCALE, ClassBudget, GovernorConfig, GovernorDirective, GovernorForecast, GovernorReason,
    PressureTier, ResourceSample, ServiceClass,
};

/// Deterministic `#![no_std]` PSI/cgroup governor state machine.
#[derive(Clone, Debug)]
pub struct ServiceGovernor {
    config: GovernorConfig,
    initialized: bool,
    tier: PressureTier,
    last_escalation_ns: u64,
    last_sample_ns: u64,
    ewma_cpu_bps: u16,
    ewma_mem_bps: u16,
    ewma_io_bps: u16,
    prev_cpu_bps: u16,
    prev_mem_bps: u16,
    generation: u64,
}

impl ServiceGovernor {
    /// Creates a new governor initialized at [`PressureTier::Normal`].
    pub const fn new(config: GovernorConfig) -> Self {
        Self {
            config,
            initialized: false,
            tier: PressureTier::Normal,
            last_escalation_ns: 0,
            last_sample_ns: 0,
            ewma_cpu_bps: 0,
            ewma_mem_bps: 0,
            ewma_io_bps: 0,
            prev_cpu_bps: 0,
            prev_mem_bps: 0,
            generation: 0,
        }
    }

    /// Current active tier.
    pub const fn current_tier(&self) -> PressureTier {
        self.tier
    }

    /// Active configuration.
    pub const fn config(&self) -> GovernorConfig {
        self.config
    }

    /// Evaluates one telemetry sample deterministically and returns forecast + directive.
    pub fn observe(&mut self, sample: &ResourceSample) -> (GovernorForecast, GovernorDirective) {
        let cpu_raw = sample
            .cpu_some_avg10_bps
            .max(sample.cpu_full_avg10_bps)
            .min(BPS_SCALE as u16);
        let mem_raw = sample
            .mem_some_avg10_bps
            .max(sample.mem_full_avg10_bps)
            .min(BPS_SCALE as u16);
        let io_raw = sample
            .io_some_avg10_bps
            .max(sample.io_full_avg10_bps)
            .min(BPS_SCALE as u16);
        let mem_ratio_bps = memory_ratio_bps(sample.memory_used_bytes, sample.memory_total_bytes);

        let (predicted_cpu_bps, predicted_mem_bps) = if !self.initialized {
            self.initialized = true;
            self.ewma_cpu_bps = cpu_raw;
            self.ewma_mem_bps = mem_raw;
            self.ewma_io_bps = io_raw;
            self.prev_cpu_bps = cpu_raw;
            self.prev_mem_bps = mem_raw;
            self.last_sample_ns = sample.timestamp_ns;
            (cpu_raw, mem_raw)
        } else {
            let alpha = (self.config.ewma_alpha_bps as u32).clamp(1, BPS_SCALE);
            self.ewma_cpu_bps = ewma_step(self.ewma_cpu_bps, cpu_raw, alpha);
            self.ewma_mem_bps = ewma_step(self.ewma_mem_bps, mem_raw, alpha);
            self.ewma_io_bps = ewma_step(self.ewma_io_bps, io_raw, alpha);

            let dt_ns = sample.timestamp_ns.saturating_sub(self.last_sample_ns);
            let horizon_ns = (self.config.horizon_secs as u64).saturating_mul(1_000_000_000);
            let pred_cpu = project_linear(self.prev_cpu_bps, cpu_raw, dt_ns, horizon_ns);
            let pred_mem = project_linear(self.prev_mem_bps, mem_raw, dt_ns, horizon_ns);

            self.prev_cpu_bps = cpu_raw;
            self.prev_mem_bps = mem_raw;
            self.last_sample_ns = sample.timestamp_ns;
            (pred_cpu, pred_mem)
        };

        let (raw_tier, raw_reason) = self.classify(
            cpu_raw.max(self.ewma_cpu_bps),
            mem_raw.max(self.ewma_mem_bps),
            io_raw.max(self.ewma_io_bps),
            predicted_cpu_bps,
            predicted_mem_bps,
            mem_ratio_bps,
        );

        let (final_tier, final_reason) = if raw_tier >= self.tier {
            if raw_tier > self.tier {
                self.last_escalation_ns = sample.timestamp_ns;
                self.generation = self.generation.saturating_add(1);
            }
            (raw_tier, raw_reason)
        } else if sample.timestamp_ns.saturating_sub(self.last_escalation_ns)
            >= self.config.cooldown_ns
        {
            self.generation = self.generation.saturating_add(1);
            (raw_tier, raw_reason)
        } else {
            (self.tier, GovernorReason::CooldownHold)
        };

        self.tier = final_tier;

        let forecast = GovernorForecast {
            tier: final_tier,
            reason: final_reason,
            ewma_cpu_bps: self.ewma_cpu_bps,
            ewma_mem_bps: self.ewma_mem_bps,
            ewma_io_bps: self.ewma_io_bps,
            predicted_cpu_bps,
            predicted_mem_bps,
            memory_ratio_bps: mem_ratio_bps,
        };
        let directive = GovernorDirective {
            generation: self.generation,
            tier: final_tier,
            reason: final_reason,
            admit_batch: matches!(final_tier, PressureTier::Normal | PressureTier::Elevated),
            shed_background: matches!(final_tier, PressureTier::High | PressureTier::Critical),
            budgets: budgets_for_tier(final_tier),
        };
        (forecast, directive)
    }

    fn classify(
        &self,
        cpu: u16,
        mem: u16,
        io: u16,
        pred_cpu: u16,
        pred_mem: u16,
        mem_ratio: u16,
    ) -> (PressureTier, GovernorReason) {
        let mut tier = PressureTier::Normal;
        let mut reason = GovernorReason::Healthy;

        let checks = [
            (
                cpu,
                self.config.critical_psi_bps,
                self.config.high_psi_bps,
                self.config.elevated_psi_bps,
                GovernorReason::CpuPsiHigh,
            ),
            (
                mem,
                self.config.critical_psi_bps,
                self.config.high_psi_bps,
                self.config.elevated_psi_bps,
                GovernorReason::MemoryPsiHigh,
            ),
            (
                io,
                self.config.critical_psi_bps,
                self.config.high_psi_bps,
                self.config.elevated_psi_bps,
                GovernorReason::IoPsiHigh,
            ),
            (
                mem_ratio,
                self.config.critical_mem_bps,
                self.config.high_mem_bps,
                self.config.elevated_mem_bps,
                GovernorReason::MemoryHeadroomLow,
            ),
            (
                pred_cpu.max(pred_mem),
                self.config.critical_psi_bps,
                self.config.high_psi_bps,
                self.config.elevated_psi_bps,
                GovernorReason::PredictiveSlope,
            ),
        ];

        for (val, crit, high, elev, sig) in checks {
            let sig_tier = if val >= crit {
                PressureTier::Critical
            } else if val >= high {
                PressureTier::High
            } else if val >= elev {
                PressureTier::Elevated
            } else {
                PressureTier::Normal
            };
            if sig_tier > tier {
                tier = sig_tier;
                reason = sig;
            }
        }
        (tier, reason)
    }
}

fn memory_ratio_bps(used: u64, total: u64) -> u16 {
    if total == 0 {
        return 0;
    }
    let ratio = (used.saturating_mul(BPS_SCALE as u64)) / total;
    ratio.min(BPS_SCALE as u64) as u16
}

fn ewma_step(prev: u16, raw: u16, alpha_bps: u32) -> u16 {
    let inv = BPS_SCALE.saturating_sub(alpha_bps);
    let blended =
        (alpha_bps.saturating_mul(raw as u32) + inv.saturating_mul(prev as u32)) / BPS_SCALE;
    blended.min(BPS_SCALE) as u16
}

fn project_linear(prev: u16, curr: u16, dt_ns: u64, horizon_ns: u64) -> u16 {
    if dt_ns == 0 || horizon_ns == 0 || curr <= prev {
        return curr;
    }
    let delta = (curr - prev) as u128;
    let extra = (delta.saturating_mul(horizon_ns as u128)) / (dt_ns as u128);
    let projected = (curr as u128).saturating_add(extra);
    projected.min(BPS_SCALE as u128) as u16
}

/// Computes deterministic per-class cgroup budgets protecting `Control` under saturation.
pub fn budgets_for_tier(tier: PressureTier) -> Vec<ClassBudget> {
    let (ctrl_q, inter_w, inter_q, inter_c, batch_w, batch_q, batch_c, bg_w, bg_q, bg_c) =
        match tier {
            PressureTier::Normal => (100, 100, 100, 32, 80, 100, 8, 50, 80, 4),
            PressureTier::Elevated => (100, 100, 90, 24, 60, 75, 4, 20, 40, 2),
            PressureTier::High => (100, 90, 75, 16, 20, 25, 0, 5, 10, 0),
            PressureTier::Critical => (100, 70, 50, 8, 10, 10, 0, 1, 5, 0),
        };
    alloc::vec![
        ClassBudget {
            service_class: ServiceClass::Control,
            cpu_weight: 200,
            cpu_quota_pct: ctrl_q,
            memory_high_pct: 95,
            io_weight: 200,
            max_concurrency: 64,
        },
        ClassBudget {
            service_class: ServiceClass::Interactive,
            cpu_weight: inter_w,
            cpu_quota_pct: inter_q,
            memory_high_pct: 85,
            io_weight: inter_w,
            max_concurrency: inter_c,
        },
        ClassBudget {
            service_class: ServiceClass::Batch,
            cpu_weight: batch_w,
            cpu_quota_pct: batch_q,
            memory_high_pct: 70,
            io_weight: batch_w,
            max_concurrency: batch_c,
        },
        ClassBudget {
            service_class: ServiceClass::Background,
            cpu_weight: bg_w,
            cpu_quota_pct: bg_q,
            memory_high_pct: 50,
            io_weight: bg_w,
            max_concurrency: bg_c,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_sample(ts: u64, cpu: u16, mem_used: u64) -> ResourceSample {
        ResourceSample {
            timestamp_ns: ts,
            cpu_some_avg10_bps: cpu,
            cpu_full_avg10_bps: 0,
            mem_some_avg10_bps: 0,
            mem_full_avg10_bps: 0,
            io_some_avg10_bps: 0,
            io_full_avg10_bps: 0,
            memory_used_bytes: mem_used,
            memory_total_bytes: 1000,
        }
    }

    #[test]
    fn escalates_on_pressure_sheds_background_and_respects_cooldown() {
        let mut gov = ServiceGovernor::new(GovernorConfig::default());

        let (f1, d1) = gov.observe(&base_sample(1_000_000_000, 100, 300));
        assert_eq!(f1.tier, PressureTier::Normal);
        assert!(d1.admit_batch && !d1.shed_background);

        let (f2, d2) = gov.observe(&base_sample(2_000_000_000, 4_000, 300));
        assert!(f2.tier >= PressureTier::High);
        assert!(!d2.admit_batch);
        assert!(d2.shed_background);

        let ctrl = d2
            .budgets
            .iter()
            .find(|b| b.service_class == ServiceClass::Control)
            .unwrap();
        assert_eq!(ctrl.cpu_weight, 200);

        let (f3, _) = gov.observe(&base_sample(5_000_000_000, 50, 300));
        assert_eq!(f3.reason, GovernorReason::CooldownHold);
        assert!(f3.tier >= PressureTier::High);

        let (f4, d4) = gov.observe(&base_sample(25_000_000_000, 50, 300));
        assert_eq!(f4.tier, PressureTier::Normal);
        assert!(d4.admit_batch && !d4.shed_background);
    }
}
