//! Icom CI-V command table — pure encoders/decoders for the CAT-parity verb set, built on
//! [`super::frame`]. No I/O. Covers the 7300-family (IC-7300/7610/9700/705/905), whose
//! command numbers are shared; per-model differences are the CI-V **address** (below) and a
//! few band/mode specifics handled by the caller.
//!
//! A command builder returns a [`Frame`] (`.to_bytes()` for the wire); a decoder takes a
//! *reply* frame and extracts the value. Set commands are acknowledged with a bare
//! `FB`/`FA` ([`Frame::is_ack`]/[`Frame::is_nak`]).

use super::frame::{bcd_to_freq, freq_to_bcd, Frame};

/// The Icom rigs Nexus knows how to drive natively over CI-V, with their factory-default
/// CI-V bus address. (The address is user-changeable on the rig; the serial engine lets the
/// operator override it, defaulting to these.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcomModel {
    Ic7300,
    Ic7610,
    Ic9700,
    Ic705,
    Ic905,
}

impl IcomModel {
    /// The factory-default CI-V address for this model.
    pub fn default_civ_addr(self) -> u8 {
        match self {
            IcomModel::Ic7300 => 0x94,
            IcomModel::Ic7610 => 0x98,
            IcomModel::Ic9700 => 0xA2,
            IcomModel::Ic705 => 0xA4,
            IcomModel::Ic905 => 0xAC,
        }
    }

    /// Recognize a model from a human/rig model name (e.g. "Icom IC-9700"). Case- and
    /// separator-insensitive on the `ic####` token.
    pub fn from_name(name: &str) -> Option<Self> {
        let n: String = name
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        // Longest/most-specific tokens first so "ic905" doesn't shadow nothing, etc.
        for (tok, m) in [
            ("ic7300", IcomModel::Ic7300),
            ("ic7610", IcomModel::Ic7610),
            ("ic9700", IcomModel::Ic9700),
            ("ic705", IcomModel::Ic705),
            ("ic905", IcomModel::Ic905),
        ] {
            if n.contains(tok) {
                return Some(m);
            }
        }
        None
    }
}

/// Operating modes as CI-V mode bytes (command `0x04`/`0x06` payload byte 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Lsb,
    Usb,
    Am,
    Cw,
    Rtty,
    Fm,
    CwR,
    RttyR,
}

impl Mode {
    pub fn to_byte(self) -> u8 {
        match self {
            Mode::Lsb => 0x00,
            Mode::Usb => 0x01,
            Mode::Am => 0x02,
            Mode::Cw => 0x03,
            Mode::Rtty => 0x04,
            Mode::Fm => 0x05,
            Mode::CwR => 0x07,
            Mode::RttyR => 0x08,
        }
    }
    pub fn from_byte(b: u8) -> Option<Mode> {
        Some(match b {
            0x00 => Mode::Lsb,
            0x01 => Mode::Usb,
            0x02 => Mode::Am,
            0x03 => Mode::Cw,
            0x04 => Mode::Rtty,
            0x05 => Mode::Fm,
            0x07 => Mode::CwR,
            0x08 => Mode::RttyR,
            _ => return None,
        })
    }
    /// The canonical uppercase name Nexus/Hamlib use.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Lsb => "LSB",
            Mode::Usb => "USB",
            Mode::Am => "AM",
            Mode::Cw => "CW",
            Mode::Rtty => "RTTY",
            Mode::Fm => "FM",
            Mode::CwR => "CWR",
            Mode::RttyR => "RTTYR",
        }
    }
    pub fn from_name(name: &str) -> Option<Mode> {
        Some(match name.to_ascii_uppercase().as_str() {
            "LSB" => Mode::Lsb,
            "USB" => Mode::Usb,
            "AM" => Mode::Am,
            "CW" => Mode::Cw,
            "RTTY" | "FSK" => Mode::Rtty,
            "FM" => Mode::Fm,
            "CWR" | "CW-R" => Mode::CwR,
            "RTTYR" | "RTTY-R" | "FSKR" => Mode::RttyR,
            _ => return None,
        })
    }
}

// ---- command builders (controller → radio) ----

