//! Zero-config station setup: identify a connected radio from its USB descriptor.
//!
//! Two pure, testable pieces (the actual USB enumeration lives in [`crate::ports`]
//! behind the `serial` feature, and the command layer joins them):
//!
//! 1. **Driver resolver** — most ham rigs talk over a generic USB-serial bridge
//!    chip (Silicon Labs CP210x, FTDI, WCH CH340, Prolific). The chip is identified
//!    by USB **vendor id**; when its port won't bind, [`driver_hint`] points the
//!    operator at the correct *official* driver for their OS.
//! 2. **Rig matcher** — native-USB rigs report their model in the USB **product**
//!    string (e.g. `"IC-705"`). [`match_rig_model`] fuzzy-matches that against the
//!    curated [`crate::rigmodels::rig_models`] table to pre-select the Hamlib model.
//!    Rigs behind a generic bridge report only the chip name → no rig match (just a
//!    driver hint), which is the honest result.

use crate::rigmodels::rig_models;

/// A known USB-serial bridge-chip family (by USB vendor id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSerialChip {
    /// Silicon Labs CP210x — Icom, Yaesu (dual), Kenwood, Elecraft, Xiegu, …
    Cp210x,
    /// FTDI FT232/FT2232 — Elecraft, many interface cables.
    Ftdi,
    /// WCH CH340/CH341 — budget rigs + clone cables.
    Ch340,
    /// Prolific PL2303 — older cables (driver is Windows-version-sensitive).
    Prolific,
    /// An unrecognized / native-CDC device (no extra driver needed).
    Other,
}

/// Identify the USB-serial bridge chip from the device's USB **vendor id**. (PID is
/// not needed — the vendor id is what selects the driver family.)
pub fn usb_serial_chip(vid: u16) -> UsbSerialChip {
    match vid {
        0x10C4 => UsbSerialChip::Cp210x,   // Silicon Laboratories
        0x0403 => UsbSerialChip::Ftdi,     // Future Technology Devices Intl
        0x1A86 => UsbSerialChip::Ch340,    // QinHeng Electronics (WCH)
        0x067B => UsbSerialChip::Prolific, // Prolific Technology
        _ => UsbSerialChip::Other,
    }
}

/// Host OS family, for OS-aware driver guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    MacOs,
    Linux,
}

/// The host OS this binary is running on (for the live "what do I need" answer).
pub fn current_os() -> HostOs {
    #[cfg(target_os = "windows")]
    {
        HostOs::Windows
    }
    #[cfg(target_os = "macos")]
    {
        HostOs::MacOs
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        HostOs::Linux
    }
}

/// Driver guidance for a bridge chip on a given OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverHint {
    /// Human chip name, e.g. "Silicon Labs CP210x".
    pub chip: &'static str,
    /// True when the OS ships the driver in-kernel (no install needed).
    pub bundled: bool,
    /// One-line guidance.
    pub note: &'static str,
    /// Official driver download URL (empty when bundled / not applicable).
    pub url: &'static str,
}

