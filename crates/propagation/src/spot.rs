//! The spot/reception-report model — mirrors weak-signal-sleuth's `Spot` so the
//! ported detector consumes the same shape, and so the PSK Reporter / RBN
//! adapters can deserialize straight into it.

use serde::{Deserialize, Serialize};

/// One reception report (a heard signal), as ingested from PSK Reporter / RBN /
/// the radio's own decodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spot {
    /// Unix seconds (UTC) of the report.
    pub time: i64,
    /// Reported (sender) callsign.
    pub callsign: String,
    /// Maidenhead grid of the sender, if known.
    #[serde(default)]
    pub grid: Option<String>,
    /// Signal-to-noise estimate (dB).
    #[serde(default)]
    pub snr: Option<f32>,
    /// Frequency (MHz).
    #[serde(default)]
    pub frequency: Option<f32>,
    /// Mode label ("FT8", "FT4", "WSPR", …).
    #[serde(default)]
    pub mode: Option<String>,
}

impl Spot {
    /// Convenience constructor for tests / synthetic feeds.
    pub fn new(time: i64, callsign: &str, grid: &str, mode: &str) -> Self {
        Self {
            time,
            callsign: callsign.to_string(),
            grid: if grid.is_empty() {
                None
            } else {
                Some(grid.to_string())
            },
            snr: None,
            frequency: None,
            mode: if mode.is_empty() {
                None
            } else {
                Some(mode.to_string())
            },
        }
    }

    /// True if this is an FT-family mode (FT8/FT4/…), matching weak-signal-sleuth's
    /// `mode.toUpperCase().includes('FT')` reciprocal-path filter.
    pub fn is_ft_mode(&self) -> bool {
        self.mode
            .as_deref()
            .map(|m| m.to_uppercase().contains("FT"))
            .unwrap_or(false)
    }
}
