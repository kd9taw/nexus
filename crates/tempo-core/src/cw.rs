//! CW keyer text: F-key macro expansion + cut numbers, shared by the keyer back-ends
//! (CAT `send_morse`, soundcard, WinKeyer). Pure + deterministic.
//! Casual/ragchew vocabulary (no contest serial/exchange).

/// The substitutions available to a CW macro. All borrowed; empty fields expand to "".
#[derive(Debug, Clone, Copy, Default)]
pub struct CwContext<'a> {
    /// The operator's own callsign — `{MYCALL}`.
    pub mycall: &'a str,
    /// The operator's own name — `{NAME}` (casual ragchew exchange).
    pub myname: &'a str,
    /// The operator's grid — `{MYGRID}`.
    pub mygrid: &'a str,
    /// The operator's US state/province — `{MYSTATE}` (from settings). Empty if unset.
    pub mystate: &'a str,
    /// The station being worked (the selected/active peer) — `!`.
    pub hiscall: &'a str,
    /// The worked station's name — `{HISNAME}` (from the QRZ lookup: nickname or name).
    /// Empty until a lookup resolves for the current call.
    pub hisname: &'a str,
    /// The worked station's US state — `{HISSTATE}` (from the QRZ lookup). Empty if unknown.
    pub hisstate: &'a str,
    /// The report being sent — `{RST}` (cut: 9→N, 0→T, e.g. "599" → "5NN").
    pub rst: &'a str,
    /// The Field Day class exchange — `{CLASS}` (e.g. "3A"). Empty outside Field Day,
    /// so an FD token collapses to nothing.
    pub class: &'a str,
    /// The Field Day ARRL/RAC section — `{SECTION}` (e.g. "WI"). Empty outside Field Day.
    pub section: &'a str,
}

