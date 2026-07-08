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
    let ones: Vec<usize> = durs
        .iter()
        .copied()
        .filter(|&d| (d as f32) < 2.0 * rough)
        .collect();
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

const CW_TRANSCRIPT_CAP: usize = 4000; // keep the tail; drop older text
                                       // Every noise gate is operator-scaled by the sensitivity slider — the Schmitt keying
                                       // rails (hi_frac/lo_frac), the sub-dit spike fraction (spike_frac), the per-mark SNR +
                                       // presence multipliers (snr_mult/present_mult), and the Morse-timing plausibility
                                       // squelch (timing_plausible). 0.5 = the historical fixed behavior.

/// Streaming, stateful CW decoder — the persistent-transcript sibling of [`decode_cw`].
///
/// Feed it receive audio incrementally (as it arrives) and it ACCUMULATES decoded text
/// that doesn't vanish when old audio scrolls off: the batch [`decode_cw`] re-reads a
/// sliding window every call, so its output churns and the last characters disappear
/// within seconds. This one emits each character once, into a growing transcript.
///
/// Pipeline per ~4 ms envelope hop, hardened toward what fldigi/CW-Skimmer do:
/// Goertzel power at the pitch → **fast-attack peak / slow noise-floor AGC** (flags a
/// signal on its first hop; adapts to fading) →
/// **Schmitt-trigger** key state (two thresholds → no chatter near the noise floor) →
/// mark/space runs with **sub-dit spike rejection** → **adaptive dit unit** tracked from
/// a rolling mark history (follows the sender's speed) → dit/dah + gap → Morse → char.
pub struct CwStreamDecoder {
    sr: f32,
    pitch: f32,
    hop: usize,
    carry: Vec<f32>, // audio not yet a full hop
    noise: f32,      // slow-tracking noise-floor follower (AGC)
    peak: f32,       // fast-attack, slow-decay signal-peak follower (AGC)
    mark_peak: f32,  // peak power within the current key-down (per-mark SNR gate)
    lo_thresh: f32,  // Schmitt lower rail (leave mark below this)
    hi_thresh: f32,  // Schmitt upper rail (enter mark at/above this)
    present: bool,   // a keyed signal is present (else don't decode noise)
    key_down: bool,
    run: usize,         // current key-run length (hops), mark or space per `key_down`
    space_run: usize,   // accumulated gap hops (carries THROUGH sub-dit spikes)
    dit_avg: f32,       // adaptive dit length (hops)
    dah_avg: f32,       // adaptive dah length (hops); the pair classify each mark
    saw_mark: bool,     // any real mark seen yet (gates WPM before sync)
    sym: String,        // in-progress Morse symbol (".-")
    flushed_char: bool, // this gap already flushed its pending character
    flushed_word: bool, // this gap already emitted a word space
    transcript: String,
    sensitivity: f32, // 0..1 operator control; scales the presence + per-mark SNR gates
}

impl CwStreamDecoder {
    /// A decoder for audio sampled at `sr` Hz, listening at `pitch_hz`.
    pub fn new(sr: f32, pitch_hz: f32) -> Self {
        let hop = (sr * 0.004).max(1.0) as usize;
        // Seed the dit unit at ~20 wpm so the first character classifies sensibly before
        // the speed tracker has any marks to learn from; it converges within a few elements.
        let default_dit = ((1.2 / 20.0) * sr / hop.max(1) as f32).max(1.0);
        Self {
            sr,
            pitch: pitch_hz,
            hop,
            carry: Vec::new(),
            noise: 1e-9,
            peak: 0.0,
            mark_peak: 0.0,
            lo_thresh: 0.0,
            hi_thresh: f32::INFINITY,
            present: false,
            key_down: false,
            run: 0,
            space_run: 0,
            dit_avg: default_dit,
            dah_avg: 3.0 * default_dit,
            saw_mark: false,
            sym: String::new(),
            flushed_char: false,
            flushed_word: false,
            transcript: String::new(),
            sensitivity: 0.5, // 0.5 == the historical fixed gates (SNR ×4, presence ×3)
        }
    }