/// Read the operating frequency (cmd `03`). Reply carries 5-byte BCD freq.
pub fn read_freq(radio: u8) -> Frame {
    Frame::command(radio, 0x03, &[])
}
/// Set the operating frequency (cmd `05`).
pub fn set_freq(radio: u8, hz: u64) -> Frame {
    Frame::command(radio, 0x05, &freq_to_bcd(hz))
}
/// Read the operating mode + filter (cmd `04`).
pub fn read_mode(radio: u8) -> Frame {
    Frame::command(radio, 0x04, &[])
}
/// Set the operating mode (cmd `06`), optionally selecting a filter (1/2/3).
pub fn set_mode(radio: u8, mode: Mode, filter: Option<u8>) -> Frame {
    match filter {
        Some(f) => Frame::command(radio, 0x06, &[mode.to_byte(), f]),
        None => Frame::command(radio, 0x06, &[mode.to_byte()]),
    }
}
/// Read PTT state (cmd `1C 00`). Reply data `[0x00, 0x00|0x01]`.
pub fn read_ptt(radio: u8) -> Frame {
    Frame::command(radio, 0x1C, &[0x00])
}
/// Set PTT (cmd `1C 00`): `tx=true` keys the transmitter.
pub fn set_ptt(radio: u8, tx: bool) -> Frame {
    Frame::command(radio, 0x1C, &[0x00, u8::from(tx)])
}
/// Read the S-meter (cmd `15 02`). Reply is a 2-byte big-endian BCD level 0000–0255.
pub fn read_smeter(radio: u8) -> Frame {
    Frame::command(radio, 0x15, &[0x02])
}
/// Read RF output power (cmd `14 0A`). Reply is a 2-byte BCD level 0000–0255.
pub fn read_rf_power(radio: u8) -> Frame {
    Frame::command(radio, 0x14, &[0x0A])
}
/// Set RF output power (cmd `14 0A`) as a percentage 0–100 (mapped to 0–255).
pub fn set_rf_power(radio: u8, percent: u8) -> Frame {
    let level = u16::from(percent.min(100)) * 255 / 100;
    let [hi, lo] = level_to_bcd2(level);
    Frame::command(radio, 0x14, &[0x0A, hi, lo])
}

// ---- extended verbs (split / VFO / RIT / CW / repeater / data mode) ----

