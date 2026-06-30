//! Standard FT1 QSO messages (build + parse).
//!
//! FT1 reuses the WSJT-X 77-bit message formats. The forms Tempo's
//! auto-sequencer uses all take the shape `<TO> <FROM> <PAYLOAD>` (plus the
//! `CQ <CALL> <GRID>` form), where PAYLOAD is one of:
//! a 4-character Maidenhead grid, a signal report (`-10`, `+05`), a rogered
//! report (`R-12`), or `RR73` / `RRR` / `73`. These all round-trip verbatim
//! through the modem (verified against the FT1 packer).

/// A parsed/buildable standard QSO message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// `CQ <de> <grid>`
    Cq {
        de: String,
        grid: String,
        /// Directed-CQ token between "CQ" and the call ("DX", "NA", "POTA",
        /// "TEST", a 3-digit kHz like "040", …). Empty = a plain CQ. WSJT-X
        /// preserves and re-emits it; dropping it silently rewrote operators'
        /// directed calls into plain ones.
        dir: String,
    },
    /// `<to> <de> <grid>` — reply to a CQ / call with grid.
    Grid {
        to: String,
        de: String,
        grid: String,
    },
    /// `<to> <de> <snr>` — signal report.
    Report { to: String, de: String, snr: i32 },
    /// `<to> <de> R<snr>` — rogered signal report.
    RReport { to: String, de: String, snr: i32 },
    /// `<to> <de> RR73`
    Rr73 { to: String, de: String },
    /// `<to> <de> RRR`
    Rrr { to: String, de: String },
    /// `<to> <de> 73`
    Bye73 { to: String, de: String },
    /// ARRL Field Day exchange: `<to> <de> [R] <class> <section>`
    /// (e.g. `W9XYZ K2DEF 3A WI` or `W9XYZ K2DEF R 3A WI`).
    FieldDay {
        to: String,
        de: String,
        roger: bool,
        class: String,
        section: String,
    },
    /// Free text or anything not recognized as a standard form.
    Other(String),
}

/// Valid signal-report range for standard messages — WSJT-X's report field runs
/// from the -50/-31 specials region up to +49. A numeric token outside this range
/// is NOT a report (so a stray "R73" is free text, not a phantom +73 dB report).
pub const REPORT_MIN: i32 = -50;
pub const REPORT_MAX: i32 = 49;

/// True when `n` is a plausible signal report (within [`REPORT_MIN`]..=[`REPORT_MAX`]).
pub fn is_report(n: i32) -> bool {
    (REPORT_MIN..=REPORT_MAX).contains(&n)
}

/// Format a signal report the way WSJT-X does: sign + two digits. Clamped to the
/// valid report ceiling (+49) so a strong-signal report like +35 is faithful
/// (the old +30 cap truncated it); the floor stays at the practical decode floor.
pub fn fmt_report(snr: i32) -> String {
    format!("{:+03}", snr.clamp(-30, REPORT_MAX))
}

/// The base callsign for matching — uppercased, with a portable prefix/suffix
/// stripped (`W9XYZ/4` → `W9XYZ`, `KH8/W1AW` → `W1AW`), mirroring WSJT-X's
/// `Radio::base_callsign`. Used so an "addressed to me" / "from the DX" test still
/// works under compound/portable operation instead of silently stalling the QSO.
pub fn base_call(call: &str) -> String {
    // Strip an i3=4 hashed-call wrapper (`<W9XYZ>`, or the unresolved `<...>`) first, so
    // a hashed call matches its plain form in the addressed-to-me / from-the-DX tests.
    let up = call
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_uppercase();
    if !up.contains('/') {
        return up;
    }
    // The base is the '/'-segment shaped like a full call — a letter that comes
    // AFTER a digit (W1AW, W9XYZ) — unlike a bare prefix (KH8) or affix (P, MM, 4).
    let looks_full = |s: &str| {
        let b = s.as_bytes();
        b.iter()
            .position(|c| c.is_ascii_digit())
            .map(|d| b[d + 1..].iter().any(|c| c.is_ascii_alphabetic()))
            .unwrap_or(false)
    };
    // Prefer the LAST full-looking segment: for prefix-portable (KH8/W1AW,
    // VP2E/AA9A) the home call comes last; for suffix-portable (W9XYZ/P, /4) the
    // affix isn't full-looking so the home call (first) still wins.
    up.split('/')
        .filter(|s| looks_full(s))
        .last()
        .or_else(|| up.split('/').filter(|s| !s.is_empty()).max_by_key(|s| s.len()))
        .map(|s| s.to_string())
        .unwrap_or(up)
}

