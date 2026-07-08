//! Opening Detection v2 — anomaly/onset + mode classifier + false-positive
//! rejection. The substance the ported `detector.rs` lacked: a real baseline/
//! onset anomaly score, operator-anchored two-way reciprocity, a rule-ordered
//! Es/F2-TEP/Aurora/Tropo classifier (geometry + space weather), and an
//! anti-flap state machine with cold-start seeding for honest onset alerting.
//!
//! Everything here is **pure** — `now` is always a parameter (no wall clock) so
//! the whole pipeline is deterministic and unit-testable with synthetic spots.
//! See `tasks/specs/opening-detection.md`.
//!
//! Data-availability constraints that shape v1 (verified against the live feed):
//! - The PSK Reporter MQTT feed is **topic-only → `snr: None`** for every spot,
//!   so all SNR-derived features and the aurora *decode* signature are Phase 2.
//!   v1 classifies on **geometry + space weather** only.
//! - The feed is **operator-centric** (own-call topic filters), so every spot has
//!   the operator on one end. Features are **operator-relative** ("open *for me*
//!   to EU"), not a band-wide census; a regional/global feed is a Phase-2 fork.

use std::collections::{HashMap, HashSet};

use crate::geo::{bearing_deg, geomagnetic_lat_deg, grid_distance_km, maidenhead_to_latlon};
use crate::model::{Band, PathSpot, PropMode, Side, SpaceWx};

#[inline]
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// Tunable thresholds for the detector. All values are research-seeded starting
/// points (`[TUNE]`); calibrate against a labeled corpus in Phase 3.
#[derive(Debug, Clone)]
pub struct OpeningConfig {
    /// Short ("now") window for the onset rate, seconds.
    pub short_w: i64,
    /// Baseline window (the short window + the prior bins it is judged against).
    pub base_w: i64,
    /// Enter threshold: anomaly z at/above which a band is "raw open".
    pub z_open: f32,
    /// Exit threshold: a band stays "warm" (won't close) while z ≥ this.
    pub z_close: f32,
    /// Floor on the baseline scale (1.4826·MAD) so a dead-quiet band can't
    /// divide to ∞.
    pub sigma_floor: f32,
    /// Min distinct far receivers (who-heard-me) to clear the gate (OR side A).
    pub min_far_rx: usize,
    /// Min distinct far transmitters (who-I-heard) to clear the gate (OR side B).
    pub min_far_tx: usize,
    /// Onset-slope reference (Δ rate, spots/min/window). Reserved as a rising-edge
    /// / terminator-ramp tuning knob — NOT a hard gate (see `raw_open`), since a
    /// plateauing opening has slope≈0 after its rising edge.
    pub slope_min: f32,
    /// Kp at/above which aurora is gated on (≥5 at high geomag latitude).
    pub kp_aurora: f32,
    /// SFI at/above which F2/TEP is plausible.
    pub sfi_tep: f32,
    /// Skip-zone inner boundary (km): an Es hole shows few spots inside this.
    pub d_near_km: f64,
    /// Skip-hole ratio: near-count ≤ this × far-count (and enough far) ⇒ a hole.
    pub skip_ratio: f64,
    /// Min far-side (≥ d_near) spots required before a skip hole can be declared.
    pub min_far_for_skip: usize,
    /// Consecutive raw-open windows required to ENTER the open state.
    pub enter_windows: u32,
    /// Consecutive cold (below z_close) windows required to EXIT.
    pub exit_windows: u32,
    /// Geomagnetic |lat| (deg) at/above which a far end is "auroral-zone".
    pub auroral_lat: f64,
    /// Phase 2: consider near-region (neither-end-is-operator) spots in the open
    /// gate. Default false → operator-anchored v1 behavior, bit-identical.
    pub regional_scope: bool,
    /// Min distinct participating stations for a REGIONAL open (Phase 2).
    pub min_regional_stations: usize,
    /// Min two-way pairs for a regional open — rejects one loud station heard by many.
    pub min_regional_reciprocal: usize,
    /// Min DISTINCT near-the-operator receivers for a regional open. The
    /// anti-superstation rule: one tall-tower station hearing twelve DX is a
    /// single endpoint, not a regional opening — the gate demands a collection
    /// of spots across MULTIPLE local endpoints before believing the band.
    pub min_regional_near_rx: usize,
    /// "Near the operator" radius (km) for the endpoint census above.
    pub region_near_km: f64,
    /// Min cross-band share for a regional open — rejects a uniform contest/Es surge
    /// lifting every band (a real opening is band-specific).
    pub min_regional_cross_band_share: f32,
    /// Baseline hold-out: exclude the most-recent N bins (the current episode +
    /// its rising edge) from the anomaly baseline so z survives a plateau.
    pub gap_bins: usize,
    /// Hard ceiling on how long an opening can stay latched open (a backstop so a
    /// perpetually-"warm" band can't pin the latch forever).
    pub max_dwell_secs: i64,
}

impl Default for OpeningConfig {
    fn default() -> Self {
        Self {
            short_w: 600, // 10 min
            base_w: 7200, // 2 h (12 × 10-min bins) — long enough that an
            // opening occupying the recent ~30 min is a small fraction of the
            // baseline, so a *sustained* opening's anomaly z stays elevated for
            // its duration instead of normalising within a couple of polls.
            z_open: 4.0,
            z_close: 2.0,
            sigma_floor: 0.05, // spots/min
            min_far_rx: 5,
            min_far_tx: 3,
            slope_min: 0.0, // any positive onset; tune up to reject ramps
            kp_aurora: 6.0,
            sfi_tep: 150.0,
            d_near_km: 550.0,
            skip_ratio: 0.15,
            min_far_for_skip: 4,
            enter_windows: 2,
            exit_windows: 3,
            auroral_lat: 55.0,
            regional_scope: false, // v1 default: operator-anchored gate only
            min_regional_stations: 12,
            min_regional_reciprocal: 2,
            min_regional_cross_band_share: 0.3,
            min_regional_near_rx: 3,
            region_near_km: 800.0,
            gap_bins: 3, // hold out the recent ~30 min (the episode + rising edge)
            max_dwell_secs: 6 * 3600, // 6 h backstop
        }
    }
}

/// Per-band features computed over a window of operator-relative path spots.
/// Public so classifier/tracker tests can construct them directly.
#[derive(Debug, Clone)]
pub struct BandFeatures {
    pub band: Band,
    pub spot_count: usize,
    /// Distinct far receivers across `Side::HeardMe` spots ("who heard me").
    pub unique_far_rx: usize,
    /// Distinct far transmitters across `Side::IHeard` spots ("who I heard").
    pub unique_far_tx: usize,
    /// Far stations confirmed BOTH ways with the operator (me→X and X→me).
    pub reciprocal_pairs: usize,
    /// Distinct stations participating on EITHER end of ALL spots — the regional
    /// density census (Phase 2). On the own-call feed this ≈ `unique_far_*` + 1.
    pub unique_stations: usize,
    /// Distinct unordered call-pairs {A,B} confirmed both ways (regional two-way),
    /// not just those involving the operator (Phase 2). Superset of `reciprocal_pairs`.
    pub reciprocal_pairs_regional: usize,
    /// Distinct RECEIVER endpoints within `region_near_km` of the operator among
    /// far↔far spots — the anti-superstation census (multiple local ears).
    pub unique_near_rx: usize,
    pub median_km: f64,
    pub max_km: f64,
    pub min_km: f64,
    pub p10_km: f64,
    /// A skip-zone hole (few spots inside `d_near`, many beyond) — Es signature.
    pub skip_hole: bool,
    pub bearing_mean_deg: f64,
    /// Circular resultant length of path bearings, 0 (isotropic) … 1 (one dir).
    pub bearing_concentration: f64,
    /// Fraction of paths within ±20° of N–S (TEP geometry input).
    pub ns_fraction: f64,
    /// Fraction of far ends whose geomagnetic-lat sign opposes the operator's
    /// (a centered-dipole proxy for crossing the geomagnetic equator — TEP).
    pub equator_crossing_frac: f64,
    /// Fraction of far ends at auroral geomagnetic latitudes.
    pub auroral_frac: f64,
    /// Spots/min in the most-recent short window.
    pub rate_short: f32,
    /// Median spots/min over the (held-out) baseline bins — the robust "normal".
    pub rate_base: f32,
    /// Onset anomaly: (rate_short − median_baseline) / max(1.4826·MAD, σ_floor).
    pub anomaly_z: f32,
    /// Δ rate vs the previous bin (spots/min) — positive = rising onset.
    pub onset_slope: f32,
    /// Short-window rate (spots/min) counting ONLY getting-out (`HeardMe`) + far↔far
    /// (`Neither`) evidence — the cross-band-share DENOMINATOR input. Excludes the
    /// operator's own `IHeard` receive-firehose (every station their radio decodes on
    /// the band they're parked on), so a busy own-band QSO session can't dilute a
    /// genuine single-band opening's share. See [`detect`].
    pub share_rate_short: f32,
    /// This band's short-window share of total cross-band activity (localization;
    /// a uniform all-band surge — contest — drives every band's share down).
    pub cross_band_share: f32,
    /// SNR features — Phase 2 (`None` on the topic-only MQTT feed).
    pub median_snr: Option<f32>,
    pub snr_var: Option<f32>,
}