/// What driver (if any) an operator needs for `chip` on `os` when the rig's serial
/// port doesn't appear/bind. Returns `None` for [`UsbSerialChip::Other`] (a native
/// CDC device needs no extra driver). The judgement of "bundled" is the common case
/// for modern OS versions — the note says so rather than over-promising.
pub fn driver_hint(chip: UsbSerialChip, os: HostOs) -> Option<DriverHint> {
    use HostOs::*;
    use UsbSerialChip::*;
    Some(match (chip, os) {
        (Cp210x, Windows) => DriverHint {
            chip: "Silicon Labs CP210x",
            bundled: false,
            // Modern Win10/11 install the CP210x VCP driver automatically via Windows Update
            // (this is the FT-710 / FTDX10 / FT-991A built-in USB bridge), so DON'T tell the
            // operator they must go install it — that + the old /developer-tools/ URL (now
            // dead; Silicon Labs moved it to /software-and-tools/) sent FT-710 users chasing a
            // driver that isn't on the page. Word it conditionally, like the CH340/macOS arm.
            note: "Windows usually installs the Silicon Labs CP210x driver automatically — install it manually only if the COM port never appears.",
            url: "https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers",
        },
        (Cp210x, MacOs | Linux) => DriverHint {
            chip: "Silicon Labs CP210x",
            bundled: true,
            note: "Your OS ships the CP210x driver in-kernel — no install needed.",
            url: "",
        },
        (Ftdi, Windows) => DriverHint {
            chip: "FTDI",
            bundled: false,
            note: "Windows needs the FTDI VCP driver — install it, then Retry.",
            url: "https://ftdichip.com/drivers/vcp-drivers/",
        },
        (Ftdi, MacOs | Linux) => DriverHint {
            chip: "FTDI",
            bundled: true,
            note: "Your OS ships the FTDI driver in-kernel — no install needed.",
            url: "",
        },
        (Ch340, Windows) => DriverHint {
            chip: "WCH CH340",
            bundled: false,
            note: "Windows needs the WCH CH340 (CH34x) driver — install it, then Retry.",
            url: "https://www.wch-ic.com/downloads/CH341SER_EXE.html",
        },
        (Ch340, Linux) => DriverHint {
            chip: "WCH CH340",
            bundled: true,
            note: "Linux ships the CH340 driver in-kernel — no install needed.",
            url: "",
        },
        (Ch340, MacOs) => DriverHint {
            chip: "WCH CH340",
            bundled: false,
            note: "Older macOS needs the WCH CH34x driver; recent macOS bundles it — install only if the port is missing.",
            url: "https://www.wch-ic.com/downloads/CH34XSER_MAC_ZIP.html",
        },
        (Prolific, Windows) => DriverHint {
            chip: "Prolific PL2303",
            bundled: false,
            note: "Windows needs the Prolific PL2303 driver matched to your chip revision — install it, then Retry.",
            url: "https://www.profilictech.com/",
        },
        (Prolific, MacOs | Linux) => DriverHint {
            chip: "Prolific PL2303",
            bundled: true,
            note: "Your OS ships the PL2303 driver in-kernel — no install needed.",
            url: "",
        },
        (Other, _) => return None,
    })
}

/// Normalize a model/product token for matching: keep ASCII alphanumerics only,
/// uppercased (so "IC-705", "ic 705", "IC705" all collapse to "IC705").
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Manufacturer / non-model words to ignore when extracting a rig's model tokens
/// from a friendly name (so "Icom" in "Icom IC-705" never matches a product string).
const MAKER_WORDS: &[&str] = &[
    "ICOM",
    "YAESU",
    "KENWOOD",
    "ELECRAFT",
    "FLEXRADIO",
    "TENTEC",
    "XIEGU",
    "QRP",
    "LABS",
    "ALINCO",
    "HAMLIB",
    "FLRIG",
    "RIGCTL",
    "RIGCTLD",
    "REMOTE",
    "SAT",
    "EMUL",
    "SLICE",
    "POWERSDR",
    "SMARTSDR",
    "DUMMY",
];

/// Model tokens worth matching in a friendly name. Split on whitespace and "/" only
/// (NOT internal dashes) so a model stays whole — "IC-705" → "IC705", not "IC"+"705"
/// (a bare "IC" would false-match "silICon"). Drop maker words and require length ≥ 3
/// so short line-prefixes (K3/K4/TS) can't substring-match generic descriptors.
fn model_tokens(name: &str) -> Vec<String> {
    name.split(|c: char| c.is_whitespace() || c == '/')
        .map(normalize)
        .filter(|t| t.len() >= 3 && !MAKER_WORDS.contains(&t.as_str()))
        .collect()
}