/// Select a VFO (cmd `07`): `VFOA`/`VFOB` (also accepts `Main`/`Sub` for the IC-9700's
/// main/sub bands). `None` for a name the rig has no equivalent for.
pub fn select_vfo(radio: u8, vfo: &str) -> Option<Frame> {
    let b = match vfo.to_ascii_uppercase().as_str() {
        "VFOA" | "A" => 0x00,
        "VFOB" | "B" => 0x01,
        "MAIN" => 0xD0,
        "SUB" => 0xD1,
        _ => return None,
    };
    Some(Frame::command(radio, 0x07, &[b]))
}
/// Split on/off (cmd `0F`, data `00`/`01`).
pub fn set_split(radio: u8, on: bool) -> Frame {
    Frame::command(radio, 0x0F, &[u8::from(on)])
}
/// FM duplex (repeater shift) — shares cmd `0F`: `10` simplex, `11` DUP−, `12` DUP+.
pub fn set_duplex(radio: u8, shift: &str) -> Frame {
    let b = match shift {
        "+" => 0x12,
        "-" => 0x11,
        _ => 0x10,
    };
    Frame::command(radio, 0x0F, &[b])
}
/// Set the UNSELECTED VFO's frequency (cmd `25 01`) — the split/duplex TX dial on the
/// 7300 family without swapping VFOs.
pub fn set_unselected_freq(radio: u8, hz: u64) -> Frame {
    let mut data = vec![0x01];
    data.extend_from_slice(&freq_to_bcd(hz));
    Frame::command(radio, 0x25, &data)
}
/// RIT/ΔTX offset (cmd `21 00`): ±9.999 kHz as 2-byte little-endian BCD magnitude + sign
/// byte (`00` = +, `01` = −). The offset register is shared by RIT and ΔTX.
pub fn set_rit_offset(radio: u8, hz: i32) -> Frame {
    let mag = hz.unsigned_abs().min(9_999);
    let lo = ((((mag / 10) % 10) as u8) << 4) | ((mag % 10) as u8);
    let hi = ((((mag / 1000) % 10) as u8) << 4) | (((mag / 100) % 10) as u8);
    Frame::command(radio, 0x21, &[0x00, lo, hi, u8::from(hz < 0)])
}
/// RIT on/off (cmd `21 01`).
pub fn set_rit_on(radio: u8, on: bool) -> Frame {
    Frame::command(radio, 0x21, &[0x01, u8::from(on)])
}
/// ΔTX (Icom's XIT) on/off (cmd `21 02`).
pub fn set_dtx_on(radio: u8, on: bool) -> Frame {
    Frame::command(radio, 0x21, &[0x02, u8::from(on)])
}
/// Icom's per-frame CW text limit (cmd `17`) — longer messages are chunked.
pub const MORSE_CHUNK: usize = 30;
/// Key CW from text (cmd `17`, ASCII payload ≤ [`MORSE_CHUNK`] chars).
pub fn send_morse(radio: u8, text: &str) -> Frame {
    let ascii: Vec<u8> = text
        .bytes()
        .filter(u8::is_ascii)
        .take(MORSE_CHUNK)
        .collect();
    Frame::command(radio, 0x17, &ascii)
}
/// Abort CW keying in progress (cmd `17` with the single byte `FF`).
pub fn stop_morse(radio: u8) -> Frame {
    Frame::command(radio, 0x17, &[0xFF])
}
/// Keyer speed (cmd `14 0C`): WPM 6–48 mapped onto the 0–255 level scale.
pub fn set_keyer_speed_wpm(radio: u8, wpm: u32) -> Frame {
    let wpm = wpm.clamp(6, 48);
    let level = ((wpm - 6) * 255 / 42) as u16;
    let [hi, lo] = level_to_bcd2(level);
    Frame::command(radio, 0x14, &[0x0C, hi, lo])
}
/// Repeater (CTCSS) tone frequency (cmd `1B 00`), in tenths of Hz as 4-digit BCD
/// (88.5 Hz → 0885).
pub fn set_repeater_tone(radio: u8, tenths: u32) -> Frame {
    let [hi, lo] = level_to_bcd2(tenths.min(9999) as u16);
    Frame::command(radio, 0x1B, &[0x00, hi, lo])
}
/// TONE function on/off (cmd `16 42`) — transmit the repeater tone.
pub fn set_tone_func(radio: u8, on: bool) -> Frame {
    Frame::command(radio, 0x16, &[0x42, u8::from(on)])
}

/// DSP / audio ON-OFF function sub-commands under CI-V command `0x16` (Icom
/// IC-7300/9700/7610/705 generation share this 16-family table): Noise Blanker, Noise
/// Reduction, Auto Notch, speech Compressor, Monitor, VOX. The Hamlib func token maps to
/// its sub-command byte here — one place, so both the getter and setter agree.
pub fn func_sub(token: &str) -> Option<u8> {
    Some(match token {
        "NB" => 0x22,   // Noise Blanker
        "NR" => 0x40,   // Noise Reduction
        "ANF" => 0x41,  // Auto Notch Filter
        "COMP" => 0x44, // Speech Compressor
        "MON" => 0x45,  // Monitor
        "VOX" => 0x46,  // VOX
        _ => return None,
    })
}
/// Read a `16 <sub>` DSP-function state — the reply carries the on/off byte.
pub fn read_dsp_func(radio: u8, sub: u8) -> Frame {
    Frame::command(radio, 0x16, &[sub])
}
/// Set a `16 <sub>` DSP function on/off.
pub fn set_dsp_func(radio: u8, sub: u8, on: bool) -> Frame {
    Frame::command(radio, 0x16, &[sub, u8::from(on)])
}
/// Extract the on/off state from a `16 <sub>` DSP-function reply.
pub fn parse_dsp_func(f: &Frame, sub: u8) -> Option<bool> {
    if f.cmd == 0x16 && f.data.first() == Some(&sub) {
        f.data.get(1).map(|&b| b != 0)
    } else {
        None
    }
}
/// Microphone gain (cmd `14 0B`): percent 0–100 mapped onto the 0–255 level scale.
pub fn set_mic_gain(radio: u8, percent: u8) -> Frame {
    let level = u16::from(percent.min(100)) * 255 / 100;
    let [hi, lo] = level_to_bcd2(level);
    Frame::command(radio, 0x14, &[0x0B, hi, lo])
}
/// Read the microphone gain (cmd `14 0B`) — raw 0–255.
pub fn read_mic_gain(radio: u8) -> Frame {
    Frame::command(radio, 0x14, &[0x0B])
}
/// Extract the raw mic-gain level (0–255) from a `14 0B` reply.
pub fn parse_mic_gain_raw(f: &Frame) -> Option<u16> {
    if f.cmd == 0x14 && f.data.first() == Some(&0x0B) && f.data.len() >= 3 {
        Some(level_from_bcd2(f.data[1], f.data[2]))
    } else {
        None
    }
}
/// FM repeater offset frequency (cmd `0D` set / `0C` read): 3-byte little-endian BCD in
/// 100 Hz units (600 kHz → `00 60 00`). The 10 MHz digit only applies on 1200 MHz.
pub fn set_rptr_offset(radio: u8, hz: u64) -> Frame {
    let bcd = freq_to_bcd(hz / 100);
    Frame::command(radio, 0x0D, &bcd[..3])
}
/// DATA mode on/off (cmd `1A 06`): `on` selects USB-D/LSB-D (soundcard digital); the
/// filter byte keeps the current selection when `None`.
pub fn set_data_mode(radio: u8, on: bool, filter: Option<u8>) -> Frame {
    let fil = filter.unwrap_or(if on { 0x01 } else { 0x00 });
    Frame::command(radio, 0x1A, &[0x06, u8::from(on), fil])
}
/// Read the DATA mode state (cmd `1A 06`).
pub fn read_data_mode(radio: u8) -> Frame {
    Frame::command(radio, 0x1A, &[0x06])
}
/// Extract the DATA-mode state from a `1A 06` reply.
pub fn parse_data_mode(f: &Frame) -> Option<bool> {
    if f.cmd == 0x1A && f.data.first() == Some(&0x06) {
        f.data.get(1).map(|&b| b != 0)
    } else {
        None
    }
}

