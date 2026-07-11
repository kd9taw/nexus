//! Icom CI-V frame + BCD codec layer — pure, no I/O, no `serialport` dependency.
//!
//! A CI-V message on the wire is:
//!
//! ```text
//! FE FE <to> <from> <cmd> [<sub>] [<data…>] FD
//! ```
//!
//! `FE FE` is the preamble, `FD` the terminator. `<to>`/`<from>` are one-byte CI-V
//! addresses (the radio's address and the controller's). Frequencies and several other
//! fields are **little-endian BCD** — two decimal digits per byte, least-significant byte
//! first. A set command is acknowledged by a bare `FB` (OK) or `FA` (NG) in the command
//! position of the reply frame.
//!
//! Framing is unambiguous for the standard command set because data bytes are BCD
//! (`0x00`–`0x99`) or bounded command/level bytes — none of the control bytes
//! (`FE`/`FD`/`FB`/`FA`) ever appear in data. (The spectrum-scope stream, command `0x27`,
//! can carry arbitrary waveform bytes; it is reassembled by its own length/sequence-aware
//! layer, not by [`FrameSplitter`].)
//!
//! Everything here is exhaustively unit-tested and hardware-independent — it is the
//! foundation the serial CI-V engine and native panadapter build on.

/// CI-V preamble byte (two of these begin a frame).
pub const PREAMBLE: u8 = 0xFE;
/// CI-V frame terminator.
pub const END: u8 = 0xFD;
/// Positive acknowledge (a set command succeeded).
pub const OK: u8 = 0xFB;
/// Negative acknowledge (a command was rejected).
pub const NG: u8 = 0xFA;

/// The controller (PC) CI-V address Nexus transmits as. `0xE0` is the Icom default for a
/// computer on the bus; the radio replies addressed to this.
pub const CONTROLLER: u8 = 0xE0;

/// A parsed CI-V frame. `cmd` is the command byte; `data` is everything between the
/// command byte and the `FD` terminator. For commands that take a sub-command, that
/// sub-command is `data[0]` — which commands take one is command-specific, so it is not
/// split out here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub to: u8,
    pub from: u8,
    pub cmd: u8,
    pub data: Vec<u8>,
}

impl Frame {
    /// Build a controller → radio command frame addressed from [`CONTROLLER`].
    pub fn command(radio: u8, cmd: u8, data: &[u8]) -> Self {
        Frame {
            to: radio,
            from: CONTROLLER,
            cmd,
            data: data.to_vec(),
        }
    }

    /// Serialize to wire bytes: `FE FE <to> <from> <cmd> <data…> FD`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(6 + self.data.len());
        v.push(PREAMBLE);
        v.push(PREAMBLE);
        v.push(self.to);
        v.push(self.from);
        v.push(self.cmd);
        v.extend_from_slice(&self.data);
        v.push(END);
        v
    }

    /// Parse ONE complete frame from bytes that include the leading `FE FE` and trailing
    /// `FD`. Returns `None` if the bytes are not a well-formed frame (bad preamble/
    /// terminator, or too short to hold `to`/`from`/`cmd`).
    pub fn parse(bytes: &[u8]) -> Option<Frame> {
        // Shortest legal frame is `FE FE to from cmd FD` = 6 bytes.
        if bytes.len() < 6 {
            return None;
        }
        if bytes[0] != PREAMBLE || bytes[1] != PREAMBLE {
            return None;
        }
        if *bytes.last().unwrap() != END {
            return None;
        }
        let body = &bytes[2..bytes.len() - 1]; // between `FE FE` and `FD`
        if body.len() < 3 {
            return None;
        }
        Some(Frame {
            to: body[0],
            from: body[1],
            cmd: body[2],
            data: body[3..].to_vec(),
        })
    }

    /// True if this frame is a bare positive acknowledge (`FB`) — a set command succeeded.
    pub fn is_ack(&self) -> bool {
        self.cmd == OK && self.data.is_empty()
    }

    /// True if this frame is a bare negative acknowledge (`FA`) — a command was rejected.
    pub fn is_nak(&self) -> bool {
        self.cmd == NG && self.data.is_empty()
    }
}

