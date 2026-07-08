//! FT1 transmit/receive slot timing.
//!
//! FT1 uses a fixed 4.0 s T/R period aligned to UTC (4-second intervals from
//! midnight, equivalently from the Unix epoch). The transmitted waveform is
//! 3.536 s, leaving ~0.46 s of guard + decode time; a 200 ms pre-TX guard and a
//! ±80 ms timing tolerance apply.
//!
//! All functions are parameterized by an explicit `now_ms` (milliseconds since
//! the Unix epoch, UTC) so the slot math is pure and unit-testable. Use
//! [`now_unix_ms`] to sample the real clock.

/// FT1 T/R period in seconds.
pub const PERIOD_S: f64 = 4.0;
/// FT1 T/R period in milliseconds.
pub const PERIOD_MS: f64 = PERIOD_S * 1000.0;
/// DX1 (robust tier) T/R period in seconds — a 15 s slot like FT8.
pub const DX1_PERIOD_S: f64 = 15.0;
/// Pre-TX guard (silence before the waveform) in milliseconds.
pub const PRE_TX_GUARD_MS: f64 = 200.0;
/// Slot-boundary timing tolerance in milliseconds.
pub const TIMING_TOLERANCE_MS: f64 = 80.0;

/// Current wall-clock time in milliseconds since the Unix epoch (UTC).
pub fn now_unix_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs_f64()
        * 1000.0
}

/// A slot clock for a fixed-period TDMA mode (FT1 by default).
#[derive(Clone, Copy, Debug)]
pub struct SlotClock {
    pub period_ms: f64,
    pub pre_tx_guard_ms: f64,
    pub tolerance_ms: f64,
}

impl Default for SlotClock {
    fn default() -> Self {
        Self {
            period_ms: PERIOD_MS,
            pre_tx_guard_ms: PRE_TX_GUARD_MS,
            tolerance_ms: TIMING_TOLERANCE_MS,
        }
    }
}

impl SlotClock {
    /// FT1 slot clock (4.0 s period).
    pub fn ft1() -> Self {
        Self::default()
    }

    /// DX1 robust-tier slot clock (15.0 s period).
    pub fn dx1() -> Self {
        Self {
            period_ms: DX1_PERIOD_S * 1000.0,
            ..Self::default()
        }
    }

    /// A slot clock for an arbitrary T/R period in seconds — used to drive the
    /// clock from the active mode (FT8 = 15, FT4 = 7.5, FT1 = 4, DX1 = 15).
    pub fn with_period_secs(secs: f64) -> Self {
        Self {
            period_ms: secs * 1000.0,
            ..Self::default()
        }
    }

    /// Milliseconds elapsed into the current slot.
    pub fn phase_ms(&self, now_ms: f64) -> f64 {
        now_ms.rem_euclid(self.period_ms)
    }

    /// Milliseconds until the next slot boundary (0 exactly on a boundary).
    pub fn ms_to_next_slot(&self, now_ms: f64) -> f64 {
        let p = self.phase_ms(now_ms);
        if p == 0.0 {
            0.0
        } else {
            self.period_ms - p
        }
    }

    /// Monotonic slot index since the epoch.
    pub fn slot_index(&self, now_ms: f64) -> u64 {
        (now_ms / self.period_ms).floor() as u64
    }

    /// Absolute ms-since-epoch of the start of the next slot.
    pub fn next_boundary_ms(&self, now_ms: f64) -> f64 {
        (self.slot_index(now_ms) as f64 + 1.0) * self.period_ms
    }

    /// True if `now_ms` is within timing tolerance of a slot boundary — i.e. a
    /// good moment to start assembling/decoding an RX frame.
    pub fn within_tolerance(&self, now_ms: f64) -> bool {
        let p = self.phase_ms(now_ms);
        p <= self.tolerance_ms || (self.period_ms - p) <= self.tolerance_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_and_next_slot() {
        let c = SlotClock::ft1();
        // 10.5 s past epoch -> 2.5 s into the 3rd slot (index 2).
        let now = 10_500.0;
        assert!((c.phase_ms(now) - 2_500.0).abs() < 1e-9);
        assert!((c.ms_to_next_slot(now) - 1_500.0).abs() < 1e-9);
        assert_eq!(c.slot_index(now), 2);
        assert!((c.next_boundary_ms(now) - 12_000.0).abs() < 1e-9);
    }

    #[test]
    fn on_boundary() {
        let c = SlotClock::ft1();
        assert_eq!(c.phase_ms(8_000.0), 0.0);
        assert_eq!(c.ms_to_next_slot(8_000.0), 0.0);
        assert_eq!(c.slot_index(8_000.0), 2);
    }

    #[test]
    fn tolerance_window() {
        let c = SlotClock::ft1();
        assert!(c.within_tolerance(8_050.0)); // 50 ms after boundary
        assert!(c.within_tolerance(7_950.0)); // 50 ms before boundary
        assert!(!c.within_tolerance(8_500.0)); // 500 ms in — outside
    }
}