// ---- reply decoders ----

/// Extract the frequency (Hz) from a `03` frequency report (or an unsolicited transceive
/// `00` report, which shares the 5-byte-BCD payload).
pub fn parse_freq(f: &Frame) -> Option<u64> {
    if (f.cmd == 0x03 || f.cmd == 0x00) && f.data.len() >= 5 {
        Some(bcd_to_freq(&f.data[..5]))
    } else {
        None
    }
}
/// Extract `(mode, filter)` from a `04` mode report (or transceive `01`).
pub fn parse_mode(f: &Frame) -> Option<(Mode, Option<u8>)> {
    if (f.cmd == 0x04 || f.cmd == 0x01) && !f.data.is_empty() {
        let mode = Mode::from_byte(f.data[0])?;
        Some((mode, f.data.get(1).copied()))
    } else {
        None
    }
}
/// Extract PTT state from a `1C 00` reply (`true` = transmitting).
pub fn parse_ptt(f: &Frame) -> Option<bool> {
    if f.cmd == 0x1C && f.data.first() == Some(&0x00) {
        f.data.get(1).map(|&b| b != 0)
    } else {
        None
    }
}
/// Extract the raw S-meter level (0–255) from a `15 02` reply.
pub fn parse_smeter_raw(f: &Frame) -> Option<u16> {
    if f.cmd == 0x15 && f.data.first() == Some(&0x02) && f.data.len() >= 3 {
        Some(level_from_bcd2(f.data[1], f.data[2]))
    } else {
        None
    }
}
/// Extract the raw RF-power level (0–255) from a `14 0A` reply.
pub fn parse_rf_power_raw(f: &Frame) -> Option<u16> {
    if f.cmd == 0x14 && f.data.first() == Some(&0x0A) && f.data.len() >= 3 {
        Some(level_from_bcd2(f.data[1], f.data[2]))
    } else {
        None
    }
}

/// Convert a raw Icom S-meter reading (0–255) to dB relative to S9, using Icom's nominal
/// scale (S9 ≈ raw 120; each S-unit ≈ 6 dB below S9; raw 120→241 ≈ 0→+60 dB over S9).
/// Approximate and per-rig-calibratable, but consistent and monotonic.
pub fn smeter_db_rel_s9(raw: u16) -> f32 {
    let raw = raw.min(255) as f32;
    if raw <= 120.0 {
        raw / 120.0 * 54.0 - 54.0 // S0 = -54 dB … S9 = 0 dB
    } else {
        (raw - 120.0) / (241.0 - 120.0) * 60.0 // S9 … S9+60
    }
}

