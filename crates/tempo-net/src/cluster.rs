//! DX-cluster / RBN spot-line parser — the pure decode half of the cluster
//! connector (the telnet transport lands in a later slice). Turns a `DX de …`
//! line from DX Spider / AR-Cluster / CC-Cluster / the Reverse Beacon Network
//! into a typed [`ClusterSpot`]; everything else (login banners, WWV, chat) is
//! rejected. No I/O. See tasks/specs/live-feeds-phase.md §3.

/// One parsed cluster spot. Band derivation is left to the consumer (which owns
/// the band model); we carry the raw frequency the cluster sent.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterSpot {
    /// The station that reported the spot (uppercased).
    pub spotter: String,
    /// The DX station spotted (uppercased).
    pub dx_call: String,
    /// Spot frequency in kHz, as sent.
    pub freq_khz: f64,
    /// Free-text comment (mode / report / notes), trailing time stripped.
    pub comment: String,
    /// UTC time token ("1234Z") if the line carried one.
    pub time_utc: Option<String>,
}

impl ClusterSpot {
    /// Spot frequency in MHz (kHz / 1000).
    pub fn freq_mhz(&self) -> f64 {
        self.freq_khz / 1000.0
    }
}

/// True for a "1234Z"-style UTC time token.
fn is_time_token(t: &str) -> bool {
    t.len() == 5 && t.ends_with('Z') && t[..4].bytes().all(|b| b.is_ascii_digit())
}

/// Parse one cluster line into a [`ClusterSpot`], or `None` if it isn't a usable
/// `DX de` spot (banner / WWV / chat / malformed).
///
/// Format (universal across cluster software):
/// `DX de <spotter>:   <freq_khz>  <dx_call>  <comment…>   [HHMMZ]`
pub fn parse_dx_spot(line: &str) -> Option<ClusterSpot> {
    let line = line.trim();
    // The `DX de ` prefix marks a spot line; it's ASCII so byte-slicing is safe.
    const PREFIX: &str = "DX de ";
    if line.len() < PREFIX.len() || !line[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let after = line[PREFIX.len()..].trim_start();
    // Spotter is everything up to the first ':'.
    let (spotter, body) = after.split_once(':')?;
    let spotter = spotter.trim().to_ascii_uppercase();
    if spotter.is_empty() {
        return None;
    }

    let mut tokens = body.split_whitespace();
    let freq_khz: f64 = tokens.next()?.parse().ok()?;
    if !freq_khz.is_finite() || freq_khz <= 0.0 {
        return None;
    }
    let dx_call = tokens.next()?.trim().to_ascii_uppercase();
    if dx_call.is_empty() {
        return None;
    }

    let rest: Vec<&str> = tokens.collect();
    // A trailing HHMMZ token is the spot time; split it off the comment.
    let (comment_tokens, time_utc) = match rest.last() {
        Some(t) if is_time_token(t) => (&rest[..rest.len() - 1], Some((*t).to_string())),
        _ => (&rest[..], None),
    };
    Some(ClusterSpot {
        spotter,
        dx_call,
        freq_khz,
        comment: comment_tokens.join(" "),
        time_utc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_canonical_dx_spider_line() {
        let s = parse_dx_spot("DX de W3LPL:     14025.0  UA9CDC       CW 599                1234Z")
            .unwrap();
        assert_eq!(s.spotter, "W3LPL");
        assert_eq!(s.dx_call, "UA9CDC");
        assert_eq!(s.freq_khz, 14025.0);
        assert_eq!(s.comment, "CW 599");
        assert_eq!(s.time_utc.as_deref(), Some("1234Z"));
        assert!((s.freq_mhz() - 14.025).abs() < 1e-9);
    }

    #[test]
    fn parses_an_rbn_ft8_line() {
        let s = parse_dx_spot("DX de KM3T-#:    14074.0  3Y0J         FT8  -6 dB  25 WPM  0312Z")
            .unwrap();
        assert_eq!(s.spotter, "KM3T-#");
        assert_eq!(s.dx_call, "3Y0J");
        assert_eq!(s.freq_khz, 14074.0);
        assert!(s.comment.contains("FT8"));
        assert_eq!(s.time_utc.as_deref(), Some("0312Z"));
    }

    #[test]
    fn rejects_non_spot_lines() {
        assert!(parse_dx_spot("Welcome to the DX cluster, W9XYZ").is_none());
        assert!(parse_dx_spot("WWV de W0MU <14>:   SFI=120, A=8, K=2").is_none());
        assert!(parse_dx_spot("To ALL de W1ABC: good morning").is_none());
        assert!(parse_dx_spot("").is_none());
    }

    #[test]
    fn handles_a_missing_time_and_missing_comment() {
        let s = parse_dx_spot("DX de N1XX: 7005.0 JA1ABC").unwrap();
        assert_eq!(s.dx_call, "JA1ABC");
        assert_eq!(s.comment, "");
        assert!(s.time_utc.is_none());

        let s2 = parse_dx_spot("DX de n1xx: 7005.0 ja1abc nice sig").unwrap();
        assert_eq!(s2.spotter, "N1XX"); // case-insensitive prefix + uppercased
        assert_eq!(s2.dx_call, "JA1ABC");
        assert_eq!(s2.comment, "nice sig");
        assert!(s2.time_utc.is_none());
    }

    #[test]
    fn rejects_malformed_freq_or_missing_call() {
        assert!(parse_dx_spot("DX de W3LPL: not-a-freq UA9CDC CW").is_none());
        assert!(parse_dx_spot("DX de W3LPL: 14025.0").is_none()); // no dx call
        assert!(parse_dx_spot("DX de W3LPL: -5.0 UA9CDC").is_none()); // nonsensical freq
    }
}
