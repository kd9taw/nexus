//! Voice-keyer WAV I/O — read/write 12 kHz mono message files for the phone keyer.
//! Pure (hound + the linear resampler), no audio device: the keyer plays these through
//! the SAME TX path the soundcard CW keyer uses (see `service.rs`), so messages are
//! stored mono at the modem rate and handed straight to `backend.play()`.

use crate::resample::resample_linear;
use std::path::Path;

/// The modem/keyer sample rate. Voice messages are stored mono at this rate so playback
/// hands them straight to the backend (which resamples to the device's rate).
pub const VOICE_RATE: u32 = 12_000;

/// Write 12 kHz mono samples to a 16-bit PCM WAV at `path` (parent dirs created).
pub fn write_wav_12k(path: impl AsRef<Path>, samples: &[f32]) -> std::io::Result<()> {
    if let Some(dir) = path.as_ref().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: VOICE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(to_io)?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        w.write_sample(v).map_err(to_io)?;
    }
    w.finalize().map_err(to_io)
}

/// Atomically write a 12 kHz mono WAV: write a sibling temp then rename into place, so a
/// failed/interrupted write (disk full, permissions) never corrupts an existing recording.
pub fn write_wav_12k_atomic(path: impl AsRef<Path>, samples: &[f32]) -> std::io::Result<()> {
    let path = path.as_ref();
    let tmp = path.with_extension("wav.partial");
    write_wav_12k(&tmp, samples)?;
    std::fs::rename(&tmp, path)
}

/// Read any WAV at `path`, downmix to mono, and resample to 12 kHz f32 in [-1, 1].
/// Accepts any channel count / sample rate / int|float format (so an operator can import
/// a 48 kHz stereo recording made elsewhere and it just works).
pub fn read_wav_12k(path: impl AsRef<Path>) -> std::io::Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path).map_err(to_io)?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    // Propagate a per-sample read error (a truncated/garbled data chunk) rather than
    // silently dropping it — otherwise a corrupt import becomes a quietly-shortened
    // recording that later transmits clipped on the air.
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_io)?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            r.samples::<i32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(to_io)?
                .into_iter()
                .map(|v| v as f32 / max)
                .collect()
        }
    };
    let mono: Vec<f32> = if ch <= 1 {
        interleaved
    } else {
        interleaved
            .chunks(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    };
    Ok(resample_linear(&mono, spec.sample_rate, VOICE_RATE))
}

fn to_io(e: hound::Error) -> std::io::Error {
    std::io::Error::other(e)
}

/// A streaming WAV sink for LONG recordings (a whole QSO) — writes 12 kHz mono 16-bit PCM
/// incrementally so the audio never has to live in RAM. It auto-checkpoints (`flush`) about
/// once per second of audio, which patches the WAV header lengths on disk: so even an
/// ABNORMAL exit (crash, force-quit, power loss — where the graceful `finish` never runs)
/// leaves a file readable up to the last checkpoint, losing at most ~1 s. An explicit
/// `finish` fully finalizes. (For short fixed-size clips use [`write_wav_12k`].)
pub struct WavSink {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    /// Samples written since the last header checkpoint (flush at ~1 s of audio).
    since_flush: u32,
}

impl WavSink {
    /// Open a new 12 kHz mono WAV at `path` (parent dirs created).
    pub fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: VOICE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(path, spec).map_err(to_io)?;
        Ok(Self {
            writer,
            since_flush: 0,
        })
    }

    /// Append a chunk of 12 kHz mono samples, checkpointing the header to disk about once
    /// per second so an abnormal exit still leaves a readable file.
    pub fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            self.writer.write_sample(v).map_err(to_io)?;
        }
        self.since_flush += samples.len() as u32;
        if self.since_flush >= VOICE_RATE {
            self.since_flush = 0;
            self.writer.flush().map_err(to_io)?; // patches header + flushes the BufWriter
        }
        Ok(())
    }

    /// Finalize the WAV header (length) and close the file.
    pub fn finish(self) -> std::io::Result<()> {
        self.writer.finalize().map_err(to_io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nexus_voice_{}_{}", std::process::id(), name))
    }

    #[test]
    fn wav_round_trips_at_12k() {
        let dir = scratch("rt");
        let p = dir.join("m.wav");
        let samples: Vec<f32> = (0..1200).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        write_wav_12k(&p, &samples).unwrap();
        let back = read_wav_12k(&p).unwrap();
        assert_eq!(back.len(), samples.len(), "same length at 12 kHz");
        for (a, b) in samples.iter().zip(&back) {
            assert!(
                (a - b).abs() < 1e-3,
                "within 16-bit quantization: {a} vs {b}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_48k_stereo_downmixes_and_resamples() {
        // Simulate an imported 48 kHz stereo WAV; read_wav_12k must downmix + resample.
        let dir = scratch("imp");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for i in 0..4800 {
            let v = ((i as f32 * 0.02).sin() * 0.4 * i16::MAX as f32) as i16;
            w.write_sample(v).unwrap(); // L
            w.write_sample(v).unwrap(); // R (same → mono == that signal)
        }
        w.finalize().unwrap();
        let out = read_wav_12k(&p).unwrap();
        // 4800 stereo frames @ 48k → 0.1 s → ~1200 samples @ 12 kHz.
        assert!(
            (out.len() as i64 - 1200).abs() <= 2,
            "resampled length ~1200: {}",
            out.len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wav_sink_streams_chunks_to_a_valid_wav() {
        // The streaming sink (QSO recording) must produce a WAV that reads back identical to
        // the concatenation of the chunks written.
        let dir = scratch("sink");
        let p = dir.join("qso.wav");
        let chunks: Vec<Vec<f32>> = (0..5)
            .map(|c| {
                (0..240)
                    .map(|i| ((c * 240 + i) as f32 * 0.03).sin() * 0.4)
                    .collect()
            })
            .collect();
        let mut sink = WavSink::create(&p).unwrap();
        for ch in &chunks {
            sink.write(ch).unwrap();
        }
        sink.finish().unwrap();
        let flat: Vec<f32> = chunks.concat();
        let back = read_wav_12k(&p).unwrap();
        assert_eq!(back.len(), flat.len(), "all streamed chunks present");
        for (a, b) in flat.iter().zip(&back) {
            assert!(
                (a - b).abs() < 1e-3,
                "within 16-bit quantization: {a} vs {b}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
