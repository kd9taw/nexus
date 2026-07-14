//! CI-V bus diagnostic logger.
//!
//! An opt-in tap that records every byte the native CI-V engine writes to and reads from
//! the serial port — timestamped and best-effort decoded — to a plain-text file. It exists
//! to root-cause hardware-only symptoms (e.g. the IC-9700 PTT flicker) that can't be
//! reproduced in the test suite: with a capture, the *actual* bus traffic during the fault
//! is visible, so we can tell a repeated PTT toggle from a scope-waveform flood from a quiet
//! bus instead of guessing.
//!
//! It is **off by default and never persisted**: [`start`] opens the file and arms the tap,
//! [`stop`] flushes and disarms it. When disarmed the hot path is a single relaxed atomic
//! load, so leaving the two hooks compiled into the engine costs nothing in normal use.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use super::frame::{Frame, END, PREAMBLE};

/// Armed flag — a cheap relaxed load gates the hot path when logging is off.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// The open sink, present only while logging is armed.
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

struct Sink {
    w: BufWriter<File>,
    start: Instant,
    path: String,
}

/// Which way a chunk of bytes crossed the wire.
#[derive(Clone, Copy)]
pub enum Dir {
    /// Controller → radio (a command Nexus sent).
    Tx,
    /// Radio → controller (a reply, transceive push, or scope frame — plus our own echo
    /// on the half-duplex bus).
    Rx,
}

/// True while a log is open. Cheap enough to call on every serial read/write.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Open `path` for logging and arm the tap, replacing any current log. The file is
/// truncated — one capture per session is the intended workflow.
pub fn start(path: &Path) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    writeln!(
        w,
        "# Nexus CI-V bus diagnostic log\n\
         # columns: +<ms since start>  <direction>  <bytes>B  <hex>  ; <decoded>\n\
         # TX\u{2192}rig = a command Nexus sent; RX\u{2190}rig = bytes from the radio.\n\
         # Reproduce the issue, then turn logging off and send this file."
    )?;
    w.flush()?;
    if let Ok(mut g) = SINK.lock() {
        *g = Some(Sink {
            w,
            start: Instant::now(),
            path: path.display().to_string(),
        });
    }
    // Arm only after the sink is in place: a reader that sees the flag then locks the sink
    // is guaranteed to find it present.
    ENABLED.store(true, Ordering::Relaxed);
    Ok(())
}

/// The path of the active log, or `None` when logging is off. Lets the UI reflect the real
/// backend state (logging survives navigating away from Settings), instead of a local toggle
/// that appears to reset — and re-arming would truncate the capture.
pub fn status() -> Option<String> {
    if !is_enabled() {
        return None;
    }
    SINK.lock().ok().and_then(|g| g.as_ref().map(|s| s.path.clone()))
}

/// Disarm the tap, then flush and close the log.
pub fn stop() {
    ENABLED.store(false, Ordering::Relaxed);
    if let Ok(mut g) = SINK.lock() {
        if let Some(mut s) = g.take() {
            let _ = s.w.flush();
        }
    }
}

/// Record a decision-point marker (e.g. "daemon Drop unkey", "tune release") interleaved
/// with the bus traffic by timestamp. This is what turns "a daemon was torn down mid-TX"
/// from an inference into a named code path in the capture. A no-op when disarmed.
pub fn note(msg: &str) {
    if !is_enabled() {
        return;
    }
    let Ok(mut g) = SINK.lock() else { return };
    let Some(s) = g.as_mut() else { return };
    let ms = s.start.elapsed().as_millis();
    let _ = writeln!(s.w, "+{ms:>7} ms  NOTE                                 ; {msg}");
    let _ = s.w.flush();
}

