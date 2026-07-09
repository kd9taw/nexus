//! POTA / SOTA reference handling — pure validation + normalization.
//!
//! Two "on-the-air" programs share one shape in the log (see [`crate::logbook::Ota`]):
//! Parks On The Air uses a park reference like `K-1234`; Summits On The Air uses a
//! summit reference like `W7A/MN-001`. The ADIF mapping differs (POTA → `SIG`/
//! `SIG_INFO`, SOTA → `SOTA_REF`), so the program is tracked alongside the reference.
//! Live activator-spot parsing + fetch live in the `propagation` crate (it has the
//! JSON + HTTP deps); this module is the dependency-free reference core.

/// An on-the-air program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaProgram {
    /// Parks On The Air.
    Pota,
    /// Summits On The Air.
    Sota,
}

impl OtaProgram {
    /// ADIF / API program code: "POTA" / "SOTA".
    pub fn code(self) -> &'static str {
        match self {
            OtaProgram::Pota => "POTA",
            OtaProgram::Sota => "SOTA",
        }
    }

    /// Parse a program code (case-insensitive). WWFF is treated as POTA-shaped here
    /// (same `SIG`/`SIG_INFO` ADIF mapping) but only POTA/SOTA are first-class.
    pub fn from_code(s: &str) -> Option<OtaProgram> {
        match s.trim().to_ascii_uppercase().as_str() {
            "POTA" => Some(OtaProgram::Pota),
            "SOTA" => Some(OtaProgram::Sota),
            _ => None,
        }
    }
}

/// Validate + normalize a POTA park reference (e.g. `"k-1234"` → `"K-1234"`).
/// Format: a 1–4 char alphanumeric prefix, a hyphen, then 4–5 digits (POTA uses
/// 4 historically, 5 for newer parks). Returns `None` if it doesn't match.
pub fn normalize_pota_ref(s: &str) -> Option<String> {
    let t = s.trim().to_ascii_uppercase();
    let (prefix, digits) = t.split_once('-')?;
    let prefix_ok =
        (1..=4).contains(&prefix.len()) && prefix.bytes().all(|b| b.is_ascii_alphanumeric());
    let digits_ok = (4..=5).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit());
    (prefix_ok && digits_ok).then_some(t)
}

/// Validate + normalize a SOTA summit reference (e.g. `"w7a/mn-001"` → `"W7A/MN-001"`).
/// Format: `<association>/<region>-<number>` — association 1–8 alphanumerics, region
/// 1–2 letters, number exactly 3 digits. Returns `None` if it doesn't match.
pub fn normalize_sota_ref(s: &str) -> Option<String> {
    let t = s.trim().to_ascii_uppercase();
    let (assoc, rest) = t.split_once('/')?;
    let (region, number) = rest.split_once('-')?;
    let assoc_ok =
        (1..=8).contains(&assoc.len()) && assoc.bytes().all(|b| b.is_ascii_alphanumeric());
    let region_ok =
        (1..=2).contains(&region.len()) && region.bytes().all(|b| b.is_ascii_alphabetic());
    let number_ok = number.len() == 3 && number.bytes().all(|b| b.is_ascii_digit());
    (assoc_ok && region_ok && number_ok).then_some(t)
}

/// Validate + normalize a reference for the given program.
pub fn normalize_ref(program: OtaProgram, s: &str) -> Option<String> {
    match program {
        OtaProgram::Pota => normalize_pota_ref(s),
        OtaProgram::Sota => normalize_sota_ref(s),
    }
}

// ---------------------------------------------------------------------------
// Local park directory — an importable/downloadable list searched OFFLINE (the
// HRD workflow a test user asked for: grab the park list once, search it locally).
// ---------------------------------------------------------------------------

/// A park directory entry (from the POTA all-parks export or an operator-imported CSV).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Park {
    /// Park reference, e.g. "K-1234" (uppercased).
    pub reference: String,
    /// Park name.
    pub name: String,
    /// Maidenhead grid, if the source has it ("" otherwise).
    pub grid: String,
    /// Location descriptor, e.g. "US-CA" ("" otherwise).
    pub location: String,
}

