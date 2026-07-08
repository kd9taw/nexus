//! Simple linear-interpolation resampling between the sound card's native rate
//! and the modem's 12 kHz. Per-block and stateless: tiny boundary glitches are
//! negligible for narrow-band FT1 audio, and it has no dependencies. (A polyphase
//! resampler can replace this later if needed.)

/// Resample `input` from `in_rate` to `out_rate` (Hz) by linear interpolation.
pub fn resample_linear(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == 0 || out_rate == 0 {
        return Vec::new();
    }
    if in_rate == out_rate {
        return input.to_vec();
    }
    let ratio = out_rate as f64 / in_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let frac = (src - i0 as f64) as f32;
        let a = input.get(i0).copied().unwrap_or(0.0);
        let b = input.get(i0 + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimate_48k_to_12k_quarters_length_and_preserves_dc() {
        let n = 48_000;
        let out = resample_linear(&vec![1.0f32; n], 48_000, 12_000);
        assert!((out.len() as i64 - 12_000).abs() <= 1, "len {}", out.len());
        assert!(out.iter().all(|&x| (x - 1.0).abs() < 1e-3));
    }

    #[test]
    fn upsample_12k_to_48k_quadruples_length() {
        let out = resample_linear(&vec![0.5f32; 12_000], 12_000, 48_000);
        assert!((out.len() as i64 - 48_000).abs() <= 1, "len {}", out.len());
    }

    #[test]
    fn equal_rate_is_identity() {
        let s = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&s, 12_000, 12_000), s);
    }
}
