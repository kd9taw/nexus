//! Geography-based Maidenhead grid-square rarity.
//!
//! A bundled 32,400-entry table (one tier per 4-char grid) derived offline from
//! Natural Earth land polygons (public domain) by `scripts/gen-grid-rarity.mjs`:
//! land fraction per 2°×1° cell, sampled 8×8, with a tiny-islet prepass so an
//! atoll between sample points can never read as open water.
//!
//! Tiers (2 bits each). The DISPLAY tier is additionally refined by the
//! measured-activity census (see the census section below): sustained heard
//! activity demotes a grid one step; silence never promotes.
//! Geography tiers:
//! - `UltraRare` — open water: only rovers, maritime mobiles, or DXpeditions
//!   can ever activate it.
//! - `Rare` — almost no land (small island / coastal sliver).
//! - `Uncommon` — mostly water, or polar wilderness (|lat| ≥ 66.5°).
//! - `Common` — everything else.
//!
//! Packing (must match the generator): `index = lonIdx*180 + latIdx` where
//! `lonIdx = (F1−'A')*10 + (D1−'0')` along longitude and `latIdx` likewise
//! along latitude; byte `index >> 2`, tier at bit offset `(index & 3) * 2`.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

/// 8,100 bytes: 32,400 grids × 2 bits (Natural Earth-derived; see module doc).
static TABLE: &[u8; 8_100] = include_bytes!("../data/grid_rarity.bin");

/// How hard a grid square is to work — a property of the GRID, not the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridRarity {
    Common,
    Uncommon,
    Rare,
    UltraRare,
}

impl GridRarity {
    fn from_bits(b: u8) -> Self {
        match b & 3 {
            3 => GridRarity::UltraRare,
            2 => GridRarity::Rare,
            1 => GridRarity::Uncommon,
            _ => GridRarity::Common,
        }
    }
}

/// The rarity tier for a grid (any length ≥ 4 — truncated to the field+square).
/// `None` for anything that isn't a valid Maidenhead grid.
pub fn grid_rarity(grid: &str) -> Option<GridRarity> {
    Some(GridRarity::from_bits(tier_bits(grid)?))
}

/// The raw 0–3 tier for the injection seam (tempo-app takes a
/// `Fn(&str) -> Option<u8>` closure so no propagation type leaks into its DTOs).
pub fn tier_u8(grid: &str) -> Option<u8> {
    tier_bits(grid)
}

fn tier_bits(grid: &str) -> Option<u8> {
    let idx = grid_index(grid)? as usize;
    Some((TABLE[idx >> 2] >> ((idx & 3) * 2)) & 3)
}

// ---------------------------------------------------------------------------
// Measured-activity census — the honesty-bounded refinement layer.
//
// Geography is TRUTH, activity is EVIDENCE: a geography-Rare islet with a
// resident superstation on the air every day isn't rare in practice, so enough
// measured activity DEMOTES the displayed tier by exactly one step. Silence
// proves nothing (maybe nobody's listening), so activity can never PROMOTE —
// a quiet Common grid stays Common. Counts decay exponentially (30-day
// half-life) so a one-off DXpedition doesn't permanently un-rare a water grid.
// ---------------------------------------------------------------------------

/// Decayed heard events at which a grid counts as "measurably active" (≈ one
/// spot a day sustained over the rolling window).
const ACTIVE_THRESHOLD: f32 = 25.0;
/// Decay half-life (seconds): 30 days.
const HALF_LIFE_SECS: f64 = 30.0 * 86_400.0;
/// Counts below this are pruned on decay (bounds the map + the persisted file).
const PRUNE_BELOW: f32 = 0.25;

/// The per-grid heard-activity census (see the section comment above). Plain
/// data + serde so the shell can persist it as a small bounded JSON file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RarityCensus {
    /// Packed grid index (the same `lonIdx*180+latIdx` as the table) → decayed count.
    counts: HashMap<u16, f32>,
    /// When decay was last applied (unix secs; 0 = never).
    #[serde(default)]
    last_decay_unix: i64,
}

