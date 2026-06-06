//! DXCC entity resolver backed by the vendored AD1C **`cty.dat`** country file
//! (`data/cty.dat`, MIT-licensed — see `data/cty.dat_copyright.txt`).
//!
//! Resolves a callsign → DXCC entity name + a representative lat/lon. Used to
//! locate live DXpeditions and to bucket the operator's logged contacts into
//! worked entities for the "needs" model ([`crate::dxped::LogNeeds`]).
//!
//! The file is embedded with `include_str!` and parsed once behind a
//! [`OnceLock`], so resolution is offline and self-contained. Matching mirrors
//! standard DXCC practice: **exact-call overrides first** (cty.dat's `=CALL`
//! entries — e.g. `3Y0J`→Bouvet, which has no plain `3Y` prefix), then
//! **longest-prefix** after stripping a portable affix.
//!
//! NB cty.dat stores **West-positive longitude**; we negate it to the usual
//! East-positive convention the rest of the crate uses.

use std::collections::HashMap;
use std::sync::OnceLock;

/// A resolved DXCC entity with a representative location (entity centroid) and
/// CQ zone (for WAZ). The zone is the prefix's `(cq)` override when cty.dat gives
/// one (multi-zone entities like W/VE/UA), else the entity's default zone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DxccInfo {
    pub entity: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub cq_zone: u8,
}

struct Entity {
    name: String,
    lat: f64,
    lon: f64,
    /// Default CQ zone (cty.dat header field 2); 0 if unparsed.
    cq_zone: u8,
}

struct Resolver {
    entities: Vec<Entity>,
    /// Full uppercased call → (entity index, optional per-call CQ-zone override).
    exact: HashMap<String, (u32, Option<u8>)>,
    /// Prefix → (entity index, optional per-prefix CQ-zone override).
    prefixes: HashMap<String, (u32, Option<u8>)>,
}

static CTY: &str = include_str!("../data/cty.dat");
static RESOLVER: OnceLock<Resolver> = OnceLock::new();

fn resolver() -> &'static Resolver {
    RESOLVER.get_or_init(|| parse_cty(CTY))
}

/// Parse cty.dat: header line `Name:CQ:ITU:Cont:Lat:Lon:GMT:Pfx:` sets the
/// current entity (name + lat + negated lon); indented continuation lines hold
/// the comma-separated alias list, terminated by `;`. Aliases are plain
/// prefixes or `=exact` calls, optionally carrying `(cq)`/`[itu]` zone (and
/// other bracketed) annotations which we strip.
fn parse_cty(text: &str) -> Resolver {
    let mut entities: Vec<Entity> = Vec::new();
    let mut exact: HashMap<String, (u32, Option<u8>)> = HashMap::new();
    let mut prefixes: HashMap<String, (u32, Option<u8>)> = HashMap::new();
    let mut cur: Option<u32> = None;
    let mut buf = String::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let header = !matches!(line.as_bytes()[0], b' ' | b'\t');
        if header {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() < 8 {
                continue;
            }
            let name = parts[0].trim().to_string();
            let cq_zone = parts[1].trim().parse::<u8>().unwrap_or(0);
            let lat = parts[4].trim().parse::<f64>().unwrap_or(0.0);
            // cty.dat longitude is West-positive → negate to East-positive.
            let lon = -parts[5].trim().parse::<f64>().unwrap_or(0.0);
            entities.push(Entity {
                name,
                lat,
                lon,
                cq_zone,
            });
            cur = Some((entities.len() - 1) as u32);
            buf.clear();
        } else if let Some(idx) = cur {
            buf.push_str(line.trim());
            if let Some(semi) = buf.find(';') {
                let aliases = buf[..semi].to_string();
                for tok in aliases.split(',') {
                    let t = tok.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let (is_exact, body) = match t.strip_prefix('=') {
                        Some(s) => (true, s),
                        None => (false, t),
                    };
                    // Cut at the first annotation char: (cq) [itu] {cont} <lat/lon> ~tz~.
                    let cut = body.find(['(', '[', '{', '<', '~']).unwrap_or(body.len());
                    let key = body[..cut].trim().to_ascii_uppercase();
                    if key.is_empty() {
                        continue;
                    }
                    // Per-prefix CQ-zone override `(N)`, when present.
                    let zone = body.find('(').and_then(|p| {
                        body[p + 1..]
                            .find(')')
                            .and_then(|q| body[p + 1..p + 1 + q].trim().parse::<u8>().ok())
                    });
                    if is_exact {
                        exact.insert(key, (idx, zone));
                    } else {
                        prefixes.insert(key, (idx, zone));
                    }
                }
                buf.clear();
            }
        }
    }

    Resolver {
        entities,
        exact,
        prefixes,
    }
}

