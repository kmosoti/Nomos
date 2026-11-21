//! Pure `#![no_std]` fixed-point math for Holt-Winters smoothing, 16-sample OLS
//! memory runway, and Gradient2 / Little's Law dynamic concurrency budgets.

use super::types::{
    BPS_SCALE, ClassBudget, GovernorConfig, GovernorReason, PressureTier, ServiceClass,
};

const GRADIENT_FLOOR_BPS: u16 = 5_000;

/// Fixed-point Holt-Winters double-exponential state (basis points).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HoltWinters {
    pub(crate) level_bps: i32,
    pub(crate) trend_bps: i32,
    pub(crate) initialized: bool,
}

impl HoltWinters {
    pub(crate) const fn new() -> Self {
        Self {
            level_bps: 0,
            trend_bps: 0,
            initialized: false,
        }
    }

    pub(crate) fn update(&mut self, raw_bps: u16, alpha_bps: u32, beta_bps: u32) -> (u16, i32) {
        let x = i32::from(raw_bps.min(BPS_SCALE as u16));
        if !self.initialized {
            self.initialized = true;
            self.level_bps = x;
            self.trend_bps = 0;
            return (x as u16, 0);
        }
        let a = alpha_bps as i64;
        let b = beta_bps as i64;
        let scale = BPS_SCALE as i64;
        let prev_l = i64::from(self.level_bps);
        let prev_t = i64::from(self.trend_bps);

        let next_l = (a * i64::from(x) + (scale - a) * (prev_l + prev_t)) / scale;
        let next_t = (b * (next_l - prev_l) + (scale - b) * prev_t) / scale;
        self.level_bps = next_l.clamp(0, scale) as i32;
        self.trend_bps = next_t.clamp(-scale, scale) as i32;
        (self.level_bps as u16, self.trend_bps)
    }

    pub(crate) fn forecast(&self, dt_ns: u64, horizon_ns: u64) -> u16 {
        if dt_ns == 0 || horizon_ns == 0 || self.trend_bps <= 0 {
            return self.level_bps.clamp(0, BPS_SCALE as i32) as u16;
        }
        let steps_num = i128::from(self.trend_bps).saturating_mul(i128::from(horizon_ns));
        let delta = steps_num / i128::from(dt_ns);
        let projected = i128::from(self.level_bps).saturating_add(delta);
        projected.clamp(0, i128::from(BPS_SCALE)) as u16
    }
}

pub(crate) fn memory_ratio_bps(used: u64, total: u64) -> u16 {
    if total == 0 {
        return 0;
    }
    let ratio = (u128::from(used).saturating_mul(u128::from(BPS_SCALE))) / u128::from(total);
    ratio.min(u128::from(BPS_SCALE)) as u16
}

pub(crate) fn ewma_step(prev: u16, raw: u16, alpha_bps: u32) -> u16 {
    let inv = BPS_SCALE.saturating_sub(alpha_bps);
    let blended = (alpha_bps.saturating_mul(u32::from(raw.min(BPS_SCALE as u16)))
        + inv.saturating_mul(u32::from(prev)))
        / BPS_SCALE;
    blended.min(BPS_SCALE) as u16
}

pub(crate) fn estimate_runway_secs(used: u64, target: u64, slope_bps: i64) -> Option<u32> {
    if target == 0 || slope_bps <= 0 {
        return None;
    }
    if used >= target {
        return Some(0);
    }
    let gap = target - used;
    let slope = slope_bps as u64;
    let secs = gap.saturating_add(slope - 1) / slope;
    Some(secs.min(u64::from(u32::MAX)) as u32)
}

pub(crate) fn gradient2_scale_bps(short_bps: u16, long_bps: u16, composite_bps: u16) -> u16 {
    let ratio_bps = (u32::from(long_bps).saturating_mul(BPS_SCALE)) / u32::from(short_bps.max(1));
    let headroom_bps = BPS_SCALE.saturating_sub(u32::from(composite_bps) / 2);
    ratio_bps
        .min(headroom_bps)
        .clamp(u32::from(GRADIENT_FLOOR_BPS), BPS_SCALE) as u16
}

