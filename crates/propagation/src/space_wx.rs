//! Space-weather gating: turn SFI / Kp into per-band multipliers that *gate and
//! contextualize* the observed-reception score (they never override real spots —
//! a decoded spot proves a path). Tables from the research's SFI-eligibility and
//! Kp-penalty findings.

use std::collections::VecDeque;

use serde::Serialize;

use crate::model::Band;

/// SFI band-eligibility multiplier (~0..1). Low SFI closes the high HF bands;
/// high SFI opens 10 m (and 6 m F2). VHF is observation-driven (Es/aurora/MS are
/// SFI-independent), so it isn't penalized by SFI here.
pub fn g_sfi(band: Band, sfi: f32) -> f32 {
    match band {
        // Low bands are physically eligible regardless of solar flux.
        // 60 m behaves like the other low bands — eligible at any SFI.
        Band::B160 | Band::B80 | Band::B60 | Band::B40 => 1.0,
        Band::B30 | Band::B20 => {
            if sfi >= 70.0 {
                1.0
            } else {
                0.7
            }
        }
        Band::B17 | Band::B15 => match sfi {
            x if x >= 100.0 => 1.0,
            x if x >= 80.0 => 0.6,
            _ => 0.2,
        },
        Band::B12 | Band::B10 => match sfi {
            x if x >= 150.0 => 1.0,
            x if x >= 100.0 => 0.6,
            x if x >= 80.0 => 0.25,
            _ => 0.05,
        },
        // VHF: Es/aurora/MS don't track SFI — let observed spots speak.
        Band::B6 | Band::B4 | Band::B2 => 1.0,
    }
}

/// Plain-language reason a high band is physically suppressed by low SFI, if so.
pub fn sfi_closed_reason(band: Band, sfi: f32) -> Option<String> {
    if g_sfi(band, sfi) < 0.1 {
        Some(format!("solar flux {} too low today", sfi.round() as i32))
    } else {
        None
    }
}

/// Kp penalty multiplier. Kp ≤ 3 excellent; each step above costs ~15%; polar
/// paths take an extra hit (auroral absorption). Kp ≥ 6 = storm.
pub fn g_kp(kp: f32, polar_path: bool) -> f32 {
    let base = if kp <= 3.0 {
        1.0
    } else {
        (1.0 - 0.15 * (kp - 3.0)).max(0.0)
    };
    if polar_path && kp > 3.0 {
        (base - 0.2 * (kp - 3.0)).max(0.0)
    } else {
        base
    }
}

// ───────────────────────── space-weather trend ─────────────────────────
// A single instantaneous SpaceWx can't say "MUF is building". We keep a small bounded
// history of recent samples and compute a rising/steady/falling slope per quantity, so
// the insight layer can say "MUF building → 6m may open soon" / "Kp rising".

/// One timestamped space-weather sample (the values plus a representative MUF).
#[derive(Debug, Clone, Copy)]
pub struct SpaceWxSample {
    pub t: i64,
    pub sfi: f32,
    pub kp: f32,
    pub xray_long: f32,
    pub muf: f32,
}

/// Direction of a quantity's recent change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TrendDir {
    #[default]
    Steady,
    Rising,
    Falling,
}

/// A scalar's current value + recent slope (per hour) + direction.
#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScalarTrend {
    pub now: f32,
    /// Value at the oldest in-window sample (the true "from" for a "14→18" display —
    /// NOT an extrapolation of the slope over the nominal window).
    pub start: f32,
    pub delta_per_hr: f32,
    pub dir: TrendDir,
}

/// The space-weather trend snapshot the UI/insight layer consumes.
#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WxTrend {
    pub sfi: ScalarTrend,
    pub kp: ScalarTrend,
    pub muf: ScalarTrend,
    pub xray: ScalarTrend,
    pub window_secs: i64,
    pub samples: usize,
}