/// An in-memory, locally-searchable park index. Built from a CSV (imported file or downloaded
/// POTA export) and searched without any network. Dependency-free.
#[derive(Debug, Clone, Default)]
pub struct ParkIndex {
    parks: Vec<Park>,
}

/// Split one CSV record into fields, honoring `"..."` quoting (doubled `""` = literal quote).
/// Single-line records only — good enough for the flat POTA export.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

impl ParkIndex {
    /// Parse a parks CSV. **Header-aware**: locates the reference / name / grid / location columns
    /// by header name (case-insensitive substring), so column order + extra columns don't matter.
    /// Rows without a usable reference are skipped. If the first row has no recognizable reference
    /// column, falls back to positional `[reference, name, …]`.
    pub fn parse_csv(csv: &str) -> ParkIndex {
        // Strip a UTF-8 BOM — Excel/Sheets exports (exactly the operator-import path) start with
        // U+FEFF, which `trim()` does NOT remove, so the header's first column ("reference") would
        // never match and grid/location silently fall out.
        let csv = csv.trim_start_matches('\u{feff}');
        let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
        let header = match lines.next() {
            Some(h) => split_csv_line(h),
            None => return ParkIndex::default(),
        };
        let find = |names: &[&str]| -> Option<usize> {
            header.iter().position(|h| {
                let h = h.trim().to_ascii_lowercase();
                names.iter().any(|n| h == *n)
            })
        };
        let ref_col = find(&["reference", "ref", "park", "parkreference"]);
        let (ref_col, positional) = match ref_col {
            Some(c) => (c, false),
            // No header → treat the "header" line as data too, positional [ref, name, ...].
            None => (0, true),
        };
        let name_col = if positional { Some(1) } else { find(&["name", "parkname"]) };
        let grid_col = if positional { None } else { find(&["grid", "grid6", "maidenhead"]) };
        let loc_col = if positional {
            None
        } else {
            find(&["locationdesc", "location", "loc", "state"])
        };
        let get = |row: &[String], col: Option<usize>| -> String {
            col.and_then(|c| row.get(c)).map(|s| s.trim().to_string()).unwrap_or_default()
        };
        let mut parks = Vec::new();
        let mut push_row = |row: Vec<String>| {
            let reference = get(&row, Some(ref_col)).to_ascii_uppercase();
            if reference.is_empty() || !reference.contains('-') {
                return;
            }
            parks.push(Park {
                reference,
                name: get(&row, name_col),
                grid: get(&row, grid_col),
                location: get(&row, loc_col),
            });
        };
        if positional {
            push_row(header);
        }
        for l in lines {
            push_row(split_csv_line(l));
        }
        ParkIndex { parks }
    }

    pub fn len(&self) -> usize {
        self.parks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.parks.is_empty()
    }

    /// Search by reference (prefix match) or name (substring), case-insensitive. Reference-prefix
    /// matches rank ahead of name matches. Returns up to `limit` results.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Park> {
        let q = query.trim().to_ascii_uppercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut ref_hits = Vec::new();
        let mut name_hits = Vec::new();
        for p in &self.parks {
            if p.reference.starts_with(&q) {
                ref_hits.push(p.clone());
                if ref_hits.len() >= limit {
                    break;
                }
            } else if name_hits.len() < limit && p.name.to_ascii_uppercase().contains(&q) {
                name_hits.push(p.clone());
            }
        }
        ref_hits.extend(name_hits);
        ref_hits.truncate(limit);
        ref_hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_codes_round_trip() {
        assert_eq!(OtaProgram::from_code("pota"), Some(OtaProgram::Pota));
        assert_eq!(OtaProgram::from_code(" SOTA "), Some(OtaProgram::Sota));
        assert_eq!(OtaProgram::Pota.code(), "POTA");
        assert_eq!(OtaProgram::from_code("WWFF"), None);
    }

