//! Embedded ITU-R/CCIR coefficient data (monthly `COEFF##W.txt`) + parsers.
//!
//! One vendored text file per month carries EVERYTHING the P.533/P.372 chain
//! needs (see `data/itu/itu_copyright.txt`): the CCIR numerical-map
//! coefficients for foF2 (`xf2(13,76,2)`) and M(3000)F2 (`xfm3(9,49,2)`) with
//! their series-structure index rows (`if2(10)`, `ifm3(10)`), and the P.372
//! atmospheric-noise arrays (`fakp(29,16,6)`, `fakabp(2,6)`, `dud(5,12,5)`,
//! `fam(14,12)`). The trailing `,2)` dimension is the two solar-activity
//! planes (12-month smoothed sunspot number 0 and 100); consumers interpolate
//! linearly in SSN between them.
//!
//! Arrays are stored FLAT in the file's own (Fortran column-major) order —
//! first index fastest — and indexed through [`FArray`] so no transposition
//! can creep in between here and the expansion math.

use std::sync::OnceLock;

/// The 12 monthly coefficient files, embedded at compile time (Jan..Dec).
static COEFF_TXT: [&str; 12] = [
    include_str!("../../data/itu/COEFF01W.txt"),
    include_str!("../../data/itu/COEFF02W.txt"),
    include_str!("../../data/itu/COEFF03W.txt"),
    include_str!("../../data/itu/COEFF04W.txt"),
    include_str!("../../data/itu/COEFF05W.txt"),
    include_str!("../../data/itu/COEFF06W.txt"),
    include_str!("../../data/itu/COEFF07W.txt"),
    include_str!("../../data/itu/COEFF08W.txt"),
    include_str!("../../data/itu/COEFF09W.txt"),
    include_str!("../../data/itu/COEFF10W.txt"),
    include_str!("../../data/itu/COEFF11W.txt"),
    include_str!("../../data/itu/COEFF12W.txt"),
];

/// A Fortran-ordered (column-major, first index fastest) N-D array of f64,
/// kept flat exactly as listed in the coefficient file.
#[derive(Debug, Clone)]
pub struct FArray {
    /// Dimensions as declared in the file's section label, e.g. `[13, 76, 2]`.
    pub dims: Vec<usize>,
    /// Values in file order (first dimension varies fastest).
    pub v: Vec<f64>,
}

impl FArray {
    /// Element at 0-based indices (Fortran order: `idx[0]` varies fastest).
    /// Panics on rank mismatch or out-of-range — coefficient indexing is
    /// compile-time-shaped by the callers, so a violation is a programmer bug.
    #[inline]
    pub fn at(&self, idx: &[usize]) -> f64 {
        debug_assert_eq!(idx.len(), self.dims.len());
        let mut off = 0usize;
        let mut stride = 1usize;
        for (i, (&ix, &d)) in idx.iter().zip(&self.dims).enumerate() {
            debug_assert!(ix < d, "index {ix} out of range {d} in dim {i}");
            off += ix * stride;
            stride *= d;
        }
        self.v[off]
    }

    /// Convenience for the ubiquitous 3-D `(i, j, ssn_plane)` access.
    #[inline]
    pub fn at3(&self, i: usize, j: usize, k: usize) -> f64 {
        self.at(&[i, j, k])
    }

    fn len_expected(&self) -> usize {
        self.dims.iter().product()
    }
}

/// One month's parsed coefficient set (the sections the P.533/P.372 chain uses).
#[derive(Debug, Clone)]
pub struct MonthCoeffs {
    /// foF2 series-structure row (`if2(10)`): the CCIR map's per-harmonic
    /// G-function block limits + (last two) the number of coefficients per
    /// order and the count of UT harmonics — consumed by the map expansion.
    pub if2: Vec<i64>,
    /// foF2 CCIR coefficients `xf2(13, 76, 2)`.
    pub xf2: FArray,
    /// M(3000)F2 series-structure row (`ifm3(10)`).
    pub ifm3: Vec<i64>,
    /// M(3000)F2 CCIR coefficients `xfm3(9, 49, 2)`.
    pub xfm3: FArray,
    /// P.372 atmospheric-noise Fourier coefficients `fakp(29, 16, 6)`.
    pub fakp: FArray,
    /// P.372 atmospheric-noise slope/intercept pairs `fakabp(2, 6)`.
    pub fakabp: FArray,
    /// P.372 `dud(5, 12, 5)` (V_d deviations).
    pub dud: FArray,
    /// P.372 `fam(14, 12)` (frequency-variation polynomial coefficients).
    pub fam: FArray,
}