// ---- 2-byte big-endian BCD level codec (S-meter, RF power: 0000–0255) ----

/// Encode a 0–255 level as Icom's 2-byte, big-endian, 4-digit BCD (e.g. 128 → `[0x01,0x28]`).
fn level_to_bcd2(v: u16) -> [u8; 2] {
    let v = v.min(9999);
    let hi = (v / 100) as u8; // 0–25 (two BCD digits: thousands+hundreds)
    let lo = (v % 100) as u8; // 0–99 (tens+ones)
    [to_bcd(hi), to_bcd(lo)]
}
/// Decode Icom's 2-byte big-endian BCD level back to 0–255.
fn level_from_bcd2(hi: u8, lo: u8) -> u16 {
    u16::from(from_bcd(hi)) * 100 + u16::from(from_bcd(lo))
}
/// Two decimal digits → one BCD byte (defensive: values > 99 wrap on the 100s).
fn to_bcd(v: u8) -> u8 {
    ((v / 10 % 10) << 4) | (v % 10)
}
/// One BCD byte → two decimal digits (non-decimal nibbles clamped to 0).
fn from_bcd(b: u8) -> u8 {
    let hi = b >> 4;
    let lo = b & 0x0F;
    (if hi > 9 { 0 } else { hi }) * 10 + (if lo > 9 { 0 } else { lo })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civ::frame::bcd_to_freq;

    #[test]
    fn model_addresses_and_name_detection() {
        assert_eq!(IcomModel::Ic9700.default_civ_addr(), 0xA2);
        assert_eq!(IcomModel::Ic7300.default_civ_addr(), 0x94);
        assert_eq!(
            IcomModel::from_name("Icom IC-9700"),
            Some(IcomModel::Ic9700)
        );
        assert_eq!(IcomModel::from_name("ic7300"), Some(IcomModel::Ic7300));
        assert_eq!(IcomModel::from_name("Yaesu FTDX10"), None);
    }

    #[test]
    fn set_freq_encodes_cmd_05_with_bcd() {
        let f = set_freq(0xA2, 145_000_000);
        assert_eq!(f.cmd, 0x05);
        assert_eq!(f.to, 0xA2);
        assert_eq!(bcd_to_freq(&f.data), 145_000_000);
    }

    #[test]
    fn dsp_func_frames_round_trip() {
        // Token → 0x16 sub-command mapping is the single source both get + set use.
        assert_eq!(func_sub("COMP"), Some(0x44));
        assert_eq!(func_sub("VOX"), Some(0x46));
        assert_eq!(func_sub("NB"), Some(0x22));
        assert_eq!(func_sub("RIT"), None); // RIT is a separate register, not a 0x16 func
        // Set builds `16 <sub> <on>`.
        let on = set_dsp_func(0xA2, 0x44, true);
        assert_eq!(on.cmd, 0x16);
        assert_eq!(on.data, vec![0x44, 0x01]);
        // A reply `16 44 01` decodes to on=true, and a mismatched sub is rejected.
        let reply = Frame::parse(&[0xFE, 0xFE, 0xE0, 0xA2, 0x16, 0x44, 0x01, 0xFD]).unwrap();
        assert_eq!(parse_dsp_func(&reply, 0x44), Some(true));
        assert_eq!(parse_dsp_func(&reply, 0x46), None); // wrong sub-command
    }

    #[test]
    fn mic_gain_scales_percent_onto_0_255() {
        // 14 0B, percent → 0..255 BCD, mirroring set_rf_power / keyer-speed.
        let f = set_mic_gain(0xA2, 100);
        assert_eq!(f.cmd, 0x14);
        assert_eq!(f.data[0], 0x0B);
        assert_eq!(level_from_bcd2(f.data[1], f.data[2]), 255);
        assert_eq!(level_from_bcd2(set_mic_gain(0xA2, 0).data[1], set_mic_gain(0xA2, 0).data[2]), 0);
    }

    #[test]
    fn freq_report_round_trip() {
        // Radio replies to a read: FE FE E0 A2 03 <bcd> FD.
        let reply = Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x03,
            data: freq_to_bcd(432_100_000).to_vec(),
        };
        assert_eq!(parse_freq(&reply), Some(432_100_000));
        // A transceive (cmd 00) report is decoded the same way.
        let xcv = Frame {
            cmd: 0x00,
            ..reply.clone()
        };
        assert_eq!(parse_freq(&xcv), Some(432_100_000));
    }

    #[test]
    fn mode_table_is_a_bijection_on_known_modes() {
        for m in [
            Mode::Lsb,
            Mode::Usb,
            Mode::Am,
            Mode::Cw,
            Mode::Rtty,
            Mode::Fm,
            Mode::CwR,
            Mode::RttyR,
        ] {
            assert_eq!(Mode::from_byte(m.to_byte()), Some(m));
            assert_eq!(Mode::from_name(m.name()), Some(m));
        }
        assert_eq!(Mode::from_byte(0x7F), None);
    }

    #[test]
    fn set_mode_with_and_without_filter() {
        assert_eq!(set_mode(0xA2, Mode::Usb, None).data, vec![0x01]);
        assert_eq!(set_mode(0xA2, Mode::Cw, Some(2)).data, vec![0x03, 0x02]);
    }

    #[test]
    fn ptt_set_and_parse() {
        assert_eq!(set_ptt(0xA2, true).data, vec![0x00, 0x01]);
        assert_eq!(set_ptt(0xA2, false).data, vec![0x00, 0x00]);
        let rx = Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x1C,
            data: vec![0x00, 0x00],
        };
        let tx = Frame {
            data: vec![0x00, 0x01],
            ..rx.clone()
        };
        assert_eq!(parse_ptt(&rx), Some(false));
        assert_eq!(parse_ptt(&tx), Some(true));
    }

    #[test]
    fn level_bcd2_round_trips() {
        for v in [0u16, 1, 99, 100, 120, 128, 241, 255] {
            let [hi, lo] = level_to_bcd2(v);
            assert_eq!(level_from_bcd2(hi, lo), v, "level {v}");
        }
        // Known Icom byte layout: 128 → 0x01 0x28.
        assert_eq!(level_to_bcd2(128), [0x01, 0x28]);
    }

    #[test]
    fn smeter_raw_reply_and_db_curve() {
        // FE FE E0 A2 15 02 <2-byte BCD> FD, raw 120 = S9.
        let [hi, lo] = level_to_bcd2(120);
        let reply = Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x15,
            data: vec![0x02, hi, lo],
        };
        assert_eq!(parse_smeter_raw(&reply), Some(120));
        assert!(
            (smeter_db_rel_s9(120) - 0.0).abs() < 0.001,
            "raw 120 = S9 = 0 dB"
        );
        assert!(smeter_db_rel_s9(0) < smeter_db_rel_s9(120)); // S0 below S9
        assert!(smeter_db_rel_s9(241) > smeter_db_rel_s9(120)); // S9+60 above S9
        assert!(
            (smeter_db_rel_s9(0) + 54.0).abs() < 0.001,
            "raw 0 = S0 = -54 dB"
        );
    }

    #[test]
    fn repeater_offset_matches_the_official_example() {
        // IC-9700 CI-V reference: 600 kHz offset → cmd 0D data 00 60 00 (3-byte LE BCD,
        // 100 Hz units).
        let f = set_rptr_offset(0xA2, 600_000);
        assert_eq!(f.cmd, 0x0D);
        assert_eq!(f.data, vec![0x00, 0x60, 0x00]);
        // 5 MHz (70 cm convention) → 00 00 05.
        assert_eq!(
            set_rptr_offset(0xA2, 5_000_000).data,
            vec![0x00, 0x00, 0x05]
        );
    }

    #[test]
    fn rf_power_set_maps_percent_to_level() {
        // 100% → raw 255 → BCD 0x02 0x55.
        assert_eq!(set_rf_power(0xA2, 100).data, vec![0x0A, 0x02, 0x55]);
        assert_eq!(set_rf_power(0xA2, 0).data, vec![0x0A, 0x00, 0x00]);
        // Round-trips through the raw decoder.
        let f = set_rf_power(0xA2, 50);
        let reply = Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x14,
            data: f.data.clone(),
        };
        assert_eq!(parse_rf_power_raw(&reply), Some(127)); // 50% of 255
    }
}
