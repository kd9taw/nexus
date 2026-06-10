//! A rolling buffer of the most recent 4 seconds of received audio.
//!
//! Captured audio arrives continuously in small chunks; the FT1 decoder wants a
//! whole 4-second ([`FRAME_LEN`]) frame at each slot boundary. [`RxRing`] keeps
//! the latest `FRAME_LEN` samples; the runtime snapshots it once per RX slot.
//! The decoder performs its own fine timing search within the window, so exact
//! sub-sample alignment is not required here.

use std::collections::VecDeque;
use tempo_core::ft1;

/// Samples in one 4-second frame at 12 kHz (= `ft1::NMAX`).
pub const FRAME_LEN: usize = ft1::NMAX;

/// Rolling buffer holding the latest `cap` audio samples (one frame window).
///
/// `cap` is the tier's frame length — [`FRAME_LEN`] (48000, 4 s) for FT1, or the
/// longer DX1 capture window (15 s). The radio loop rebuilds the ring with the
/// right capacity when the operator switches tier.
#[derive(Debug)]
pub struct RxRing {
    buf: VecDeque<f32>,
    cap: usize,
}

impl Default for RxRing {
    fn default() -> Self {
        Self::new()
    }
}

impl RxRing {
    /// A ring sized for an FT1 frame ([`FRAME_LEN`] samples).
    pub fn new() -> Self {
        Self::with_capacity(FRAME_LEN)
    }

    /// A ring holding the latest `cap` samples (the tier's frame window).
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// The window length this ring retains.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Resize the retained window to `cap`, keeping the most recent samples.
    /// Used when the operator switches mode/tier (FT8 = 180000, FT4 = 72576,
    /// FT1 = 48000 samples) so the next decode frame is the right length.
    pub fn resize(&mut self, cap: usize) {
        self.cap = cap;
        while self.buf.len() > cap {
            self.buf.pop_front();
        }
        if cap > self.buf.capacity() {
            self.buf.reserve(cap - self.buf.len());
        }
    }

    /// Append captured samples, dropping the oldest beyond the capacity.
    pub fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples.iter().copied());
        while self.buf.len() > self.cap {
            self.buf.pop_front();
        }
    }

    /// The current frame: exactly `cap` samples, front-zero-padded if we have
    /// not yet captured a full window.
    pub fn frame(&self) -> Vec<f32> {
        if self.buf.len() == self.cap {
            return self.buf.iter().copied().collect();
        }
        let mut out = vec![0.0f32; self.cap - self.buf.len()];
        out.extend(self.buf.iter().copied());
        out
    }

    /// A frame for the WSJT-X-style EARLY decode pass: the latest `n` captured
    /// samples placed at the START of the window (their true position relative
    /// to the slot boundary), zero-padded to `cap` at the TAIL. `frame()`
    /// front-pads instead — which would shift a partial slot's audio toward the
    /// window end and push every decode's dt out of the sync search range.
    /// Taking only the latest `n` also drops any tail of the PREVIOUS slot still
    /// rolling in the ring (consecutive RX slots), which would otherwise sit at
    /// the front of the window and corrupt the time alignment.
    pub fn frame_latest_padded(&self, n: usize) -> Vec<f32> {
        let take = n.min(self.buf.len()).min(self.cap);
        let skip = self.buf.len() - take;
        let mut out: Vec<f32> = self.buf.iter().skip(skip).copied().collect();
        out.resize(self.cap, 0.0);
        out
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    pub fn is_full(&self) -> bool {
        self.buf.len() == self.cap
    }
    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_latest_padded_places_audio_at_front_with_zero_tail() {
        let mut r = RxRing::with_capacity(10);
        r.push(&[9.0, 9.0, 1.0, 2.0, 3.0]); // 9s = stale previous-slot tail
        let f = r.frame_latest_padded(3);
        assert_eq!(f.len(), 10, "always the full window length");
        assert_eq!(&f[..3], &[1.0, 2.0, 3.0], "latest n at the FRONT");
        assert!(f[3..].iter().all(|&x| x == 0.0), "zero TAIL padding");
        // Asking for more than captured takes everything, still tail-padded.
        let f = r.frame_latest_padded(99);
        assert_eq!(&f[..5], &[9.0, 9.0, 1.0, 2.0, 3.0]);
        assert!(f[5..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn frame_is_always_frame_len_and_holds_latest() {
        let mut r = RxRing::new();
        r.push(&[1.0; 1000]);
        let f = r.frame();
        assert_eq!(f.len(), FRAME_LEN);
        // Front zero-padded, latest samples at the end.
        assert_eq!(f[FRAME_LEN - 1], 1.0);
        assert_eq!(f[0], 0.0);

        // Overfill: keeps only the most recent FRAME_LEN.
        r.push(&vec![2.0; FRAME_LEN]);
        let f = r.frame();
        assert!(r.is_full());
        assert!(f.iter().all(|&x| x == 2.0));
    }

    #[test]
    fn full_slot_ring_keeps_the_signal_head_unlike_an_nmax_ring() {
        // FT4: the slot (90000 = 7.5 s) is longer than the decode frame
        // (NMAX = 72576 = 6.048 s). The signal sits at the slot HEAD (leading
        // Costas sync). A ring sized to the FULL SLOT retains the head; a ring
        // sized to only NMAX keeps the slot TAIL and drops the head — the bug.
        const NMAX: usize = 72_576;
        const SLOT: usize = 90_000;
        let lead = SLOT - NMAX; // 17424 = the head an NMAX ring drops
        // Mark the leading sync (2.0) distinctly from the signal body (1.0).
        let mut sig = vec![2.0f32; lead];
        sig.extend(std::iter::repeat(1.0f32).take(SLOT - lead));

        // Full-slot ring (the fix): the leading sync is retained at the head.
        let mut good = RxRing::with_capacity(SLOT);
        good.push(&sig);
        let f = good.frame();
        assert_eq!(f.len(), SLOT);
        assert_eq!(f[0], 2.0, "full-slot ring retains the leading sync");

        // NMAX-sized ring (the old bug): keeps the latest NMAX = the slot tail, so
        // the leading sync is gone entirely.
        let mut buggy = RxRing::with_capacity(NMAX);
        buggy.push(&sig);
        assert_eq!(buggy.frame()[0], 1.0, "NMAX ring starts mid-signal");
        assert!(
            !buggy.frame().iter().any(|&x| x == 2.0),
            "NMAX ring has dropped the leading sync entirely (the bug)"
        );
    }
}