impl BandFeatures {
    /// A zeroed feature set for a band with no activity (used as a test base and
    /// for the closed-band path).
    pub fn empty(band: Band) -> Self {
        Self {
            band,
            spot_count: 0,
            unique_far_rx: 0,
            unique_far_tx: 0,
            reciprocal_pairs: 0,
            unique_stations: 0,
            reciprocal_pairs_regional: 0,
            unique_near_rx: 0,
            median_km: 0.0,
            max_km: 0.0,
            min_km: 0.0,
            p10_km: 0.0,
            skip_hole: false,
            bearing_mean_deg: 0.0,
            bearing_concentration: 0.0,
            ns_fraction: 0.0,
            equator_crossing_frac: 0.0,
            auroral_frac: 0.0,
            rate_short: 0.0,
            rate_base: 0.0,
            anomaly_z: 0.0,
            onset_slope: 0.0,
            share_rate_short: 0.0,
            cross_band_share: 0.0,
            median_snr: None,
            snr_var: None,
        }
    }

    /// Does this band clear the generic opening gate at the enter threshold?
    /// Anomaly ≥ z_open AND (enough far-rx OR enough far-tx). Onset slope is NOT
    /// a hard gate here: a sustained (plateauing) opening has slope≈0 after its
    /// rising edge, so gating on slope would make `raw_open` true for only one
    /// window and defeat the ≥`enter_windows` enter requirement. Slope is kept as
    /// a feature (a rising-edge / terminator-ramp signal) for confidence + Phase-2
    /// tuning, not as the open gate.
    pub fn raw_open(&self, cfg: &OpeningConfig) -> bool {
        if self.anomaly_z < cfg.z_open {
            return false;
        }
        // VHF/Es bands (6/4/2 m) open SUDDENLY with FEW stations and no cross-band
        // breadth — a real Es burst shows 2–3 far stations, not the 5/3 + reciprocity
        // + cross-band-share an HF F2 opening needs. The HF-tuned thresholds were
        // effectively disabling 6 m detection, so loosen them on VHF (never tighten).
        let vhf = self.band.is_vhf();
        let (far_rx, far_tx) = if vhf {
            (cfg.min_far_rx.min(3), cfg.min_far_tx.min(2))
        } else {
            (cfg.min_far_rx, cfg.min_far_tx)
        };
        // Operator-anchored gate (v1): enough far stations on either direction.
        let op_gate = self.unique_far_rx >= far_rx || self.unique_far_tx >= far_tx;
        // Regional gate (Phase 2, opt-in): a band-wide surge near the operator.
        // Multi-condition so neither a single loud station (needs two-way pairs)
        // nor a uniform contest/Es lifting every band (needs band-specificity)
        // can fabricate an opening. Band-agnostic: the cross-band-share denominator
        // is now computed over getting-out + far↔far evidence only (see `detect` /
        // `share_rate_short`), so the operator's own IHeard receive-firehose on a
        // busy band no longer dilutes a genuine single-band opening's share. That is
        // a DENOMINATOR fix, not a threshold relax — a uniform contest surge still
        // drives every band's share below `min_regional_cross_band_share`, so contest
        // rejection is preserved and this gate is left as-is.
        let regional_gate = cfg.regional_scope
            && self.unique_stations >= cfg.min_regional_stations
            && self.unique_near_rx >= cfg.min_regional_near_rx
            && self.reciprocal_pairs_regional >= cfg.min_regional_reciprocal
            && self.cross_band_share >= cfg.min_regional_cross_band_share;
        op_gate || regional_gate
    }

    /// Is the band still "warm" (above the exit threshold)?
    pub fn warm(&self, cfg: &OpeningConfig) -> bool {
        self.anomaly_z >= cfg.z_close
    }
}

/// One band's per-window signal: features + classification + raw open/warm flags.
#[derive(Debug, Clone)]
pub struct BandSignal {
    pub band: Band,
    pub features: BandFeatures,
    pub mode: PropMode,
    /// Combined honest confidence in [0, 1].
    pub confidence: f32,
    pub raw_open: bool,
    pub warm: bool,
}

/// Operator-anchored two-way reciprocity: distinct far stations X for which BOTH
/// `me→X` (X heard me) AND `X→me` (I heard X) exist in the window. Keyed by far
/// **callsign** (two stations in the same grid count separately). On the v1
/// own-call feed one end is always the operator; the unordered-pair form keeps it
/// forward-compatible with a Phase-2 far↔far feed.
pub fn reciprocity(spots: &[PathSpot], me_call: &str, now: i64, window: i64) -> usize {
    let cutoff = now - window;
    let mut heard_me: HashSet<String> = HashSet::new();
    let mut i_heard: HashSet<String> = HashSet::new();
    for s in spots.iter().filter(|s| s.time >= cutoff) {
        match s.side(me_call) {
            Side::HeardMe => {
                if let Some(c) = s.far_call(me_call) {
                    heard_me.insert(c.to_uppercase());
                }
            }
            Side::IHeard => {
                if let Some(c) = s.far_call(me_call) {
                    i_heard.insert(c.to_uppercase());
                }
            }
            Side::Neither => {}
        }
    }
    heard_me.intersection(&i_heard).count()
}

/// Regional two-way reciprocity: distinct unordered call-pairs {A,B} for which
/// BOTH directions exist (A heard B AND B heard A) in the window — NOT just pairs
/// involving the operator. The operator-anchored [`reciprocity`] is the special
/// case where one end is `me`. Keyed by callsign (matching the far-call contract).
pub fn reciprocity_regional(spots: &[PathSpot], now: i64, window: i64) -> usize {
    let cutoff = now - window;
    let mut directed: HashSet<(String, String)> = HashSet::new();
    for s in spots.iter().filter(|s| s.time >= cutoff) {
        directed.insert((
            s.tx_call.to_ascii_uppercase(),
            s.rx_call.to_ascii_uppercase(),
        ));
    }
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    for (a, b) in &directed {
        if directed.contains(&(b.clone(), a.clone())) {
            // Canonicalize the unordered pair (smaller call first) to dedupe mirrors.
            let key = if a <= b {
                (a.clone(), b.clone())
            } else {
                (b.clone(), a.clone())
            };
            pairs.insert(key);
        }
    }
    pairs.len()
}

/// The FARTHER of two candidate grids from `me` (for folding a near-region
/// neither-end-is-operator spot into the me-anchored geometry — one symmetric far
/// sample). `None` if neither grid resolves a distance.
fn farther_grid<'a>(me: &str, a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    let da = a.and_then(|g| grid_distance_km(me, g));
    let db = b.and_then(|g| grid_distance_km(me, g));
    match (da, db) {
        (Some(x), Some(y)) => {
            if x >= y {
                a
            } else {
                b
            }
        }
        (Some(_), None) => a,
        (None, Some(_)) => b,
        (None, None) => None,
    }
}

/// Percentile (0..1) of an ascending-sorted slice (linear interpolation).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let idx = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let f = idx - lo as f64;
    sorted[lo] * (1.0 - f) + sorted[hi] * f
}