    /// Operator decode sensitivity in [0, 1]. Higher = catch weaker/off-pitch marks (like the
    /// wideband skimmer, at the cost of more noise); lower = stricter. 0.5 keeps the original
    /// gates. Scales the presence gate (×`present_mult`) and the per-mark SNR gate.
    pub fn set_sensitivity(&mut self, s: f32) {
        self.sensitivity = s.clamp(0.0, 1.0);
    }

    /// Per-mark SNR multiplier: 0→×6 (strict) · 0.5→×4 (default) · 1→×2 (loose).
    fn snr_mult(&self) -> f32 {
        6.0 - 4.0 * self.sensitivity
    }

    /// Presence-gate multiplier: 0→×4 · 0.5→×3 (default) · 1→×2.
    fn present_mult(&self) -> f32 {
        4.0 - 2.0 * self.sensitivity
    }

    /// Schmitt upper-rail fraction of the AGC span: 0→0.85 (strict) · 0.5→0.60
    /// (the historical fixed rail) · 1→0.35 (loose). THIS is the gate that
    /// actually bites on a noisy band: the noise floor follower rides the
    /// envelope MINIMUM, so against real band noise the peak/floor ratio dwarfs
    /// the snr/present multipliers and they saturate at every slider position —
    /// the operator-visible effect must come from how much of the peak−floor
    /// span a mark must climb before it keys.
    fn hi_frac(&self) -> f32 {
        0.85 - 0.5 * self.sensitivity
    }

    /// Schmitt lower rail rides 0.20 below the upper (hysteresis width unchanged).
    fn lo_frac(&self) -> f32 {
        self.hi_frac() - 0.20
    }

    /// Sub-dit spike-rejection fraction: 0→0.55 (strict) · 0.5→0.40 (the historical
    /// CW_SPIKE_FRAC) · 1→0.25 (loose). Noise storms are made of barely-dit marks;
    /// the strict end simply refuses them.
    fn spike_frac(&self) -> f32 {
        0.55 - 0.3 * self.sensitivity
    }

    /// Morse-timing plausibility half-width: real keying holds dah ≈ 3×dit. A noise
    /// storm drags one tracker toward spike length while the other lags → the ratio
    /// leaves the physical band, and characters emitted in that state are muted.
    /// Band = [3/k, 3k]: 0→k 1.6 (strict) · 0.5→k 3.0 (permissive ≈ historical) ·
    /// 1→k 5.8 (anything goes).
    fn timing_plausible(&self) -> bool {
        let k = 1.6 + 2.8 * self.sensitivity;
        let ratio = self.dah_avg / self.dit_avg.max(1e-6);
        (3.0 / k..=3.0 * k).contains(&ratio)
    }

    /// Retune to a new marker pitch. No-op if unchanged; otherwise resets all state +
    /// transcript (the old text belonged to a different signal) but keeps the operator's
    /// sensitivity setting.
    pub fn retune(&mut self, pitch_hz: f32) {
        if (pitch_hz - self.pitch).abs() > 1.0 {
            let s = self.sensitivity;
            *self = Self::new(self.sr, pitch_hz);
            self.sensitivity = s;
        }
    }

    /// The accumulated decoded text.
    pub fn transcript(&self) -> &str {
        &self.transcript
    }

    /// Estimated sending speed (WPM) from the current dit unit; 0 before any marks.
    pub fn wpm(&self) -> u32 {
        if !self.saw_mark {
            return 0;
        }
        let dit_secs = self.dit_avg * self.hop as f32 / self.sr;
        if dit_secs > 0.0 {
            (1.2 / dit_secs).round().clamp(0.0, 99.0) as u32
        } else {
            0
        }
    }

    /// Clear the accumulated text (keeps the learned speed/threshold so decoding of an
    /// in-progress signal continues cleanly).
    pub fn clear(&mut self) {
        self.transcript.clear();
        self.sym.clear();
        self.flushed_char = false;
        self.flushed_word = false;
    }

