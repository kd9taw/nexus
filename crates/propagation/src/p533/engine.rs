//! The `"p533"` [`PathPredictor`] engine — the validated P.533/P.372 chain
//! packaged behind the app's swappable prediction seam.
//!
//! Per band (at its digital watering-hole dial) the engine runs the full chain
//! for each of the 24 UT hours of the day containing the request, and maps the
//! outputs onto the existing [`BandOutlook`] shape:
//! - `hourly[h]` = Basic Circuit Reliability / 100 (a REAL per-hour "percent of
//!   days this circuit works" — the number the heuristic only approximated),
//! - `reliability` = the share of hours with BCR ≥ 50%,
//! - `muf_hourly` = the operational MUF, `window` formatted identically to the
//!   heuristic's (same contiguous-run + greyline logic and format strings).
//!
//! Inputs: the smoothed sunspot number rides in [`SpaceWx::ssn`] when the live
//! solar-cycle feed has it; otherwise it is derived from SFI by inverting the
//! Covington relation (documented approximation). Assumptions (isotropic
//! antennas, TX power, man-made noise class, required SNR) live in
//! [`P533Config`] — every constant tunable, none buried in the math.

use crate::geo;
use crate::likelihood::{hhmm, hhmm_z, mode_now_at, BandOutlook, ModeHourly, Workability};
use crate::model::{band_digital_mhz, Band, SpaceWx};
use crate::predict::{PathPrediction, PathPredictor};

use super::geometry::Location;
use super::noise::ManMadeCategory;
use super::reliability::bcr_for_required_snr;
use super::{run_p533, P533Params};

/// Required SNR (dB in 1 Hz) per operating mode, for the per-mode "workable
/// now" rows. VOACAP-conventional figures: FT8 13 dB·Hz (−21 dB in 2.5 kHz),
/// FT4 ≈ 3.5 dB less sensitive, CW 24 dB·Hz (skilled ear), SSB 38 dB·Hz
/// (communication quality). Table order = display order.
const MODE_SNR_DBHZ: [(&str, f64); 4] = [("FT8", 13.0), ("FT4", 16.5), ("CW", 24.0), ("SSB", 38.0)];

/// Tunable system assumptions for the p533 engine.
#[derive(Debug, Clone, Copy)]
pub struct P533Config {
    /// Transmit power, dB(1 kW). 100 W = −10 dB(1 kW), the app default.
    pub txpower_dbkw: f64,
    /// Man-made noise environment at the receiver.
    pub man_made: ManMadeCategory,
    /// Required SNR (dB in 1 Hz) for the "circuit works" criterion. 13 dB·Hz
    /// ≈ FT8's −21 dB in 2500 Hz — the digital-first default.
    pub required_snr_dbhz: f64,
    /// BCR (%) a hour must clear to count toward `BandOutlook.reliability`.
    pub bcr_usable_pct: f64,
    /// Combined antenna gain (dBi, TX + RX) applied as a plain dB adder to the
    /// link budget (v1: no pattern/takeoff-angle modelling — the P.372 noise is
    /// referenced to a short lossless antenna, so signal-side gain shifts SNR 1:1).
    pub ant_gain_dbi: f64,
}

impl Default for P533Config {
    fn default() -> Self {
        Self {
            txpower_dbkw: -10.0,
            man_made: ManMadeCategory::Rural,
            required_snr_dbhz: 13.0,
            bcr_usable_pct: 50.0,
            ant_gain_dbi: 0.0,
        }
    }
}

impl P533Config {
    /// Config with the transmit power taken from a station-power setting (W).
    pub fn with_power_watts(watts: f64) -> Self {
        Self {
            txpower_dbkw: 10.0 * (watts.max(0.1) / 1000.0).log10(),
            ..Self::default()
        }
    }
}

/// Smoothed sunspot number from SFI by inverting Covington
/// (SFI ≈ 63.75 + 0.728·R + 0.00089·R²) — the offline fallback when the live
/// solar-cycle feed hasn't supplied a real R12.
pub fn ssn_from_sfi(sfi: f64) -> f64 {
    // Quadratic inversion; clamped to the physical range.
    let a = 0.00089;
    let b = 0.728;
    let c = 63.75 - sfi;
    let disc = (b * b - 4.0 * a * c).max(0.0);
    ((-b + disc.sqrt()) / (2.0 * a)).clamp(0.0, 250.0)
}

