//! Pure `#![no_std]` zero-alloc PSI/cgroup resource governor unifying `Nomos` + `green-infra`.
//!
//! Combines Holt-Winters double-exponential trend smoothing, 16-sample OLS memory
//! runway regression, Gradient2 / Little's Law dynamic concurrency caps over
//! `[ClassBudget; 4]`, and sustained-pressure cooldown refreshing.

mod math;
pub mod types;

use math::{
    HoltWinters, classify_pressure, estimate_runway_secs, ewma_step, gradient2_scale_bps,
    memory_ratio_bps,
};
pub use math::{budgets_for_tier, budgets_for_tier_scaled};
pub use types::{
    BPS_SCALE, ClassBudget, GOVERNOR_WINDOW, GovernorConfig, GovernorDirective, GovernorForecast,
    GovernorReason, PressureTier, ResourceSample, ServiceClass,
};

const GRADIENT_SHORT_ALPHA_BPS: u32 = 4_000;
const GRADIENT_LONG_ALPHA_BPS: u32 = 1_000;

/// Deterministic, zero-allocation PSI/cgroup resource governor.
#[derive(Clone, Debug)]
pub struct ServiceGovernor {
    config: GovernorConfig,
    ring: [ResourceSample; GOVERNOR_WINDOW],
    ring_len: u8,
    ring_head: u8,
    cpu_hw: HoltWinters,
    mem_hw: HoltWinters,
    ewma_io_bps: u16,
    short_pressure_bps: u16,
    long_pressure_bps: u16,
    last_timestamp_ns: Option<u64>,
    current_tier: PressureTier,
    last_escalation_ns: u64,
    generation: u64,
}

impl ServiceGovernor {
    /// Initializes a clean zero-alloc governor in [`PressureTier::Normal`].
    pub const fn new(config: GovernorConfig) -> Self {
        let empty = ResourceSample {
            timestamp_ns: 0,
            cpu_some_avg10_bps: 0,
            cpu_full_avg10_bps: 0,
            mem_some_avg10_bps: 0,
            mem_full_avg10_bps: 0,
            io_some_avg10_bps: 0,
            io_full_avg10_bps: 0,
            memory_used_bytes: 0,
            memory_total_bytes: 0,
        };
        Self {
            config,
            ring: [empty; GOVERNOR_WINDOW],
            ring_len: 0,
            ring_head: 0,
            cpu_hw: HoltWinters::new(),
            mem_hw: HoltWinters::new(),
            ewma_io_bps: 0,
            short_pressure_bps: 0,
            long_pressure_bps: 0,
            last_timestamp_ns: None,
            current_tier: PressureTier::Normal,
            last_escalation_ns: 0,
            generation: 0,
        }
    }

    /// Active pressure tier.
    pub const fn current_tier(&self) -> PressureTier {
        self.current_tier
    }

