//! Icom scope-waveform assembly (CI-V command `0x27`) — pure, no I/O.
//!
//! With waveform output enabled, the radio streams each scope sweep as a numbered burst of
//! `27 00` frames: the first frame of a burst carries a header (center/fixed mode, the RF
//! position, an out-of-range flag), the rest carry the waveform points (one byte each,
//! `0x00..=0xA0` display height). [`ScopeAssembler`] reassembles bursts into complete
//! sweeps normalized to the waterfall's 0..1 row contract, tagged with the absolute RF span.
//!
//! Scope bytes never collide with CI-V framing (`FE`/`FD`/`FB`/`FA`): sequence counters are
//! BCD, frequencies/spans are BCD, and waveform points top out at `0xA0` — so the ordinary
//! [`super::frame::FrameSplitter`] splits scope frames safely and hands them here.
//!
//! Byte layout follows the Icom CI-V reference (IC-7300/9700 family) as implemented by
//! wfview; the on-rig IC-9700 session is the calibration gate before this leaves the flag.

use super::frame::{bcd_to_freq, Frame};

/// Scope display height ceiling: waveform bytes run 0..=160.
const POINT_MAX: f32 = 160.0;
/// Cap accumulated points per sweep — a corrupt total can't grow the buffer unbounded.
const MAX_POINTS: usize = 4096;

/// One completed scope sweep, normalized for [`tempo_app::dto::Spectrum`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeSweep {
    /// Waveform points normalized 0..1 (the UI's AGC/LUT does the display stretch).
    pub row: Vec<f32>,
    /// Absolute RF span the row covers.
    pub lo_hz: f64,
    pub hi_hz: f64,
}

/// The CI-V frames that enable/disable the scope waveform stream to the controller:
/// `27 10` turns the scope itself on (harmless if already on), `27 11` switches the
/// waveform-data output to the CI-V port. Sent on every enable transition — both are
/// idempotent on the radio.
pub fn scope_stream_frames(radio: u8, on: bool) -> Vec<Frame> {
    let b = u8::from(on);
    if on {
        vec![
            Frame::command(radio, 0x27, &[0x10, 0x01]),
            Frame::command(radio, 0x27, &[0x11, b]),
        ]
    } else {
        // Leave the scope display itself as the operator had it; just stop the stream.
        vec![Frame::command(radio, 0x27, &[0x11, b])]
    }
}

/// One decimal from a BCD byte pair position (two digits per byte).
fn bcd_byte(b: u8) -> u32 {
    let hi = (b >> 4) as u32;
    let lo = (b & 0x0F) as u32;
    (if hi > 9 { 0 } else { hi }) * 10 + if lo > 9 { 0 } else { lo }
}

/// The header carried by the FIRST frame of each sweep burst.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SweepHeader {
    /// Absolute RF edges the sweep covers.
    lo_hz: f64,
    hi_hz: f64,
    /// The radio flags the sweep invalid while retuning (out-of-range) — dropped.
    out_of_range: bool,
}

/// Parse one `27 00` waveform frame's data (everything after the `27` command byte,
/// i.e. `data[0] == 0x00`). Returns `(sequence, total, header-if-first, points)`.
///
/// Layout (after the `00` sub-command):
/// `[main/sub] [seq BCD] [seq-total BCD]` then, in the first frame of a burst:
/// `[mode 00=center|01=fixed] [freq 5B BCD] [span-or-upper 5B BCD] [oor]`;
/// in every later frame: waveform points, one byte each.
fn parse_waveform(data: &[u8]) -> Option<(u32, u32, Option<SweepHeader>, &[u8])> {
    // data[0] = 0x00 sub-command, [1] = main(00)/sub(01) receiver, [2] = seq, [3] = total.
    if data.len() < 4 || data[0] != 0x00 {
        return None;
    }
    let seq = bcd_byte(data[2]);
    let total = bcd_byte(data[3]);
    if seq == 0 || total == 0 || seq > total {
        return None;
    }
    if seq == 1 {
        // Header frame: mode + position + span/edge + out-of-range flag.
        if data.len() < 16 {
            return None;
        }
        let fixed = data[4] != 0x00;
        let a = bcd_to_freq(&data[5..10]) as f64;
        let b = bcd_to_freq(&data[10..15]) as f64;
        let (lo, hi) = if fixed {
            // Fixed mode: lower edge, upper edge.
            (a, b)
        } else {
            // Center mode: center frequency ± span (the span value is the ± half-width).
            (a - b, a + b)
        };
        let header = SweepHeader {
            lo_hz: lo,
            hi_hz: hi,
            out_of_range: data[15] != 0x00,
        };
        Some((seq, total, Some(header), &data[16..]))
    } else {
        Some((seq, total, None, &data[4..]))
    }
}

/// Reassembles waveform bursts into [`ScopeSweep`]s. Feed every `cmd == 0x27` frame;
/// a completed, in-range sweep pops out. Missed/out-of-order frames drop the burst and
/// resync on the next header — a lossy stream degrades to a lower frame rate, never to
/// a corrupted row.
#[derive(Debug, Default)]
pub struct ScopeAssembler {
    header: Option<SweepHeader>,
    points: Vec<u8>,
    next_seq: u32,
    total: u32,
}