/// Compute the onset anomaly: bin the base window into `short_w`-sized bins
/// ending at `now`; bin 0 is the "now" window, bins 1.. are the baseline.
/// Returns (rate_short, rate_base_mean, anomaly_z, onset_slope) in spots/min.
fn anomaly(times: &[i64], now: i64, cfg: &OpeningConfig) -> (f32, f32, f32, f32) {
    let bin_secs = cfg.short_w.max(1);
    let n_bins = (cfg.base_w / bin_secs).max(2) as usize;
    let mut bins = vec![0u32; n_bins];
    for &t in times {
        let age = now - t;
        if age < 0 || age >= cfg.base_w {
            continue;
        }
        let b = (age / bin_secs) as usize;
        if b < n_bins {
            bins[b] += 1;
        }
    }
    let per_min = (bin_secs as f32) / 60.0;
    let rate = |count: u32| count as f32 / per_min;
    let rate_short = rate(bins[0]);
    let rate_prev = if n_bins > 1 { rate(bins[1]) } else { 0.0 };
    // Baseline = the OLDER bins, holding out the most-recent `gap_bins` (the
    // current episode + its rising edge). Without the hold-out the opening's own
    // hot bins age into the baseline within a window or two and collapse z, so a
    // *sustained* (plateauing) opening would stop registering after one poll.
    // The hold-out keeps z high for the bins where the baseline is still the
    // pre-onset norm (~the opening's first hour); a multi-hour opening eventually
    // becomes "the new normal" and z decays — a known v1 limitation (a true
    // persistent-opening latch using absolute activity is Phase 2).
    let gap = cfg.gap_bins.clamp(1, n_bins.saturating_sub(1)).max(1);
    let base: Vec<f32> = bins[gap..].iter().map(|&c| rate(c)).collect();
    // Robust baseline: median + MAD, NOT mean/σ. As a sustained opening ages, its
    // hot bins leak into the baseline one at a time; with mean/σ a single hot bin
    // inflates σ and collapses z within a couple of polls (the plateau would stop
    // registering). The median/MAD ignore a minority of hot bins, so z stays
    // elevated for the opening's duration until it occupies >½ the baseline.
    let (med, mad) = median_mad(&base);
    let scale = (1.4826 * mad).max(cfg.sigma_floor); // 1.4826·MAD ≈ σ for normal data
    let z = (rate_short - med) / scale;
    let slope = rate_short - rate_prev;
    (rate_short, med, z, slope)
}