/// Two callsigns refer to the same station — base-call (portable) and
/// case-insensitive comparison, the WSJT-X way.
pub fn same_call(a: &str, b: &str) -> bool {
    base_call(a) == base_call(b)
}

/// True if `call` is a NONSTANDARD/compound call that can't ride the standard 28-bit
/// field — a slash prefix/suffix form (W9XYZ/P, KH8/W1AW, PJ4/K1ABC, KH1/KH7Z). Such a
/// call must be transmitted in FULL with the OTHER call wrapped in `<...>` (i3=4), or
/// the modem silently strips the prefix off a bare compound call. Brackets are ignored.
/// Requires at least one '/'-segment to be callsign-shaped, so ham free-text slashes
/// ("5/9", "S/N", "W/L", "2X/3") are NOT mistaken for compound calls.
/// A valid WSJT-X directed-CQ token: 1–4 letters ("DX", "NA", "POTA", "TEST")
/// or exactly 3 digits (a kHz QSY like "CQ 040 …").
fn is_cq_dir(s: &str) -> bool {
    (!s.is_empty() && s.len() <= 4 && s.chars().all(|c| c.is_ascii_alphabetic()))
        || (s.len() == 3 && s.chars().all(|c| c.is_ascii_digit()))
}

pub fn is_compound(call: &str) -> bool {
    let c = call.trim().trim_start_matches('<').trim_end_matches('>');
    c.contains('/') && c.split('/').any(is_callsign)
}

/// The inner text of an i3=4 hashed token (`<W9XYZ>` → `W9XYZ`, `<...>` → `...`).
fn hashed_inner(s: &str) -> &str {
    s.trim_start_matches('<').trim_end_matches('>')
}

/// True for a `<...>`-wrapped token whose inner text is a real call (or the unresolved
/// `...`) — a genuine i3=4 hash, not arbitrary `<bracketed>` free text.
fn is_valid_hashed(s: &str) -> bool {
    s.starts_with('<') && s.ends_with('>') && {
        let i = hashed_inner(s);
        i == "..." || is_callsign(i) || is_compound(i)
    }
}

/// True if a token reads as a real callsign for parsing a 2-call message — a plain or
/// compound call, or a valid i3=4 hashed-call token. Guards the 2-token parse forms so
/// free text (incl. slash shorthand and arbitrary `<...>`) isn't misread as a call pair.
/// Parse a Tempo id-bearing ACK frame `"<to> <de> RR73 <id>"` (e.g. "W9XYZ K2DEF RR73 A")
/// into `(to, de, chunk-id)`. The id is the store chunk-id char of the message being
/// acknowledged, so the sender confirms THAT specific message (not a FIFO guess) and a
/// resend's ACK is idempotent. Tempo-to-Tempo only — a free-text frame, not a standard
/// WSJT-X message (so it never collides with a plain `RR73` roger).
pub fn parse_ack(s: &str) -> Option<(String, String, char)> {
    let t: Vec<&str> = s.split_whitespace().collect();
    if t.len() == 4 && t[2] == "RR73" && t[3].chars().count() == 1 {
        let id = t[3].chars().next()?;
        if id.is_ascii_uppercase() && looks_like_call(t[0]) && looks_like_call(t[1]) {
            return Some((t[0].to_string(), t[1].to_string(), id));
        }
    }
    None
}

pub fn looks_like_call(s: &str) -> bool {
    is_valid_hashed(s) || is_compound(s) || is_callsign(s)
}