/// The parsed coefficients for month `m0` (0-based, 0 = January). Parsed once
/// per month on first use, then served from a static cache.
pub fn month(m0: usize) -> &'static MonthCoeffs {
    assert!(m0 < 12, "month index {m0} out of range");
    static CACHE: [OnceLock<MonthCoeffs>; 12] = [const { OnceLock::new() }; 12];
    CACHE[m0].get_or_init(|| {
        parse_month(COEFF_TXT[m0], m0).unwrap_or_else(|e| {
            // The data is embedded at compile time — a parse failure is a
            // build-input defect, not a runtime condition an operator can fix.
            panic!("embedded COEFF{:02}W.txt is malformed: {e}", m0 + 1)
        })
    })
}

/// Parse one monthly file into the sections we consume. `m0` is 0-based and is
/// checked against the file's own `month = N` header line.
fn parse_month(text: &str, m0: usize) -> Result<MonthCoeffs, String> {
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty file")?;
    let declared: i64 = header
        .split('=')
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad header {header:?}"))?;
    if declared != m0 as i64 + 1 {
        return Err(format!("header says month {declared}, expected {}", m0 + 1));
    }

    // Walk the labeled sections: a section starts at a line whose first
    // non-space char is alphabetic ("xf2(13,76,2)"); its values are every
    // whitespace token until the next label. Collect them all, then pick the
    // ones we need with shape checks.
    let mut sections: Vec<(String, Vec<usize>, Vec<f64>)> = Vec::new();
    for line in lines {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            let (name, dims) = parse_label(t)?;
            sections.push((name, dims, Vec::new()));
        } else if let Some(cur) = sections.last_mut() {
            // These are DOS-era files: the last line carries a Ctrl-Z (0x1A)
            // EOF marker — treat it as whitespace, not data.
            for tok in t
                .split(|c: char| c.is_whitespace() || c == '\u{1a}')
                .filter(|s| !s.is_empty())
            {
                let val: f64 = tok
                    .parse()
                    .map_err(|_| format!("bad number {tok:?} in section {}", cur.0))?;
                cur.2.push(val);
            }
        } else {
            return Err(format!("data before first section label: {t:?}"));
        }
    }

    let take = |name: &str| -> Result<FArray, String> {
        let (_, dims, v) = sections
            .iter()
            .find(|(n, _, _)| n == name)
            .ok_or_else(|| format!("missing section {name:?}"))?;
        let arr = FArray {
            dims: dims.clone(),
            v: v.clone(),
        };
        if arr.v.len() != arr.len_expected() {
            return Err(format!(
                "section {name:?}: {} values, label declares {}",
                arr.v.len(),
                arr.len_expected()
            ));
        }
        Ok(arr)
    };
    let take_ints = |name: &str| -> Result<Vec<i64>, String> {
        let arr = take(name)?;
        Ok(arr.v.iter().map(|&x| x as i64).collect())
    };

    Ok(MonthCoeffs {
        if2: take_ints("if2")?,
        xf2: take("xf2")?,
        ifm3: take_ints("ifm3")?,
        xfm3: take("xfm3")?,
        fakp: take("fakp")?,
        fakabp: take("fakabp")?,
        dud: take("dud")?,
        fam: take("fam")?,
    })
}

/// The P.1239-3 within-the-month foF2 variability decile factors
/// ("P1239-3 Decile Factors.txt", Tables 2+3): 18 blocks of 19 latitude rows
/// (90°..0° in 5° steps) × 24 local-time hours, ordered decile (lower, upper)
/// → season (winter, equinox, summer) → SSN range (<50, 50–100, >100).
pub struct P1239 {
    /// Flat `[decile][season][ssn][lat_idx][hour]`, lat_idx 0 = 0°, 18 = 90°.
    v: Vec<f64>,
}

impl P1239 {
    /// Decile factor — indices per the reference `foF2var` array:
    /// `season` 0..3, `hour` 0..24 (local time), `lat_idx` 0..19 (5° steps),
    /// `ssn_idx` 0..3, `decile` 0 = lower / 1 = upper.
    pub fn fof2var(
        &self,
        season: usize,
        hour: usize,
        lat_idx: usize,
        ssn_idx: usize,
        decile: usize,
    ) -> f64 {
        self.v[((((decile * 3) + season) * 3 + ssn_idx) * 19 + lat_idx) * 24 + hour]
    }
}