    /// Monotonic transition generation counter.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Processes one [`ResourceSample`] deterministically with zero heap allocations.
    pub fn observe(&mut self, sample: &ResourceSample) -> (GovernorForecast, GovernorDirective) {
        self.push_sample(*sample);

        let (raw_cpu, raw_mem, raw_io) = (
            sample.cpu_some_avg10_bps.max(sample.cpu_full_avg10_bps),
            sample.mem_some_avg10_bps.max(sample.mem_full_avg10_bps),
            sample.io_some_avg10_bps.max(sample.io_full_avg10_bps),
        );
        let mem_ratio = memory_ratio_bps(sample.memory_used_bytes, sample.memory_total_bytes);
        let alpha = u32::from(self.config.ewma_alpha_bps).clamp(1, BPS_SCALE);
        let beta = u32::from(self.config.trend_beta_bps).clamp(1, BPS_SCALE);
        let dt_ns = self
            .last_timestamp_ns
            .map_or(0, |prev| sample.timestamp_ns.saturating_sub(prev));
        let (ewma_cpu, _) = self.cpu_hw.update(raw_cpu, alpha, beta);
        let (ewma_mem, _) = self.mem_hw.update(raw_mem, alpha, beta);
        self.ewma_io_bps = match self.last_timestamp_ns {
            None => raw_io.min(BPS_SCALE as u16),
            Some(_) => ewma_step(self.ewma_io_bps, raw_io, alpha),
        };
        let pred_cpu = self.cpu_hw.forecast(dt_ns, self.config.forecast_horizon_ns);
        let pred_mem = self.mem_hw.forecast(dt_ns, self.config.forecast_horizon_ns);
        let mem_slope = self.mem_slope_bytes_per_sec();
        let mem_high_target = sample
            .memory_total_bytes
            .saturating_mul(u64::from(self.config.high_mem_bps))
            / u64::from(BPS_SCALE);
        let eta_mem_high_secs =
            estimate_runway_secs(sample.memory_used_bytes, mem_high_target, mem_slope);
        let eta_oom_secs = estimate_runway_secs(
            sample.memory_used_bytes,
            sample.memory_total_bytes,
            mem_slope,
        );

        let composite = raw_cpu
            .max(raw_mem)
            .max(raw_io)
            .max(mem_ratio)
            .min(BPS_SCALE as u16);
        if self.last_timestamp_ns.is_none() {
            self.short_pressure_bps = composite.max(100);
            self.long_pressure_bps = composite.max(100);
        } else {
            self.short_pressure_bps =
                ewma_step(self.short_pressure_bps, composite, GRADIENT_SHORT_ALPHA_BPS).max(100);
            self.long_pressure_bps =
                ewma_step(self.long_pressure_bps, composite, GRADIENT_LONG_ALPHA_BPS).max(100);
        }
        let concurrency_scale_bps =
            gradient2_scale_bps(self.short_pressure_bps, self.long_pressure_bps, composite);

        self.last_timestamp_ns = Some(sample.timestamp_ns);

        let (raw_tier, raw_reason) = classify_pressure(
            &self.config,
            ewma_cpu.max(raw_cpu),
            ewma_mem.max(raw_mem),
            self.ewma_io_bps.max(raw_io),
            mem_ratio,
            pred_cpu,
            pred_mem,
            eta_mem_high_secs,
            eta_oom_secs,
        );

        // Refresh last_escalation_ns whenever sustained pressure stays at or above current_tier.
        let (final_tier, final_reason) = if raw_tier > self.current_tier {
            self.current_tier = raw_tier;
            self.last_escalation_ns = sample.timestamp_ns;
            self.generation = self.generation.saturating_add(1);
            (raw_tier, raw_reason)
        } else if raw_tier == self.current_tier {
            if raw_tier > PressureTier::Normal {
                self.last_escalation_ns = sample.timestamp_ns;
            }
            (raw_tier, raw_reason)
        } else if sample.timestamp_ns.saturating_sub(self.last_escalation_ns)
            < self.config.cooldown_ns
        {
            (self.current_tier, GovernorReason::CooldownHold)
        } else {
            self.current_tier = raw_tier;
            self.generation = self.generation.saturating_add(1);
            (raw_tier, raw_reason)
        };

        let forecast = GovernorForecast {
            tier: final_tier,
            reason: final_reason,
            ewma_cpu_bps: ewma_cpu,
            ewma_mem_bps: ewma_mem,
            ewma_io_bps: self.ewma_io_bps,
            predicted_cpu_bps: pred_cpu,
            predicted_mem_bps: pred_mem,
            memory_ratio_bps: mem_ratio,
            mem_slope_bytes_per_sec: mem_slope,
            eta_mem_high_secs,
            eta_oom_secs,
            concurrency_scale_bps,
        };
        let directive = GovernorDirective {
            tier: final_tier,
            reason: final_reason,
            generation: self.generation,
            admit_batch: matches!(final_tier, PressureTier::Normal | PressureTier::Elevated),
            shed_background: matches!(final_tier, PressureTier::High | PressureTier::Critical),
            budgets: budgets_for_tier_scaled(final_tier, concurrency_scale_bps),
        };
        (forecast, directive)
    }