fn scale_cap(base: u16, scale_bps: u16, min_cap: u16) -> u16 {
    if base == 0 {
        return 0;
    }
    let scaled = (u32::from(base).saturating_mul(u32::from(scale_bps))) / BPS_SCALE;
    (scaled as u16).max(min_cap).min(base)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn classify_pressure(
    cfg: &GovernorConfig,
    cpu: u16,
    mem: u16,
    io: u16,
    mem_ratio: u16,
    pred_cpu: u16,
    pred_mem: u16,
    eta_high: Option<u32>,
    eta_oom: Option<u32>,
) -> (PressureTier, GovernorReason) {
    let mut tier = PressureTier::Normal;
    let mut reason = GovernorReason::Healthy;
    let checks = [
        (
            cpu,
            cfg.critical_psi_bps,
            cfg.high_psi_bps,
            cfg.elevated_psi_bps,
            GovernorReason::CpuPsiHigh,
        ),
        (
            mem,
            cfg.critical_psi_bps,
            cfg.high_psi_bps,
            cfg.elevated_psi_bps,
            GovernorReason::MemoryPsiHigh,
        ),
        (
            io,
            cfg.critical_psi_bps,
            cfg.high_psi_bps,
            cfg.elevated_psi_bps,
            GovernorReason::IoPsiHigh,
        ),
        (
            mem_ratio,
            cfg.critical_mem_bps,
            cfg.high_mem_bps,
            cfg.elevated_mem_bps,
            GovernorReason::MemoryHeadroomLow,
        ),
        (
            pred_cpu.max(pred_mem),
            cfg.critical_psi_bps,
            cfg.high_psi_bps,
            cfg.elevated_psi_bps,
            GovernorReason::PredictiveSlope,
        ),
    ];
    for (val, crit, high, elev, sig) in checks {
        let sig_tier = match val {
            v if v >= crit => PressureTier::Critical,
            v if v >= high => PressureTier::High,
            v if v >= elev => PressureTier::Elevated,
            _ => PressureTier::Normal,
        };
        if sig_tier > tier {
            tier = sig_tier;
            reason = sig;
        }
    }
    if eta_oom.is_some_and(|s| s <= cfg.runway_crit_secs) && PressureTier::Critical > tier {
        tier = PressureTier::Critical;
        reason = GovernorReason::PredictiveSlope;
    } else if eta_high.is_some_and(|s| s <= cfg.runway_warn_secs) && PressureTier::Elevated > tier {
        tier = PressureTier::Elevated;
        reason = GovernorReason::PredictiveSlope;
    }
    (tier, reason)
}

/// Computes zero-alloc `[ClassBudget; 4]` with Gradient2 concurrency scaling protecting `Control`.
pub fn budgets_for_tier(tier: PressureTier) -> [ClassBudget; 4] {
    budgets_for_tier_scaled(tier, BPS_SCALE as u16)
}

/// Computes zero-alloc `[ClassBudget; 4]` dynamically scaled by Gradient2 `concurrency_scale_bps`.
pub fn budgets_for_tier_scaled(tier: PressureTier, concurrency_scale_bps: u16) -> [ClassBudget; 4] {
    let (inter_w, inter_q, inter_c, batch_w, batch_q, batch_c, bg_w, bg_q, bg_c) = match tier {
        PressureTier::Normal => (100, 100, 32, 80, 100, 8, 50, 80, 4),
        PressureTier::Elevated => (100, 90, 24, 60, 75, 4, 20, 40, 2),
        PressureTier::High => (90, 75, 16, 20, 25, 0, 5, 10, 0),
        PressureTier::Critical => (70, 50, 8, 10, 10, 0, 1, 5, 0),
    };
    [
        ClassBudget {
            service_class: ServiceClass::Control,
            cpu_weight: 200,
            cpu_quota_pct: 100,
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
            max_concurrency: scale_cap(inter_c, concurrency_scale_bps, 2),
        },
        ClassBudget {
            service_class: ServiceClass::Batch,
            cpu_weight: batch_w,
            cpu_quota_pct: batch_q,
            memory_high_pct: 70,
            io_weight: batch_w,
            max_concurrency: scale_cap(batch_c, concurrency_scale_bps, 1),
        },
        ClassBudget {
            service_class: ServiceClass::Background,
            cpu_weight: bg_w,
            cpu_quota_pct: bg_q,
            memory_high_pct: 50,
            io_weight: bg_w,
            max_concurrency: scale_cap(bg_c, concurrency_scale_bps, 1),
        },
    ]
}