impl RarityCensus {
    /// Record one heard event for a grid (invalid grids are ignored).
    pub fn observe(&mut self, grid: &str) {
        if let Some(idx) = grid_index(grid) {
            *self.counts.entry(idx).or_insert(0.0) += 1.0;
        }
    }

    /// Apply exponential decay for the time elapsed since the last call and
    /// prune negligible entries. Call on a slow cadence (minutes–hours).
    pub fn decay(&mut self, now_unix: i64) {
        if self.last_decay_unix > 0 && now_unix > self.last_decay_unix {
            let dt = (now_unix - self.last_decay_unix) as f64;
            let factor = 0.5f64.powf(dt / HALF_LIFE_SECS) as f32;
            self.counts.retain(|_, c| {
                *c *= factor;
                *c >= PRUNE_BELOW
            });
        }
        self.last_decay_unix = now_unix;
    }

    /// The decayed heard count for a grid (0 when unknown/never heard).
    pub fn count(&self, grid: &str) -> f32 {
        grid_index(grid)
            .and_then(|i| self.counts.get(&i))
            .copied()
            .unwrap_or(0.0)
    }

    /// Number of tracked grids (bounded by the prune; ≤ 32,400 by construction).
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// The process-wide census consulted by `effective_*` (the stamp sites live
/// deep in the needs/map assembly — a shared instance beats threading it
/// through every call chain). The shell feeds it (`census()`) and persists it.
static CENSUS: LazyLock<RwLock<RarityCensus>> = LazyLock::new(Default::default);

/// The shared census (observe/decay/load/save through this).
pub fn census() -> &'static RwLock<RarityCensus> {
    &CENSUS
}

/// One step less rare — the only move activity is allowed to make.
fn demote(t: GridRarity) -> GridRarity {
    match t {
        GridRarity::UltraRare => GridRarity::Rare,
        GridRarity::Rare => GridRarity::Uncommon,
        GridRarity::Uncommon | GridRarity::Common => GridRarity::Common,
    }
}

/// The DISPLAY rarity: the geography tier, demoted one step when the census
/// shows sustained activity. Falls back to pure geography when the census is
/// unavailable (poisoned lock) — never fabricates.
pub fn effective_rarity(grid: &str) -> Option<GridRarity> {
    let geo = grid_rarity(grid)?;
    if geo == GridRarity::Common {
        return Some(geo); // nothing to demote — skip the lock
    }
    let active = CENSUS
        .read()
        .map(|c| c.count(grid) >= ACTIVE_THRESHOLD)
        .unwrap_or(false);
    Some(if active { demote(geo) } else { geo })
}

/// `effective_rarity` as the raw 0–3 tier (the tempo-app injection seam).
pub fn effective_tier_u8(grid: &str) -> Option<u8> {
    effective_rarity(grid).map(|t| t as u8)
}