/// The parsed P.1239-3 decile table (parsed once on first use).
pub fn p1239() -> &'static P1239 {
    static CACHE: OnceLock<P1239> = OnceLock::new();
    CACHE.get_or_init(|| {
        // Latin-1 file (degree signs are 0xB0) — lossy conversion keeps the
        // numeric fields intact.
        let raw = include_bytes!("../../data/itu/P1239-3_Decile_Factors.txt");
        let text = String::from_utf8_lossy(raw);
        // Data rows: a latitude label ("90°") + 24 floats. Header/hour rows
        // carry no decimal points, so require one in the second token.
        let mut rows: Vec<(f64, Vec<f64>)> = Vec::new();
        for line in text.lines() {
            let toks: Vec<&str> = line.split_whitespace().collect();
            if toks.len() != 25 || !toks[1].contains('.') {
                continue;
            }
            let lat_deg: f64 = toks[0]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(-1.0);
            let vals: Option<Vec<f64>> = toks[1..].iter().map(|t| t.parse().ok()).collect();
            if let (true, Some(vals)) = (lat_deg >= 0.0, vals) {
                rows.push((lat_deg, vals));
            }
        }
        assert_eq!(
            rows.len(),
            2 * 3 * 3 * 19,
            "P1239 table: expected 342 data rows, got {}",
            rows.len()
        );
        // File order: blocks of 19 rows from 90° down to 0°; store by
        // ascending lat_idx (row r in a block → lat_idx 18−r), like the
        // reference's backward read.
        let mut v = vec![0.0f64; 2 * 3 * 3 * 19 * 24];
        for (b, block) in rows.chunks(19).enumerate() {
            for (r, (lat_deg, vals)) in block.iter().enumerate() {
                let lat_idx = 18 - r;
                debug_assert_eq!(*lat_deg as usize, lat_idx * 5);
                for (h, &val) in vals.iter().enumerate() {
                    v[(b * 19 + lat_idx) * 24 + h] = val;
                }
            }
        }
        P1239 { v }
    })
}

/// Parse a section label like `xf2(13,76,2)` or `if2(10)` → (name, dims).
fn parse_label(t: &str) -> Result<(String, Vec<usize>), String> {
    let open = t
        .find('(')
        .ok_or_else(|| format!("label without dims: {t:?}"))?;
    let close = t
        .rfind(')')
        .ok_or_else(|| format!("label without ')': {t:?}"))?;
    let name = t[..open].trim().to_ascii_lowercase();
    let dims: Result<Vec<usize>, _> = t[open + 1..close]
        .split(',')
        .map(|d| d.trim().parse::<usize>())
        .collect();
    Ok((name, dims.map_err(|_| format!("bad dims in {t:?}"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_twelve_months_parse_with_declared_shapes() {
        for m in 0..12 {
            let c = month(m); // panics on malformed data
            assert_eq!(c.xf2.dims, vec![13, 76, 2]);
            assert_eq!(c.xfm3.dims, vec![9, 49, 2]);
            assert_eq!(c.fakp.dims, vec![29, 16, 6]);
            assert_eq!(c.fakabp.dims, vec![2, 6]);
            assert_eq!(c.dud.dims, vec![5, 12, 5]);
            assert_eq!(c.fam.dims, vec![14, 12]);
            assert_eq!(c.if2.len(), 10);
            assert_eq!(c.ifm3.len(), 10);
        }
    }

    #[test]
    fn january_spot_values_match_the_source_file() {
        // Hand-checked against data/itu/COEFF01W.txt (the vendored original).
        let c = month(0);
        assert_eq!(c.if2, vec![11, 35, 53, 63, 67, 69, 71, 73, 75, 6]);
        assert_eq!(c.ifm3, vec![6, 22, 34, 40, 44, 46, 48, 48, 48, 4]);
        // First values of each array (Fortran order: first index fastest).
        assert_eq!(c.xf2.at3(0, 0, 0), 0.52396593e+01);
        assert_eq!(c.xf2.at(&[1, 0, 0]), -0.56523629e-01);
        assert_eq!(c.xfm3.at3(0, 0, 0), 0.30831585e+01);
        assert_eq!(c.fakp.at3(0, 0, 0), 0.84990568e+01);
        assert_eq!(c.fakp.at(&[1, 0, 0]), 0.20480766e+02);
        assert_eq!(c.fakabp.at(&[0, 0]), 0.27210815e+02);
        assert_eq!(c.dud.at3(0, 0, 0), 0.60209274e+00);
    }

    #[test]
    fn farray_indexing_is_first_index_fastest() {
        // xf2 line 2 of the file holds values 1..5; value index 13 (0-based)
        // is therefore the start of the SECOND column: xf2(0, 1, 0).
        let c = month(0);
        assert_eq!(c.xf2.at(&[0, 1, 0]), c.xf2.v[13]);
        // The second SSN plane starts after 13*76 values.
        assert_eq!(c.xf2.at(&[0, 0, 1]), c.xf2.v[13 * 76]);
    }
}