/// Best Hamlib model guess from a USB **product** (and manufacturer) string. Native-
/// USB rigs report their model there (e.g. `"IC-705"`); generic bridges report only
/// the chip (e.g. `"CP2102 USB to UART Bridge"`) → `None`. Picks the LONGEST model
/// token that appears in the haystack so "K3S" beats "K3" and "IC-7610" beats noise.
/// Skips the Hamlib built-ins (Dummy/NET/FLRig, model ≤ 4) — never a physical USB rig.
pub fn match_rig_model(product: &str, manufacturer: &str) -> Option<(u32, &'static str)> {
    let hay = normalize(&format!("{manufacturer} {product}"));
    if hay.is_empty() {
        return None;
    }
    let mut best: Option<(usize, u32, &'static str)> = None;
    for (model, name) in rig_models() {
        if model <= 4 {
            continue;
        }
        for tok in model_tokens(name) {
            if hay.contains(&tok) && best.is_none_or(|(len, ..)| tok.len() > len) {
                best = Some((tok.len(), model, name));
            }
        }
    }
    best.map(|(_, m, n)| (m, n))
}

/// A fully-resolved detection result for one connected USB radio — everything the
/// setup wizard needs to one-click configure it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRig {
    pub port_name: String,
    pub vid: u16,
    pub pid: u16,
    pub product: String,
    pub manufacturer: String,
    /// Hamlib model guessed from the product string (None = couldn't identify the
    /// rig, only the bridge chip — the operator picks the model, `driver` still helps).
    pub suggested_model: Option<u32>,
    pub suggested_model_name: Option<&'static str>,
    pub chip: UsbSerialChip,
    /// Driver guidance when the chip needs one on this OS (None = native/bundled).
    pub driver: Option<DriverHint>,
    /// Best-guess paired sound device (the rig's USB-Audio CODEC), by name.
    pub suggested_audio: Option<String>,
}

/// Join enumerated USB ports + audio device names into per-rig suggestions. Pure, so
/// the matching/pairing is testable; the command layer supplies the live enumeration.
pub fn detect_rigs(
    ports: &[crate::ports::UsbPort],
    audio: &[String],
    os: HostOs,
) -> Vec<DetectedRig> {
    ports
        .iter()
        .map(|p| {
            let (suggested_model, suggested_model_name) =
                match match_rig_model(&p.product, &p.manufacturer) {
                    Some((m, n)) => (Some(m), Some(n)),
                    None => (None, None),
                };
            let chip = usb_serial_chip(p.vid);
            DetectedRig {
                port_name: p.port_name.clone(),
                vid: p.vid,
                pid: p.pid,
                product: p.product.clone(),
                manufacturer: p.manufacturer.clone(),
                suggested_model,
                suggested_model_name,
                chip,
                driver: driver_hint(chip, os),
                suggested_audio: pair_audio(&p.product, audio),
            }
        })
        .collect()
}