    /// Feed newly-arrived audio; decoded characters are appended to the transcript.
    pub fn push(&mut self, samples: &[f32]) {
        if self.sr <= 0.0 || self.hop == 0 {
            return;
        }
        self.carry.extend_from_slice(samples);
        let mut i = 0;
        while i + self.hop <= self.carry.len() {
            let p = tone_power(&self.carry[i..i + self.hop], self.sr, self.pitch);
            i += self.hop;
            // Threshold from the CURRENT followers (before updating them) so a signal onset
            // is judged against the PRIOR noise floor and detected on its first hop — no
            // warmup dead-zone (a percentile window can't flag a signal until it fills a
            // fifth of the window, which truncated the first character into a dit).
            let span = (self.peak - self.noise).max(0.0);
            self.present = self.peak >= self.noise * self.present_mult() && span > 1e-9;
            self.hi_thresh = self.noise + self.hi_frac() * span; // Schmitt upper rail
            self.lo_thresh = self.noise + self.lo_frac() * span; // Schmitt lower rail (hysteresis)
            self.step(p);
            // Update the AGC: the peak attacks instantly and decays slowly (holds through
            // inter-character gaps); the noise floor drops instantly to a quieter hop and
            // creeps up slowly (so a long key-down barely lifts it).
            self.peak = if p > self.peak {
                p
            } else {
                self.peak * 0.999 + p * 0.001
            };
            self.noise = if p < self.noise {
                p
            } else {
                self.noise * 0.9995 + p * 0.0005
            };
        }
        self.carry.drain(0..i);
    }

    fn step(&mut self, p: f32) {
        if self.key_down {
            self.run += 1;
            self.mark_peak = self.mark_peak.max(p);
            if p < self.lo_thresh {
                self.on_mark_end();
                self.key_down = false;
                self.run = 0;
            }
        } else if self.present && p >= self.hi_thresh {
            // A real mark starts. Any pending character was already flushed by the gap
            // ticks below, so just begin the mark.
            self.key_down = true;
            self.run = 1;
            self.mark_peak = p;
        } else {
            self.space_run += 1;
            self.on_gap_tick();
        }
    }

    fn on_mark_end(&mut self) {
        let spike_min = (self.spike_frac() * self.dit_avg).max(1.0);
        // Reject a mark that is too SHORT (impulse) OR too WEAK (its peak barely clears the
        // noise floor). The weak case kills the "E E E…" storm the decoder otherwise emits
        // from band noise between signals — a real keyed element sits well above the floor.
        if (self.run as f32) < spike_min || self.mark_peak < self.noise * self.snr_mult() {
            // Not a keyed element — roll its hops into the surrounding gap so it doesn't
            // reset the character timing.
            self.space_run += self.run;
            return;
        }
        self.saw_mark = true;
        let run = self.run as f32;
        // Classify by the midpoint between the tracked dit and dah lengths, then nudge the
        // matching average toward this mark -- a standard adaptive-Morse speed tracker. It
        // follows the sender's speed and, unlike a min-cluster estimate, never lets one
        // early mark redefine the unit as itself (which read every leading dah as a dit).
        if run < (self.dit_avg + self.dah_avg) * 0.5 {
            self.sym.push('.');
            self.dit_avg += 0.25 * (run - self.dit_avg);
        } else {
            self.sym.push('-');
            self.dah_avg += 0.25 * (run - self.dah_avg);
        }
        // A real element begins a fresh gap.
        self.space_run = 0;
        self.flushed_char = false;
        self.flushed_word = false;
    }

    fn on_gap_tick(&mut self) {
        let g = self.space_run as f32;
        // Inter-character gap (≈3 dit): the symbol is complete → decode + emit it —
        // unless the tracked timing is outside plausible Morse (the storm squelch;
        // scaled by the sensitivity slider). The trackers keep adapting either way,
        // so a real sender re-opens the gate within a few elements.
        if !self.flushed_char && !self.sym.is_empty() && g >= 2.0 * self.dit_avg {
            if self.timing_plausible() {
                if let Some(c) = morse_to_char(&self.sym) {
                    self.push_char(c);
                }
            }
            self.sym.clear();
            self.flushed_char = true;
        }
        // Word gap (≈7 dit): one space between words.
        if !self.flushed_word && self.flushed_char && g >= 5.0 * self.dit_avg {
            self.push_char(' ');
            self.flushed_word = true;
        }
    }