impl Msg {
    /// Render to the on-air text form.
    pub fn to_text(&self) -> String {
        match self {
            // An empty grid = the i3=4 compound form (a compound call carries no grid):
            // render without the trailing grid token.
            Msg::Cq { de, grid, dir } => {
                let d = if dir.is_empty() { String::new() } else { format!("{dir} ") };
                if grid.is_empty() {
                    format!("CQ {d}{de}")
                } else {
                    format!("CQ {d}{de} {grid}")
                }
            }
            Msg::Grid { to, de, grid } if grid.is_empty() => format!("{to} {de}"),
            Msg::Grid { to, de, grid } => format!("{to} {de} {grid}"),
            Msg::Report { to, de, snr } => format!("{to} {de} {}", fmt_report(*snr)),
            Msg::RReport { to, de, snr } => format!("{to} {de} R{}", fmt_report(*snr)),
            Msg::Rr73 { to, de } => format!("{to} {de} RR73"),
            Msg::Rrr { to, de } => format!("{to} {de} RRR"),
            Msg::Bye73 { to, de } => format!("{to} {de} 73"),
            Msg::FieldDay {
                to,
                de,
                roger,
                class,
                section,
            } => {
                if *roger {
                    format!("{to} {de} R {class} {section}")
                } else {
                    format!("{to} {de} {class} {section}")
                }
            }
            Msg::Other(s) => s.clone(),
        }
    }