/// Pick the sound device most likely to be this rig's USB-Audio CODEC: prefer a
/// device whose name references the rig's product/model, else a generic "USB Audio
/// CODEC" (the near-universal FT8 rig-audio device name). `None` if neither.
fn pair_audio(product: &str, audio: &[String]) -> Option<String> {
    let pn = normalize(product);
    if !pn.is_empty() {
        if let Some(a) = audio.iter().find(|a| normalize(a).contains(&pn)) {
            return Some(a.clone());
        }
    }
    audio
        .iter()
        .find(|a| {
            let n = a.to_ascii_uppercase();
            n.contains("USB AUDIO") || n.contains("USB CODEC")
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::UsbPort;

    #[test]
    fn chip_id_from_vendor() {
        assert_eq!(usb_serial_chip(0x10C4), UsbSerialChip::Cp210x);
        assert_eq!(usb_serial_chip(0x0403), UsbSerialChip::Ftdi);
        assert_eq!(usb_serial_chip(0x1A86), UsbSerialChip::Ch340);
        assert_eq!(usb_serial_chip(0x067B), UsbSerialChip::Prolific);
        assert_eq!(usb_serial_chip(0x1234), UsbSerialChip::Other);
    }

    #[test]
    fn driver_hint_os_aware() {
        // Windows needs a download; *nix/mac bundle CP210x.
        let w = driver_hint(UsbSerialChip::Cp210x, HostOs::Windows).unwrap();
        assert!(!w.bundled && w.url.contains("silabs"));
        assert!(
            driver_hint(UsbSerialChip::Cp210x, HostOs::Linux)
                .unwrap()
                .bundled
        );
        assert!(
            driver_hint(UsbSerialChip::Ftdi, HostOs::MacOs)
                .unwrap()
                .bundled
        );
        // A native CDC device needs nothing.
        assert_eq!(driver_hint(UsbSerialChip::Other, HostOs::Windows), None);
    }

    #[test]
    fn matches_native_usb_rig_from_product_string() {
        assert_eq!(
            match_rig_model("IC-705", "Icom Inc."),
            Some((3085, "Icom IC-705"))
        );
        assert_eq!(match_rig_model("IC-7300", ""), Some((3073, "Icom IC-7300")));
        // Case / spacing / dash insensitive.
        assert_eq!(
            match_rig_model("ft991a", "Yaesu").map(|(m, _)| m),
            Some(1035)
        );
        assert_eq!(
            match_rig_model("TS-590SG", "Kenwood").map(|(m, _)| m),
            Some(2037)
        );
    }

    #[test]
    fn longest_token_wins_for_overlapping_models() {
        // "K3S" must beat "K3" (Elecraft) — the longer, more specific token.
        assert_eq!(
            match_rig_model("Elecraft K3S", "").map(|(m, _)| m),
            Some(2043)
        );
    }

    #[test]
    fn generic_bridge_product_is_no_rig_match() {
        // A generic CP210x descriptor identifies the chip, not the rig.
        assert_eq!(
            match_rig_model("CP2102 USB to UART Bridge Controller", "Silicon Labs"),
            None
        );
        assert_eq!(match_rig_model("USB-Serial Controller", "Prolific"), None);
        assert_eq!(match_rig_model("", ""), None);
    }

    fn port(name: &str, vid: u16, product: &str, maker: &str) -> UsbPort {
        UsbPort {
            port_name: name.into(),
            vid,
            pid: 0xEA60,
            product: product.into(),
            manufacturer: maker.into(),
        }
    }

    #[test]
    fn detect_native_usb_rig_full_resolution() {
        // An IC-705 (native USB, Silicon Labs bridge) + its USB-Audio CODEC.
        let ports = vec![port("COM5", 0x10C4, "IC-705", "Icom Inc.")];
        let audio = vec![
            "Microphone (USB Audio CODEC)".to_string(),
            "Realtek HD".to_string(),
        ];
        let got = detect_rigs(&ports, &audio, HostOs::Windows);
        assert_eq!(got.len(), 1);
        let r = &got[0];
        assert_eq!(r.suggested_model, Some(3085));
        assert_eq!(r.suggested_model_name, Some("Icom IC-705"));
        assert_eq!(r.chip, UsbSerialChip::Cp210x);
        // Windows → CP210x driver hint present.
        assert!(r.driver.as_ref().is_some_and(|d| !d.bundled));
        assert_eq!(
            r.suggested_audio.as_deref(),
            Some("Microphone (USB Audio CODEC)")
        );
    }

    #[test]
    fn detect_generic_bridge_gives_driver_only() {
        // A CH340-cabled rig that reports only the chip → no model, but a driver hint
        // and (on Linux) bundled. No audio match → None.
        let ports = vec![port("/dev/ttyUSB0", 0x1A86, "USB Serial", "wch.cn")];
        let got = detect_rigs(&ports, &["Built-in Audio".into()], HostOs::Linux);
        assert_eq!(got[0].suggested_model, None);
        assert_eq!(got[0].chip, UsbSerialChip::Ch340);
        assert!(got[0].driver.as_ref().is_some_and(|d| d.bundled)); // Linux ships CH340
        assert_eq!(got[0].suggested_audio, None);
    }

    #[test]
    fn pair_audio_prefers_model_named_device_over_generic() {
        let audio = vec!["Generic USB Audio".to_string(), "IC-705 Audio".to_string()];
        assert_eq!(
            pair_audio("IC-705", &audio).as_deref(),
            Some("IC-705 Audio")
        );
        // No model-named device → falls back to the generic USB-audio device.
        assert_eq!(
            pair_audio("FT-991A", &["Generic USB Audio".into()]).as_deref(),
            Some("Generic USB Audio"),
        );
    }
}