    fn push_char(&mut self, c: char) {
        self.transcript.push(c);
        if self.transcript.len() > CW_TRANSCRIPT_CAP {
            let drop = self.transcript.len() - CW_TRANSCRIPT_CAP;
            self.transcript.drain(0..drop);
        }
    }
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
        assert_eq!(
            decode_cw(&vec![0.0f32; 48_000], SR, PITCH),
            CwDecode::default()
        );
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
        assert!(
            hits.iter().any(|h| h.text == "CQ TEST"),
            "decoded the text: {hits:?}"
        );
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

    /// Morse audio with lead + `trail_secs` trailing silence (the trailing gap must be
    /// ≥ ~3 dit for the streaming decoder to flush the final character).
    fn morse_audio(text: &str, wpm: u32, trail_secs: f32) -> Vec<f32> {
        let mut a = vec![0.0f32; (SR * 0.1) as usize];
        a.extend(morse_samples(text, wpm, PITCH, SR as u32));
        a.extend(vec![0.0f32; (SR * trail_secs) as usize]);
        a
    }

    #[test]
    fn stream_decodes_a_callsign_and_estimates_wpm() {
        let mut d = CwStreamDecoder::new(SR, PITCH);
        d.push(&morse_audio("CQ TEST DE W1ABC", 20, 0.6));
        assert_eq!(d.transcript().trim(), "CQ TEST DE W1ABC");
        assert!((d.wpm() as i32 - 20).abs() <= 3, "≈20 wpm, got {}", d.wpm());
    }

    #[test]
    fn stream_accumulates_across_small_chunks() {
        // Fed in tiny slices (as real audio arrives) the transcript must match a single
        // push — proving the incremental state machine carries across chunk boundaries.
        let audio = morse_audio("PARIS DE K1ABC", 25, 0.6);
        let mut d = CwStreamDecoder::new(SR, PITCH);
        for chunk in audio.chunks(1000) {
            d.push(chunk);
        }
        assert_eq!(d.transcript().trim(), "PARIS DE K1ABC");
    }

    #[test]
    fn stream_keeps_earlier_text_when_more_arrives() {
        // The whole point of the rewrite: earlier text does NOT vanish when new audio
        // scrolls the window — the transcript grows.
        let mut d = CwStreamDecoder::new(SR, PITCH);
        d.push(&morse_audio("K", 20, 0.6));
        assert_eq!(d.transcript().trim(), "K");
        d.push(&morse_audio("W1ABC", 20, 0.6));
        assert!(
            d.transcript().contains('K'),
            "kept the earlier K: {:?}",
            d.transcript()
        );
        assert!(
            d.transcript().contains("W1ABC"),
            "added the new call: {:?}",
            d.transcript()
        );
    }

    #[test]
    fn stream_stays_silent_on_noise_and_unkeyed_carrier() {
        let mut d = CwStreamDecoder::new(SR, PITCH);
        d.push(&vec![0.0f32; SR as usize]); // 1 s silence
        assert_eq!(d.transcript(), "");
        // A steady, un-keyed carrier has no on/off ratio → nothing to decode.
        let mut d2 = CwStreamDecoder::new(SR, PITCH);
        let steady: Vec<f32> = (0..SR as usize)
            .map(|i| (2.0 * std::f32::consts::PI * PITCH * i as f32 / SR).sin())
            .collect();
        d2.push(&steady);
        assert_eq!(d2.transcript(), "");
    }