    /// Parse decoded text into a standard form (falls back to [`Msg::Other`]).
    pub fn parse(s: &str) -> Msg {
        let t: Vec<&str> = s.split_whitespace().collect();
        if t.len() >= 3 && t[0] == "CQ" {
            // "CQ <call> <grid>" or directed "CQ DX/NA/POTA/TEST/nnn <call> <grid>"
            // — the modifier is PRESERVED (WSJT-X re-emits it; we used to eat it).
            let de = t[t.len() - 2].to_string();
            let grid = t[t.len() - 1].to_string();
            if is_grid(&grid) {
                let dir = if t.len() == 4 && is_cq_dir(t[1]) {
                    t[1].to_string()
                } else {
                    String::new()
                };
                return Msg::Cq { de, grid, dir };
            }
        }
        // Grid-less DIRECTED CQ: "CQ DX <call>" (3 tokens — WSJT-X emits this
        // for compound senders and some band plans). Without this branch the
        // form fell to free text and the station was invisible to the
        // sequencer. The token must validate AND the third must be a real call
        // so "5/9 NJ2X"-style free text never misreads.
        if t.len() == 3
            && t[0] == "CQ"
            && is_cq_dir(t[1])
            && (is_callsign(t[2]) || is_compound(t[2]))
        {
            return Msg::Cq {
                de: t[2].to_string(),
                grid: String::new(),
                dir: t[1].to_string(),
            };
        }
        // i3=4 compound CQ: "CQ <compound-call>" with NO grid (a compound call can't
        // carry one). Only a real COMPOUND call qualifies — "CQ W1AW" (a grid-less plain
        // call) stays free text, and "CQ <...>" is invalid (the modem rejects it).
        if t.len() == 2 && t[0] == "CQ" && is_compound(t[1]) {
            return Msg::Cq { de: t[1].to_string(), grid: String::new(), dir: String::new() };
        }
        // Two-call message with no payload: "<to> <de>" — an i3=4 call (no grid), e.g. a
        // compound/hashed station answering. A grid-less Grid = "calling <to>". REQUIRE
        // both tokens to read as real calls, at least one to be hashed/compound, and NOT
        // both hashed (i3=4 hashes exactly one call) — so free text (incl. slash shorthand
        // like "5/9 NJ2X" and arbitrary "<x> <y>") is never misread as a call pair.
        if t.len() == 2
            && looks_like_call(t[0])
            && looks_like_call(t[1])
            && (is_valid_hashed(t[0]) || is_valid_hashed(t[1]) || is_compound(t[0]) || is_compound(t[1]))
            && !(is_valid_hashed(t[0]) && is_valid_hashed(t[1]))
        {
            return Msg::Grid {
                to: t[0].to_string(),
                de: t[1].to_string(),
                grid: String::new(),
            };
        }
        if t.len() == 3 {
            let to = t[0].to_string();
            let de = t[1].to_string();
            let p = t[2];
            match p {
                "RR73" => return Msg::Rr73 { to, de },
                "RRR" => return Msg::Rrr { to, de },
                "73" => return Msg::Bye73 { to, de },
                _ => {}
            }
            if let Some(rest) = p.strip_prefix('R') {
                // Only a value in the report range is an R-report; "R73" (n=73) is
                // NOT a report — fall through so it routes as free text, not a
                // phantom +73 dB RST.
                if let Ok(n) = rest.parse::<i32>() {
                    if is_report(n) {
                        return Msg::RReport { to, de, snr: n };
                    }
                }
            }
            if let Ok(n) = p.parse::<i32>() {
                if is_report(n) {
                    return Msg::Report { to, de, snr: n };
                }
            }
            if is_grid(p) {
                return Msg::Grid {
                    to,
                    de,
                    grid: p.to_string(),
                };
            }
        }
        // ARRL Field Day exchange: "<to> <de> [R] <class> <section>".
        if t.len() == 4 || t.len() == 5 {
            let class_idx = if t.len() == 5 && t[2] == "R" {
                Some(3)
            } else if t.len() == 4 {
                Some(2)
            } else {
                None
            };
            if let Some(ci) = class_idx {
                let class = t[ci];
                let section = t.get(ci + 1).copied().unwrap_or("");
                if is_fd_class(class) && is_section(section) {
                    return Msg::FieldDay {
                        to: t[0].to_string(),
                        de: t[1].to_string(),
                        roger: t.len() == 5,
                        class: class.to_string(),
                        section: section.to_string(),
                    };
                }
            }
        }
        Msg::Other(s.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// The callsign this message is directed to, if any.
    pub fn addressee(&self) -> Option<&str> {
        match self {
            Msg::Grid { to, .. }
            | Msg::Report { to, .. }
            | Msg::RReport { to, .. }
            | Msg::Rr73 { to, .. }
            | Msg::Rrr { to, .. }
            | Msg::Bye73 { to, .. }
            | Msg::FieldDay { to, .. } => Some(to),
            _ => None,
        }
    }

    /// The callsign that sent this message, if identifiable.
    pub fn sender(&self) -> Option<&str> {
        match self {
            Msg::Cq { de, .. }
            | Msg::Grid { de, .. }
            | Msg::Report { de, .. }
            | Msg::RReport { de, .. }
            | Msg::Rr73 { de, .. }
            | Msg::Rrr { de, .. }
            | Msg::Bye73 { de, .. }
            | Msg::FieldDay { de, .. } => Some(de),
            _ => None,
        }
    }
}

/// True for an ARRL Field Day class like `3A`, `12A`, `1B`, `3H` (1–2 digits + letter).
fn is_fd_class(s: &str) -> bool {
    let b = s.as_bytes();
    let n = b.len();
    (2..=3).contains(&n)
        && b[..n - 1].iter().all(|c| c.is_ascii_digit())
        && b[n - 1].is_ascii_uppercase()
}

/// True for an ARRL/RAC section abbreviation (2–5 uppercase letters, e.g. WI, ENY, STX).
fn is_section(s: &str) -> bool {
    (2..=5).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_uppercase())
}

/// True for a 4-character Maidenhead grid like `EN37`.
fn is_grid(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 4
        && b[0].is_ascii_uppercase()
        && b[1].is_ascii_uppercase()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
}

/// True if `s` is a grid usable in a standard (FT8/FT4) message: a 4-char field
/// (`AA00`), optionally with a 6-char subsquare (`AA00aa`). Used to GATE keying —
/// WSJT-X won't build a CQ/Tx1 without a valid grid, so neither will we (an empty
/// or malformed grid would otherwise emit a grid-less call on the air).
pub fn is_valid_grid(s: &str) -> bool {
    let s = s.trim();
    let b = s.as_bytes();
    // Field letters are case-INSENSITIVE: the FT8 packer treats "en52" and "EN52"
    // identically (byte-identical tones), and operators type either — so validating
    // (and gating TX on) only uppercase would wrongly block a perfectly legal grid.
    let four = s.len() >= 4
        && b[0].is_ascii_alphabetic()
        && b[1].is_ascii_alphabetic()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit();
    match s.len() {
        4 => four,
        6 => four && b[4].is_ascii_alphabetic() && b[5].is_ascii_alphabetic(),
        _ => false,
    }
}

/// True if `s` is plausibly a real amateur callsign (3–10 chars, has a letter AND
/// a digit, only alphanumerics + `/`). Mirrors the cluster/PSKReporter feed gate;
/// used to refuse keying a standard message with no/blank callsign.
pub fn is_callsign(s: &str) -> bool {
    let c = s.trim();
    let len = c.chars().count();
    (3..=10).contains(&len)
        && c.chars().any(|ch| ch.is_ascii_digit())
        && c.chars().any(|ch| ch.is_ascii_alphabetic())
        && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '/')
}

