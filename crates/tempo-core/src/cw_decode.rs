//! Single-signal CW decoder — reads Morse from the receive audio at the operator's CW
//! pitch. Pipeline: Goertzel envelope at the pitch → adaptive threshold → mark/space
//! segments → adaptive unit (WPM) → dit/dah + gap classification → Morse → text.
//!
//! Pure + deterministic. Tuned + tested against [`crate::cw::morse_samples`] (clean,
//! machine-timed CW); weak/hand-sent signals are the expected on-air tuning frontier.

use crate::cw::morse_code;
use crate::spectrum::tone_power;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Reverse of [`morse_code`]: a Morse string ("-.-.") → its character. Built once from
/// the forward table (so it stays in sync with whatever glyphs the table supports).
fn morse_to_char(code: &str) -> Option<char> {
    static REV: OnceLock<HashMap<&'static str, char>> = OnceLock::new();
    let map = REV.get_or_init(|| {
        let mut m = HashMap::new();
        for u in 0x20u8..0x7f {
            let c = (u as char).to_ascii_uppercase();
            if let Some(code) = morse_code(c) {
                m.entry(code).or_insert(c);
            }
        }
        m
    });
    map.get(code).copied()
}

/// A CW decode result: the recovered text and the estimated sending speed (WPM).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CwDecode {
    pub text: String,
    pub wpm: u32,
}

/// Decode CW from `samples` (mono f32) heard at `pitch_hz`, sampled at `sr` Hz.
/// Returns empty text when there's no clear keyed signal in the buffer.
pub fn decode_cw(samples: &[f32], sr: f32, pitch_hz: f32) -> CwDecode {
    if sr <= 0.0 || samples.len() < (sr * 0.05) as usize {
        return CwDecode::default(); // < ~50 ms — nothing to decode
    }
    // 1. Envelope: non-overlapping ~4 ms hops of Goertzel power at the pitch.
    let hop = (sr * 0.004).max(1.0) as usize;
    let env: Vec<f32> = samples
        .chunks(hop)
        .filter(|c| c.len() == hop)
        .map(|c| tone_power(c, sr, pitch_hz))
        .collect();
    if env.len() < 8 {
        return CwDecode::default();
    }
    // 2. Threshold: midpoint between the noise floor (low percentile) and the signal
    //    (high percentile). Require a clear on/off ratio, else there's no keying.
    let mut sorted = env.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo = sorted[sorted.len() / 10].max(1e-9);
    let hi = sorted[sorted.len() * 9 / 10];
    if hi < lo * 3.0 {
        return CwDecode::default(); // no clear keyed signal (steady noise/carrier/silence)
    }
    let thresh = (lo + hi) * 0.5;
    // 3. Segments: runs of mark (key-down) / space (key-up), in hops.
    let mut segs: Vec<(bool, usize)> = Vec::new();
    for &p in &env {
        let mark = p >= thresh;
        match segs.last_mut() {
            Some((m, n)) if *m == mark => *n += 1,
            _ => segs.push((mark, 1)),
        }
    }
    // Trim leading silence so a long pre-signal gap can't emit spurious spaces.
    let start = segs.iter().position(|(m, _)| *m).unwrap_or(segs.len());
    let segs = &segs[start..];
    if segs.is_empty() {
        return CwDecode::default();
    }
    // 4. Adaptive unit (1 dit, in hops): a low percentile of all element durations —
    //    dits and intra-character gaps are the shortest, most-common elements (1 unit).
    let mut durs: Vec<usize> = segs.iter().map(|(_, n)| *n).collect();
    durs.sort_unstable();
    let rough = (durs[durs.len() / 5].max(1)) as f32;
    // Refine the dit length by averaging the whole 1-unit cluster (elements < 2× rough):
    // threshold-crossing trims a hop off each dit but adds it to the adjacent intra-char
    // gap, so the dit-and-gap mean cancels that bias → an accurate WPM.
    let ones: Vec<usize> = durs.iter().copied().filter(|&d| (d as f32) < 2.0 * rough).collect();
    let unit = if ones.is_empty() {
        rough
    } else {
        ones.iter().sum::<usize>() as f32 / ones.len() as f32
    };
    // 5. Decode: marks → dit/dah (boundary 2 units); spaces → intra (<2u) / inter (2–5u,
    //    emit the character) / word (≥5u, also a space).
    let mut text = String::new();
    let mut sym = String::new();
    let flush = |sym: &mut String, text: &mut String| {
        if !sym.is_empty() {
            if let Some(c) = morse_to_char(sym) {
                text.push(c);
            }
            sym.clear();
        }
    };
    for (mark, n) in segs {
        let u = *n as f32 / unit;
        if *mark {
            sym.push(if u < 2.0 { '.' } else { '-' });
        } else if u >= 2.0 {
            flush(&mut sym, &mut text);
            if u >= 5.0 {
                text.push(' ');
            }
        }
    }
    flush(&mut sym, &mut text);
    // WPM from the dit unit: dit_secs = unit_hops × hop / sr; PARIS wpm = 1.2 / dit_secs.
    let dit_secs = unit * hop as f32 / sr;
    let wpm = if dit_secs > 0.0 {
        (1.2 / dit_secs).round().clamp(0.0, 99.0) as u32
    } else {
        0
    };
    CwDecode {
        text: text.trim().to_string(),
        wpm,
    }
}