    /// Static-crash storm: the tone randomly keyed with sub-dit-to-dit-scale
    /// bursts — the classic "ton of false characters" band condition.
    fn crash_storm(secs: f32) -> Vec<f32> {
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / 16_777_216.0
        };
        let n = (SR * secs) as usize;
        let mut audio = vec![0.0f32; n];
        let mut i = 0usize;
        while i < n {
            let on = (SR * (0.004 + 0.05 * rnd())) as usize; // 4–54 ms burst
            let off = (SR * (0.01 + 0.08 * rnd())) as usize; // 10–90 ms gap
            let amp = 0.3 + 0.7 * rnd();
            for j in 0..on.min(n - i) {
                audio[i + j] =
                    amp * (2.0 * std::f32::consts::PI * PITCH * (i + j) as f32 / SR).sin();
            }
            i += on + off;
        }
        audio
    }

    fn storm_glyphs(sens: f32) -> usize {
        let mut d = CwStreamDecoder::new(SR, PITCH);
        d.set_sensitivity(sens);
        d.push(&crash_storm(12.0));
        d.transcript()
            .chars()
            .filter(|c| !c.is_whitespace())
            .count()
    }

    #[test]
    fn sensitivity_slider_visibly_gates_a_noise_storm() {
        // The operator-reported bug: sliding the CW sensitivity did NOTHING to a
        // false-character storm (all gates saturated against the min-following
        // noise floor). The slider now drives the Schmitt rails, the spike gate,
        // and the timing-plausibility squelch — each end must be VISIBLY
        // different on the same storm.
        let strict = storm_glyphs(0.0);
        let default = storm_glyphs(0.5);
        let loose = storm_glyphs(1.0);
        assert!(
            strict * 2 < default,
            "strict must at least halve the storm: strict={strict} default={default}"
        );
        assert!(
            default < loose,
            "loose must admit more than default: default={default} loose={loose}"
        );
    }

    #[test]
    fn strict_sensitivity_still_copies_clean_cw() {
        // Turning the slider all the way down must never break solid copy.
        let mut d = CwStreamDecoder::new(SR, PITCH);
        d.set_sensitivity(0.0);
        d.push(&morse_audio("CQ DX DE W1ABC", 22, 0.6));
        assert_eq!(d.transcript().trim(), "CQ DX DE W1ABC");
    }

    #[test]
    fn stream_suppresses_near_noise_marks() {
        // A gappy, LOW-contrast tone whose "on" segments sit only ~3× the floor in power —
        // the kind of thing band noise makes the threshold chase. Without the per-mark SNR
        // gate this decodes into an "E E E…" storm; with it (needs ≥4× the floor) the marks
        // are rejected, so the transcript stays essentially empty. Clean full-amplitude
        // signals (the other tests) are unaffected.
        let mut d = CwStreamDecoder::new(SR, PITCH);
        let dit = (SR * 0.06) as usize; // ~20 wpm element
        let tone = |amp: f32, len: usize| -> Vec<f32> {
            (0..len)
                .map(|i| amp * (2.0 * std::f32::consts::PI * PITCH * i as f32 / SR).sin())
                .collect()
        };
        let mut buf: Vec<f32> = Vec::new();
        for _ in 0..40 {
            buf.extend(tone(0.09, dit)); // "on" ≈ 3.2× the "off" power — below the 4× gate
            buf.extend(tone(0.05, dit)); // "off" = the noise floor
        }
        d.push(&buf);
        let junk = d
            .transcript()
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        assert!(
            junk <= 6,
            "near-noise produced a storm: {:?}",
            d.transcript()
        );
    }

    #[test]
    fn stream_clear_and_retune_reset_text() {
        let mut d = CwStreamDecoder::new(SR, PITCH);
        d.push(&morse_audio("K", 20, 0.6));
        assert!(!d.transcript().is_empty());
        d.clear();
        assert_eq!(d.transcript(), "");
        d.push(&morse_audio("E", 20, 0.6)); // "E" = "."
        assert_eq!(d.transcript().trim(), "E");
        d.retune(PITCH + 200.0); // new pitch → full reset
        assert_eq!(d.transcript(), "");
    }
}