#[cfg(test)]
mod fidelity_tests {
    use super::*;

    #[test]
    fn valid_grid_accepts_4_and_6_char_rejects_blank_and_malformed() {
        assert!(is_valid_grid("EN52"));
        assert!(is_valid_grid("en52"), "lowercase field letters are valid (encoder is case-insensitive)");
        assert!(is_valid_grid("En52"));
        assert!(is_valid_grid("EN52aa")); // 6-char subsquare
        assert!(is_valid_grid(" EN52 ")); // trimmed
        assert!(!is_valid_grid(""), "blank grid (the bug) is rejected");
        assert!(!is_valid_grid("EN5")); // too short
        assert!(!is_valid_grid("E152")); // 2nd char must be a letter
        assert!(!is_valid_grid("ENXX")); // 3rd/4th must be digits
        assert!(!is_valid_grid("EN52a")); // 5-char is not a valid form
        assert!(!is_valid_grid("EN5212")); // subsquare must be letters
    }

    #[test]
    fn callsign_validation_matches_the_feed_gate() {
        assert!(is_callsign("W9XYZ"));
        assert!(is_callsign("KD9TAW"));
        assert!(is_callsign("W9XYZ/P"));
        assert!(!is_callsign(""), "blank call is rejected");
        assert!(!is_callsign("XYZ"), "no digit");
        assert!(!is_callsign("12345"), "no letter");
        assert!(!is_callsign("AB"), "too short");
    }

    #[test]
    fn compound_and_hashed_call_matching() {
        // base_call strips i3=4 <...> brackets so a hashed call matches its plain form.
        assert_eq!(base_call("<W9XYZ>"), "W9XYZ");
        assert!(same_call("<W9XYZ>", "W9XYZ"));
        assert!(same_call("<W9XYZ>", "w9xyz"));
        assert_eq!(base_call("<...>"), "...", "an unresolved hash matches nothing real");
        // Compound detection (slash forms, with or without brackets).
        assert!(is_compound("PJ4/K1ABC"));
        assert!(is_compound("KH1/KH7Z"));
        assert!(is_compound("W9XYZ/P"));
        assert!(is_compound("<W9XYZ/P>"));
        assert!(!is_compound("W9XYZ"));
        assert!(!is_compound("<W9XYZ>"));
    }

    #[test]
    fn parses_compound_cq_and_i3_4_call_forms() {
        assert_eq!(
            Msg::parse("CQ PJ4/K1ABC"),
            Msg::Cq { de: "PJ4/K1ABC".into(), grid: String::new(), dir: String::new() },
            "compound CQ has no grid"
        );
        // Two-call no-payload i3=4 forms (a hashed or compound token present).
        assert_eq!(
            Msg::parse("<PJ4/K1ABC> W9XYZ"),
            Msg::Grid { to: "<PJ4/K1ABC>".into(), de: "W9XYZ".into(), grid: String::new() }
        );
        assert_eq!(
            Msg::parse("PJ4/K1ABC <W9XYZ>"),
            Msg::Grid { to: "PJ4/K1ABC".into(), de: "<W9XYZ>".into(), grid: String::new() }
        );
        // i3=4 reply WITH a report parses via the existing 3-token path (brackets in to/de).
        assert_eq!(
            Msg::parse("<PJ4/K1ABC> W9XYZ R-10"),
            Msg::RReport { to: "<PJ4/K1ABC>".into(), de: "W9XYZ".into(), snr: -10 }
        );
        // Plain free text / a bare standard pair is NOT misread as an i3=4 call pair.
        assert!(matches!(Msg::parse("NET TONIGHT"), Msg::Other(_)));
        assert!(matches!(Msg::parse("HELLO ALL"), Msg::Other(_)));
        assert!(matches!(Msg::parse("W9XYZ K2DEF"), Msg::Other(_)));
        // Ham free-text SLASH shorthand must NOT read as a compound call pair.
        for s in ["5/9 NJ2X", "W/L 20M", "S/N 10DB", "2X/3 W1AW", "I/O TEST"] {
            assert!(matches!(Msg::parse(s), Msg::Other(_)), "{s} must be free text");
        }
        // Arbitrary bracketed tokens / both-hashed are not a valid i3=4 call pair.
        assert!(matches!(Msg::parse("<X> <Y>"), Msg::Other(_)));
        assert!(matches!(Msg::parse("<...> <...>"), Msg::Other(_)));
        // A grid-less plain CQ and "CQ <...>" stay free text (only a compound CQ is i3=4).
        assert!(matches!(Msg::parse("CQ W1AW"), Msg::Other(_)));
        assert!(matches!(Msg::parse("CQ <...>"), Msg::Other(_)));
        assert!(matches!(Msg::parse("CQ <DX>"), Msg::Other(_)));
    }