impl ScopeAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, f: &Frame) -> Option<ScopeSweep> {
        if f.cmd != 0x27 {
            return None;
        }
        let (seq, total, header, points) = parse_waveform(&f.data)?;
        if seq == 1 {
            // A new burst always resets the assembler (implicitly drops a partial one).
            self.header = header;
            self.points.clear();
            self.points.extend_from_slice(points);
            self.next_seq = 2;
            self.total = total;
        } else {
            // Continuation: must be the frame we expect, in the burst we're building.
            if self.header.is_none() || seq != self.next_seq || total != self.total {
                self.reset();
                return None;
            }
            if self.points.len() + points.len() > MAX_POINTS {
                self.reset();
                return None;
            }
            self.points.extend_from_slice(points);
            self.next_seq += 1;
        }
        // Burst complete?
        if seq == self.total {
            let header = self.header.take()?;
            let points = std::mem::take(&mut self.points);
            self.reset();
            if header.out_of_range || points.is_empty() || header.hi_hz <= header.lo_hz {
                return None;
            }
            let row = points
                .iter()
                .map(|&p| (f32::from(p) / POINT_MAX).min(1.0))
                .collect();
            return Some(ScopeSweep {
                row,
                lo_hz: header.lo_hz,
                hi_hz: header.hi_hz,
            });
        }
        None
    }

    fn reset(&mut self) {
        self.header = None;
        self.points.clear();
        self.next_seq = 0;
        self.total = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::frame::{freq_to_bcd, Frame};
    use super::*;

    /// Build a `27 00` waveform frame as the radio would send it.
    fn wf_frame(seq: u8, total: u8, body: &[u8]) -> Frame {
        // BCD-encode seq/total (two decimal digits per byte).
        let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
        let mut data = vec![0x00, 0x00, bcd(seq), bcd(total)];
        data.extend_from_slice(body);
        Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x27,
            data,
        }
    }

    /// A center-mode header body: center 145 MHz, span ±25 kHz, in range.
    fn center_header() -> Vec<u8> {
        let mut b = vec![0x00]; // center mode
        b.extend_from_slice(&freq_to_bcd(145_000_000));
        b.extend_from_slice(&freq_to_bcd(25_000));
        b.push(0x00); // in range
        b
    }

    #[test]
    fn assembles_a_three_frame_center_mode_sweep() {
        let mut asm = ScopeAssembler::new();
        assert!(asm.push(&wf_frame(1, 3, &center_header())).is_none());
        assert!(asm.push(&wf_frame(2, 3, &[0, 40, 80])).is_none());
        let sweep = asm.push(&wf_frame(3, 3, &[120, 160])).expect("complete");
        assert_eq!(sweep.row.len(), 5);
        assert_eq!(sweep.row[0], 0.0);
        assert!((sweep.row[1] - 0.25).abs() < 0.01);
        assert_eq!(sweep.row[4], 1.0);
        // Center ± span → absolute RF edges.
        assert_eq!(sweep.lo_hz, 144_975_000.0);
        assert_eq!(sweep.hi_hz, 145_025_000.0);
    }

    #[test]
    fn fixed_mode_header_uses_edges_directly() {
        let mut asm = ScopeAssembler::new();
        let mut hdr = vec![0x01]; // fixed mode
        hdr.extend_from_slice(&freq_to_bcd(144_000_000)); // lower
        hdr.extend_from_slice(&freq_to_bcd(144_500_000)); // upper
        hdr.push(0x00);
        assert!(asm.push(&wf_frame(1, 2, &hdr)).is_none());
        let sweep = asm.push(&wf_frame(2, 2, &[10, 20])).expect("complete");
        assert_eq!(sweep.lo_hz, 144_000_000.0);
        assert_eq!(sweep.hi_hz, 144_500_000.0);
    }

    #[test]
    fn a_missed_frame_drops_the_burst_and_resyncs_on_the_next_header() {
        let mut asm = ScopeAssembler::new();
        assert!(asm.push(&wf_frame(1, 3, &center_header())).is_none());
        // Frame 2 lost; frame 3 arrives → burst dropped, no bogus sweep.
        assert!(asm.push(&wf_frame(3, 3, &[1, 2])).is_none());
        // The next full burst still assembles.
        assert!(asm.push(&wf_frame(1, 2, &center_header())).is_none());
        assert!(asm.push(&wf_frame(2, 2, &[5, 6])).is_some());
    }

    #[test]
    fn out_of_range_sweeps_are_dropped() {
        let mut asm = ScopeAssembler::new();
        let mut hdr = vec![0x00];
        hdr.extend_from_slice(&freq_to_bcd(145_000_000));
        hdr.extend_from_slice(&freq_to_bcd(25_000));
        hdr.push(0x01); // OUT of range (mid-retune)
        assert!(asm.push(&wf_frame(1, 2, &hdr)).is_none());
        assert!(asm.push(&wf_frame(2, 2, &[1, 2, 3])).is_none(), "dropped");
    }

    #[test]
    fn enable_disable_frames() {
        let on = scope_stream_frames(0xA2, true);
        assert_eq!(on.len(), 2);
        assert_eq!(on[0].data, vec![0x10, 0x01]); // scope on
        assert_eq!(on[1].data, vec![0x11, 0x01]); // waveform output on
        let off = scope_stream_frames(0xA2, false);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].data, vec![0x11, 0x00]); // stream off, display untouched
    }

    #[test]
    fn garbage_and_foreign_subcommands_are_ignored() {
        let mut asm = ScopeAssembler::new();
        // A 27 14 (mode set ack echo) or short/foreign frame must not panic or emit.
        let foreign = Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x27,
            data: vec![0x14, 0x00, 0x01],
        };
        assert!(asm.push(&foreign).is_none());
        let short = Frame {
            to: 0xE0,
            from: 0xA2,
            cmd: 0x27,
            data: vec![0x00],
        };
        assert!(asm.push(&short).is_none());
    }
}