/// The packed table index for a grid, shared by the reader and the census.
fn grid_index(grid: &str) -> Option<u16> {
    let b = grid.trim().as_bytes();
    if b.len() < 4 {
        return None;
    }
    let f1 = b[0].to_ascii_uppercase();
    let f2 = b[1].to_ascii_uppercase();
    let (d1, d2) = (b[2], b[3]);
    if !(b'A'..=b'R').contains(&f1)
        || !(b'A'..=b'R').contains(&f2)
        || !d1.is_ascii_digit()
        || !d2.is_ascii_digit()
    {
        return None;
    }
    let lon_idx = (f1 - b'A') as u16 * 10 + (d1 - b'0') as u16;
    let lat_idx = (f2 - b'A') as u16 * 10 + (d2 - b'0') as u16;
    Some(lon_idx * 180 + lat_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_geography_anchors() {
        // Land: Wisconsin, Connecticut, Bavaria.
        assert_eq!(grid_rarity("EN52"), Some(GridRarity::Common));
        assert_eq!(grid_rarity("FN31"), Some(GridRarity::Common));
        assert_eq!(grid_rarity("JN58"), Some(GridRarity::Common));
        // Open water: Gulf of Guinea origin, south Pacific corner — and RR73,
        // which really is an all-water Arctic grid north of Chukotka.
        assert_eq!(grid_rarity("JJ00"), Some(GridRarity::UltraRare));
        assert_eq!(grid_rarity("AA00"), Some(GridRarity::UltraRare));
        assert_eq!(grid_rarity("RR73"), Some(GridRarity::UltraRare));
    }

    #[test]
    fn six_char_grids_truncate_and_case_folds() {
        assert_eq!(grid_rarity("en52ab"), grid_rarity("EN52"));
        assert_eq!(grid_rarity(" jj00 "), Some(GridRarity::UltraRare));
    }

    #[test]
    fn invalid_grids_are_none() {
        for g in ["", "EN", "E5N2", "SS00", "EN5X", "1234"] {
            assert_eq!(grid_rarity(g), None, "{g}");
        }
    }

    #[test]
    fn every_index_decodes_in_bounds() {
        // Walk the whole space once — proves the packing math never panics and
        // only emits valid tiers.
        for f1 in b'A'..=b'R' {
            for d1 in b'0'..=b'9' {
                for f2 in b'A'..=b'R' {
                    for d2 in b'0'..=b'9' {
                        let g = String::from_utf8(vec![f1, f2, d1, d2]).unwrap();
                        assert!(grid_rarity(&g).is_some(), "{g}");
                    }
                }
            }
        }
    }

    #[test]
    fn census_counts_decay_and_prune() {
        let mut c = RarityCensus::default();
        c.decay(1_000_000); // first call only stamps
        for _ in 0..8 {
            c.observe("JJ00");
        }
        c.observe("not a grid"); // ignored
        assert_eq!(c.count("JJ00"), 8.0);
        assert_eq!(c.len(), 1);
        // One half-life → half the count; five more → pruned to empty.
        c.decay(1_000_000 + 30 * 86_400);
        assert!((c.count("jj00 ") - 4.0).abs() < 1e-3); // case/space-normalized
        c.decay(1_000_000 + 6 * 30 * 86_400);
        assert!(c.is_empty(), "negligible counts must prune");
    }

    #[test]
    fn activity_demotes_exactly_one_step_and_never_promotes() {
        // AA00 (open water) and EN52 (Wisconsin) are the anchor grids; other
        // tests only READ them (geography is pure), so writing census counts
        // here can't interfere.
        assert_eq!(grid_rarity("AA00"), Some(GridRarity::UltraRare));
        assert_eq!(grid_rarity("EN52"), Some(GridRarity::Common));
        {
            let mut c = census().write().unwrap();
            for _ in 0..30 {
                c.observe("AA00");
                c.observe("EN52");
            }
        }
        // Sustained activity: ultra shows one step down — never two.
        assert_eq!(effective_rarity("AA00"), Some(GridRarity::Rare));
        // Common NEVER promotes, no matter the silence or the activity.
        assert_eq!(effective_rarity("EN52"), Some(GridRarity::Common));
        // Geography truth is untouched.
        assert_eq!(grid_rarity("AA00"), Some(GridRarity::UltraRare));
    }

    #[test]
    fn quiet_grids_keep_their_geography_tier() {
        // AB01: water grid nobody observes in these tests.
        assert_eq!(grid_rarity("AB01"), Some(GridRarity::UltraRare));
        assert_eq!(effective_rarity("AB01"), Some(GridRarity::UltraRare));
    }

    #[test]
    fn the_ocean_is_mostly_ultra_rare() {
        // ~71% of Earth is water; the table must broadly reflect that.
        let mut ultra = 0u32;
        for f1 in b'A'..=b'R' {
            for d1 in b'0'..=b'9' {
                for f2 in b'A'..=b'R' {
                    for d2 in b'0'..=b'9' {
                        let g = String::from_utf8(vec![f1, f2, d1, d2]).unwrap();
                        if grid_rarity(&g) == Some(GridRarity::UltraRare) {
                            ultra += 1;
                        }
                    }
                }
            }
        }
        let frac = ultra as f64 / 32_400.0;
        assert!((0.45..0.80).contains(&frac), "ultra fraction {frac}");
    }
}