/// Expand a CW macro template into the literal text to key. Recognized tokens:
/// `{MYCALL}` `{NAME}` `{MYGRID}` `{MYSTATE}` `{RST}` (cut numbers), `!` (worked call),
/// `{HISNAME}` / `{HISSTATE}` (the worked station's QRZ name/state), and the Field Day
/// exchange `{CLASS}` / `{SECTION}` / `{EXCH}` (= "`{CLASS} {SECTION}`", e.g. "3A WI").
/// The FD tokens are empty outside Field Day, and `{HISNAME}`/`{HISSTATE}` are empty until a
/// QRZ lookup resolves for the current call — an empty token collapses to nothing (like an
/// unfilled `{NAME}`). Unknown `{...}` tokens are left as-is so typos are visible rather than
/// silently dropped. Plain typed text (no tokens) passes through unchanged.
pub fn expand(template: &str, ctx: &CwContext) -> String {
    let mut out = template.to_string();
    // Order matters only in that {RST} is cut; the rest are literal substitutions. The
    // brace-delimited tokens are distinct literals (e.g. "{HISNAME}" never contains "{NAME}"),
    // so replacement order among them is irrelevant.
    out = out.replace("{MYCALL}", ctx.mycall);
    out = out.replace("{NAME}", ctx.myname);
    out = out.replace("{MYGRID}", ctx.mygrid);
    out = out.replace("{MYSTATE}", ctx.mystate);
    out = out.replace("{HISNAME}", ctx.hisname);
    out = out.replace("{HISSTATE}", ctx.hisstate);
    out = out.replace("{RST}", &cut_numbers(ctx.rst));
    // Field Day exchange. `{EXCH}` is the full "CLASS SECTION"; trimmed so it (and the
    // whitespace-collapse below) leaves nothing when not operating FD (both fields blank).
    out = out.replace("{CLASS}", ctx.class);
    out = out.replace("{SECTION}", ctx.section);
    let exch = format!("{} {}", ctx.class, ctx.section);
    out = out.replace("{EXCH}", exch.trim());
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

/// The key-down / key-up schedule for `text` as Morse — the timing the **serial
/// keyline** back-end walks to toggle a DTR/RTS line (the rig, in CW mode, shapes the
/// actual envelope). Each entry is `(key_down, duration_ms)`. PARIS timing: dit =
/// 1.2/wpm s, dah = 3 dits, intra-char gap = 1 dit, inter-char = 3 dits, word = 7 dits.
/// Unsupported chars are skipped. Empty/blank text → empty schedule. The first entry is
/// always a key-down (no leading gap); there is no trailing gap.
pub fn morse_key_events(text: &str, wpm: u32) -> Vec<(bool, u32)> {
    let dit = (1200 / wpm.clamp(5, 60)).max(1);
    let mut ev: Vec<(bool, u32)> = Vec::new();
    // Append a gap only between real elements (never leading), coalescing adjacent gaps.
    let gap = |ev: &mut Vec<(bool, u32)>, ms: u32| {
        if ev.is_empty() {
            return;
        }
        match ev.last_mut() {
            Some(last) if !last.0 => last.1 += ms,
            _ => ev.push((false, ms)),
        }
    };
    for word in text.split_whitespace() {
        gap(&mut ev, 7 * dit); // inter-word (no-op before the first word)
        let mut first_char = true;
        for ch in word.chars() {
            let Some(code) = morse_code(ch) else { continue };
            if !first_char {
                gap(&mut ev, 3 * dit); // inter-character
            }
            first_char = false;
            for (i, el) in code.chars().enumerate() {
                if i > 0 {
                    gap(&mut ev, dit); // intra-character
                }
                ev.push((true, if el == '-' { 3 * dit } else { dit }));
            }
        }
    }
    ev
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

/// Keying duration of `text` as Morse at `wpm`, in milliseconds — the exact time
/// [`morse_samples`] would produce (dit = 1200/wpm ms; dah 3, intra-char gap 1,
/// inter-char gap 3, inter-word gap 7). The radio loop uses this to pace a word-by-word
/// CW send so at most one word sits in the rig's keyer buffer (Stop TX can drop the rest).
pub fn morse_duration_ms(text: &str, wpm: u32) -> f64 {
    let unit = 1200.0 / wpm.clamp(5, 60) as f64;
    let mut units = 0.0f64;
    for (wi, word) in text.split_whitespace().enumerate() {
        if wi > 0 {
            units += 7.0; // word gap
        }
        for (ci, ch) in word.chars().enumerate() {
            let Some(code) = morse_code(ch) else { continue };
            if ci > 0 {
                units += 3.0; // inter-character gap
            }
            for (ei, el) in code.chars().enumerate() {
                if ei > 0 {
                    units += 1.0; // intra-character gap
                }
                units += if el == '-' { 3.0 } else { 1.0 };
            }
        }
    }
    units * unit
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
            myname: "SETH",
            mygrid: "EN61",
            mystate: "WI",
            hiscall: "K2DEF",
            hisname: "BOB",
            hisstate: "OH",
            rst: "599",
            class: "3A",
            section: "WI",
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
            "K2DEF DE W9XYZ UR 5NN 5NN NAME SETH SETH HW? K2DEF",
        );
        assert_eq!(expand("{MYGRID}", &c), "EN61");
    }

    #[test]
    fn expands_state_and_worked_station_tokens() {
        let c = ctx();
        // My QTH + addressing the other op by his QRZ name.
        assert_eq!(
            expand(
                "! DE {MYCALL} UR {RST} QTH {MYSTATE} NAME {NAME} HW CPY BOB? !",
                &c
            ),
            "K2DEF DE W9XYZ UR 5NN QTH WI NAME SETH HW CPY BOB? K2DEF",
        );
        assert_eq!(expand("R FB {HISNAME}", &c), "R FB BOB");
        assert_eq!(expand("{HISSTATE}", &c), "OH");
        // {HISNAME}/{HISSTATE}/{MYSTATE} are empty until known → collapse cleanly, no double space.
        let bare = CwContext {
            mycall: "W9XYZ",
            hiscall: "K2DEF",
            ..Default::default()
        };
        assert_eq!(expand("R TU {HISNAME} DE {MYCALL}", &bare), "R TU DE W9XYZ");
        assert_eq!(expand("QTH {MYSTATE}", &bare), "QTH");
        // {HISNAME} must NOT be touched by the {NAME} substitution.
        assert_eq!(expand("{NAME} vs {HISNAME}", &c), "SETH vs BOB");
    }

    #[test]
    fn expands_the_field_day_exchange_tokens() {
        let c = ctx();
        assert_eq!(expand("{CLASS}", &c), "3A");
        assert_eq!(expand("{SECTION}", &c), "WI");
        assert_eq!(expand("{EXCH}", &c), "3A WI");
        // A default FD call macro sends the exchange twice for copy.
        assert_eq!(
            expand("! DE {MYCALL} {EXCH} {EXCH} K", &c),
            "K2DEF DE W9XYZ 3A WI 3A WI K",
        );
        // Outside Field Day class/section are empty → the FD tokens collapse cleanly,
        // just like an unfilled {NAME}, leaving no stray double space.
        let off = CwContext {
            mycall: "W9XYZ",
            ..Default::default()
        };
        assert_eq!(expand("{CLASS}", &off), "");
        assert_eq!(expand("{SECTION}", &off), "");
        assert_eq!(expand("{EXCH}", &off), "");
        assert_eq!(expand("! DE {MYCALL} {EXCH} K", &off), "DE W9XYZ K");
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
    fn morse_key_events_paris_timing() {
        let dit = 60u32; // 1200/20 at 20 wpm
        assert_eq!(morse_key_events("E", 20), vec![(true, dit)]); // single dit
        assert_eq!(morse_key_events("T", 20), vec![(true, 3 * dit)]); // single dah
        // "A" = ".-": dit, intra-char gap (1 dit), dah
        assert_eq!(
            morse_key_events("A", 20),
            vec![(true, dit), (false, dit), (true, 3 * dit)]
        );
        // "EE" (one word): dit, inter-char gap (3 dits), dit
        assert_eq!(
            morse_key_events("EE", 20),
            vec![(true, dit), (false, 3 * dit), (true, dit)]
        );
        // "E E" (two words): dit, word gap (7 dits), dit — no leading/trailing gap
        assert_eq!(
            morse_key_events("E E", 20),
            vec![(true, dit), (false, 7 * dit), (true, dit)]
        );
        // blank / empty → nothing; unsupported chars skipped; first event is always key-DOWN
        assert!(morse_key_events("   ", 20).is_empty());
        assert!(morse_key_events("", 20).is_empty());
        assert_eq!(morse_key_events("E#", 20), vec![(true, dit)]); // '#' skipped
        assert!(morse_key_events("CQ", 20).first().unwrap().0, "no leading gap");
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

    #[test]
    fn morse_duration_matches_the_paris_calibration() {
        // "PARIS" is the classic WPM yardstick = 50 dit-units WITH its trailing word space;
        // the word's own keying (no trailing space) is 43 units. At 20 WPM a dit is 60 ms.
        assert!((morse_duration_ms("PARIS", 20) - 43.0 * 60.0).abs() < 1e-6);
        // "E" is one dit; two words insert a 7-dit gap: "E E" = 1 + 7 + 1 = 9 units.
        assert!((morse_duration_ms("E", 20) - 60.0).abs() < 1e-6);
        assert!((morse_duration_ms("E E", 20) - 9.0 * 60.0).abs() < 1e-6);
        // Duration scales inversely with speed: 40 WPM is half the time of 20.
        assert!(
            (morse_duration_ms("PARIS", 40) - morse_duration_ms("PARIS", 20) / 2.0).abs() < 1e-6
        );
    }
}
