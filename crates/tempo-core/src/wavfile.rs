//! Minimal WAV (RIFF / PCM-16) read + write, and a `.wav` replay path for the
//! FT1 decoder.
//!
//! Purpose: turn a captured-audio recording into a **deterministic regression
//! fixture** — replay a `.wav` of an off-air slot through the exact same
//! acquisition + decode (and IR-HARQ) pipeline the live app uses, with
//! reproducible results. No external crates (tempo-core stays dependency-light);
//! handles mono 16-bit PCM, the common single-channel capture format, and reads
//! channel 0 of a multi-channel file.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

const SAMPLE_RATE_HZ: u32 = 12_000;

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Write mono 16-bit PCM samples to a `.wav` at `sample_rate` (for fixtures).
pub fn write_wav_i16(path: impl AsRef<Path>, samples: &[i16], sample_rate: u32) -> io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * 2; // mono, 2 bytes/sample
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits/sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let mut f = fs::File::create(path)?;
    f.write_all(&out)
}

/// Read a 16-bit PCM `.wav`, returning `(samples, sample_rate)`. For multi-
/// channel files only channel 0 is returned.
pub fn read_wav_i16(path: impl AsRef<Path>) -> io::Result<(Vec<i16>, u32)> {
    let buf = fs::read(path)?;
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    if buf.len() < 12 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut pos = 12;
    let mut sample_rate = SAMPLE_RATE_HZ;
    let mut channels = 1u16;
    let mut bits = 16u16;
    let mut samples: Vec<i16> = Vec::new();
    let mut saw_data = false;
    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32le(&buf[pos + 4..pos + 8]) as usize;
        let body = pos + 8;
        if body + size > buf.len() {
            break;
        }
        if id == b"fmt " && size >= 16 {
            channels = u16le(&buf[body + 2..body + 4]);
            sample_rate = u32le(&buf[body + 4..body + 8]);
            bits = u16le(&buf[body + 14..body + 16]);
        } else if id == b"data" {
            if bits != 16 {
                return Err(bad("only 16-bit PCM is supported"));
            }
            let ch = channels.max(1) as usize;
            let frame_bytes = 2 * ch;
            let nframes = size / frame_bytes;
            samples.reserve(nframes);
            for i in 0..nframes {
                let o = body + i * frame_bytes; // channel 0 of each frame
                samples.push(i16::from_le_bytes([buf[o], buf[o + 1]]));
            }
            saw_data = true;
        }
        pos = body + size + (size & 1); // chunks are word-aligned
    }
    if !saw_data {
        return Err(bad("no data chunk"));
    }
    Ok((samples, sample_rate))
}

/// Replay a captured `.wav` (12 kHz mono) through the full FT1 acquisition +
/// decode pipeline (including IR-HARQ — call [`ft1::harq_reset`] first for a
/// clean state, or replay a sequence to exercise combining). Pads/truncates to
/// one frame ([`ft1::NMAX`] samples). Returns every decode found — deterministic
/// for a given file, so captured slots become regression fixtures.
pub fn decode_wav(path: impl AsRef<Path>) -> io::Result<Vec<modes::Decode>> {
    let (samples, sr) = read_wav_i16(path)?;
    if sr != SAMPLE_RATE_HZ {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("FT1 expects {SAMPLE_RATE_HZ} Hz audio, got {sr} Hz"),
        ));
    }
    let mut frame = vec![0i16; ft1::NMAX];
    let n = samples.len().min(ft1::NMAX);
    frame[..n].copy_from_slice(&samples[..n]);
    Ok(ft1::decode_frame(&frame, 200, 2900, 3, "", "", 0, 0)
        .into_iter()
        .map(Into::into)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{to_i16, VirtualAir, ON_TIME_OFFSET};
    use crate::tx;

    #[test]
    fn wav_roundtrip_decodes_captured_frame() {
        // Synthesize an off-air capture: a frame at −5 dB SNR placed in a slot.
        let msg = "CQ W9XYZ EN37";
        let frame = tx::build(msg, ft1::SAMPLE_RATE, 1500.0);
        let mut air = VirtualAir::new(ft1::SAMPLE_RATE, 11);
        let rx = to_i16(&air.receive(&frame.wave, ON_TIME_OFFSET, -5.0));

        // Write it to a .wav, then replay that file through the decoder.
        let path = std::env::temp_dir().join("ft1_wav_fixture_test.wav");
        write_wav_i16(&path, &rx, 12_000).expect("write wav");
        ft1::harq_reset();
        let decodes = decode_wav(&path).expect("decode wav");
        let _ = fs::remove_file(&path);

        assert!(
            decodes.iter().any(|d| d.message == msg),
            "replaying the captured .wav should recover the message; got {decodes:?}"
        );
    }

    #[test]
    fn read_rejects_non_wav() {
        let path = std::env::temp_dir().join("ft1_not_a_wav_test.bin");
        fs::write(&path, b"this is not a wav file at all").unwrap();
        let r = read_wav_i16(&path);
        let _ = fs::remove_file(&path);
        assert!(r.is_err(), "non-WAV input must error, not mis-parse");
    }
}
