//! CI-V receive-state demux — folds parsed reply / unsolicited-transceive frames into the
//! radio's live state. Pure: the serial CI-V engine feeds it every frame the
//! [`super::frame::FrameSplitter`] yields, and reads back a coherent snapshot for the UI /
//! rigctld broker. Reuses the tested decoders in [`super::commands`], so this is just the
//! routing layer.

use super::commands::{
    parse_freq, parse_mode, parse_ptt, parse_rf_power_raw, parse_smeter_raw, Mode,
};
use super::frame::Frame;

/// The live state distilled from the CI-V stream. Every field is `Option` until first seen —
/// the radio reports each independently (polled reads + unsolicited transceive updates).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CivState {
    pub freq_hz: Option<u64>,
    pub mode: Option<Mode>,
    pub filter: Option<u8>,
    pub ptt: Option<bool>,
    pub smeter_raw: Option<u16>,
    pub rf_power_raw: Option<u16>,
}

impl CivState {
    /// Fold one received frame (a solicited reply or an unsolicited transceive `00`/`01` report)
    /// into the state. Returns `true` if it updated something, so the caller can push a fresh
    /// snapshot only on a real change. Acknowledge frames (`FB`/`FA`) and anything unrecognized
    /// leave the state untouched.
    pub fn apply(&mut self, f: &Frame) -> bool {
        if let Some(hz) = parse_freq(f) {
            self.freq_hz = Some(hz);
            return true;
        }
        if let Some((m, filt)) = parse_mode(f) {
            self.mode = Some(m);
            if filt.is_some() {
                self.filter = filt;
            }
            return true;
        }
        if let Some(p) = parse_ptt(f) {
            self.ptt = Some(p);
            return true;
        }
        if let Some(s) = parse_smeter_raw(f) {
            self.smeter_raw = Some(s);
            return true;
        }
        if let Some(p) = parse_rf_power_raw(f) {
            self.rf_power_raw = Some(p);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::commands::{read_ptt, set_freq, set_mode};
    use super::super::frame::{freq_to_bcd, Frame};
    use super::*;

    /// Build a radio→controller reply frame (from the radio addr to the PC).
    fn reply(cmd: u8, data: &[u8]) -> Frame {
        Frame {
            to: 0xE0,
            from: 0xA2,
            cmd,
            data: data.to_vec(),
        }
    }

    #[test]
    fn folds_freq_mode_ptt_smeter_into_state() {
        let mut st = CivState::default();
        assert!(st.apply(&reply(0x03, &freq_to_bcd(145_000_000))));
        assert_eq!(st.freq_hz, Some(145_000_000));

        assert!(st.apply(&reply(0x04, &[Mode::Usb.to_byte(), 0x02])));
        assert_eq!(st.mode, Some(Mode::Usb));
        assert_eq!(st.filter, Some(0x02));

        assert!(st.apply(&reply(0x1C, &[0x00, 0x01])));
        assert_eq!(st.ptt, Some(true));

        assert!(st.apply(&reply(0x15, &[0x02, 0x01, 0x20]))); // raw 120 = S9
        assert_eq!(st.smeter_raw, Some(120));
    }

    #[test]
    fn transceive_report_updates_like_a_reply() {
        // The radio pushes an unsolicited freq (cmd 00) when the operator turns the knob.
        let mut st = CivState::default();
        assert!(st.apply(&reply(0x00, &freq_to_bcd(14_074_000))));
        assert_eq!(st.freq_hz, Some(14_074_000));
    }

    #[test]
    fn ack_and_unknown_frames_do_not_change_state() {
        let mut st = CivState::default();
        assert!(!st.apply(&Frame::parse(&[0xFE, 0xFE, 0xE0, 0xA2, 0xFB, 0xFD]).unwrap())); // ack
        assert_eq!(st, CivState::default());
        // A command frame we send (not a report) shouldn't be mistaken for state either — the
        // splitter drops our echoes, but be defensive: set_freq is cmd 05, not a report.
        assert!(!st.apply(&set_freq(0xA2, 145_000_000)));
        assert!(!st.apply(&set_mode(0xA2, Mode::Cw, None)));
        assert!(!st.apply(&read_ptt(0xA2)));
    }
}