/// Record one direction's bytes. A no-op (one relaxed load) when disarmed.
pub fn log(dir: Dir, bytes: &[u8]) {
    if !is_enabled() || bytes.is_empty() {
        return;
    }
    let Ok(mut g) = SINK.lock() else { return };
    let Some(s) = g.as_mut() else { return };
    let ms = s.start.elapsed().as_millis();
    let arrow = match dir {
        Dir::Tx => "TX\u{2192}rig",
        Dir::Rx => "RX\u{2190}rig",
    };
    // Flush every line: a fault we're chasing may hang or crash immediately after, and the
    // last lines written are the ones that matter.
    let _ = writeln!(
        s.w,
        "+{ms:>7} ms  {arrow}  {:>3}B  {}  ; {}",
        bytes.len(),
        to_hex(bytes),
        describe(bytes)
    );
    let _ = s.w.flush();
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// Best-effort human label for a chunk. A read from the bus can carry several frames at
/// once (e.g. our own echo + the radio's ack), so we walk the chunk frame-by-frame — each
/// frame runs from a `FE FE` preamble to its *first* `FD` terminator — and label each part.
/// (`Frame::parse` on the whole chunk can't be used: it treats everything up to the *last*
/// `FD` as one frame's data, swallowing the boundary.) Fragments that hold no complete frame
/// fall back to hex only. The scope stream (`0x27`) can embed `FD` in waveform data, so its
/// label may fragment — the raw hex column is the ground truth.
fn describe(bytes: &[u8]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == PREAMBLE && bytes[i + 1] == PREAMBLE {
            if let Some(rel) = bytes[i..].iter().position(|&x| x == END) {
                if let Some(f) = Frame::parse(&bytes[i..=i + rel]) {
                    parts.push(describe_frame(&f));
                }
                i += rel + 1;
                continue;
            }
        }
        i += 1;
    }
    parts.join(" + ")
}

fn describe_frame(f: &Frame) -> String {
    match (f.cmd, f.data.first().copied()) {
        (0x00, _) | (0x05, _) => "set freq".into(),
        (0x03, _) => "read freq".into(),
        (0x01, _) | (0x06, _) => "set mode".into(),
        (0x04, _) => "read mode".into(),
        (0x07, _) => "set VFO".into(),
        (0x0F, _) => "split/dup".into(),
        (0x14, _) => "set level".into(),
        (0x15, _) => "read meter".into(),
        (0x1A, Some(0x06)) => "set data-mode".into(),
        (0x1C, Some(0x00)) => match f.data.get(1) {
            Some(0) => "PTT OFF (RX)".into(),
            Some(1) => "PTT ON (TX)".into(),
            _ => "PTT ?".into(),
        },
        (0x1C, Some(0x01)) => "send/other".into(),
        (0x27, _) => "scope waveform".into(),
        (0xFB, _) => "ACK".into(),
        (0xFA, _) => "NAK".into(),
        (c, _) => format!("cmd {c:#04X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_the_common_frames() {
        // PTT on/off — the frames the IC-9700 flicker turns on and off.
        let ptt_on = Frame::command(0xA2, 0x1C, &[0x00, 0x01]);
        assert_eq!(describe(&ptt_on.to_bytes()), "PTT ON (TX)");
        let ptt_off = Frame::command(0xA2, 0x1C, &[0x00, 0x00]);
        assert_eq!(describe(&ptt_off.to_bytes()), "PTT OFF (RX)");
        // A scope-waveform frame — the flood hypothesis.
        let scope = Frame::command(0xE0, 0x27, &[0x00, 0x11, 0x22]);
        assert_eq!(describe(&scope.to_bytes()), "scope waveform");
    }

    #[test]
    fn splits_a_chunk_of_two_frames() {
        // The bus echoes our command (from == controller), then the radio acks it.
        let mut chunk = Frame::command(0xA2, 0x1C, &[0x00, 0x01]).to_bytes();
        chunk.extend_from_slice(&[0xFE, 0xFE, 0xE0, 0xA2, 0xFB, 0xFD]); // ack
        assert_eq!(describe(&chunk), "PTT ON (TX) + ACK");
    }

    #[test]
    fn disabled_by_default_is_a_noop() {
        assert!(!is_enabled());
        log(Dir::Tx, &[0xFE, 0xFE, 0xA2, 0xE0, 0x03, 0xFD]); // must not panic
    }
}
