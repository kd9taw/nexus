//! Geography-based Maidenhead grid-square rarity.
//!
//! A bundled 32,400-entry table (one tier per 4-char grid) derived offline from
//! Natural Earth land polygons (public domain) by `scripts/gen-grid-rarity.mjs`:
//! land fraction per 2°×1° cell, sampled 8×8, with a tiny-islet prepass so an
//! atoll between sample points can never read as open water.
//!
//! Tiers (2 bits each; the format has headroom for a future measured-activity
//! refinement — regenerate the table, the reader is unchanged):
//! - `UltraRare` — open water: only rovers, maritime mobiles, or DXpeditions
//!   can ever activate it.
//! - `Rare` — almost no land (small island / coastal sliver).
//! - `Uncommon` — mostly water, or polar wilderness (|lat| ≥ 66.5°).
//! - `Common` — everything else.
//!
//! Packing (must match the generator): `index = lonIdx*180 + latIdx` where
//! `lonIdx = (F1−'A')*10 + (D1−'0')` along longitude and `latIdx` likewise
//! along latitude; byte `index >> 2`, tier at bit offset `(index & 3) * 2`.

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
    let lon_idx = (f1 - b'A') as usize * 10 + (d1 - b'0') as usize;
    let lat_idx = (f2 - b'A') as usize * 10 + (d2 - b'0') as usize;
    let idx = lon_idx * 180 + lat_idx;
    Some((TABLE[idx >> 2] >> ((idx & 3) * 2)) & 3)
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
