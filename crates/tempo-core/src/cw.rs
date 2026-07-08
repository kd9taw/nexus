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

/// International Morse for a character (lowercased), as a dit/dah (`.`/`-`) string;
/// `None` for an unsupported char (skipped when keying). Covers A–Z, 0–9, and the
/// punctuation a casual op uses (`. , ? / = + - ( ) : ;`).
pub fn morse_code(ch: char) -> Option<&'static str> {
    Some(match ch.to_ascii_lowercase() {
        'a' => ".-",
        'b' => "-...",
        'c' => "-.-.",
        'd' => "-..",
        'e' => ".",
        'f' => "..-.",
        'g' => "--.",
        'h' => "....",
        'i' => "..",
        'j' => ".---",
        'k' => "-.-",
        'l' => ".-..",
        'm' => "--",
        'n' => "-.",
        'o' => "---",
        'p' => ".--.",
        'q' => "--.-",
        'r' => ".-.",
        's' => "...",
        't' => "-",
        'u' => "..-",
        'v' => "...-",
        'w' => ".--",
        'x' => "-..-",
        'y' => "-.--",
        'z' => "--..",
        '0' => "-----",
        '1' => ".----",
        '2' => "..---",
        '3' => "...--",
        '4' => "....-",
        '5' => ".....",
        '6' => "-....",
        '7' => "--...",
        '8' => "---..",
        '9' => "----.",
        '.' => ".-.-.-",
        ',' => "--..--",
        '?' => "..--..",
        '/' => "-..-.",
        '=' => "-...-",
        '+' => ".-.-.",
        '-' => "-....-",
        '(' => "-.--.",
        ')' => "-.--.-",
        ':' => "---...",
        ';' => "-.-.-.",
        '@' => ".--.-.",
        _ => return None,
    })
}

/// Generate keyed-tone PCM (mono f32 in -1..1) for `text` as Morse — the **soundcard
/// CW** back-end (rig in USB; the app keys an audio tone). PARIS timing: dit =
/// 1.2/wpm s, dah = 3 dits, intra-char gap = 1 dit, inter-char = 3 dits, word = 7 dits.
/// A ~5 ms raised-cosine rise/fall on every element kills key clicks. `pitch_hz` is the
/// CW tone (e.g. 600). Empty text → empty buffer.
pub fn morse_samples(text: &str, wpm: u32, pitch_hz: f32, sample_rate: u32) -> Vec<f32> {
    let wpm = wpm.clamp(5, 60) as f32;
    let sr = sample_rate.max(1) as f32;
    let dit_n = ((1.2 / wpm) * sr).max(1.0) as usize;
    let ramp_n = (((0.005 * sr) as usize).max(1)).min(dit_n / 2 + 1);
    let step = 2.0 * std::f32::consts::PI * pitch_hz / sr;
    let mut out = Vec::new();
    let mut phase = 0.0f32;
    for (wi, word) in text.split_whitespace().enumerate() {
        if wi > 0 {
            append_gap(&mut out, dit_n * 7); // word gap
        }
        for (ci, ch) in word.chars().enumerate() {
            let Some(code) = morse_code(ch) else { continue };
            if ci > 0 {
                append_gap(&mut out, dit_n * 3); // inter-character gap
            }
            for (ei, el) in code.chars().enumerate() {
                if ei > 0 {
                    append_gap(&mut out, dit_n); // intra-character gap (1 dit)
                }
                let dits = if el == '-' { 3 } else { 1 };
                append_tone(&mut out, dit_n * dits, ramp_n, step, &mut phase);
            }
        }
    }
    out
}

fn append_gap(out: &mut Vec<f32>, n: usize) {
    out.extend(std::iter::repeat_n(0.0, n));
}

fn append_tone(out: &mut Vec<f32>, n: usize, ramp_n: usize, step: f32, phase: &mut f32) {
    for i in 0..n {
        // Raised-cosine envelope on the leading/trailing `ramp_n` samples.
        let env = if i < ramp_n {
            0.5 - 0.5 * (std::f32::consts::PI * i as f32 / ramp_n as f32).cos()
        } else if i + ramp_n >= n {
            0.5 - 0.5 * (std::f32::consts::PI * (n - 1 - i) as f32 / ramp_n as f32).cos()
        } else {
            1.0
        };
        out.push(env * phase.sin());
        *phase += step;
    }
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
            myname: "PAT",
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
        assert_eq!(
            expand("CQ CQ DE {MYCALL} {MYCALL} K", &c),
            "CQ CQ DE W9XYZ W9XYZ K"
        );
        assert_eq!(expand("{MYCALL}", &c), "W9XYZ");
        // The default F2 answer macro.
        assert_eq!(
            expand("! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? !", &c),
            "K2DEF DE W9XYZ UR 5NN 5NN NAME PAT PAT HW? K2DEF",
        );
        assert_eq!(expand("{MYGRID}", &c), "EN61");
    }

    #[test]
    fn empty_fields_do_not_leave_double_spaces() {
        let c = CwContext {
            mycall: "W9XYZ",
            ..Default::default()
        };
        // No worked call yet and no name → tokens collapse cleanly.
        assert_eq!(expand("! DE {MYCALL} NAME {NAME} K", &c), "DE W9XYZ NAME K");
    }

    #[test]
    fn plain_text_passes_through() {
        let c = ctx();
        assert_eq!(expand("RR FB ON THE NICE SIG", &c), "RR FB ON THE NICE SIG");
    }

    #[test]
    fn morse_code_table_basics() {
        assert_eq!(morse_code('E'), Some("."));
        assert_eq!(morse_code('t'), Some("-")); // case-insensitive
        assert_eq!(morse_code('5'), Some("....."));
        assert_eq!(morse_code('?'), Some("..--.."));
        assert_eq!(morse_code(' '), None); // spaces are word gaps, not a glyph
        assert_eq!(morse_code('#'), None);
    }

    #[test]
    fn morse_samples_have_correct_timing_and_amplitude() {
        let sr = 12_000u32;
        let wpm = 20u32;
        let dit_n = ((1.2 / wpm as f32) * sr as f32) as usize; // 720 samples

        // "E" = one dit.
        assert_eq!(morse_samples("E", wpm, 600.0, sr).len(), dit_n);
        // "T" = one dah = 3 dits.
        assert_eq!(morse_samples("T", wpm, 600.0, sr).len(), 3 * dit_n);
        // "EE" = dit + 3-dit inter-char gap + dit = 5 dits.
        assert_eq!(morse_samples("EE", wpm, 600.0, sr).len(), 5 * dit_n);
        // Two words add a 7-dit word gap: "E E" = dit + 7-dit gap + dit = 9 dits.
        assert_eq!(morse_samples("E E", wpm, 600.0, sr).len(), 9 * dit_n);

        // Samples stay in range and the envelope starts/ends near zero (no click).
        let s = morse_samples("E", wpm, 600.0, sr);
        assert!(s.iter().all(|&x| (-1.0..=1.0).contains(&x)));
        assert!(s[0].abs() < 0.1, "soft attack");
        assert!(s[s.len() - 1].abs() < 0.1, "soft decay");

        assert!(morse_samples("", wpm, 600.0, sr).is_empty());
        assert!(
            morse_samples("#", wpm, 600.0, sr).is_empty(),
            "unsupported char keys nothing"
        );
    }
}