/// Strip a portable affix and pick the side that indicates the DXCC. A plain
/// operating suffix (`/P`, `/M`, `/QRP`, a digit, …) → the base call; otherwise
/// the location side is usually the shorter one (e.g. `KH8/W1AW` → `KH8`).
fn base_call(up: &str) -> &str {
    match up.split_once('/') {
        Some((a, b)) => {
            let suffix = matches!(b, "P" | "M" | "MM" | "AM" | "A" | "QRP" | "QRPP")
                || (b.len() == 1 && b.chars().all(|c| c.is_ascii_digit()));
            if suffix || a.len() <= b.len() {
                a
            } else {
                b
            }
        }
        None => up,
    }
}

/// Resolve a callsign to a DXCC entity + representative location.
pub fn resolve(call: &str) -> Option<DxccInfo> {
    let r = resolver();
    let full = call.trim().to_ascii_uppercase();
    if full.is_empty() {
        return None;
    }
    // Exact-call exceptions win (full call, before affix stripping).
    if let Some(&(i, zone)) = r.exact.get(&full) {
        return Some(info(r, i, zone));
    }
    // Longest-prefix on the base call.
    let base = base_call(&full);
    let mut n = base.len();
    while n > 0 {
        if let Some(&(i, zone)) = r.prefixes.get(&base[..n]) {
            return Some(info(r, i, zone));
        }
        n -= 1;
    }
    None
}

/// Build a [`DxccInfo`], using the per-prefix CQ-zone override when present, else
/// the entity's default zone.
fn info(r: &'static Resolver, i: u32, zone_override: Option<u8>) -> DxccInfo {
    let e = &r.entities[i as usize];
    DxccInfo {
        entity: e.name.as_str(),
        lat: e.lat,
        lon: e.lon,
        cq_zone: zone_override.unwrap_or(e.cq_zone),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_full_list() {
        // cty.dat 2025-01-15 carries 346 entities; assert we got the full file.
        assert!(
            resolver().entities.len() >= 340,
            "entities: {}",
            resolver().entities.len()
        );
        assert!(resolver().prefixes.len() > 1000);
    }

    #[test]
    fn resolves_common_entities() {
        assert_eq!(resolve("KD9TAW").unwrap().entity, "United States");
        assert_eq!(resolve("JA1XYZ").unwrap().entity, "Japan");
        assert_eq!(resolve("C91RU").unwrap().entity, "Mozambique");
        assert_eq!(resolve("SP1ABC").unwrap().entity, "Poland");
        assert_eq!(resolve("XE1ABC").unwrap().entity, "Mexico");
        assert_eq!(resolve("SM3ABC").unwrap().entity, "Sweden");
        assert_eq!(resolve("EA4XYZ").unwrap().entity, "Spain");
        // longest-prefix: Hawaii/American Samoa beat the bare "K"/"N"/"W".
        assert_eq!(resolve("KH6ABC").unwrap().entity, "Hawaii");
        assert_eq!(resolve("KL7XX").unwrap().entity, "Alaska");
    }

    #[test]
    fn exact_call_overrides_prefix() {
        // Bouvet & Peter I have NO plain "3Y" prefix — only `=CALL` overrides.
        assert_eq!(resolve("3Y0J").unwrap().entity, "Bouvet");
        assert_eq!(resolve("3Y0X").unwrap().entity, "Peter 1 Island");
    }

    #[test]
    fn longitude_is_east_positive() {
        // Mexico is ~100°W → negative; Japan ~138°E → positive.
        assert!(resolve("XE1ABC").unwrap().lon < -90.0);
        assert!(resolve("JA1XYZ").unwrap().lon > 130.0);
    }

    #[test]
    fn handles_portable_and_unknown() {
        assert_eq!(resolve("DL1ABC/P").unwrap().entity, "Fed. Rep. of Germany");
        assert_eq!(resolve("KH8/N0CALL").unwrap().entity, "American Samoa");
        assert!(resolve("").is_none());
    }
}

#[cfg(test)]
mod zone_tests {
    use super::*;
    #[test]
    fn resolves_cq_zone() {
        // W = multi-zone (3/4/5); W1 (New England) is CQ zone 5 via prefix override.
        assert_eq!(resolve("W1AW").unwrap().cq_zone, 5, "W1 → CQ 5");
        assert_eq!(resolve("JA1XYZ").unwrap().cq_zone, 25, "Japan → CQ 25");
        assert_eq!(resolve("G3XYZ").unwrap().cq_zone, 14, "England → CQ 14");
        // every resolvable major call yields a valid zone 1..=40
        for c in ["DL1AA", "VK2AA", "PY2AA", "UA3AA", "ZL1AA"] {
            let z = resolve(c).unwrap().cq_zone;
            assert!((1..=40).contains(&z), "{c} → zone {z} out of range");
        }
    }
}
