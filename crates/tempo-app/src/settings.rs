//! Operator / station settings — persisted by the shell as JSON so the user
//! configures the app without recompiling.
//!
//! `#[serde(default)]` makes every field optional on load, so older settings
//! files (and UI forms that don't yet send every field) still deserialize.

use crate::dto::SourceKind;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Everything the operator configures: identity, band/frequency, Field Day
/// exchange, rig/PTT control, and network (WSJT-X UDP API + PSK Reporter).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    // --- identity / operating ---
    pub mycall: String,
    pub mygrid: String,
    pub band: String,
    pub dial_mhz: f64,
    pub sideband: String,
    /// ARRL Field Day class, e.g. "1D", "3A".
    pub fd_class: String,
    /// ARRL/RAC section, e.g. "WI".
    pub fd_section: String,
    /// Periodically transmit a presence beacon ("CQ <call> <grid>") in Chat
    /// mode. **Off by default** — the app starts passive (hunt-and-pounce):
    /// it listens and only transmits when the operator acts (sends a message,
    /// answers, calls CQ, or enables this).
    pub beacon: bool,
    /// IR-HARQ: buffer failed RV0 frames and joint-combine RV1/RV2
    /// retransmissions at the receiver, and escalate the redundancy version on
    /// unacknowledged QSO transmissions. **On by default.** Turn off to force
    /// RV0-only (each frame decoded independently) — useful for A/B comparison
    /// or as a fallback.
    pub harq_enabled: bool,

    // --- rig / PTT ---
    /// PTT method: "cat" (Tempo launches/uses rigctld), "rts", "dtr", or "vox".
    pub ptt_method: String,
    /// Hamlib rig model number (for rigctld `-m`). 0 = none / VOX.
    pub rig_model: u32,
    /// Friendly rig name (display only).
    pub rig_model_name: String,
    /// Serial port for CAT / serial-PTT, e.g. "COM5" or "/dev/ttyUSB0" ("" = none).
    pub serial_port: String,
    /// Serial baud rate for CAT.
    pub baud: u32,
    /// Local TCP port Tempo uses for rigctld (it spawns rigctld on this port).
    pub rigctld_port: u16,

    // --- network (WSJT-X parity) ---
    /// Emit the WSJT-X-compatible UDP protocol (for JTAlert/GridTracker/loggers).
    pub wsjtx_udp: bool,
    /// UDP address to send WSJT-X messages to (WSJT-X default is 127.0.0.1:2237).
    pub wsjtx_udp_addr: String,
    /// UDP address to *listen* on for an upstream WSJT-X/JTDX/MSHV decode stream
    /// when the signal source is Companion (the sink those apps transmit to;
    /// WSJT-X default 127.0.0.1:2237).
    pub companion_addr: String,
    /// Persisted RX signal source — native decode vs a WSJT-X/JTDX/MSHV companion
    /// stream. Restored at startup so the operator's choice survives restart.
    pub source: SourceKind,
    /// Upload heard stations to PSK Reporter.
    pub pskreporter: bool,

    // --- audio I/O ---
    /// Input (capture) device name. Empty = system default input.
    pub audio_in: String,
    /// Output (playback) device name. Empty = system default output.
    pub audio_out: String,
    /// Tx audio level (0.0–1.0) applied to outgoing samples before they reach
    /// the sound card.
    pub tx_level: f32,
    /// Transmit watchdog: auto-halt TX after this many minutes of continuous
    /// keying. 0 = off.
    pub tx_watchdog_min: u32,

    // --- timing & tuning (FT8-style) ---
    /// Transmit on the even ("1st") T/R slots when true, odd ("2nd") when false.
    /// Two stations must pick OPPOSITE periods to complete a QSO (like WSJT-X's
    /// "Tx even/1st").
    pub tx_even: bool,
    /// Receive audio offset (Hz) — the green waterfall marker; where the operator
    /// is listening for the station being worked.
    pub rx_offset_hz: f32,
    /// Transmit audio offset (Hz) — the red waterfall marker; where our signal is
    /// placed in the SSB passband.
    pub tx_offset_hz: f32,
    /// Keep the TX offset fixed when the RX offset changes (WSJT-X "Hold Tx Freq").
    /// When false, setting RX (left-click) also moves TX to match.
    pub hold_tx_freq: bool,
    /// Periodically query an NTP server to show the real PC-clock-vs-UTC offset.
    /// On by default; fails silently when off-grid. Disable for fully-offline use.
    pub clock_check: bool,

    // --- logbook ---
    /// Auto-log a contact to the ADIF logbook when a QSO completes. On by
    /// default — every completed auto-sequenced QSO is recorded once.
    pub auto_log: bool,

    // --- coordinated QSY ("move together") — a SEPARATE, opt-in function ---
    /// Master opt-in for coordinated QSY. **Off by default** and fully isolated:
    /// while false, the engine never emits or acts on a QSY directive and the
    /// primary Chat/QSO/Field-Day modes behave exactly as without the feature.
    /// Announced-in-the-clear only — NOT encryption / NOT a secret hop.
    pub qsy_enabled: bool,
    /// The set of band-plan channel tokens (e.g. "20m", "40m", "70cm") the
    /// initiator round-robins through when hopping. Empty = nowhere to move.
    pub qsy_set: Vec<String>,
    /// Announce cadence: the initiator hops every this-many of its TX overs.
    /// Conservative by default so it reads as a normal QSY, not a hopping pattern.
    pub qsy_cadence: u64,

    // --- alerts / comforts ---
    /// Alert (sound + visual) when your callsign is decoded (someone calling you).
    pub alert_my_call: bool,
    /// Alert on a decoded CQ.
    pub alert_cq: bool,
    /// Alert when a new (not previously heard) station is decoded.
    pub alert_new: bool,
    /// Editable quick-reply macros per mode (the Composer chips).
    pub macros: Macros,
}

