//! CAT port auto-test — find which serial port (and baud) actually drives the rig.
//!
//! Many rigs expose several COM ports (a CAT port, a second data/PTT port, a GPS or
//! tuner port); only one answers Hamlib, and which one is unlabelled and rig-specific.
//! Rather than make the operator guess, probe each candidate port: spawn a throwaway
//! `rigctld` on a private TCP port, ask it for the dial frequency (**read-only — never
//! any TX/PTT**), and keep the first (port, baud) that returns a plausible frequency.
//!
//! The pure pieces (frequency sanity, candidate building) are unit-tested here; the
//! spawn/connect orchestration ([`probe_cat_ports`]) needs `rigctld` + real ports and is
//! validated by the Windows build + on-air.

use crate::ports::UsbPort;
use crate::usbrig::match_rig_model;

/// Baud rates tried per port, most-common-for-CAT first. A port that answers at one baud
/// ends the probe immediately (auto-select), so the rare high speeds rarely run.
pub const PROBE_BAUDS: &[u32] = &[38400, 9600, 19200, 4800, 115200];

/// A plausible CAT dial frequency (Hz): 100 kHz .. 500 MHz covers HF through 2 m / 70 cm.
/// A port that isn't really the rig returns 0, an error, or garbage — all rejected.
pub fn is_plausible_cat_freq(hz: u64) -> bool {
    (100_000..=500_000_000).contains(&hz)
}

/// The winning port found by [`probe_cat_ports`] — ready to write into settings.
/// (The src-tauri command maps this to its own serde DTO; tempo-audio stays serde-free.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeHit {
    pub port_name: String,
    pub baud: u32,
    pub model: u32,
    pub model_name: String,
    pub freq_hz: u64,
    /// The model was a GUESS (a common-rig seed tried because no model was configured and the USB
    /// descriptor didn't name one). The port + baud are confirmed working, but the model is not —
    /// the UI should apply port/baud and still make the operator confirm Rig Model, since a wrong
    /// same-family model (FT-991A answering the FTDX10 probe) would carry the wrong Hamlib tables.
    pub model_seeded: bool,
}

/// A port + the Hamlib model to try on it, before the baud sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub port_name: String,
    pub model: u32,
    pub model_name: String,
    /// If set, probe ONLY this baud (a seed's family default) instead of the full sweep — keeps
    /// the no-answer worst case from ballooning to minutes across every seeded model × 5 bauds.
    pub baud: Option<u32>,
    /// True = a common-rig seed (unconfirmed model); false = a native-USB match or the configured
    /// fallback (trusted model).
    pub seeded: bool,
}

/// Common CAT rigs to try when a bridge-chip port yields no model AND the operator hasn't set one
/// yet — so Auto-test can still find the port. Yaesu (and some others) report only the bridge
/// chip's name in the USB descriptor, so Detect can't identify them; that's the whole reason
/// Auto-test exists, but it used to find nothing until a model was picked (chicken-and-egg). Kept
/// to one representative per popular family so the sweep stays quick (the first that answers wins).
/// `(hamlib_model, display_name, family_default_baud)`. Seeded probes try only the family baud.
pub const COMMON_CAT_MODELS: &[(u32, &str, u32)] = &[
    (1042, "Yaesu FTDX10", 38400),
    (1035, "Yaesu FT-991 / FT-991A", 38400),
    (1049, "Yaesu FT-710", 38400),
    (1040, "Yaesu FTDX101D", 38400),
    (3073, "Icom IC-7300", 115200),
    (2037, "Kenwood TS-590SG", 115200),
    (2029, "Elecraft K3", 38400),
];

/// Build probe candidates from enumerated USB ports. A native-USB rig (IC-705, FT-710…)
/// names its model in the USB product string → [`match_rig_model`] resolves it; a rig
/// behind a generic bridge chip (CP2102/FTDI) names only the chip → fall back to the
/// operator's currently-configured `fallback_model` (the rig is known, only the *port* is in
/// doubt), or — when no model is configured yet — seed [`COMMON_CAT_MODELS`] so Auto-test isn't
/// dead before setup. Candidates with no usable model (0) are dropped.
pub fn candidates_from(ports: &[UsbPort], fallback_model: u32) -> Vec<Candidate> {
    ports
        .iter()
        .flat_map(|p| match match_rig_model(&p.product, &p.manufacturer) {
            // Native-USB rig names its model → one exact, trusted candidate (full baud sweep).
            Some((m, name)) => vec![Candidate {
                port_name: p.port_name.clone(),
                model: m,
                model_name: name.to_string(),
                baud: None,
                seeded: false,
            }],
            // Bridge chip WITH a configured model → use it (the rig is known, only the port isn't).
            None if fallback_model > 0 => vec![Candidate {
                port_name: p.port_name.clone(),
                model: fallback_model,
                model_name: if p.product.is_empty() {
                    p.port_name.clone()
                } else {
                    p.product.clone()
                },
                baud: None,
                seeded: false,
            }],
            // Bridge chip, no model yet → try the common rigs (family baud only) so Auto-test can
            // still find the PORT; the model is a guess (flagged seeded).
            None => COMMON_CAT_MODELS
                .iter()
                .map(|(m, name, baud)| Candidate {
                    port_name: p.port_name.clone(),
                    model: *m,
                    model_name: (*name).to_string(),
                    baud: Some(*baud),
                    seeded: true,
                })
                .collect(),
        })
        .filter(|c| c.model > 0)
        .collect()
}