    #[test]
    fn pota_refs_validate_and_normalize() {
        assert_eq!(normalize_pota_ref("k-1234").as_deref(), Some("K-1234"));
        assert_eq!(normalize_pota_ref("VE-5678").as_deref(), Some("VE-5678"));
        assert_eq!(normalize_pota_ref("US-12345").as_deref(), Some("US-12345")); // 5-digit
                                                                                 // Rejects junk.
        assert_eq!(normalize_pota_ref("K1234"), None); // no hyphen
        assert_eq!(normalize_pota_ref("K-12"), None); // too few digits
        assert_eq!(normalize_pota_ref("LONGPFX-1234"), None); // prefix too long
        assert_eq!(normalize_pota_ref("K-12AB"), None); // non-digit suffix
        assert_eq!(normalize_pota_ref(""), None);
    }

    #[test]
    fn sota_refs_validate_and_normalize() {
        assert_eq!(
            normalize_sota_ref("w7a/mn-001").as_deref(),
            Some("W7A/MN-001")
        );
        assert_eq!(normalize_sota_ref("G/LD-001").as_deref(), Some("G/LD-001"));
        assert_eq!(
            normalize_sota_ref("VK3/VN-012").as_deref(),
            Some("VK3/VN-012")
        );
        // Rejects junk.
        assert_eq!(normalize_sota_ref("W7A-MN-001"), None); // no slash
        assert_eq!(normalize_sota_ref("W7A/MN-01"), None); // 2-digit number
        assert_eq!(normalize_sota_ref("W7A/M1-001"), None); // region not letters
        assert_eq!(normalize_sota_ref("W7A/MN-0012"), None); // 4-digit number
    }

    #[test]
    fn normalize_ref_dispatches_by_program() {
        assert_eq!(normalize_ref(OtaProgram::Pota, "k-1").as_deref(), None);
        assert_eq!(
            normalize_ref(OtaProgram::Pota, "k-1234").as_deref(),
            Some("K-1234")
        );
        assert_eq!(
            normalize_ref(OtaProgram::Sota, "g/ld-001").as_deref(),
            Some("G/LD-001")
        );
    }

    const PARKS_CSV: &str = "reference,name,active,locationDesc,latitude,longitude,grid\n\
K-0001,Acadia National Park,1,US-ME,44.35,-68.21,FN54\n\
K-1234,\"Big Bend, Texas\",1,US-TX,29.25,-103.25,DL89\n\
K-5678,Yellowstone National Park,1,US-WY,44.6,-110.5,DN44\n";

    #[test]
    fn parses_pota_csv_header_aware() {
        let idx = ParkIndex::parse_csv(PARKS_CSV);
        assert_eq!(idx.len(), 3);
        let p = idx.search("K-1234", 5);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].reference, "K-1234");
        assert_eq!(p[0].name, "Big Bend, Texas"); // quoted comma survived
        assert_eq!(p[0].location, "US-TX");
        assert_eq!(p[0].grid, "DL89");
    }

    #[test]
    fn search_matches_ref_prefix_and_name_substring() {
        let idx = ParkIndex::parse_csv(PARKS_CSV);
        // Reference prefix.
        let byref = idx.search("K-5", 5);
        assert_eq!(byref.len(), 1);
        assert_eq!(byref[0].reference, "K-5678");
        // Name substring (case-insensitive), ranked after any ref matches.
        let byname = idx.search("national", 5);
        assert_eq!(byname.len(), 2); // Acadia + Yellowstone
        // Empty query → nothing.
        assert!(idx.search("  ", 5).is_empty());
    }

    #[test]
    fn search_respects_the_limit() {
        let idx = ParkIndex::parse_csv(PARKS_CSV);
        assert_eq!(idx.search("K-", 2).len(), 2);
    }

    #[test]
    fn parses_csv_with_a_utf8_bom() {
        // Excel/Sheets exports (the operator-import path) prepend a BOM; it must not defeat the
        // header detection (else grid/location silently drop out via the positional fallback).
        let idx = ParkIndex::parse_csv(&format!("\u{feff}{PARKS_CSV}"));
        assert_eq!(idx.len(), 3);
        let p = idx.search("K-0001", 1);
        assert_eq!(p[0].grid, "FN54"); // grid column still resolved despite the BOM
        assert_eq!(p[0].location, "US-ME");
    }
}
