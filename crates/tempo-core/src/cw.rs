//! CW keyer text: F-key macro expansion + cut numbers, shared by the keyer back-ends
//! (CAT `send_morse`, soundcard, WinKeyer). Pure + deterministic — see
//! `tasks/specs/cw-operating.md`. Casual/ragchew vocabulary (no contest serial/exchange).

/// The substitutions available to a CW macro. All borrowed; empty fields expand to "".
#[derive(Debug, Clone, Copy, Default)]
pub struct CwContext<'a> {
    /// The operator's own callsign — `{MYCALL}`.
    pub mycall: &'a str,
    /// The operator's own name — `{NAME}` (casual ragchew exchange).
    pub myname: &'a str,
    /// The operator's grid — `{MYGRID}`.
    pub mygrid: &'a str,
    /// The station being worked (the selected/active peer) — `!`.
    pub hiscall: &'a str,
    /// The report being sent — `{RST}` (cut: 9→N, 0→T, e.g. "599" → "5NN").
    pub rst: &'a str,
}

/// Expand a CW macro template into the literal text to key. Recognized tokens:
/// `{MYCALL}` `{NAME}` `{MYGRID}` `{RST}` (cut numbers) and `!` (worked call). Unknown
/// `{...}` tokens are left as-is so typos are visible rather than silently dropped.
/// Plain typed text (no tokens) passes through unchanged.
pub fn expand(template: &str, ctx: &CwContext) -> String {
    let mut out = template.to_string();
    // Order matters only in that {RST} is cut; the rest are literal substitutions.
    out = out.replace("{MYCALL}", ctx.mycall);
    out = out.replace("{NAME}", ctx.myname);
    out = out.replace("{MYGRID}", ctx.mygrid);
    out = out.replace("{RST}", &cut_numbers(ctx.rst));
    // `!` = the worked station's call (N1MM/WinWarbler convention).
    out = out.replace('!', ctx.hiscall);
    // Collapse runs of whitespace a substitution may have left (e.g. empty {NAME}),
    // and trim, so an unfilled token doesn't leave a double space mid-message.
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
}

/// Casual CW "cut numbers": speed up the report by sending letters for the common
/// digits — 9→N, 0→T (so "599" → "5NN", "579" → "57N"). Other digits are left alone
/// (casual ops rarely cut 1→A/5→E). Non-digits pass through.
pub fn cut_numbers(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '9' => 'N',
            '0' => 'T',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>() -> CwContext<'a> {
        CwContext {
            mycall: "W9XYZ",
            myname: "SETH",
            mygrid: "EN61",
            hiscall: "K2DEF",
            rst: "599",
        }
    }

    #[test]
    fn cut_numbers_only_touches_9_and_0() {
        assert_eq!(cut_numbers("599"), "5NN");
        assert_eq!(cut_numbers("579"), "57N");
        assert_eq!(cut_numbers("000"), "TTT");
        assert_eq!(cut_numbers("123"), "123"); // casual cut leaves these
    }

    #[test]
    fn expands_the_casual_tokens() {
        let c = ctx();
        assert_eq!(expand("CQ CQ DE {MYCALL} {MYCALL} K", &c), "CQ CQ DE W9XYZ W9XYZ K");
        assert_eq!(expand("{MYCALL}", &c), "W9XYZ");
        // The default F2 answer macro.
        assert_eq!(
            expand("! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? !", &c),
            "K2DEF DE W9XYZ UR 5NN 5NN NAME SETH SETH HW? K2DEF",
        );
        assert_eq!(expand("{MYGRID}", &c), "EN61");
    }

    #[test]
    fn empty_fields_do_not_leave_double_spaces() {
        let c = CwContext { mycall: "W9XYZ", ..Default::default() };
        // No worked call yet and no name → tokens collapse cleanly.
        assert_eq!(expand("! DE {MYCALL} NAME {NAME} K", &c), "DE W9XYZ NAME K");
    }

    #[test]
    fn plain_text_passes_through() {
        let c = ctx();
        assert_eq!(expand("RR FB ON THE NICE SIG", &c), "RR FB ON THE NICE SIG");
    }
}