/// Editable quick-reply macro sets per mode (shown as Composer chips). Field Day
/// uses the live class+section exchange, so it isn't user-editable here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Macros {
    pub chat: Vec<String>,
    pub qso: Vec<String>,
    pub band: Vec<String>,
}

impl Default for Macros {
    fn default() -> Self {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        Self {
            chat: v(&["73", "QSL", "Name?", "QTH?", "CQ"]),
            qso: v(&["R-09", "RRR", "RR73", "73"]),
            band: v(&["CQ CQ", "QRZ?", "Net check-in", "73 to all"]),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mycall: "KD9TAW".to_string(),
            mygrid: "EN52".to_string(),
            band: "20m".to_string(),
            dial_mhz: 14.0905,
            sideband: "USB".to_string(),
            fd_class: "1D".to_string(),
            fd_section: "WI".to_string(),
            beacon: false,
            harq_enabled: true,
            ptt_method: "vox".to_string(),
            rig_model: 0,
            rig_model_name: "None / VOX".to_string(),
            serial_port: String::new(),
            baud: 38400,
            rigctld_port: 4532,
            wsjtx_udp: false,
            wsjtx_udp_addr: "127.0.0.1:2237".to_string(),
            companion_addr: "127.0.0.1:2237".to_string(),
            source: SourceKind::Native,
            pskreporter: false,
            audio_in: String::new(),
            audio_out: String::new(),
            tx_level: 0.9,
            tx_watchdog_min: 6,
            tx_even: true,
            rx_offset_hz: 1500.0,
            tx_offset_hz: 1500.0,
            hold_tx_freq: false,
            clock_check: true,
            auto_log: true,
            qsy_enabled: false,
            qsy_set: vec!["20m".to_string(), "40m".to_string(), "30m".to_string()],
            qsy_cadence: tempo_core::qsy::DEFAULT_CADENCE,
            alert_my_call: true,
            alert_cq: false,
            alert_new: false,
            macros: Macros::default(),
        }
    }
}

impl Settings {
    /// Load settings from `path`, or return defaults if missing/invalid.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist settings to `path` (creating parent directories).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Dial frequency in Hz (for the rig / PSK Reporter).
    pub fn dial_hz(&self) -> u64 {
        (self.dial_mhz * 1_000_000.0).round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_json_camelcase() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"mycall\":\"KD9TAW\""));
        assert!(json.contains("\"fdClass\"") && json.contains("\"pttMethod\""));
        assert!(json.contains("\"wsjtxUdpAddr\"") && json.contains("\"rigModel\""));
        assert!(json.contains("\"txEven\"") && json.contains("\"rxOffsetHz\""));
        assert!(json.contains("\"txOffsetHz\"") && json.contains("\"holdTxFreq\""));
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(s.dial_hz(), 14_090_500); // default = Tempo's 20 m channel
    }

    #[test]
    fn partial_json_fills_defaults() {
        // An old/partial settings file with only identity fields still loads.
        let partial = r#"{"mycall":"W9XYZ","mygrid":"EN37"}"#;
        let s: Settings = serde_json::from_str(partial).unwrap();
        assert_eq!(s.mycall, "W9XYZ");
        assert_eq!(s.ptt_method, "vox"); // default
        assert_eq!(s.rigctld_port, 4532); // default
        assert_eq!(s.wsjtx_udp_addr, "127.0.0.1:2237"); // default
    }

    #[test]
    fn save_then_load() {
        let path = std::env::temp_dir()
            .join("tempo_settings_test2")
            .join("settings.json");
        let s = Settings {
            mycall: "W9XYZ".into(),
            serial_port: "/dev/ttyUSB0".into(),
            ptt_method: "cat".into(),
            ..Settings::default()
        };
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.mycall, "W9XYZ");
        assert_eq!(back.serial_port, "/dev/ttyUSB0");
        assert_eq!(back.ptt_method, "cat");
        let _ = std::fs::remove_file(&path);
    }
}
