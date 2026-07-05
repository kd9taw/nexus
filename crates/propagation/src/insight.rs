//! Plain-language predictive insights — the "MUF is building, 6m may open soon"
//! layer. Turns the modeled band states + space-weather trend + observed openings
//! into a small ranked list of dual-audience lines: a `plain` sentence any operator
//! gets, and a `technical` detail a seasoned chaser trusts. Pure logic over the
//! existing primitives (no new physics): the trend ([`crate::WxTrend`]), the NOAA
//! R-scale ([`crate::model::r_scale`]), the greyline terminator ([`solar_elevation_deg`]),
//! and Es season ([`is_es_season`]).

use serde::Serialize;

use crate::engine::OpeningView;
use crate::geo::solar_elevation_deg;
use crate::likelihood::is_es_season;
use crate::model::{r_scale, Band, SpaceWx};
use crate::solar_wind::SolarWind;
use crate::space_wx::{TrendDir, WxTrend};

/// How urgently/positively an insight reads (drives color + ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InsightLevel {
    /// An opportunity opening up (green).
    Good,
    /// Neutral context (dim).
    Info,
    /// Degrading conditions worth noting (amber).
    Caution,
    /// Active disruption now (red).
    Alert,
}

impl InsightLevel {
    /// Sort key — most prominent first: Alert, then Caution, then Good, then Info.
    fn rank(self) -> u8 {
        match self {
            InsightLevel::Alert => 0,
            InsightLevel::Caution => 1,
            InsightLevel::Good => 2,
            InsightLevel::Info => 3,
        }
    }
}

/// What an insight is about (drives the icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InsightKind {
    MufTrend,
    SolarFlux,
    Geomagnetic,
    Flare,
    Greyline,
    EsWatch,
    /// Which band sits at today's MUF ceiling + how close the next one is to opening.
    BandHeadroom,
    /// A fresh/accelerating opening — the "go now" moment.
    OpeningMomentum,
    /// Whether the strongest opening is two-way, or a one-way path to stop calling into.
    Reciprocity,
    /// Real-time solar wind (Bz south / fast stream) — the leading geomagnetic warning.
    SolarWind,
}

/// One predictive insight line.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Insight {
    pub kind: InsightKind,
    pub level: InsightLevel,
    /// Plain sentence for any operator.
    pub plain: String,
    /// The numbers/mechanism for a seasoned chaser.
    pub technical: String,
    /// The band this is about, if specific (lets the UI link/highlight it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
}

/// The band ladder used for "which band is the MUF at / next" — HF plus 6 m (the
/// F2-reachable top), but not 4 m / 2 m (Es/MS, not MUF-driven).
fn ladder() -> impl Iterator<Item = Band> {
    Band::ALL
        .iter()
        .copied()
        .filter(|b| !matches!(b, Band::B4 | Band::B2))
}

/// Highest ladder band whose center frequency is at or below the MUF (the band
/// sitting at the ceiling now).
fn band_at_or_below(muf_mhz: f32) -> Option<&'static str> {
    ladder()
        .filter(|b| b.center_mhz() as f32 <= muf_mhz)
        .last()
        .map(|b| b.label())
}

/// The next ladder band above the MUF (the one that "may follow" as MUF rises).
fn next_band_above(muf_mhz: f32) -> Option<&'static str> {
    ladder()
        .find(|b| b.center_mhz() as f32 > muf_mhz)
        .map(|b| b.label())
}

fn dir_suffix(dir: TrendDir) -> &'static str {
    match dir {
        TrendDir::Rising => " and rising",
        TrendDir::Falling => " and falling",
        TrendDir::Steady => "",
    }
}