/// The p533 engine: the operator location is baked in like [`crate::predict::HeuristicEngine`].
pub struct P533Engine {
    me: Option<(f64, f64)>,
    cfg: P533Config,
}

impl P533Engine {
    pub fn new(me_latlon: Option<(f64, f64)>) -> Self {
        Self {
            me: me_latlon,
            cfg: P533Config::default(),
        }
    }

    pub fn with_config(me_latlon: Option<(f64, f64)>, cfg: P533Config) -> Self {
        Self { me: me_latlon, cfg }
    }
}

/// The heuristic's window formatting over a 24×1-hour score array: the
/// contiguous ≥Fair run containing the peak, with the low-band greyline label
/// (byte-identical format strings so the two engines read the same in the UI).
fn best_window(scores: &[f32; 24], day0: i64, band: Band) -> (String, bool, f32, usize) {
    const STEP: i64 = 3600;
    let (mut peak_i, mut peak) = (0usize, 0.0f32);
    for (i, &s) in scores.iter().enumerate() {
        if s > peak {
            peak = s;
            peak_i = i;
        }
    }
    let mut grayline = false;
    let window = if peak < 0.30 {
        "—".to_string()
    } else {
        let thresh = 0.30f32;
        let mut lo = peak_i;
        while lo > 0 && scores[lo - 1] >= thresh {
            lo -= 1;
        }
        let mut hi = peak_i;
        while (hi + 1) < 24 && scores[hi + 1] >= thresh {
            hi += 1;
        }
        let start = day0 + lo as i64 * STEP;
        let end = day0 + (hi as i64 + 1) * STEP;
        let width = end - start;
        let low_band = matches!(band, Band::B160 | Band::B80 | Band::B40);
        if low_band && width <= 2 * 3600 {
            grayline = true;
            format!("~{} greyline", hhmm_z(day0 + peak_i as i64 * STEP))
        } else {
            format!("{}–{}", hhmm(start), hhmm_z(end))
        }
    };
    (window, grayline, peak, peak_i)
}

