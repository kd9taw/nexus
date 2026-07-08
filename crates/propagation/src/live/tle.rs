//! Celestrak amateur-satellite TLE adapter (the `live` feature) — the orbital
//! elements the pure geometry in [`crate::sat`] runs on.
//!
//! Celestrak's `gp.php?GROUP=amateur&FORMAT=tle` returns the amateur birds in
//! three-line (3LE) form: a name line, then TLE lines 1 and 2. Celestrak asks
//! consumers to cache — elements are day-scale — so the shell fetches every 12 h
//! and serves the last good `tles.json` on failure (the aurora/protons pattern).
//! The parse is pure and unit-tested; the fetch returns `Err` on trouble so the
//! caller keeps its cache rather than fabricating orbits.
//!
//! Data courtesy of CelesTrak (Dr. T.S. Kelso).

use std::time::Duration;

use crate::sat::Tle;

const TLE_URL: &str = "https://celestrak.org/NORAD/elements/gp.php?GROUP=amateur&FORMAT=tle";
const UA: &str = "nexus-propagation/0.1 (+ham radio satellite tracking)";

/// True if `line` looks like TLE line `n` (1 or 2): the `n ` prefix and the full
/// 69-column body. Tolerant of trailing junk (length only floored).
fn is_tle_line(line: &str, n: u8) -> bool {
    let prefix = if n == 1 { "1 " } else { "2 " };
    line.starts_with(prefix) && line.len() >= 69
}

/// Parse Celestrak 3LE (or bare 2LE) text into TLEs. Pure — unit-testable
/// without the network. Tolerant of `\r\n` and trailing junk; malformed triples
/// are skipped rather than fabricated. Names are trimmed; a name-less 2LE pair
/// keeps an empty name.
pub fn parse_tles(text: &str) -> Vec<Tle> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        // Named 3LE: <name> / "1 …" / "2 …".
        if i + 2 < lines.len()
            && !is_tle_line(lines[i], 1)
            && !is_tle_line(lines[i], 2)
            && is_tle_line(lines[i + 1], 1)
            && is_tle_line(lines[i + 2], 2)
        {
            out.push(Tle {
                name: lines[i].trim_start_matches("0 ").trim().to_string(),
                line1: lines[i + 1].to_string(),
                line2: lines[i + 2].to_string(),
            });
            i += 3;
        // Bare 2LE: "1 …" / "2 …" with no name line.
        } else if i + 1 < lines.len() && is_tle_line(lines[i], 1) && is_tle_line(lines[i + 1], 2) {
            out.push(Tle {
                name: String::new(),
                line1: lines[i].to_string(),
                line2: lines[i + 1].to_string(),
            });
            i += 2;
        } else {
            // Junk / half a malformed pair — skip and resync.
            i += 1;
        }
    }
    out
}

/// Fetch + parse the current amateur TLE set from Celestrak. `Err` on network,
/// HTTP, or empty-payload trouble (Celestrak returns 200 + "No GP data found"
/// when its cache is cold — that must not read as "zero satellites"), so the
/// caller serves stale-or-nothing, never fabricated orbits.
pub fn fetch_tles() -> Result<Vec<Tle>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let text = client
        .get(TLE_URL)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())?;
    let tles = parse_tles(&text);
    if tles.is_empty() {
        return Err("no TLEs parsed from Celestrak amateur group".to_string());
    }
    Ok(tles)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two real, published element sets (AIAA-2006-6753 verification vectors) in
    // Celestrak 3LE form, with `\r\n` and a trailing malformed pair to exercise
    // the tolerance paths.
    const FIXTURE: &str = "ISS (ZARYA)             \r\n\
1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927\r\n\
2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537\r\n\
VANGUARD 1\r\n\
1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753\r\n\
2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667\r\n\
BROKEN BIRD\r\n\
1 99999U short line\r\n";

    #[test]
    fn parses_two_birds_and_skips_malformed() {
        let tles = parse_tles(FIXTURE);
        assert_eq!(
            tles.len(),
            2,
            "two well-formed birds, the broken one skipped"
        );
        assert_eq!(tles[0].name, "ISS (ZARYA)"); // trailing spaces trimmed
        assert!(tles[0].line1.starts_with("1 25544U"));
        assert!(tles[0].line2.starts_with("2 25544"));
        assert_eq!(tles[0].line1.len(), 69); // no trailing \r left on the lines
        assert_eq!(tles[1].name, "VANGUARD 1");
        assert!(tles[1].line1.starts_with("1 00005U"));
    }

    #[test]
    fn empty_or_garbage_yields_no_tles() {
        assert!(parse_tles("").is_empty());
        assert!(parse_tles("No GP data found").is_empty());
        assert!(parse_tles("random\ntext\nlines\n").is_empty());
    }

    #[test]
    fn accepts_bare_two_line_pairs() {
        let two_le = "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927\n\
2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537\n";
        let tles = parse_tles(two_le);
        assert_eq!(tles.len(), 1);
        assert_eq!(tles[0].name, "");
        assert!(tles[0].line1.starts_with("1 25544U"));
    }
}