/// Median of a slice (copies + sorts; returns 0 for empty).
fn median(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Median and median-absolute-deviation (robust location + scale).
fn median_mad(xs: &[f32]) -> (f32, f32) {
    if xs.is_empty() {
        return (0.0, 0.0);
    }
    let med = median(xs);
    let dev: Vec<f32> = xs.iter().map(|x| (x - med).abs()).collect();
    (med, median(&dev))
}

/// Compute features for one band from operator-relative spots already filtered to
/// that band. `me_grid` anchors the path geometry. `cross_band_share` is filled
/// later by [`detect_bands`].
pub fn band_features(
    band: Band,
    band_spots: &[&PathSpot],
    me_call: &str,
    me_grid: &str,
    now: i64,
    cfg: &OpeningConfig,
) -> BandFeatures {
    let mut bf = BandFeatures::empty(band);
    bf.spot_count = band_spots.len();
    if band_spots.is_empty() {
        return bf;
    }

    let mut far_rx: HashSet<String> = HashSet::new();
    let mut far_tx: HashSet<String> = HashSet::new();
    let mut near_rx: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_stations: HashSet<String> = HashSet::new();
    let mut dists: Vec<f64> = Vec::new();
    let mut bearings: Vec<f64> = Vec::new();
    let mut snrs: Vec<f64> = Vec::new(); // SNRs (dB) from the MQTT payload, when present
    let me_geomag = maidenhead_to_latlon(me_grid).map(|(la, lo)| geomagnetic_lat_deg(la, lo));
    let me_ll = maidenhead_to_latlon(me_grid);
    let mut equator_cross = 0usize;
    let mut auroral = 0usize;
    let mut geo_far = 0usize; // far ends with a usable grid (denominator for fracs)
    let times: Vec<i64> = band_spots.iter().map(|s| s.time).collect();
    // Cross-band-share denominator input: count only getting-out (HeardMe) + far↔far
    // (Neither) spots in the SAME short window `rate_short` uses. Excluding the
    // operator's IHeard receive-firehose keeps a busy own-band QSO session from
    // inflating the cross-band denominator (see `detect`).
    let short_cutoff = now - cfg.short_w;
    let mut share_short_count = 0usize;

    for s in band_spots {
        // Regional density census: every distinct station on either end.
        all_stations.insert(s.tx_call.to_ascii_uppercase());
        all_stations.insert(s.rx_call.to_ascii_uppercase());
        if let Some(snr) = s.snr {
            snrs.push(snr as f64);
        }
        let side = s.side(me_call);
        if s.time > short_cutoff && s.time <= now && side != Side::IHeard {
            share_short_count += 1;
        }
        // The single grid to fold into the operator-anchored geometry pools:
        // operator spots → the far end (bit-identical to the old far_grid path);
        // a near-region (Neither) spot → its FARTHER end from me (one symmetric
        // sample, NOT both — folding both would inject a spurious short leg and
        // suppress the very skip-hole it should drive).
        let geo_grid: Option<&str> = match side {
            Side::HeardMe => {
                if let Some(c) = s.far_call(me_call) {
                    far_rx.insert(c.to_ascii_uppercase());
                }
                s.rx_grid.as_deref()
            }
            Side::IHeard => {
                if let Some(c) = s.far_call(me_call) {
                    far_tx.insert(c.to_ascii_uppercase());
                }
                s.tx_grid.as_deref()
            }
            Side::Neither => {
                // Anti-superstation census: which DISTINCT local receivers (within
                // region_near_km) are independently copying this band?
                if let Some(rxg) = s.rx_grid.as_deref() {
                    if let Some(d) = grid_distance_km(me_grid, rxg) {
                        if d <= cfg.region_near_km {
                            near_rx.insert(s.rx_call.to_ascii_uppercase());
                        }
                    }
                }
                farther_grid(me_grid, s.tx_grid.as_deref(), s.rx_grid.as_deref())
            }
        };
        if let Some(fg) = geo_grid {
            if let Some(d) = grid_distance_km(me_grid, fg) {
                dists.push(d);
            }
            if let (Some(me), Some((fla, flo))) = (me_ll, maidenhead_to_latlon(fg)) {
                bearings.push(bearing_deg(me, (fla, flo)));
                geo_far += 1;
                let fg_geomag = geomagnetic_lat_deg(fla, flo);
                if let Some(mg) = me_geomag {
                    if mg.signum() != fg_geomag.signum() {
                        equator_cross += 1;
                    }
                }
                if fg_geomag.abs() >= cfg.auroral_lat {
                    auroral += 1;
                }
            }
        }
    }

    bf.unique_far_rx = far_rx.len();
    bf.unique_far_tx = far_tx.len();
    bf.unique_near_rx = near_rx.len();
    bf.unique_stations = all_stations.len();
    let owned: Vec<PathSpot> = band_spots.iter().map(|s| (*s).clone()).collect();
    bf.reciprocal_pairs = reciprocity(&owned, me_call, now, cfg.base_w);
    bf.reciprocal_pairs_regional = reciprocity_regional(&owned, now, cfg.base_w);

    dists.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if !dists.is_empty() {
        bf.min_km = dists[0];
        bf.max_km = *dists.last().unwrap();
        bf.median_km = percentile(&dists, 0.5);
        bf.p10_km = percentile(&dists, 0.10);
        // Skip-hole is an Es signature and only meaningful on the Es-capable
        // bands (10/6/4/2 m), where short ground/tropo contacts normally exist so
        // their absence beyond a dead zone is notable. On lower HF a long-haul
        // cluster trivially has no near spots — that's normal DX, not a skip hole
        // — so leaving skip_hole false there lets the long-haul F2 path classify.
        if matches!(band, Band::B10 | Band::B6 | Band::B4 | Band::B2) {
            let near = dists.iter().filter(|&&d| d < cfg.d_near_km).count();
            let far = dists.len() - near;
            bf.skip_hole =
                far >= cfg.min_far_for_skip && (near as f64) <= cfg.skip_ratio * (far as f64);
        }
    }

    // SNR distribution (now that the MQTT payload carries `rp`): the median is a band
    // "how loud" signal and the variance distinguishes a steady opening from a flutter-y
    // (aurora/scatter) one. Populated as features for confidence/diagnostics; gating the
    // open/aurora decision on them is a separate tuning pass (kept Phase-2).
    if !snrs.is_empty() {
        snrs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        bf.median_snr = Some(percentile(&snrs, 0.5) as f32);
        let mean = snrs.iter().sum::<f64>() / snrs.len() as f64;
        let var = snrs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / snrs.len() as f64;
        bf.snr_var = Some(var as f32);
    }

    if !bearings.is_empty() {
        let (mut sx, mut sy) = (0.0f64, 0.0f64);
        let mut ns = 0usize;
        for &b in &bearings {
            let r = b.to_radians();
            sx += r.cos();
            sy += r.sin();
            // within ±20° of due N (0/360) or due S (180)
            let d_n = ((b + 180.0) % 360.0 - 180.0).abs();
            let d_s = ((b - 180.0 + 540.0) % 360.0 - 180.0).abs();
            if d_n <= 20.0 || d_s <= 20.0 {
                ns += 1;
            }
        }
        let n = bearings.len() as f64;
        bf.bearing_mean_deg = (sy.atan2(sx).to_degrees() + 360.0) % 360.0;
        bf.bearing_concentration = ((sx / n).powi(2) + (sy / n).powi(2)).sqrt();
        bf.ns_fraction = ns as f64 / n;
    }
    if geo_far > 0 {
        bf.equator_crossing_frac = equator_cross as f64 / geo_far as f64;
        bf.auroral_frac = auroral as f64 / geo_far as f64;
    }

    let (rate_short, rate_base, z, slope) = anomaly(&times, now, cfg);
    bf.rate_short = rate_short;
    bf.rate_base = rate_base;
    bf.anomaly_z = z;
    bf.onset_slope = slope;
    // Same per-minute unit as `rate_short` (short_w-sized bin), but over the
    // getting-out + far↔far spots only — the un-inflated cross-band denominator.
    bf.share_rate_short = share_short_count as f32 / (cfg.short_w.max(1) as f32 / 60.0);
    bf
}

/// Rule-ordered mode classifier. Returns `(mode, geom_fit, sw_fit)` where the two
/// fits are 0..1 confidence factors for the chosen mode (SNR is omitted in v1).
/// Order clears Es's signatures before claiming F2/TEP so a multi-hop Es 2nd lobe
/// (2800–4500 km, often during high SFI) is not mislabeled F2.
pub fn classify(
    bf: &BandFeatures,
    band: Band,
    wx: &SpaceWx,
    cfg: &OpeningConfig,
) -> (PropMode, f32, f32) {
    let kp = wx.kp;
    let sfi = wx.sfi;
    // Aurora can fire at Kp ≥ 5 when the operator's own paths are auroral-zone.
    let kp_au = if bf.auroral_frac >= 0.5 {
        cfg.kp_aurora - 1.0
    } else {
        cfg.kp_aurora
    };

    // TROPO — 2 m, geomagnetically quiet, continuous (no skip hole), 800–1600 km,
    // directional corridor. SW-flat. (70 cm "≥ 2 m" loss test is Phase 2.)
    if band == Band::B2
        && kp < 4.0
        && !bf.skip_hole
        && (800.0..=1600.0).contains(&bf.median_km)
        && bf.bearing_concentration > 0.5
    {
        let geom = clamp01(bf.bearing_concentration as f32);
        return (PropMode::Tropo, geom, 1.0); // SW-flat ⇒ always consistent
    }

    // AURORA — VHF, Kp-gated, far ends in the auroral zone (poleward scatter).
    // skip_hole discriminates: aurora is oval SCATTER (no skip zone), while Es has
    // one — a strong poleward Es opening during a Kp≥6 storm must classify Es, not
    // aurora (the Phase-2 SNR/decode-quality signature isn't available yet, so the
    // geometry signature carries the discrimination). auroral_frac 0.55: nearly
    // half the far ends sub-auroral is not an aurora picture.
    if matches!(band, Band::B6 | Band::B4 | Band::B2)
        && kp >= kp_au
        && bf.auroral_frac >= 0.55
        && bf.max_km <= 2200.0
        && !bf.skip_hole
    {
        let geom = clamp01(bf.auroral_frac as f32);
        let sw = clamp01((kp - kp_au + 1.0) / 3.0);
        return (PropMode::Aurora, geom, sw);
    }

    // F2 / TEP — HF…6 m, high SFI, geomagnetically quiet-ish, no skip hole, long
    // N–S / equator-crossing paths.
    if sfi >= cfg.sfi_tep
        && kp < 5.0
        && !bf.skip_hole
        && (bf.equator_crossing_frac > 0.3 || bf.ns_fraction > 0.5)
        && (2500.0..=6800.0).contains(&bf.median_km)
    {
        let geom = clamp01((bf.equator_crossing_frac.max(bf.ns_fraction)) as f32);
        let sw = clamp01((sfi - cfg.sfi_tep) / 80.0 + 0.4);
        return (PropMode::F2, geom, sw);
    }

    // Plain long-haul F2 — geometry-free, very long, no skip hole, high SFI. This
    // is the documented Es-vs-F2 hard boundary (a long single/multi-hop path with
    // no skip-hole and no equator-crossing geometry), so it is held to low
    // confidence (≤ Marginal) — an honest "long-haul F2, low certainty", not a
    // confident classification.
    if bf.max_km > 4500.0 && !bf.skip_hole && sfi >= cfg.sfi_tep {
        return (PropMode::F2, 0.25, 0.4);
    }

    // SPORADIC-E — 10/6/4/2 m, a skip hole OR isotropic-without-equator-crossing,
    // single/multi-hop distances. SW-independent.
    if matches!(band, Band::B10 | Band::B6 | Band::B4 | Band::B2)
        && (640.0..=4500.0).contains(&bf.median_km)
        && (bf.skip_hole || (bf.bearing_concentration < 0.4 && bf.equator_crossing_frac < 0.2))
    {
        let geom = if bf.skip_hole {
            0.9
        } else {
            clamp01(0.8 - bf.bearing_concentration as f32)
        };
        return (PropMode::SporadicE, geom, 1.0);
    }

    (PropMode::Unknown, 0.3, 0.5)
}

/// Combine the confidence factors into one honest 0..1 score. SNR is structurally
/// absent in v1 so it contributes no factor. Tropo is capped at Marginal.
fn confidence_score(
    bf: &BandFeatures,
    mode: PropMode,
    geom: f32,
    sw: f32,
    cfg: &OpeningConfig,
) -> f32 {
    let conf_anom = clamp01(bf.anomaly_z / (2.0 * cfg.z_open));
    // Localization: a band-specific opening has a high cross-band share; a uniform
    // all-band surge (contest) drives shares down. Floor so a lone-active band
    // (share≈1) isn't penalised and a missing total doesn't zero it out.
    let conf_local = clamp01(0.3 + 0.7 * bf.cross_band_share);
    let factors = [conf_anom, clamp01(geom), clamp01(sw), conf_local];
    // Geometric mean of the present factors (reads more sensibly than a raw
    // product of four sub-unity terms).
    let prod: f32 = factors.iter().copied().map(|f| f.max(1e-4)).product();
    let mut score = prod.powf(1.0 / factors.len() as f32);
    if mode == PropMode::Tropo {
        score = score.min(0.5); // Tropo: never above Marginal (geometry-only v1)
    }
    clamp01(score).max(0.05)
}

/// Build a [`BandSignal`] from features + the live space weather.
pub fn classify_signal(bf: BandFeatures, wx: &SpaceWx, cfg: &OpeningConfig) -> BandSignal {
    let band = bf.band;
    let (mode, geom, sw) = classify(&bf, band, wx, cfg);
    let confidence = confidence_score(&bf, mode, geom, sw, cfg);
    let raw_open = bf.raw_open(cfg);
    let warm = bf.warm(cfg);
    BandSignal {
        band,
        features: bf,
        mode,
        confidence,
        raw_open,
        warm,
    }
}

/// Detect openings end-to-end with the real space weather: features → classify →
/// signal, for each band. (Wraps [`band_features`] + [`classify_signal`] so the
/// caller threads the actual `wx`.)
pub fn detect(
    spots: &[PathSpot],
    me_call: &str,
    me_grid: &str,
    now: i64,
    wx: &SpaceWx,
    cfg: &OpeningConfig,
    bands: &[Band],
) -> Vec<BandSignal> {
    let cutoff = now - cfg.base_w;
    let mut by_band: HashMap<Band, Vec<&PathSpot>> = HashMap::new();
    for s in spots.iter().filter(|s| s.time >= cutoff) {
        by_band.entry(s.band).or_default().push(s);
    }
    let mut feats: Vec<BandFeatures> = bands
        .iter()
        .map(|&b| {
            let empty = Vec::new();
            let bs = by_band.get(&b).unwrap_or(&empty);
            band_features(b, bs, me_call, me_grid, now, cfg)
        })
        .collect();
    // Cross-band localization share. Denominator = Σ `share_rate_short`
    // (getting-out + far↔far evidence), NOT Σ `rate_short`: on the operator-centric
    // feed the operator's own IHeard receive-firehose on a busy band (e.g. a 20 m FT8
    // run) would otherwise inflate the denominator and dilute a genuine single-band
    // (6 m Es) opening below the regional cross-band gate. It stays a RELATIVE share,
    // so a uniform multi-band contest surge still drives every band's share down —
    // contest rejection is preserved (no threshold relaxed). See `share_rate_short`.
    let total_share: f32 = feats.iter().map(|f| f.share_rate_short).sum();
    for f in &mut feats {
        f.cross_band_share = if total_share > 0.0 {
            f.share_rate_short / total_share
        } else {
            0.0
        };
    }
    feats
        .into_iter()
        .map(|bf| classify_signal(bf, wx, cfg))
        .collect()
}

// --- The anti-flap state machine -------------------------------------------

#[derive(Debug, Clone)]
struct OpeningState {
    open: bool,
    open_windows: u32,
    closed_windows: u32,
    onset_time: i64,
    /// False when the opening was seeded (already live at startup / within the
    /// grace window) — its true onset is unknown, so `onset_secs` is reported 0.
    onset_known: bool,
    mode: PropMode,
}

/// A surfaced opening event from the tracker.
#[derive(Debug, Clone)]
pub struct OpeningEvent {
    pub band: Band,
    pub open: bool,
    /// True only on the update where a genuine closed→open transition occurred
    /// (never on cold-start/grace seeding) — drives the one-shot alert.
    pub is_new: bool,
    /// Seconds since onset, or 0 for a seeded opening (true onset unknown).
    pub onset_secs: i64,
    pub mode: PropMode,
    pub confidence: f32,
    pub features: BandFeatures,
}

/// Stateful hysteresis + startup seeding over successive [`detect`] passes.
/// The ONLY stateful piece; `update` is pure given its state + `now`.
///
/// **Startup grace:** for the first `base_w` after the tracker's first update
/// (the time the buffer/baseline needs to become representative — a partial
/// buffer makes z unreliable and everything look "open"), an opening is *seeded*
/// into the open state WITHOUT `is_new`/alert. Genuine closed→open transitions
/// observed AFTER the grace window fire `is_new` exactly once. This is a
/// time-based grace (not a global flag), so a band that comes alive mid-session
/// after grace still alerts, while a pre-existing opening that takes a few polls
/// to register at startup does NOT false-alert.
pub struct OpeningTracker {
    cfg: OpeningConfig,
    states: HashMap<Band, OpeningState>,
    start_time: Option<i64>,
}

impl Default for OpeningTracker {
    fn default() -> Self {
        Self::new(OpeningConfig::default())
    }
}

impl OpeningTracker {
    pub fn new(cfg: OpeningConfig) -> Self {
        Self {
            cfg,
            states: HashMap::new(),
            start_time: None,
        }
    }

    fn min_dwell(mode: PropMode) -> i64 {
        match mode {
            PropMode::Tropo => 1800,
            PropMode::F2 => 900,
            PropMode::Aurora => 600,
            _ => 600, // Es / MeteorScatter / Unknown
        }
    }

    /// Advance the state machine one window and return events for all OPEN bands.
    pub fn update(&mut self, now: i64, signals: &[BandSignal]) -> Vec<OpeningEvent> {
        let cfg = self.cfg.clone();
        let start = *self.start_time.get_or_insert(now);
        // Within the grace window, z is not yet trustworthy (partial baseline), so
        // openings are seeded silently rather than alerted.
        let in_grace = now < start + cfg.base_w;

        // Union of bands seen this window and bands we already track (so an
        // open band that vanishes from the feed still ages toward exit).
        let mut by_band: HashMap<Band, &BandSignal> = HashMap::new();
        for s in signals {
            by_band.insert(s.band, s);
        }
        let mut bands: HashSet<Band> = by_band.keys().copied().collect();
        bands.extend(self.states.keys().copied());

        let mut events = Vec::new();
        for band in bands {
            let sig = by_band.get(&band);
            let raw_open = sig.map(|s| s.raw_open).unwrap_or(false);
            let warm = sig.map(|s| s.warm).unwrap_or(false);
            let mode = sig.map(|s| s.mode).unwrap_or(PropMode::Unknown);
            let confidence = sig.map(|s| s.confidence).unwrap_or(0.0);
            let features = sig
                .map(|s| s.features.clone())
                .unwrap_or_else(|| BandFeatures::empty(band));

            let st = self.states.entry(band).or_insert(OpeningState {
                open: false,
                open_windows: 0,
                closed_windows: 0,
                onset_time: now,
                onset_known: false,
                mode,
            });

            // Update the consecutive-window counters.
            if raw_open {
                st.open_windows = st.open_windows.saturating_add(1);
                st.closed_windows = 0;
            } else if warm {
                // Hold: neither opening nor closing.
                st.closed_windows = 0;
            } else {
                st.closed_windows = st.closed_windows.saturating_add(1);
                st.open_windows = 0;
            }

            let mut is_new = false;
            if !st.open {
                if in_grace && raw_open {
                    // Seed a pre-existing opening silently (onset unknown).
                    st.open = true;
                    st.onset_time = now;
                    st.onset_known = false;
                    st.mode = mode;
                } else if !in_grace && st.open_windows >= cfg.enter_windows {
                    // Genuine in-session onset → alert once.
                    st.open = true;
                    st.onset_time = now;
                    st.onset_known = true;
                    st.mode = mode;
                    is_new = true;
                }
            } else {
                // Already open — close after the exit count AND min dwell, or once
                // the hard max-dwell backstop is hit (so a perpetually-"warm" band
                // can't pin the latch forever).
                let age = now - st.onset_time;
                let dwell_ok = age >= Self::min_dwell(st.mode);
                let max_hit = age >= cfg.max_dwell_secs;
                if (st.closed_windows >= cfg.exit_windows && dwell_ok) || max_hit {
                    st.open = false;
                    st.open_windows = 0;
                    st.closed_windows = 0;
                }
            }

            if st.open {
                events.push(OpeningEvent {
                    band,
                    open: true,
                    is_new,
                    onset_secs: if st.onset_known {
                        (now - st.onset_time).max(0)
                    } else {
                        0
                    },
                    mode: if mode == PropMode::Unknown {
                        st.mode
                    } else {
                        mode
                    },
                    confidence,
                    features,
                });
            }
        }

        events.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;
    const ME: &str = "KD9TAW";
    const ME_GRID: &str = "EN52";

    #[test]
    fn vhf_gate_opens_on_few_stations_where_hf_would_not() {
        // A modest real 6m Es burst: anomaly is up, and the operator has heard
        // just 2 far stations (typical of a fresh opening). On 6m this must OPEN
        // (loosened VHF gate); the identical evidence on 20m must NOT (HF needs 3
        // tx / 5 rx). This is the fix for "I see 6m open but get no alert."
        let cfg = OpeningConfig::default();
        let mut six = BandFeatures::empty(Band::B6);
        six.anomaly_z = cfg.z_open + 1.0;
        six.unique_far_tx = 2; // I heard 2 far stations on 6m
        assert!(
            six.raw_open(&cfg),
            "6m should open on 2 far stations during an anomaly"
        );

        let mut twenty = BandFeatures::empty(Band::B20);
        twenty.anomaly_z = cfg.z_open + 1.0;
        twenty.unique_far_tx = 2; // same evidence on HF
        assert!(
            !twenty.raw_open(&cfg),
            "20m must NOT open on only 2 far stations (HF needs the full gate)"
        );
    }

    fn heard_me(far: &str, fg: &str, band: Band, dt: i64) -> PathSpot {
        PathSpot {
            time: NOW - dt,
            tx_call: ME.into(),
            tx_grid: Some(ME_GRID.into()),
            rx_call: far.into(),
            rx_grid: Some(fg.into()),
            band,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        }
    }
    fn i_heard(far: &str, fg: &str, band: Band, dt: i64) -> PathSpot {
        PathSpot {
            time: NOW - dt,
            tx_call: far.into(),
            tx_grid: Some(fg.into()),
            rx_call: ME.into(),
            rx_grid: Some(ME_GRID.into()),
            band,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        }
    }

    // ---- reciprocity -------------------------------------------------------
    #[test]
    fn reciprocity_is_operator_anchored_by_far_call() {
        let spots = vec![
            heard_me("DL1AAA", "JN58", Band::B20, 10), // DL1AAA heard me
            i_heard("DL1AAA", "JN58", Band::B20, 12),  // I heard DL1AAA  → reciprocal
            i_heard("DL2BBB", "JN58", Band::B20, 14),  // one-way only
            heard_me("DL3CCC", "JN58", Band::B20, 16), // one-way only
        ];
        // Only DL1AAA is two-way.
        assert_eq!(reciprocity(&spots, ME, NOW, 5400), 1);
    }

    #[test]
    fn reciprocity_distinguishes_same_grid_stations() {
        // Two stations in the SAME grid, each two-way → two reciprocal pairs.
        let spots = vec![
            heard_me("W1AAA", "FN42", Band::B6, 10),
            i_heard("W1AAA", "FN42", Band::B6, 11),
            heard_me("W1BBB", "FN42", Band::B6, 12),
            i_heard("W1BBB", "FN42", Band::B6, 13),
        ];
        assert_eq!(reciprocity(&spots, ME, NOW, 5400), 2);
    }

    // ---- features ----------------------------------------------------------
    #[test]
    fn or_gate_fires_on_one_directional_opening() {
        // Many stations hear ME, I hear none — a one-directional opening that an
        // AND gate would miss. Cluster them in the most-recent bin for a spike.
        let mut spots = Vec::new();
        let grids = ["FN42", "FN31", "FM18", "EM73", "EL96", "FN20", "EN61"];
        for (i, g) in grids.iter().enumerate() {
            spots.push(heard_me(&format!("W{i}XX"), g, Band::B6, (i as i64) * 5));
        }
        let cfg = OpeningConfig::default();
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B6, &bs, ME, ME_GRID, NOW, &cfg);
        assert_eq!(bf.unique_far_tx, 0, "I heard nobody");
        assert!(bf.unique_far_rx >= cfg.min_far_rx, "many heard me");
        assert!(
            bf.unique_far_rx >= cfg.min_far_rx || bf.unique_far_tx >= cfg.min_far_tx,
            "OR gate side satisfied"
        );
    }

    #[test]
    fn band_features_computes_snr_median_and_variance_from_payload() {
        let cfg = OpeningConfig::default();
        let snrs = [-20.0f32, -10.0, 0.0]; // sorted median -10; mean -10; pop var 66.67
        let mut spots = Vec::new();
        for (i, &snr) in snrs.iter().enumerate() {
            let mut s = heard_me(&format!("W{i}SN"), "FN42", Band::B20, (i as i64) * 5);
            s.snr = Some(snr);
            spots.push(s);
        }
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B20, &bs, ME, ME_GRID, NOW, &cfg);
        assert_eq!(bf.median_snr, Some(-10.0));
        let var = bf.snr_var.expect("variance present");
        assert!(
            (var - 66.667).abs() < 0.1,
            "population variance ~66.67: {var}"
        );
    }

    #[test]
    fn band_features_leaves_snr_none_without_payload_snrs() {
        let cfg = OpeningConfig::default();
        let spots = [heard_me("W0XX", "FN42", Band::B20, 5)]; // snr None (topic-only)
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B20, &bs, ME, ME_GRID, NOW, &cfg);
        assert_eq!(bf.median_snr, None);
        assert_eq!(bf.snr_var, None);
    }

    #[test]
    fn anomaly_z_spikes_on_a_burst_against_a_quiet_baseline() {
        let cfg = OpeningConfig::default();
        // Quiet baseline: a trickle spread across the older bins; then a burst in
        // the most-recent 10 min.
        let mut spots = Vec::new();
        for k in 1..9 {
            // one spot per older bin (~quiet)
            spots.push(i_heard("W0BASE", "FN42", Band::B6, (k as i64) * 600 + 30));
        }
        for i in 0..20 {
            spots.push(heard_me(
                &format!("W{i}NOW"),
                "FN42",
                Band::B6,
                (i as i64) * 5,
            ));
        }
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B6, &bs, ME, ME_GRID, NOW, &cfg);
        assert!(
            bf.anomaly_z >= cfg.z_open,
            "burst z={} should exceed z_open",
            bf.anomaly_z
        );
        assert!(bf.onset_slope > 0.0, "rising onset");
    }

    #[test]
    fn skip_hole_detected_with_far_cluster_and_empty_inside() {
        let cfg = OpeningConfig::default();
        // Far grids ~1500 km, none inside the skip zone → a hole.
        let far = ["FN42", "FM18", "EL96", "EM73", "FN20", "FM07"];
        let spots: Vec<PathSpot> = far
            .iter()
            .enumerate()
            .map(|(i, g)| heard_me(&format!("W{i}",), g, Band::B6, (i as i64) * 5))
            .collect();
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B6, &bs, ME, ME_GRID, NOW, &cfg);
        assert!(bf.min_km > cfg.d_near_km, "all far (min {})", bf.min_km);
        assert!(bf.skip_hole, "should detect skip hole");
    }

    // ---- classifier --------------------------------------------------------
    fn feats(band: Band) -> BandFeatures {
        let mut f = BandFeatures::empty(band);
        f.anomaly_z = 5.0;
        f.cross_band_share = 0.8;
        f.unique_far_rx = 8;
        f.onset_slope = 1.0;
        f
    }

    #[test]
    fn classifies_sporadic_e_from_skip_hole() {
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B6);
        f.median_km = 1500.0;
        f.max_km = 1900.0;
        f.skip_hole = true;
        let calm = SpaceWx {
            sfi: 95.0,
            kp: 1.0,
            ..Default::default()
        };
        let (mode, _, _) = classify(&f, Band::B6, &calm, &cfg);
        assert_eq!(mode, PropMode::SporadicE);
    }

    #[test]
    fn multi_hop_es_2nd_lobe_not_mislabeled_f2_under_high_sfi() {
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B6);
        f.median_km = 3200.0; // 2nd-hop Es lobe
        f.max_km = 4200.0;
        f.skip_hole = true; // Es has a skip hole; F2 must not claim it
        f.equator_crossing_frac = 0.0;
        let high = SpaceWx {
            sfi: 180.0,
            kp: 2.0,
            ..Default::default()
        };
        let (mode, _, _) = classify(&f, Band::B6, &high, &cfg);
        assert_eq!(
            mode,
            PropMode::SporadicE,
            "skip-hole must keep it Es, not F2"
        );
    }

    #[test]
    fn classifies_f2_tep_from_equator_crossing_geometry() {
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B6);
        f.median_km = 5000.0;
        f.max_km = 6000.0;
        f.skip_hole = false;
        f.equator_crossing_frac = 0.7; // crosses the geomagnetic equator
        let high = SpaceWx {
            sfi: 175.0,
            kp: 2.0,
            ..Default::default()
        };
        let (mode, _, _) = classify(&f, Band::B6, &high, &cfg);
        assert_eq!(mode, PropMode::F2);
    }

    #[test]
    fn classifies_aurora_from_kp_and_geomag_latitude() {
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B6);
        f.median_km = 1400.0;
        f.max_km = 1700.0;
        f.auroral_frac = 0.8; // far ends in the auroral zone
        let storm = SpaceWx {
            sfi: 110.0,
            kp: 7.0,
            ..Default::default()
        };
        let (mode, _, _) = classify(&f, Band::B6, &storm, &cfg);
        assert_eq!(mode, PropMode::Aurora);
    }

    #[test]
    fn skip_hole_discriminates_es_from_aurora_during_a_storm() {
        // A poleward 6m opening WITH a skip zone during Kp 7 is sporadic-E, not
        // aurora — aurora is oval scatter and produces no skip hole. (The Phase-2
        // SNR/decode-quality signature isn't available; geometry carries this.)
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B6);
        f.median_km = 1400.0;
        f.max_km = 1700.0;
        f.auroral_frac = 0.8;
        f.skip_hole = true; // the Es signature
        let storm = SpaceWx {
            sfi: 110.0,
            kp: 7.0,
            ..Default::default()
        };
        let (mode, _, _) = classify(&f, Band::B6, &storm, &cfg);
        assert_ne!(
            mode,
            PropMode::Aurora,
            "skip hole ⇒ not aurora, got {mode:?}"
        );
    }

    #[test]
    fn classifies_tropo_on_2m_capped_marginal() {
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B2);
        f.median_km = 1100.0;
        f.max_km = 1400.0;
        f.skip_hole = false;
        f.bearing_concentration = 0.8; // a corridor
        let calm = SpaceWx {
            sfi: 100.0,
            kp: 1.0,
            ..Default::default()
        };
        let (mode, geom, sw) = classify(&f, Band::B2, &calm, &cfg);
        assert_eq!(mode, PropMode::Tropo);
        let score = confidence_score(&f, mode, geom, sw, &cfg);
        assert!(score <= 0.5, "tropo capped at Marginal: {score}");
    }

    #[test]
    fn meteor_scatter_is_not_surfaced() {
        // Short ground distance, low everything → not a sustained opening; the
        // classifier returns Unknown (MS is intentionally dropped in v2) and the
        // gate/tracker suppress it.
        let cfg = OpeningConfig::default();
        let mut f = feats(Band::B6);
        f.median_km = 300.0;
        f.max_km = 1000.0;
        let calm = SpaceWx {
            sfi: 100.0,
            kp: 1.0,
            ..Default::default()
        };
        let (mode, _, _) = classify(&f, Band::B6, &calm, &cfg);
        assert_ne!(mode, PropMode::MeteorScatter);
    }

    // ---- tracker -----------------------------------------------------------
    fn open_sig(band: Band, mode: PropMode) -> BandSignal {
        let mut f = BandFeatures::empty(band);
        f.anomaly_z = 6.0;
        f.unique_far_rx = 8;
        f.onset_slope = 1.0;
        BandSignal {
            band,
            features: f,
            mode,
            confidence: 0.8,
            raw_open: true,
            warm: true,
        }
    }
    fn closed_sig(band: Band) -> BandSignal {
        let f = BandFeatures::empty(band);
        BandSignal {
            band,
            features: f,
            mode: PropMode::Unknown,
            confidence: 0.0,
            raw_open: false,
            warm: false,
        }
    }

    // The startup grace window = base_w (default 7200s); genuine in-session
    // onsets must occur AFTER it to alert.
    const GRACE: i64 = 7200;

    #[test]
    fn tracker_requires_sustained_windows_then_flags_new_once() {
        let mut t = OpeningTracker::default();
        // First update at NOW starts the grace clock; stay quiet through grace.
        let e0 = t.update(NOW, &[closed_sig(Band::B6)]);
        assert!(e0.is_empty());
        t.update(NOW + GRACE / 2, &[closed_sig(Band::B6)]);
        let g = NOW + GRACE; // past the grace window → genuine onsets alert
                             // window 1 open (enter_windows=2 → not yet)
        let e1 = t.update(g, &[open_sig(Band::B6, PropMode::SporadicE)]);
        assert!(e1.is_empty(), "one window shouldn't open");
        // window 2 open → opens, is_new once
        let e2 = t.update(g + 600, &[open_sig(Band::B6, PropMode::SporadicE)]);
        assert_eq!(e2.len(), 1);
        assert!(e2[0].is_new, "genuine onset flags is_new");
        // window 3 still open → is_new false now
        let e3 = t.update(g + 1200, &[open_sig(Band::B6, PropMode::SporadicE)]);
        assert_eq!(e3.len(), 1);
        assert!(!e3[0].is_new, "no re-alert while still open");
        assert!(e3[0].onset_secs >= 600);
    }

    #[test]
    fn cold_start_seeds_open_band_without_alert() {
        let mut t = OpeningTracker::default();
        // The very first update sees an already-open band (within grace) → seeded
        // open, no is_new, and onset reported as unknown (0).
        let e = t.update(NOW, &[open_sig(Band::B6, PropMode::SporadicE)]);
        assert_eq!(e.len(), 1, "still reported as open");
        assert!(e[0].open);
        assert!(
            !e[0].is_new,
            "cold-start must NOT alert for a pre-existing opening"
        );
        assert_eq!(e[0].onset_secs, 0, "seeded onset is unknown");
    }

    #[test]
    fn onset_within_grace_does_not_alert() {
        let mut t = OpeningTracker::default();
        t.update(NOW, &[closed_sig(Band::B6)]);
        // Two consecutive raw-open windows but still INSIDE the grace window →
        // seeded silently, never is_new (avoids startup carpet-alerting).
        t.update(NOW + 600, &[open_sig(Band::B6, PropMode::SporadicE)]);
        let e = t.update(NOW + 1200, &[open_sig(Band::B6, PropMode::SporadicE)]);
        assert!(e[0].open, "seeded open during grace");
        assert!(!e[0].is_new, "no alert inside the grace window");
    }

    #[test]
    fn tracker_closes_after_exit_windows_and_dwell() {
        let mut t = OpeningTracker::default();
        let g = NOW + GRACE;
        t.update(NOW, &[closed_sig(Band::B6)]);
        t.update(g, &[open_sig(Band::B6, PropMode::SporadicE)]);
        let e = t.update(g + 600, &[open_sig(Band::B6, PropMode::SporadicE)]);
        assert!(e[0].open && e[0].is_new);
        // Now go cold for exit_windows(3) updates, past the Es min dwell (600s).
        t.update(g + 1200, &[closed_sig(Band::B6)]);
        t.update(g + 1800, &[closed_sig(Band::B6)]);
        let e_end = t.update(g + 2400, &[closed_sig(Band::B6)]);
        assert!(
            e_end.is_empty(),
            "should have closed after exit windows + dwell"
        );
    }

    #[test]
    fn open_band_vanishing_from_feed_still_closes() {
        let mut t = OpeningTracker::default();
        let g = NOW + GRACE;
        t.update(NOW, &[closed_sig(Band::B6)]);
        t.update(g, &[open_sig(Band::B6, PropMode::SporadicE)]);
        t.update(g + 600, &[open_sig(Band::B6, PropMode::SporadicE)]);
        // Band disappears entirely from later updates (no signal at all).
        t.update(g + 1200, &[]);
        t.update(g + 1800, &[]);
        let e = t.update(g + 2400, &[]);
        assert!(e.is_empty(), "vanished open band must age out to closed");
    }

    // ---- end-to-end detect -------------------------------------------------
    #[test]
    fn detect_flags_a_six_meter_burst_and_classifies() {
        let cfg = OpeningConfig::default();
        // Baseline trickle on 6m, then a wide burst (many far stations both ways).
        let grids = [
            "FN42", "FM18", "EL96", "EM73", "FN20", "FM07", "EN61", "FN31",
        ];
        let mut spots = Vec::new();
        for k in 1..9 {
            spots.push(i_heard("W0BASE", "FN42", Band::B6, (k as i64) * 600 + 30));
        }
        for (i, g) in grids.iter().enumerate() {
            spots.push(heard_me(&format!("W{i}A"), g, Band::B6, (i as i64) * 6));
            spots.push(i_heard(&format!("W{i}A"), g, Band::B6, (i as i64) * 6 + 2));
        }
        let calm = SpaceWx {
            sfi: 100.0,
            kp: 1.0,
            ..Default::default()
        };
        let sigs = detect(&spots, ME, ME_GRID, NOW, &calm, &cfg, &[Band::B6, Band::B2]);
        let six = sigs.iter().find(|s| s.band == Band::B6).unwrap();
        assert!(
            six.raw_open,
            "6m burst should be raw_open (z={})",
            six.features.anomaly_z
        );
        assert_eq!(six.mode, PropMode::SporadicE);
        assert!(six.features.reciprocal_pairs >= 5, "two-way paths counted");
        let two = sigs.iter().find(|s| s.band == Band::B2).unwrap();
        assert!(!two.raw_open, "quiet 2m not open");
    }

    #[test]
    fn cross_band_share_denominator_excludes_own_band_ihear_firehose() {
        // The DENOMINATOR-dilution bug: a genuine single-band 6 m opening (the
        // operator getting out to many far stations) is drowned in the cross-band
        // share because a busy 20 m FT8 run floods the operator-centric feed with
        // own-call IHeard decodes. The fix computes the share over getting-out
        // (HeardMe) + far↔far evidence only, so the 6 m opening keeps a healthy
        // share while the OLD (all-sides) denominator would dilute it below the 0.3
        // regional gate.
        let cfg = OpeningConfig::default();
        let mut spots = Vec::new();
        // 6 m: a real getting-out burst — 10 distinct far stations hear ME
        // (HeardMe), all in the most-recent short window, empty 6 m baseline.
        for i in 0..10 {
            spots.push(heard_me(
                &format!("W{i}SIX"),
                "FN42",
                Band::B6,
                (i as i64) * 20,
            ));
        }
        // 20 m: a busy own-band QSO run — the operator DECODES 40 stations (the
        // IHeard receive-firehose, own-call traffic) and is heard back by only a few.
        for i in 0..40 {
            spots.push(i_heard(
                &format!("D{i}TW"),
                "JN58",
                Band::B20,
                (i as i64) * 10,
            ));
        }
        for i in 0..4 {
            spots.push(heard_me(
                &format!("D{i}HM"),
                "JN58",
                Band::B20,
                (i as i64) * 10,
            ));
        }
        let calm = SpaceWx {
            sfi: 100.0,
            kp: 1.0,
            ..Default::default()
        };
        let sigs = detect(
            &spots,
            ME,
            ME_GRID,
            NOW,
            &calm,
            &cfg,
            &[Band::B20, Band::B6],
        );
        let six_sig = sigs.iter().find(|s| s.band == Band::B6).unwrap();
        let six = &six_sig.features;
        let twenty = &sigs.iter().find(|s| s.band == Band::B20).unwrap().features;

        // OLD denominator (Σ rate_short over ALL sides) would dilute 6 m below the gate.
        let old_share = six.rate_short / (six.rate_short + twenty.rate_short);
        assert!(
            old_share < 0.3,
            "old all-sides share dilutes the 6 m opening: {old_share}"
        );
        // NEW denominator (getting-out + far↔far only): 6 m keeps a healthy share.
        assert!(
            six.cross_band_share >= 0.3,
            "6 m share survives the own-band firehose post-fix: {}",
            six.cross_band_share
        );
        assert!(six_sig.raw_open, "the genuine 6 m opening is still flagged");
    }

    #[test]
    fn cross_band_share_still_diluted_by_a_uniform_multi_band_surge() {
        // Contest / uniform-Es lift: equal getting-out on every band. The new
        // (getting-out) denominator is still a RELATIVE share, so no single band
        // clears the 0.3 regional cross-band gate — contest rejection is preserved.
        let cfg = OpeningConfig::default();
        let bands = [
            Band::B20,
            Band::B15,
            Band::B12,
            Band::B10,
            Band::B6,
            Band::B4,
            Band::B2,
        ];
        let mut spots = Vec::new();
        for &b in &bands {
            for i in 0..8 {
                spots.push(heard_me(&format!("W{i}X"), "FN42", b, (i as i64) * 20));
            }
        }
        let calm = SpaceWx {
            sfi: 100.0,
            kp: 1.0,
            ..Default::default()
        };
        let sigs = detect(&spots, ME, ME_GRID, NOW, &calm, &cfg, &bands);
        for s in &sigs {
            assert!(
                s.features.cross_band_share < 0.3,
                "{:?} share must stay diluted under a uniform surge: {}",
                s.band,
                s.features.cross_band_share
            );
        }
    }

    /// The regression that the prior single-window-slope gate would fail: a
    /// SUSTAINED (plateauing) opening, driven through the REAL pipeline
    /// (band_features → detect → classify_signal → tracker) across several polls,
    /// must stay open and fire `is_new` exactly once.
    #[test]
    fn sustained_plateau_opening_via_real_pipeline_fires_is_new_once() {
        let cfg = OpeningConfig::default();
        let onset = NOW + GRACE; // genuine onset AFTER the grace window
        let grids = [
            "FN42", "FM18", "EL96", "EM73", "FN20", "FM07", "EN61", "FN31", "DM79", "EM73", "FN30",
            "FM19",
        ];
        let mk = |far: &str, fg: &str, t: i64, me_tx: bool| PathSpot {
            time: t,
            tx_call: if me_tx { ME.into() } else { far.into() },
            tx_grid: Some(if me_tx { ME_GRID.into() } else { fg.into() }),
            rx_call: if me_tx { far.into() } else { ME.into() },
            rx_grid: Some(if me_tx { fg.into() } else { ME_GRID.into() }),
            band: Band::B6,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        let mut spots = Vec::new();
        // Quiet baseline: a trickle across the 2 h before onset.
        let mut tb = NOW + 600;
        while tb < onset {
            spots.push(mk("W0BASE", "FN42", tb, false));
            tb += 600;
        }
        // Sustained opening: a fresh batch of distinct stations (both ways) every
        // ~3 min from onset onward — a steady high rate (a plateau, not a spike).
        let mut to = onset;
        while to <= onset + 2400 {
            for (i, g) in grids.iter().enumerate() {
                spots.push(mk(&format!("W{i}P"), g, to + (i as i64) * 5, false));
                spots.push(mk(&format!("W{i}P"), g, to + (i as i64) * 5 + 2, true));
            }
            to += 180;
        }
        let wx = SpaceWx {
            sfi: 100.0,
            kp: 1.0,
            ..Default::default()
        };
        let mut t = OpeningTracker::new(cfg.clone());
        // Start the grace clock at NOW (quiet).
        t.update(
            NOW,
            &detect(&spots, ME, ME_GRID, NOW, &wx, &cfg, &[Band::B6]),
        );

        let mut new_count = 0;
        let mut last_open = false;
        for p in [onset + 600, onset + 1200, onset + 1800, onset + 2400] {
            let sigs = detect(&spots, ME, ME_GRID, p, &wx, &cfg, &[Band::B6]);
            let six = sigs.iter().find(|s| s.band == Band::B6).unwrap();
            assert!(
                six.raw_open,
                "plateau must stay raw_open at poll {p} (z={})",
                six.features.anomaly_z
            );
            let evs = t.update(p, &sigs);
            if let Some(e) = evs.iter().find(|e| e.band == Band::B6) {
                last_open = e.open;
                if e.is_new {
                    new_count += 1;
                }
            }
        }
        assert!(
            last_open,
            "sustained opening should still be open after 40 min"
        );
        assert_eq!(new_count, 1, "is_new must fire exactly once for the onset");
    }

    // ---- Phase 2: near-region generalization -------------------------------
    fn far_far(tx: &str, txg: &str, rx: &str, rxg: &str, band: Band, dt: i64) -> PathSpot {
        PathSpot {
            time: NOW - dt,
            tx_call: tx.into(),
            tx_grid: Some(txg.into()),
            rx_call: rx.into(),
            rx_grid: Some(rxg.into()),
            band,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        }
    }

    #[test]
    fn reciprocity_regional_counts_far_far_pairs() {
        // A<->B both ways, NEITHER is the operator → 1 regional pair; the
        // operator-anchored reciprocity sees none.
        let spots = vec![
            far_far("DL1AAA", "JN58", "G3XYZ", "IO91", Band::B6, 10),
            far_far("G3XYZ", "IO91", "DL1AAA", "JN58", Band::B6, 12), // reciprocal
            far_far("F5ABC", "JN12", "DL1AAA", "JN58", Band::B6, 14), // one-way only
        ];
        assert_eq!(reciprocity_regional(&spots, NOW, 7200), 1);
        assert_eq!(reciprocity(&spots, ME, NOW, 7200), 0);
    }

    #[test]
    fn band_features_counts_neither_into_census_and_far_geometry() {
        let cfg = OpeningConfig::default();
        let grids = ["FN42", "EM12", "FM18", "DM79", "EN90", "FM07"];
        let spots: Vec<PathSpot> = grids
            .iter()
            .enumerate()
            .map(|(i, g)| {
                far_far(
                    &format!("A{i}"),
                    g,
                    &format!("B{i}"),
                    "FN31",
                    Band::B6,
                    (i as i64) * 5,
                )
            })
            .collect();
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B6, &bs, ME, ME_GRID, NOW, &cfg);
        assert_eq!(bf.unique_far_rx, 0);
        assert_eq!(bf.unique_far_tx, 0, "no operator-side spots");
        assert!(bf.unique_stations >= 6, "regional census counts both ends");
        assert!(
            bf.min_km > cfg.d_near_km,
            "single farther endpoint folded; none near"
        );
        assert!(bf.skip_hole, "a near-region Es burst drives skip_hole");
        assert!(
            (640.0..=4500.0).contains(&bf.median_km),
            "Es-window distance"
        );
    }

    #[test]
    fn neither_single_far_endpoint_does_not_suppress_skip_hole() {
        // Each Neither spot has a NEAR end (EN61 ~200 km) + a FAR end. Folding only
        // the FARTHER end means the near end never fills the sub-d_near bucket, so
        // skip_hole stays true (the bug a dual-endpoint fold would cause).
        let cfg = OpeningConfig::default();
        let far = ["FN42", "EM12", "FM18", "DM79", "EN90", "FM07"];
        let spots: Vec<PathSpot> = far
            .iter()
            .enumerate()
            .map(|(i, g)| {
                far_far(
                    &format!("N{i}"),
                    "EN61",
                    &format!("F{i}"),
                    g,
                    Band::B6,
                    (i as i64) * 5,
                )
            })
            .collect();
        let bs: Vec<&PathSpot> = spots.iter().collect();
        let bf = band_features(Band::B6, &bs, ME, ME_GRID, NOW, &cfg);
        assert!(bf.skip_hole, "near end must not be folded");
        assert!(
            bf.min_km > cfg.d_near_km,
            "no near sample despite a near end present"
        );
    }

    #[test]
    fn regional_gate_is_multi_condition_and_opt_in() {
        let mut cfg = OpeningConfig::default();
        let mut f = BandFeatures::empty(Band::B6);
        f.anomaly_z = 6.0;
        f.unique_stations = 15;
        f.unique_near_rx = 4; // a real collection of local endpoints
        f.reciprocal_pairs_regional = 4;
        f.cross_band_share = 0.7;
        // v1 default (regional_scope off): regional spots can't open (no op far counts).
        assert!(
            !f.raw_open(&cfg),
            "regional spots don't open under the v1 default"
        );
        cfg.regional_scope = true;
        assert!(f.raw_open(&cfg), "opens with the regional gate enabled");
        // One loud station heard by many: many stations, but no two-way pairs.
        let mut one_loud = f.clone();
        one_loud.reciprocal_pairs_regional = 0;
        assert!(
            !one_loud.raw_open(&cfg),
            "one-way (no reciprocity) must not open"
        );
        // Contest: every band up → low cross-band share.
        let mut contest = f.clone();
        contest.cross_band_share = 0.1;
        assert!(
            !contest.raw_open(&cfg),
            "uniform multi-band surge must not open"
        );
        // SUPERSTATION: one tall-tower receiver hearing 14 DX — plenty of
        // "stations", but a single local endpoint. Must NOT open.
        let mut superstation = f.clone();
        superstation.unique_near_rx = 1;
        assert!(
            !superstation.raw_open(&cfg),
            "one big local receiver must not fabricate a regional opening"
        );
    }
}