    #[test]
    fn compound_forms_render_without_a_grid() {
        assert_eq!(
            Msg::Cq { de: "PJ4/K1ABC".into(), grid: String::new(), dir: String::new() }.to_text(),
            "CQ PJ4/K1ABC"
        );
        assert_eq!(
            Msg::Grid { to: "<PJ4/K1ABC>".into(), de: "W9XYZ".into(), grid: String::new() }.to_text(),
            "<PJ4/K1ABC> W9XYZ"
        );
    }

    #[test]
    fn base_call_strips_portable_affixes() {
        assert_eq!(base_call("W9XYZ"), "W9XYZ");
        assert_eq!(base_call("w9xyz"), "W9XYZ");
        assert_eq!(base_call("W9XYZ/P"), "W9XYZ");
        assert_eq!(base_call("W9XYZ/4"), "W9XYZ");
        assert_eq!(base_call("KD9TAW/MM"), "KD9TAW");
        // Prefix-portable: the home call (letter after the digit) is the base.
        assert_eq!(base_call("KH8/W1AW"), "W1AW");
        assert_eq!(base_call("VP2E/AA9A"), "AA9A");
    }

    #[test]
    fn same_call_matches_across_portable_and_case() {
        assert!(same_call("KD9TAW", "kd9taw/p"));
        assert!(same_call("KD9TAW/P", "KD9TAW"));
        assert!(!same_call("KD9TAW", "KD9TAX"));
    }

    #[test]
    fn r73_is_not_a_phantom_report() {
        // "R73" must NOT become RReport{snr:73}; it routes to free text.
        assert!(matches!(Msg::parse("K2DEF W9XYZ R73"), Msg::Other(_)));
        // Out-of-range bare numbers are not reports either.
        assert!(matches!(Msg::parse("K2DEF W9XYZ 99"), Msg::Other(_)));
    }

    #[test]
    fn strong_signal_reports_are_faithful_to_wsjtx() {
        // +35 must survive (the old +30 cap truncated it).
        assert_eq!(fmt_report(35), "+35");
        assert_eq!(fmt_report(49), "+49");
        assert_eq!(fmt_report(60), "+49"); // clamped at the WSJT-X ceiling
        assert!(matches!(
            Msg::parse("K2DEF W9XYZ +35"),
            Msg::Report { snr: 35, .. }
        ));
        assert!(matches!(
            Msg::parse("K2DEF W9XYZ R+35"),
            Msg::RReport { snr: 35, .. }
        ));
    }

    #[test]
    fn forms_roundtrip_through_text() {
        let cases = [
            Msg::Cq {
                de: "W9XYZ".into(),
                grid: "EN37".into(),
                dir: String::new(),
            },
            Msg::Cq {
                de: "W9XYZ".into(),
                grid: "EN37".into(),
                dir: "DX".into(),
            },
            Msg::Grid {
                to: "W9XYZ".into(),
                de: "K2DEF".into(),
                grid: "FN31".into(),
            },
            Msg::Report {
                to: "K2DEF".into(),
                de: "W9XYZ".into(),
                snr: -10,
            },
            Msg::Report {
                to: "K2DEF".into(),
                de: "W9XYZ".into(),
                snr: 5,
            },
            Msg::RReport {
                to: "W9XYZ".into(),
                de: "K2DEF".into(),
                snr: -12,
            },
            Msg::Rr73 {
                to: "K2DEF".into(),
                de: "W9XYZ".into(),
            },
            Msg::Rrr {
                to: "K2DEF".into(),
                de: "W9XYZ".into(),
            },
            Msg::Bye73 {
                to: "K2DEF".into(),
                de: "W9XYZ".into(),
            },
        ];
        for c in cases {
            assert_eq!(Msg::parse(&c.to_text()), c, "roundtrip failed for {c:?}");
        }
    }