/// Generate the ranked insight list (cap 6). `trend` is optional so an offline
/// snapshot (no history yet) still emits the threshold-only insights (flux, Kp,
/// flare, greyline, Es); the live command layer passes the real trend for the
/// "building/falling" lines.
pub fn generate_insights(
    now: i64,
    wx: &SpaceWx,
    trend: Option<&WxTrend>,
    _bands: &[crate::advisor::BandReport],
    openings: &[OpeningView],
    me: Option<(f64, f64)>,
    solar_wind: Option<&SolarWind>,
) -> Vec<Insight> {
    let mut out: Vec<Insight> = Vec::new();

    // 1. Flare / radio blackout (most urgent).
    let r = r_scale(wx.xray_long);
    if r >= 1 {
        out.push(Insight {
            kind: InsightKind::Flare,
            level: if r >= 3 {
                InsightLevel::Alert
            } else {
                InsightLevel::Caution
            },
            plain: format!(
                "Solar flare (R{r}) — daytime low-band HF may be absorbed on the sunlit side"
            ),
            technical: format!(
                "GOES X-ray {:.1e} W/m² ({}-class) → R{r} radio blackout; D-layer absorption ∝ 1/f²",
                wx.xray_long,
                wx.xray_class()
            ),
            band: Some("40m".to_string()),
        });
    }

    // 2. Geomagnetic (Kp).
    let kp_rising = trend.map(|t| t.kp.dir == TrendDir::Rising).unwrap_or(false);
    if wx.kp >= 5.0 {
        out.push(Insight {
            kind: InsightKind::Geomagnetic,
            level: InsightLevel::Alert,
            plain: format!(
                "Geomagnetic storm (Kp {:.0}{}) — polar paths fading, watch for aurora",
                wx.kp,
                if kp_rising { " and climbing" } else { "" }
            ),
            technical: format!(
                "Kp {:.1}, A {:.0} — auroral absorption on high-latitude HF; auroral VHF possible",
                wx.kp, wx.a_index
            ),
            band: None,
        });
    } else if wx.kp >= 4.0 || kp_rising {
        out.push(Insight {
            kind: InsightKind::Geomagnetic,
            level: InsightLevel::Caution,
            plain: "Unsettled geomagnetic field — high-latitude paths degraded".to_string(),
            technical: format!(
                "Kp {:.1}{}",
                wx.kp,
                trend
                    .map(|t| format!(", Δ{:+.1}/hr", t.kp.delta_per_hr))
                    .unwrap_or_default()
            ),
            band: None,
        });
    }

    // 2b. Solar wind — the LEADING geomagnetic warning. Kp/A lag real conditions by hours;
    // a southward IMF (Bz negative) or a fast stream tells the operator polar/high-latitude
    // paths are about to fade before Kp catches up.
    if let Some(sw) = solar_wind {
        if sw.bz_nt <= -5.0 {
            let strong = sw.bz_nt <= -10.0;
            out.push(Insight {
                kind: InsightKind::SolarWind,
                level: if strong {
                    InsightLevel::Alert
                } else {
                    InsightLevel::Caution
                },
                plain: format!(
                    "Solar wind turned stormy (magnetic field tilted south) — polar paths to EU/Asia will fade over the next 1–2 h{}",
                    if strong { "; watch for aurora on 6m/2m" } else { "" }
                ),
                technical: format!(
                    "IMF Bz {:.1} nT south, Bt {:.1} nT, wind {:.0} km/s (DSCOVR real-time — leads Kp)",
                    sw.bz_nt, sw.bt_nt, sw.speed_kms
                ),
                band: None,
            });
        } else if sw.speed_kms >= 600.0 {
            out.push(Insight {
                kind: InsightKind::SolarWind,
                level: InsightLevel::Caution,
                plain: "Fast solar-wind stream arriving — high-latitude paths may get unsettled in the next few hours".to_string(),
                technical: format!(
                    "solar wind {:.0} km/s, Bz {:.1} nT (DSCOVR real-time — leads Kp)",
                    sw.speed_kms, sw.bz_nt
                ),
                band: None,
            });
        }
    }

    // 3. MUF trend ("building" / "falling").
    if let Some(t) = trend {
        if t.muf.dir == TrendDir::Rising && t.muf.now > 0.0 {
            // Use the REAL oldest in-window value (not a slope extrapolation over the
            // nominal window, which overstates the swing until the buffer fills).
            let from = t.muf.start.max(0.0);
            let at = band_at_or_below(t.muf.now).unwrap_or("the low bands");
            let next = next_band_above(t.muf.now);
            out.push(Insight {
                kind: InsightKind::MufTrend,
                level: InsightLevel::Good,
                plain: format!(
                    "MUF building ({:.0}→{:.0} MHz) — {} strengthening{}",
                    from,
                    t.muf.now,
                    at,
                    next.map(|b| format!(", {b} may follow"))
                        .unwrap_or_default()
                ),
                technical: format!(
                    "controlling MUF {:.1} MHz, +{:.1} MHz/hr",
                    t.muf.now, t.muf.delta_per_hr
                ),
                band: next.map(|b| b.to_string()),
            });
        } else if t.muf.dir == TrendDir::Falling && t.muf.now > 0.0 {
            out.push(Insight {
                kind: InsightKind::MufTrend,
                level: InsightLevel::Caution,
                plain: format!(
                    "Bands closing from the top (MUF {:.0} MHz, falling) — work the high bands now",
                    t.muf.now
                ),
                technical: format!(
                    "controlling MUF {:.1} MHz, {:.1} MHz/hr",
                    t.muf.now, t.muf.delta_per_hr
                ),
                band: band_at_or_below(t.muf.now).map(|b| b.to_string()),
            });
        }
    }

    // 3b. Band headroom — the persistent "what's my top open band, and what's about to
    // open next" readout. Only on a STEADY MUF: when the MUF is moving, 3 above already
    // tells that story (avoids two MUF lines competing for the 6-line cap).
    if let Some(t) = trend {
        let muf = t.muf.now;
        if muf > 0.0 && t.muf.dir == TrendDir::Steady {
            let at = ladder().filter(|b| b.center_mhz() as f32 <= muf).last();
            let next = ladder().find(|b| b.center_mhz() as f32 > muf);
            match (at, next) {
                (Some(at), Some(nb)) => {
                    let gap = nb.center_mhz() as f32 - muf;
                    let close = gap <= 3.0;
                    out.push(Insight {
                        kind: InsightKind::BandHeadroom,
                        level: if close {
                            InsightLevel::Good
                        } else {
                            InsightLevel::Info
                        },
                        plain: if close {
                            format!(
                                "{} is your top open band — and {} is on the edge of opening; check it shortly",
                                at.label(),
                                nb.label()
                            )
                        } else {
                            format!(
                                "{} is your top open band right now; {} needs the bands to lift before it opens",
                                at.label(),
                                nb.label()
                            )
                        },
                        technical: format!(
                            "controlling MUF {:.1} MHz; next band {} ({:.1} MHz) is {:.1} MHz above the ceiling",
                            muf,
                            nb.label(),
                            nb.center_mhz(),
                            gap
                        ),
                        band: Some(if close { nb.label() } else { at.label() }.to_string()),
                    });
                }
                (Some(at), None) => {
                    out.push(Insight {
                        kind: InsightKind::BandHeadroom,
                        level: InsightLevel::Good,
                        plain: format!(
                            "The bands are open all the way up to {} — take your pick up high",
                            at.label()
                        ),
                        technical: format!(
                            "controlling MUF {muf:.1} MHz (above the {} ceiling)",
                            at.label()
                        ),
                        band: Some(at.label().to_string()),
                    });
                }
                _ => {}
            }
        }
    }

    // 4. Solar flux level (+ trend suffix when known).
    let sfi_dir = trend.map(|t| t.sfi.dir).unwrap_or(TrendDir::Steady);
    let flux = if wx.sfi >= 150.0 {
        Some((
            InsightLevel::Good,
            format!(
                "SFI {:.0}{} — 10m/12m open for DX",
                wx.sfi,
                dir_suffix(sfi_dir)
            ),
        ))
    } else if wx.sfi >= 100.0 {
        Some((
            InsightLevel::Info,
            format!(
                "SFI {:.0}{} — solid 20–15m, upper bands marginal",
                wx.sfi,
                dir_suffix(sfi_dir)
            ),
        ))
    } else if wx.sfi < 80.0 {
        Some((
            InsightLevel::Caution,
            format!(
                "Low SFI {:.0}{} — high bands weak, favour 40/30/20m",
                wx.sfi,
                dir_suffix(sfi_dir)
            ),
        ))
    } else {
        None
    };
    if let Some((level, plain)) = flux {
        out.push(Insight {
            kind: InsightKind::SolarFlux,
            level,
            plain,
            technical: format!(
                "10.7 cm flux {:.0}{}",
                wx.sfi,
                trend
                    .map(|t| format!(", Δ{:+.1}/hr", t.sfi.delta_per_hr))
                    .unwrap_or_default()
            ),
            band: None,
        });
    }

    // 5. Greyline (operator terminator crossing).
    if let Some(me) = me {
        let elev = solar_elevation_deg(me.0, me.1, now);
        if elev.abs() < 6.0 {
            out.push(Insight {
                kind: InsightKind::Greyline,
                level: InsightLevel::Good,
                plain: "Greyline now — 80/40m long-path enhancement for ~30–60 min".to_string(),
                technical: format!("operator terminator crossing (sun elevation {elev:.1}°)"),
                band: Some("40m".to_string()),
            });
        }
    }

    // 6. 6 m Es watch — suppressed if a live 6 m opening already exists (it's its own
    // card / the advisor already lit it).
    let six_open = openings.iter().any(|o| o.band == "6m");
    if is_es_season(now) && !six_open {
        out.push(Insight {
            kind: InsightKind::EsWatch,
            level: InsightLevel::Info,
            plain: "6m: watch 50.313 for sudden DX (sporadic-E season)".to_string(),
            technical:
                "boreal Es season (solar declination > 15°); Es is minutes-long, 500–2500 km"
                    .to_string(),
            band: Some("6m".to_string()),
        });
    }

    // 7. Opening momentum — a FRESH opening is a "go now" moment the static openings card
    // doesn't convey. Newest strong opening only (avoid feed spam); two-way status folded in.
    if let Some(o) = openings
        .iter()
        .filter(|o| o.is_new || o.onset_secs < 600)
        .max_by(|a, b| {
            a.confidence_score
                .partial_cmp(&b.confidence_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        let two_way = o.reciprocal_pairs > 0;
        let s = if o.stations == 1 { "" } else { "s" };
        let plain = if o.is_new {
            format!(
                "{} just opened toward your {} — jump on it now{}",
                o.band,
                o.octant,
                if two_way {
                    "; stations there can hear you back"
                } else {
                    ""
                }
            )
        } else {
            format!(
                "{} to your {} is building — {} station{} in the last few min{}",
                o.band,
                o.octant,
                o.stations,
                s,
                if two_way { ", and it's two-way" } else { "" }
            )
        };
        out.push(Insight {
            kind: InsightKind::OpeningMomentum,
            level: InsightLevel::Good,
            plain,
            technical: format!(
                "onset {} s, anomaly z {:.1}, {} station{}, {} reciprocal pair{}",
                o.onset_secs,
                o.anomaly_z,
                o.stations,
                s,
                o.reciprocal_pairs,
                if o.reciprocal_pairs == 1 { "" } else { "s" }
            ),
            band: Some(o.band.clone()),
        });
    }

    // 8. Reciprocity — warn when an ESTABLISHED opening is one-way (nobody's confirmed to
    // hear the operator): change something, don't keep calling into a dead path. A fresh
    // opening is covered by 7 above; a two-way opening needs no warning.
    if let Some(o) = openings
        .iter()
        .filter(|o| !o.is_new && o.onset_secs >= 600 && o.stations > 0 && o.reciprocal_pairs == 0)
        .max_by_key(|o| o.stations)
    {
        let s = if o.stations == 1 { "" } else { "s" };
        out.push(Insight {
            kind: InsightKind::Reciprocity,
            level: InsightLevel::Caution,
            plain: format!(
                "{} to your {} looks one-way — you're hearing them, but no one's confirmed hearing you. Try more power or another band.",
                o.band, o.octant
            ),
            technical: format!(
                "{} station{} heard, 0 confirmed reciprocal (my↔far) over the window — likely one-way now",
                o.stations, s
            ),
            band: Some(o.band.clone()),
        });
    }

    // Rank most-prominent first, cap the feed.
    out.sort_by_key(|i| i.level.rank());
    out.truncate(6);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space_wx::{ScalarTrend, WxTrend};

    const NOW: i64 = 1_700_000_000; // Nov — not Es season

    fn wx(sfi: f32, kp: f32, xray: f32) -> SpaceWx {
        SpaceWx {
            sfi,
            ssn: None,
            kp,
            a_index: 8.0,
            xray_long: xray,
        }
    }

    fn rising_muf(now_mhz: f32, per_hr: f32) -> WxTrend {
        WxTrend {
            muf: ScalarTrend {
                now: now_mhz,
                start: now_mhz - per_hr * 3.0,
                delta_per_hr: per_hr,
                dir: TrendDir::Rising,
            },
            window_secs: 3 * 3600,
            samples: 3,
            ..Default::default()
        }
    }

    #[test]
    fn muf_building_names_the_next_band() {
        // MUF at 25 MHz rising → 12m (24.9) at the ceiling, 10m (28.5) may follow.
        let t = rising_muf(25.0, 3.0);
        let ins = generate_insights(NOW, &wx(150.0, 2.0, 1e-7), Some(&t), &[], &[], None, None);
        let muf = ins
            .iter()
            .find(|i| i.kind == InsightKind::MufTrend)
            .unwrap();
        assert_eq!(muf.level, InsightLevel::Good);
        assert_eq!(muf.band.as_deref(), Some("10m"));
        assert!(muf.plain.contains("building") && muf.plain.contains("10m may follow"));
        assert!(!muf.technical.is_empty());
    }

    #[test]
    fn muf_building_to_6m_is_the_north_star() {
        // MUF ~29 MHz rising → 10m at ceiling, 6m may follow (the magic-band moment).
        let t = rising_muf(29.0, 2.0);
        let ins = generate_insights(NOW, &wx(190.0, 1.0, 1e-7), Some(&t), &[], &[], None, None);
        let muf = ins
            .iter()
            .find(|i| i.kind == InsightKind::MufTrend)
            .unwrap();
        assert_eq!(muf.band.as_deref(), Some("6m"));
    }

    #[test]
    fn m_and_x_flares_map_to_r_scale() {
        let m = generate_insights(NOW, &wx(140.0, 2.0, 2e-5), None, &[], &[], None, None);
        let mf = m.iter().find(|i| i.kind == InsightKind::Flare).unwrap();
        assert_eq!(mf.level, InsightLevel::Caution); // R1
        assert!(mf.plain.contains("R1"));
        let x = generate_insights(NOW, &wx(140.0, 2.0, 2e-4), None, &[], &[], None, None);
        let xf = x.iter().find(|i| i.kind == InsightKind::Flare).unwrap();
        assert_eq!(xf.level, InsightLevel::Alert); // R3
        assert!(xf.plain.contains("R3"));
    }

    #[test]
    fn storm_kp_raises_an_alert() {
        let ins = generate_insights(NOW, &wx(140.0, 6.0, 1e-7), None, &[], &[], None, None);
        let g = ins
            .iter()
            .find(|i| i.kind == InsightKind::Geomagnetic)
            .unwrap();
        assert_eq!(g.level, InsightLevel::Alert);
        assert!(g.plain.contains("aurora"));
    }

    #[test]
    fn es_watch_suppressed_when_6m_already_open() {
        const JUNE: i64 = 1_687_000_000; // Es season
        let none = generate_insights(JUNE, &wx(120.0, 2.0, 1e-7), None, &[], &[], None, None);
        assert!(none.iter().any(|i| i.kind == InsightKind::EsWatch));
        let open = OpeningView {
            band: "6m".to_string(),
            mode: "Es".to_string(),
            octant: "W".to_string(),
            bearing_deg: 270.0,
            max_km: 1500.0,
            probability: 0.8,
            stations: 9,
            confidence: "likely".to_string(),
            confidence_score: 0.8,
            reciprocal_pairs: 3,
            anomaly_z: 4.0,
            onset_secs: 0,
            is_new: false,
            note: String::new(),
        };
        let suppressed =
            generate_insights(JUNE, &wx(120.0, 2.0, 1e-7), None, &[], &[open], None, None);
        assert!(!suppressed.iter().any(|i| i.kind == InsightKind::EsWatch));
    }

    #[test]
    fn ranking_orders_alert_before_info() {
        // An X-flare (Alert) + benign SFI 120 (Info) — the alert must come first, and
        // both plain + technical are always populated.
        let ins = generate_insights(NOW, &wx(120.0, 2.0, 2e-4), None, &[], &[], None, None);
        assert!(ins.len() >= 2);
        assert_eq!(ins[0].level, InsightLevel::Alert);
        assert!(ins
            .iter()
            .all(|i| !i.plain.is_empty() && !i.technical.is_empty()));
    }

    fn steady_muf(now_mhz: f32) -> WxTrend {
        WxTrend {
            muf: ScalarTrend {
                now: now_mhz,
                start: now_mhz,
                delta_per_hr: 0.0,
                dir: TrendDir::Steady,
            },
            window_secs: 3 * 3600,
            samples: 3,
            ..Default::default()
        }
    }

    fn opening(
        band: &str,
        stations: u32,
        reciprocal_pairs: u32,
        onset_secs: i64,
        is_new: bool,
    ) -> OpeningView {
        OpeningView {
            band: band.to_string(),
            mode: "F2".to_string(),
            octant: "NE".to_string(),
            bearing_deg: 45.0,
            max_km: 8000.0,
            probability: 0.7,
            stations,
            confidence: "likely".to_string(),
            confidence_score: 0.7,
            reciprocal_pairs,
            anomaly_z: 3.0,
            onset_secs,
            is_new,
            note: String::new(),
        }
    }

    #[test]
    fn band_headroom_flags_the_next_band_to_open() {
        // Steady MUF 16 MHz → 20m is the ceiling; 17m (~18 MHz) is ~2 MHz off = on the edge.
        let t = steady_muf(16.0);
        let ins = generate_insights(NOW, &wx(120.0, 2.0, 1e-7), Some(&t), &[], &[], None, None);
        let h = ins
            .iter()
            .find(|i| i.kind == InsightKind::BandHeadroom)
            .expect("headroom insight on a steady MUF");
        assert_eq!(h.band.as_deref(), Some("17m"));
        assert!(h.plain.contains("20m") && h.plain.contains("17m"));
        assert!(!h.plain.is_empty() && !h.technical.is_empty());
    }

    #[test]
    fn fresh_opening_emits_go_now_momentum() {
        let o = opening("10m", 5, 2, 30, true);
        let ins = generate_insights(NOW, &wx(150.0, 2.0, 1e-7), None, &[], &[o], None, None);
        let m = ins
            .iter()
            .find(|i| i.kind == InsightKind::OpeningMomentum)
            .expect("momentum insight for a fresh opening");
        assert_eq!(m.level, InsightLevel::Good);
        assert!(m.plain.contains("just opened") && m.plain.contains("10m"));
        // A fresh two-way opening isn't a one-way warning.
        assert!(!ins.iter().any(|i| i.kind == InsightKind::Reciprocity));
    }

    #[test]
    fn established_one_way_opening_warns() {
        let o = opening("20m", 6, 0, 1800, false);
        let ins = generate_insights(NOW, &wx(150.0, 2.0, 1e-7), None, &[], &[o], None, None);
        let r = ins
            .iter()
            .find(|i| i.kind == InsightKind::Reciprocity)
            .expect("one-way warning for an established no-reciprocal opening");
        assert_eq!(r.level, InsightLevel::Caution);
        assert!(r.plain.contains("one-way"));
        // An established (not fresh) opening isn't a "go now" momentum line.
        assert!(!ins.iter().any(|i| i.kind == InsightKind::OpeningMomentum));
    }

    #[test]
    fn southward_bz_warns_polar_paths_and_calm_wind_is_quiet() {
        let sw = crate::solar_wind::SolarWind {
            bz_nt: -12.0,
            bt_nt: 14.0,
            speed_kms: 650.0,
            density: 6.0,
        };
        let ins = generate_insights(NOW, &wx(150.0, 2.0, 1e-7), None, &[], &[], None, Some(&sw));
        let s = ins
            .iter()
            .find(|i| i.kind == InsightKind::SolarWind)
            .expect("solar-wind warning on a strongly southward Bz");
        assert_eq!(s.level, InsightLevel::Alert); // Bz <= -10 nT
        assert!(s.plain.contains("stormy") && !s.technical.is_empty());

        // Northward Bz + slow wind = quiet → no solar-wind line.
        let calm = crate::solar_wind::SolarWind {
            bz_nt: 2.0,
            bt_nt: 5.0,
            speed_kms: 380.0,
            density: 4.0,
        };
        let none = generate_insights(
            NOW,
            &wx(150.0, 2.0, 1e-7),
            None,
            &[],
            &[],
            None,
            Some(&calm),
        );
        assert!(!none.iter().any(|i| i.kind == InsightKind::SolarWind));
    }
}