impl PathPredictor for P533Engine {
    fn name(&self) -> &'static str {
        "p533"
    }

    fn predict(&self, dx: (f64, f64), from_unix: i64, wx: &SpaceWx) -> PathPrediction {
        // No operator anchor → empty outlooks, mirroring HeuristicEngine.
        let Some(me) = self.me else {
            return PathPrediction {
                engine: "p533".to_string(),
                bands: Vec::new(),
                muf_now: 0.0,
                muf_hourly: vec![0.0; 24],
            };
        };

        let ssn = wx
            .ssn
            .map(f64::from)
            .unwrap_or_else(|| ssn_from_sfi(wx.sfi as f64));

        // The day containing the request, anchored to UTC midnight like the
        // heuristic (predict.rs) so heatmap x-axes align; month from that day.
        let day0 = from_unix - from_unix.rem_euclid(86_400);
        let (_, month1, _) = geo::civil_from_days(day0.div_euclid(86_400));
        let month0 = (month1 - 1) as usize;
        let now_hour = (from_unix.rem_euclid(86_400) / 3600) as usize;

        let tx = Location::new(me.0.to_radians(), me.1.to_radians());
        let rx = Location::new(dx.0.to_radians(), dx.1.to_radians());
        let sys = P533Params {
            // Antenna gain rides on the TX power term (see P533Config.ant_gain_dbi).
            txpower: self.cfg.txpower_dbkw + self.cfg.ant_gain_dbi,
            bw_hz: 1.0,
            snrr: self.cfg.required_snr_dbhz,
            snrxxp: 90,
            man_made: self.cfg.man_made,
        };

        let mut bands: Vec<BandOutlook> = Vec::new();
        let mut muf_hourly = vec![0.0f32; 24];
        for &band in Band::ALL.iter().filter(|b| !b.is_vhf()) {
            let freq = band_digital_mhz(band);
            let mut hourly = [0.0f32; 24];
            let mut mode_hourly: Vec<ModeHourly> = MODE_SNR_DBHZ
                .iter()
                .map(|&(mode, _)| ModeHourly {
                    mode: mode.to_string(),
                    hourly: [0.0; 24],
                })
                .collect();
            for (h, slot_score) in hourly.iter_mut().enumerate() {
                // UT hour h ↔ reference slot h−1 (slot i is UT i+1).
                let slot = ((h + 23) % 24) as i32;
                let run = run_p533(tx, rx, month0, slot, ssn, freq, &sys);
                *slot_score = (run.rel.bcr / 100.0).clamp(0.0, 1.0) as f32;
                // The MUF ceiling is per-path (band-independent); fill once.
                if band == Band::B20 {
                    muf_hourly[h] = run.path.opmuf as f32;
                }
                // Per-mode workability: the hour's SNR distribution is
                // mode-independent, so each mode is one closed-form BCR
                // re-evaluation against its required SNR — no chain re-run.
                // ALL hours are kept (mode_hourly) so a day-scale cache can
                // re-derive the "now" chips at serve time.
                for (mi, &(_, snrr)) in MODE_SNR_DBHZ.iter().enumerate() {
                    mode_hourly[mi].hourly[h] =
                        (bcr_for_required_snr(run.rel.snr, run.rel.du_sn, run.rel.dl_sn, snrr)
                            / 100.0)
                            .clamp(0.0, 1.0) as f32;
                }
            }
            let mode_now = mode_now_at(&mode_hourly, now_hour);
            let (window, grayline, peak, _) = best_window(&hourly, day0, band);
            let usable = hourly
                .iter()
                .filter(|&&s| s * 100.0 >= self.cfg.bcr_usable_pct as f32)
                .count();
            bands.push(BandOutlook {
                band: band.label().to_string(),
                workability: Workability::from_score(peak).label().to_string(),
                score: peak,
                window,
                grayline,
                hourly: hourly.to_vec(),
                reliability: (usable as f32 / 24.0) * 100.0,
                mode_now,
                mode_hourly,
            });
        }
        bands.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        PathPrediction {
            engine: "p533".to_string(),
            bands,
            muf_now: muf_hourly[now_hour],
            muf_hourly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covington_inversion_roundtrips() {
        for r in [0.0f64, 25.0, 70.0, 120.0, 160.0] {
            let sfi = 63.75 + 0.728 * r + 0.00089 * r * r;
            assert!((ssn_from_sfi(sfi) - r).abs() < 0.5, "R {r}");
        }
        // Quiet-sun floor clamps at 0.
        assert_eq!(ssn_from_sfi(50.0), 0.0);
    }

    #[test]
    fn engine_produces_a_sane_prediction() {
        // Chicago → Munich, May, SSN via a mid-cycle SFI.
        let eng = P533Engine::new(Some((41.98, -87.9)));
        let wx = SpaceWx {
            sfi: 150.0,
            ..Default::default()
        };
        // ~2026-05-15 12:00 UT.
        let t = crate::geo::days_from_civil(2026, 5, 15) * 86_400 + 12 * 3600;
        let pred = eng.predict((48.35, 11.79), t, &wx);
        assert_eq!(pred.engine, "p533");
        assert_eq!(eng.name(), "p533");
        assert!(!pred.bands.is_empty());
        // HF only, sorted best-first, sane ranges.
        assert!(pred
            .bands
            .iter()
            .all(|b| !matches!(b.band.as_str(), "6m" | "4m" | "2m")));
        for w in pred.bands.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
        for b in &pred.bands {
            assert_eq!(b.hourly.len(), 24);
            assert!(b.hourly.iter().all(|&s| (0.0..=1.0).contains(&s)));
            assert!((0.0..=100.0).contains(&b.reliability));
        }
        // A mid-latitude 7 Mm path at SFI 150 should find a usable band.
        assert!(
            pred.bands.iter().any(|b| b.score >= 0.3),
            "{:?}",
            pred.bands
                .iter()
                .map(|b| (&b.band, b.score))
                .collect::<Vec<_>>()
        );
        assert_eq!(pred.muf_hourly.len(), 24);
        assert!(pred.muf_now > 0.0, "OPMUF ceiling at SFI 150");
        // No anchor → empty.
        let none = P533Engine::new(None).predict((48.35, 11.79), t, &wx);
        assert!(none.bands.is_empty());
    }
}