    #[test]
    fn parses_known_text() {
        assert_eq!(
            Msg::parse("CQ W9XYZ EN37"),
            Msg::Cq {
                de: "W9XYZ".into(),
                grid: "EN37".into(),
                dir: String::new(),
            }
        );
        // Grid-less directed CQ (3 tokens) — WSJT-X emits this for compound
        // senders and some band plans; it must be a CQ, not free text.
        assert_eq!(
            Msg::parse("CQ DX K1ABC"),
            Msg::Cq { de: "K1ABC".into(), grid: String::new(), dir: "DX".into() }
        );
        assert_eq!(
            Msg::parse("CQ DX PJ4/K1ABC"),
            Msg::Cq { de: "PJ4/K1ABC".into(), grid: String::new(), dir: "DX".into() }
        );
        assert_eq!(Msg::parse("CQ DX K1ABC").to_text(), "CQ DX K1ABC");
        // Free text must NOT misread as a gridless directed CQ ("5" is no
        // callsign). (It still hits a PRE-EXISTING 3-token report quirk —
        // Report{to:"CQ"} — which predates the dir branch; assert only that
        // the new branch doesn't claim it.)
        assert!(!matches!(Msg::parse("CQ UP 5"), Msg::Cq { .. }));
        // Directed CQ: the modifier is preserved, de/grid still parse right.
        assert_eq!(
            Msg::parse("CQ DX W9XYZ EN37"),
            Msg::Cq {
                de: "W9XYZ".into(),
                grid: "EN37".into(),
                dir: "DX".into(),
            }
        );
        assert_eq!(
            Msg::parse("CQ DX W9XYZ EN37").to_text(),
            "CQ DX W9XYZ EN37",
            "directed CQ round-trips"
        );
        assert_eq!(
            Msg::parse("CQ 040 W9XYZ EN37").to_text(),
            "CQ 040 W9XYZ EN37",
            "kHz-QSY directed CQ round-trips"
        );
        assert_eq!(
            Msg::parse("K2DEF W9XYZ +05"),
            Msg::Report {
                to: "K2DEF".into(),
                de: "W9XYZ".into(),
                snr: 5
            }
        );
        assert_eq!(
            Msg::parse("W9XYZ K2DEF R-12"),
            Msg::RReport {
                to: "W9XYZ".into(),
                de: "K2DEF".into(),
                snr: -12
            }
        );
        assert_eq!(Msg::parse("K2DEF W9XYZ RR73").addressee(), Some("K2DEF"));
        assert_eq!(Msg::parse("CQ W9XYZ EN37").sender(), Some("W9XYZ"));
    }

    #[test]
    fn report_formatting() {
        assert_eq!(fmt_report(5), "+05");
        assert_eq!(fmt_report(-10), "-10");
        assert_eq!(fmt_report(0), "+00");
        assert_eq!(fmt_report(-99), "-30"); // clamped
    }

    #[test]
    fn field_day_forms() {
        let fd = Msg::FieldDay {
            to: "W9XYZ".into(),
            de: "K2DEF".into(),
            roger: false,
            class: "3A".into(),
            section: "WI".into(),
        };
        assert_eq!(fd.to_text(), "W9XYZ K2DEF 3A WI");
        assert_eq!(Msg::parse("W9XYZ K2DEF 3A WI"), fd);

        let fdr = Msg::FieldDay {
            to: "W9XYZ".into(),
            de: "K2DEF".into(),
            roger: true,
            class: "12A".into(),
            section: "IL".into(),
        };
        assert_eq!(fdr.to_text(), "W9XYZ K2DEF R 12A IL");
        assert_eq!(Msg::parse("W9XYZ K2DEF R 12A IL"), fdr);

        // Not confused with adjacent forms.
        assert!(matches!(
            Msg::parse("W9XYZ K2DEF R-12"),
            Msg::RReport { .. }
        ));
        assert!(matches!(Msg::parse("CQ FD W9XYZ EN37"), Msg::Cq { .. }));
        assert_eq!(Msg::parse("W9XYZ K2DEF 3A WI").addressee(), Some("W9XYZ"));
        assert_eq!(Msg::parse("W9XYZ K2DEF 3A WI").sender(), Some("K2DEF"));
    }
}