/// Encode a frequency in Hz as Icom's 5-byte little-endian BCD (10 decimal digits, so up
/// to 9,999,999,999 Hz — comfortably past the IC-9700's 1.3 GHz). Any digits above the
/// tenth are dropped.
pub fn freq_to_bcd(hz: u64) -> [u8; 5] {
    let mut out = [0u8; 5];
    let mut n = hz % 10_000_000_000; // keep 10 digits
    for b in out.iter_mut() {
        let lo = (n % 10) as u8;
        n /= 10;
        let hi = (n % 10) as u8;
        n /= 10;
        *b = (hi << 4) | lo;
    }
    out
}

/// Decode little-endian BCD bytes back to Hz. Any non-decimal nibble (`> 9`, e.g. from a
/// corrupt read) is treated as `0` rather than producing a wild value. Works for any
/// length; Icom frequency fields are 5 bytes.
pub fn bcd_to_freq(bytes: &[u8]) -> u64 {
    let mut hz = 0u64;
    let mut mult = 1u64;
    for &b in bytes {
        let lo = (b & 0x0F) as u64;
        let hi = (b >> 4) as u64;
        hz += if lo > 9 { 0 } else { lo } * mult;
        mult *= 10;
        hz += if hi > 9 { 0 } else { hi } * mult;
        mult *= 10;
    }
    hz
}

/// Reassembles complete CI-V frames from an arbitrary incoming byte stream (serial reads
/// arrive in arbitrary chunks). Resynchronizes on the `FE FE` preamble, discards garbage
/// between frames, buffers partial frames until the `FD` terminator arrives, and drops
/// **echoes** of our own transmissions (a half-duplex CI-V bus echoes back what the
/// controller sends — those frames carry `from == CONTROLLER`).
#[derive(Debug, Default)]
pub struct FrameSplitter {
    buf: Vec<u8>,
}

impl FrameSplitter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed newly-received bytes; return every complete, non-echo frame now available.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            // Align to the next `FE FE` preamble, dropping any leading garbage.
            match find_preamble(&self.buf) {
                Some(0) => {}
                Some(start) => {
                    self.buf.drain(..start);
                }
                None => {
                    // No preamble in the buffer. Keep only a trailing lone `FE` (it may be
                    // the first half of a preamble split across reads); drop the rest.
                    let keep = usize::from(self.buf.last() == Some(&PREAMBLE));
                    let cut = self.buf.len() - keep;
                    self.buf.drain(..cut);
                    break;
                }
            }
            // Find the terminator after the preamble (data never contains `FD`).
            let Some(end) = self.buf.iter().position(|&b| b == END) else {
                break; // frame not complete yet — wait for more bytes
            };
            let frame_bytes: Vec<u8> = self.buf.drain(..=end).collect();
            if let Some(f) = Frame::parse(&frame_bytes) {
                if f.from != CONTROLLER {
                    out.push(f);
                }
            }
            // A malformed span was drained; keep scanning what's left.
        }
        out
    }
}