/// A CW signal found by the wideband skimmer: its audio pitch (Hz), decoded text, and WPM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkimHit {
    pub pitch_hz: u32,
    pub text: String,
    pub wpm: u32,
}

/// Wideband CW skimmer: decode `samples` at every audio pitch from `lo_hz` to `hi_hz` in
/// `step_hz` steps and return the channels that yielded readable text. A single signal
/// smears across a few neighbouring channels (the Goertzel has finite bandwidth), so runs
/// of adjacent channels are merged, keeping the longest (best-centred) decode. This reuses
/// the single-signal [`decode_cw`] per channel — Tier 2 of CW decode.
pub fn skim_cw(samples: &[f32], sr: f32, lo_hz: u32, hi_hz: u32, step_hz: u32) -> Vec<SkimHit> {
    let step = step_hz.max(1) as usize;
    let channels: Vec<u32> = (lo_hz..=hi_hz).step_by(step).collect();
    if channels.len() < 3 {
        return Vec::new();
    }
    // 1. Total Goertzel power per channel over the whole buffer — the band's spectrum.
    let powers: Vec<f32> = channels
        .iter()
        .map(|&f| tone_power(samples, sr, f as f32))
        .collect();
    // 2. Noise floor = median power; a real carrier peak stands well above it. Decoding
    //    every channel (not just peaks) is what let one signal's Goertzel sidelobes post
    //    garbage on far-off channels — so decode ONLY at local-maximum peaks above the floor.
    let mut sorted = powers.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = sorted[sorted.len() / 2].max(1e-9);
    let mut hits: Vec<SkimHit> = Vec::new();
    for i in 0..channels.len() {
        let p = powers[i];
        let peak = p > floor * 4.0
            && powers.get(i.wrapping_sub(1)).is_none_or(|&l| p >= l)
            && powers.get(i + 1).is_none_or(|&r| p >= r);
        if !peak {
            continue;
        }
        let d = decode_cw(samples, sr, channels[i] as f32);
        let glyphs = d.text.chars().filter(|c| !c.is_whitespace()).count();
        if glyphs >= 3 && (8..=50).contains(&d.wpm) && !hits.iter().any(|h| h.text == d.text) {
            hits.push(SkimHit {
                pitch_hz: channels[i],
                text: d.text,
                wpm: d.wpm,
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cw::morse_samples;

    const SR: f32 = 48_000.0;
    const PITCH: f32 = 600.0;

    fn decode(text: &str, wpm: u32) -> CwDecode {
        // Pad with leading + trailing silence (a real capture is never perfectly trimmed).
        let mut audio = vec![0.0f32; (SR * 0.1) as usize];
        audio.extend(morse_samples(text, wpm, PITCH, SR as u32));
        audio.extend(vec![0.0f32; (SR * 0.1) as usize]);
        decode_cw(&audio, SR, PITCH)
    }

    #[test]
    fn decodes_a_clean_callsign_and_estimates_wpm() {
        let d = decode("CQ TEST DE W1ABC", 20);
        assert_eq!(d.text, "CQ TEST DE W1ABC");
        assert!((d.wpm as i32 - 20).abs() <= 2, "≈20 wpm, got {}", d.wpm);
    }

    #[test]
    fn decodes_across_speeds() {
        assert_eq!(decode("PARIS", 15).text, "PARIS");
        assert_eq!(decode("599 TU", 25).text, "599 TU");
        assert_eq!(decode("K", 30).text, "K");
    }

    #[test]
    fn empty_on_silence_and_steady_tone() {
        assert_eq!(decode_cw(&vec![0.0f32; 48_000], SR, PITCH), CwDecode::default());
        // A steady (un-keyed) carrier — no on/off ratio → nothing to decode.
        let steady: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * PITCH * i as f32 / SR).sin())
            .collect();
        assert_eq!(decode_cw(&steady, SR, PITCH).text, "");
    }

    #[test]
    fn skimmer_finds_a_signal_at_its_pitch() {
        let mut audio = vec![0.0f32; (SR * 0.1) as usize];
        audio.extend(morse_samples("CQ TEST", 22, 600.0, SR as u32));
        let hits = skim_cw(&audio, SR, 300, 1200, 50);
        assert!(!hits.is_empty(), "found the signal");
        assert!(hits.iter().any(|h| h.text == "CQ TEST"), "decoded the text: {hits:?}");
        // Decoding channels cluster near the 600 Hz tone — no spurious far-off hits.
        assert!(
            hits.iter().all(|h| (h.pitch_hz as i32 - 600).abs() <= 300),
            "clustered near 600 Hz: {hits:?}"
        );
    }

    #[test]
    fn skimmer_empty_on_silence() {
        assert!(skim_cw(&vec![0.0f32; 48_000], SR, 300, 1200, 50).is_empty());
    }

    #[test]
    fn morse_to_char_reverses_the_table() {
        assert_eq!(morse_to_char("."), Some('E'));
        assert_eq!(morse_to_char("-.-."), Some('C'));
        assert_eq!(morse_to_char("....."), Some('5'));
        assert_eq!(morse_to_char(".-.-"), None); // not a glyph
    }
}