/// Per-quantity "noise floor" — a change smaller than this over the window reads as
/// Steady. Grounded in how fast each quantity actually moves (per hour).
const SFI_EPS: f32 = 2.0; // SFI ±2/hr
const KP_EPS: f32 = 0.5; // Kp ±0.5/hr
const MUF_EPS: f32 = 1.0; // MUF ±1 MHz/hr

fn classify(delta_per_hr: f32, eps: f32) -> TrendDir {
    if delta_per_hr > eps {
        TrendDir::Rising
    } else if delta_per_hr < -eps {
        TrendDir::Falling
    } else {
        TrendDir::Steady
    }
}

/// Bounded rolling history of [`SpaceWxSample`]s (≈5 h at the 300 s SWPC cadence).
#[derive(Debug, Clone, Default)]
pub struct SpaceWxHistory {
    buf: VecDeque<SpaceWxSample>,
}

impl SpaceWxHistory {
    /// Max retained samples (~5 h at a 300 s refresh).
    const CAP: usize = 64;
    /// Samples closer in time than this collapse (a cache hit re-stamping the same wx).
    const DEDUP_SECS: i64 = 60;

    /// Append a sample, deduping near-simultaneous re-stamps and bounding the buffer.
    pub fn push(&mut self, s: SpaceWxSample) {
        if let Some(last) = self.buf.back() {
            if (s.t - last.t).abs() < Self::DEDUP_SECS {
                *self.buf.back_mut().unwrap() = s; // replace the near-duplicate
                return;
            }
        }
        self.buf.push_back(s);
        while self.buf.len() > Self::CAP {
            self.buf.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Trend over the samples within `[now - window_secs, now]`. Needs ≥2 samples
    /// spanning ≥~30 min for a non-Steady verdict; otherwise everything reads Steady.
    pub fn trend(&self, now: i64, window_secs: i64) -> WxTrend {
        let lo = now - window_secs;
        let win: Vec<&SpaceWxSample> = self.buf.iter().filter(|s| s.t >= lo && s.t <= now).collect();
        let latest = self.buf.back();
        let scalar = |get: &dyn Fn(&SpaceWxSample) -> f32, eps: f32| -> ScalarTrend {
            let now_v = latest.map(|s| get(s)).unwrap_or(0.0);
            if win.len() < 2 {
                return ScalarTrend {
                    now: now_v,
                    start: now_v,
                    delta_per_hr: 0.0,
                    dir: TrendDir::Steady,
                };
            }
            let oldest = win.first().unwrap();
            let newest = win.last().unwrap();
            let hours = (newest.t - oldest.t) as f32 / 3600.0;
            if hours < 0.5 {
                return ScalarTrend {
                    now: now_v,
                    start: now_v,
                    delta_per_hr: 0.0,
                    dir: TrendDir::Steady,
                };
            }
            let delta_per_hr = (get(newest) - get(oldest)) / hours;
            ScalarTrend {
                now: now_v,
                start: get(oldest),
                delta_per_hr,
                dir: classify(delta_per_hr, eps),
            }
        };
        // X-ray varies over orders of magnitude — classify on the ratio, not the delta.
        let xray = {
            let now_v = latest.map(|s| s.xray_long).unwrap_or(0.0);
            if win.len() < 2 {
                ScalarTrend { now: now_v, start: now_v, delta_per_hr: 0.0, dir: TrendDir::Steady }
            } else {
                let oldest = win.first().unwrap();
                let newest = win.last().unwrap();
                let hours = (newest.t - oldest.t) as f32 / 3600.0;
                let ratio = if oldest.xray_long > 0.0 { newest.xray_long / oldest.xray_long } else { 1.0 };
                let dir = if ratio > 2.0 {
                    TrendDir::Rising
                } else if ratio < 0.5 {
                    TrendDir::Falling
                } else {
                    TrendDir::Steady
                };
                let delta_per_hr = if hours >= 0.5 { (newest.xray_long - oldest.xray_long) / hours } else { 0.0 };
                ScalarTrend { now: now_v, start: oldest.xray_long, delta_per_hr, dir }
            }
        };
        WxTrend {
            sfi: scalar(&|s| s.sfi, SFI_EPS),
            kp: scalar(&|s| s.kp, KP_EPS),
            muf: scalar(&|s| s.muf, MUF_EPS),
            xray,
            window_secs,
            samples: win.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfi_gate() {
        assert_eq!(g_sfi(Band::B40, 60.0), 1.0); // low band always
        assert!(g_sfi(Band::B10, 60.0) < 0.1); // 10m dead at low SFI
        assert_eq!(g_sfi(Band::B10, 180.0), 1.0); // 10m great at high SFI
        assert_eq!(g_sfi(Band::B6, 70.0), 1.0); // VHF observation-driven
    }

    #[test]
    fn kp_gate() {
        assert_eq!(g_kp(2.0, false), 1.0);
        assert!(g_kp(6.0, false) < 0.6);
        assert!(g_kp(6.0, true) < g_kp(6.0, false)); // polar harsher
    }

    fn sample(t: i64, sfi: f32, kp: f32, muf: f32) -> SpaceWxSample {
        SpaceWxSample { t, sfi, kp, xray_long: 1e-7, muf }
    }

    #[test]
    fn trend_detects_rising_sfi_and_building_muf() {
        let now = 1_700_000_000;
        let mut h = SpaceWxHistory::default();
        // 3 h of climbing SFI + MUF.
        h.push(sample(now - 3 * 3600, 120.0, 2.0, 14.0));
        h.push(sample(now - 2 * 3600, 130.0, 2.0, 18.0));
        h.push(sample(now, 150.0, 2.0, 24.0));
        let t = h.trend(now, 3 * 3600 + 60);
        assert_eq!(t.sfi.dir, TrendDir::Rising);
        assert!(t.sfi.delta_per_hr > 2.0, "{}", t.sfi.delta_per_hr);
        assert_eq!(t.muf.dir, TrendDir::Rising);
        assert!((t.muf.now - 24.0).abs() < 1e-3);
        assert_eq!(t.samples, 3);
    }

    #[test]
    fn trend_detects_falling_kp() {
        let now = 1_700_000_000;
        let mut h = SpaceWxHistory::default();
        h.push(sample(now - 2 * 3600, 120.0, 5.0, 20.0));
        h.push(sample(now, 120.0, 2.0, 20.0));
        let t = h.trend(now, 3 * 3600);
        assert_eq!(t.kp.dir, TrendDir::Falling);
        assert_eq!(t.sfi.dir, TrendDir::Steady); // flat SFI
    }

    #[test]
    fn trend_is_steady_with_one_sample_or_outside_window() {
        let now = 1_700_000_000;
        let mut h = SpaceWxHistory::default();
        h.push(sample(now, 150.0, 2.0, 24.0));
        assert_eq!(h.trend(now, 3 * 3600).sfi.dir, TrendDir::Steady); // single sample
        // An older second sample OUTSIDE the window leaves only one in-window → Steady.
        let mut h2 = SpaceWxHistory::default();
        h2.push(sample(now - 6 * 3600, 100.0, 2.0, 12.0));
        h2.push(sample(now, 150.0, 2.0, 24.0));
        assert_eq!(h2.trend(now, 3 * 3600).muf.dir, TrendDir::Steady);
    }

    #[test]
    fn history_caps_and_dedups() {
        let now = 1_700_000_000;
        let mut h = SpaceWxHistory::default();
        // Two samples within the dedup window collapse to one.
        h.push(sample(now, 150.0, 2.0, 24.0));
        h.push(sample(now + 10, 151.0, 2.0, 25.0));
        assert_eq!(h.len(), 1);
        assert!((h.trend(now + 10, 3600).muf.now - 25.0).abs() < 1e-3); // replaced
        // Cap: push far more than CAP distinct samples.
        let mut h2 = SpaceWxHistory::default();
        for i in 0..200 {
            h2.push(sample(now + i * 600, 120.0, 2.0, 14.0));
        }
        assert!(h2.len() <= 64, "bounded, got {}", h2.len());
    }
}