/// Index of the first `FE FE` in `buf`, if any.
fn find_preamble(buf: &[u8]) -> Option<usize> {
    buf.windows(2)
        .position(|w| w[0] == PREAMBLE && w[1] == PREAMBLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_frame_round_trips_on_the_wire() {
        // Read frequency (cmd 0x03) to the IC-9700 (addr 0xA2).
        let f = Frame::command(0xA2, 0x03, &[]);
        let bytes = f.to_bytes();
        assert_eq!(bytes, vec![0xFE, 0xFE, 0xA2, 0xE0, 0x03, 0xFD]);
        assert_eq!(Frame::parse(&bytes), Some(f));
    }

    #[test]
    fn parse_extracts_addresses_cmd_and_data() {
        // A frequency-report reply from the radio: FE FE E0 A2 03 <5 BCD> FD.
        let bytes = [
            0xFE, 0xFE, 0xE0, 0xA2, 0x03, 0x00, 0x00, 0x00, 0x45, 0x01, 0xFD,
        ];
        let f = Frame::parse(&bytes).unwrap();
        assert_eq!(f.to, 0xE0);
        assert_eq!(f.from, 0xA2);
        assert_eq!(f.cmd, 0x03);
        assert_eq!(f.data, vec![0x00, 0x00, 0x00, 0x45, 0x01]);
        assert_eq!(bcd_to_freq(&f.data), 145_000_000);
    }

    #[test]
    fn ack_and_nak_frames() {
        let ack = Frame::parse(&[0xFE, 0xFE, 0xE0, 0xA2, 0xFB, 0xFD]).unwrap();
        assert!(ack.is_ack() && !ack.is_nak());
        let nak = Frame::parse(&[0xFE, 0xFE, 0xE0, 0xA2, 0xFA, 0xFD]).unwrap();
        assert!(nak.is_nak() && !nak.is_ack());
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(Frame::parse(&[0xFE, 0xFE, 0xA2, 0xE0, 0xFD]).is_none()); // too short (no cmd)
        assert!(Frame::parse(&[0xFE, 0xA2, 0xE0, 0x03, 0x00, 0xFD]).is_none()); // one preamble
        assert!(Frame::parse(&[0xFE, 0xFE, 0xA2, 0xE0, 0x03, 0x00]).is_none()); // no terminator
    }

    #[test]
    fn freq_bcd_exact_byte_layout() {
        // Little-endian BCD, two digits per byte, LSB first.
        assert_eq!(freq_to_bcd(145_000_000), [0x00, 0x00, 0x00, 0x45, 0x01]);
        assert_eq!(freq_to_bcd(14_074_000), [0x00, 0x40, 0x07, 0x14, 0x00]);
        assert_eq!(freq_to_bcd(0), [0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn freq_bcd_round_trips_across_the_band() {
        for hz in [
            0,
            1_800_000,
            14_074_000,
            50_313_000,
            145_000_000,
            432_100_000,
            1_296_000_000,
        ] {
            assert_eq!(bcd_to_freq(&freq_to_bcd(hz)), hz, "round-trip {hz} Hz");
        }
    }

    #[test]
    fn bcd_decode_tolerates_a_corrupt_nibble() {
        // A stray 0xAB nibble pair must not blow up the decode (defensive 0).
        assert_eq!(bcd_to_freq(&[0xAB, 0x00, 0x00, 0x00, 0x00]), 0);
    }

    #[test]
    fn splitter_yields_a_whole_frame() {
        let mut s = FrameSplitter::new();
        let out = s.push(&[0xFE, 0xFE, 0xE0, 0xA2, 0xFB, 0xFD]);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_ack());
    }

    #[test]
    fn splitter_reassembles_a_frame_split_across_reads() {
        let mut s = FrameSplitter::new();
        assert!(s.push(&[0xFE, 0xFE, 0xE0]).is_empty()); // partial
        assert!(s.push(&[0xA2, 0x03, 0x00, 0x00]).is_empty()); // still partial
        let out = s.push(&[0x00, 0x45, 0x01, 0xFD]); // completes it
        assert_eq!(out.len(), 1);
        assert_eq!(bcd_to_freq(&out[0].data), 145_000_000);
    }

    #[test]
    fn splitter_resyncs_past_leading_garbage() {
        let mut s = FrameSplitter::new();
        let out = s.push(&[0x11, 0x22, 0xFE, 0xFE, 0xE0, 0xA2, 0xFB, 0xFD]);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_ack());
    }

    #[test]
    fn splitter_returns_two_frames_from_one_chunk() {
        let mut s = FrameSplitter::new();
        let out = s.push(&[
            0xFE, 0xFE, 0xE0, 0xA2, 0xFB, 0xFD, // ack
            0xFE, 0xFE, 0xE0, 0xA2, 0x03, 0x00, 0x00, 0x00, 0x45, 0x01, 0xFD, // freq
        ]);
        assert_eq!(out.len(), 2);
        assert!(out[0].is_ack());
        assert_eq!(bcd_to_freq(&out[1].data), 145_000_000);
    }

    #[test]
    fn splitter_drops_our_own_echo() {
        let mut s = FrameSplitter::new();
        // The bus echoes our transmitted command (from == CONTROLLER 0xE0), then the
        // radio's reply (from == 0xA2) follows. Only the reply should surface.
        let echoed = Frame::command(0xA2, 0x03, &[]).to_bytes();
        let reply = [
            0xFE, 0xFE, 0xE0, 0xA2, 0x03, 0x00, 0x00, 0x00, 0x45, 0x01, 0xFD,
        ];
        let mut stream = echoed;
        stream.extend_from_slice(&reply);
        let out = s.push(&stream);
        assert_eq!(out.len(), 1, "echo dropped, only the radio's reply kept");
        assert_eq!(out[0].from, 0xA2);
    }
}