/// Probe every enumerated USB port for a working CAT connection and return the first
/// that reads back a plausible dial frequency (auto-select). `fallback_model` is the
/// operator's configured Hamlib model, used for ports whose USB descriptor doesn't name
/// one. `tcp_port` is a free local port for the throwaway `rigctld` (distinct from the
/// live daemon's). Read-only: it never keys the rig.
///
/// Run this when no live `rigctld` is holding the ports (the setup wizard, or a
/// not-yet-working CAT) — a daemon already bound to the real port blocks that port's
/// probe (the other ports still test fine).
#[cfg(feature = "serial")]
pub fn probe_cat_ports(fallback_model: u32, tcp_port: u16) -> Option<ProbeHit> {
    use crate::rig::Rig;
    use crate::rigctld_proc::spawn_rigctld;
    use std::time::Duration;

    let ports = crate::ports::available_usb_ports();
    let addr = format!("127.0.0.1:{tcp_port}");
    for c in candidates_from(&ports, fallback_model) {
        // A seeded (guessed-model) candidate probes only its family baud; a trusted model sweeps.
        let bauds: &[u32] = c.baud.as_ref().map(std::slice::from_ref).unwrap_or(PROBE_BAUDS);
        for &baud in bauds {
            // Throwaway daemon for this (port, baud, model) — killed on drop.
            let Ok(proc) = spawn_rigctld(c.model, &c.port_name, baud, tcp_port, false) else {
                continue;
            };
            // Let rigctld open the port + settle, then ask for the dial frequency.
            std::thread::sleep(Duration::from_millis(700));
            let hz = Rig::rigctld(&addr)
                .read_freq()
                .ok()
                .filter(|&hz| is_plausible_cat_freq(hz));
            drop(proc); // kill rigctld + free the serial port before the next attempt
            std::thread::sleep(Duration::from_millis(200));
            if let Some(freq_hz) = hz {
                return Some(ProbeHit {
                    port_name: c.port_name,
                    baud,
                    model: c.model,
                    model_name: c.model_name,
                    freq_hz,
                    model_seeded: c.seeded,
                });
            }
        }
    }
    None
}

/// Without the `serial` feature there is no port enumeration → nothing to probe.
#[cfg(not(feature = "serial"))]
pub fn probe_cat_ports(_fallback_model: u32, _tcp_port: u16) -> Option<ProbeHit> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(port: &str, product: &str, mfr: &str) -> UsbPort {
        UsbPort {
            port_name: port.to_string(),
            vid: 0,
            pid: 0,
            product: product.to_string(),
            manufacturer: mfr.to_string(),
        }
    }

    #[test]
    fn plausible_freq_accepts_ham_bands_rejects_junk() {
        assert!(is_plausible_cat_freq(14_074_000)); // 20 m
        assert!(is_plausible_cat_freq(144_200_000)); // 2 m
        assert!(!is_plausible_cat_freq(0)); // dead port
        assert!(!is_plausible_cat_freq(50)); // garbage
        assert!(!is_plausible_cat_freq(2_000_000_000)); // out of range
    }

    #[test]
    fn native_usb_rig_resolves_its_own_model_ignoring_the_fallback() {
        let cands = candidates_from(&[usb("COM5", "IC-705", "Icom Inc.")], 1);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].port_name, "COM5");
        assert!(
            cands[0].model > 4,
            "resolved a real Hamlib model, not a built-in"
        );
        assert!(cands[0].model_name.contains("705"));
    }

    #[test]
    fn bridge_chip_rig_uses_the_fallback_model() {
        let cands = candidates_from(
            &[usb("COM3", "CP2102 USB to UART Bridge", "Silicon Labs")],
            3073,
        );
        assert_eq!(cands.len(), 1);
        assert_eq!(
            cands[0].model, 3073,
            "no model in the product → operator's configured model"
        );
    }

    #[test]
    fn unmatched_port_with_no_fallback_seeds_common_models() {
        // A bridge chip with no configured model used to be dropped (Auto-test found nothing until
        // a model was picked — the FTdx10 chicken-and-egg). Now it seeds the common rigs on that
        // port so the probe can still find it.
        let cands = candidates_from(
            &[usb("COM3", "CP2102 USB to UART Bridge", "Silicon Labs")],
            0,
        );
        assert_eq!(cands.len(), COMMON_CAT_MODELS.len());
        assert!(cands.iter().all(|c| c.port_name == "COM3"));
        assert!(cands.iter().any(|c| c.model == 1042)); // FTDX10 is seeded
    }
}