    fn push_sample(&mut self, sample: ResourceSample) {
        self.ring[self.ring_head as usize] = sample;
        self.ring_head = (self.ring_head + 1) % (GOVERNOR_WINDOW as u8);
        if (self.ring_len as usize) < GOVERNOR_WINDOW {
            self.ring_len += 1;
        }
    }

    fn mem_slope_bytes_per_sec(&self) -> i64 {
        let n = self.ring_len as usize;
        if n < 2 {
            return 0;
        }
        let start = if n < GOVERNOR_WINDOW {
            0
        } else {
            self.ring_head as usize
        };
        let t0 = self.ring[start].timestamp_ns;
        let (mut sx, mut sy, mut sxx, mut sxy) = (0i128, 0i128, 0i128, 0i128);
        for k in 0..n {
            let s = &self.ring[(start + k) % GOVERNOR_WINDOW];
            let x = i128::from(s.timestamp_ns.saturating_sub(t0) / 1_000_000);
            let y = i128::from(s.memory_used_bytes);
            sx += x;
            sy += y;
            sxx += x * x;
            sxy += x * y;
        }
        let n128 = n as i128;
        let den = n128 * sxx - sx * sx;
        if den <= 0 {
            return 0;
        }
        let num = 1_000i128 * (n128 * sxy - sx * sy);
        (num / den).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts_secs: u64, cpu: u16, mem_used: u64) -> ResourceSample {
        ResourceSample {
            timestamp_ns: ts_secs * 1_000_000_000,
            cpu_some_avg10_bps: cpu,
            cpu_full_avg10_bps: 0,
            mem_some_avg10_bps: 0,
            mem_full_avg10_bps: 0,
            io_some_avg10_bps: 0,
            io_full_avg10_bps: 0,
            memory_used_bytes: mem_used,
            memory_total_bytes: 1_000_000,
        }
    }

    #[test]
    fn sustained_pressure_holds_cooldown_and_ols_runway_escalates() {
        let mut gov = ServiceGovernor::new(GovernorConfig::default());
        let (f1, d1) = gov.observe(&sample(1, 100, 300_000));
        assert_eq!(f1.tier, PressureTier::Normal);
        assert!(d1.admit_batch && !d1.shed_background);

        let (f2, _) = gov.observe(&sample(2, 7_000, 300_000));
        assert_eq!(f2.tier, PressureTier::Critical);
        let (f3, _) = gov.observe(&sample(20, 7_000, 300_000));
        assert_eq!(f3.tier, PressureTier::Critical);

        let (f4, _) = gov.observe(&sample(25, 50, 300_000));
        assert_eq!(f4.reason, GovernorReason::CooldownHold);
        assert_eq!(f4.tier, PressureTier::Critical);

        // Calm samples decay Holt-Winters level smoothly; once calm for > cooldown_ns, returns Normal.
        for t in [38u64, 40, 42, 44, 60] {
            let _ = gov.observe(&sample(t, 50, 300_000));
        }
        let (f5, d5) = gov.observe(&sample(62, 50, 300_000));
        assert_eq!(f5.tier, PressureTier::Normal);
        assert!(d5.admit_batch);

        let mut ramp_gov = ServiceGovernor::new(GovernorConfig::default());
        for t in 1..=6u64 {
            let _ = ramp_gov.observe(&sample(t, 100, 500_000 + t * 40_000));
        }
        let (fr, _) = ramp_gov.observe(&sample(7, 100, 780_000));
        assert!(fr.mem_slope_bytes_per_sec > 30_000);
        assert!(fr.eta_oom_secs.is_some_and(|eta| eta <= 15));
        assert_eq!(fr.tier, PressureTier::Critical);
    }
}
