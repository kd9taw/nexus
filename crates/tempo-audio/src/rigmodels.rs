//! Hamlib rig model numbers for the rig-control UI, split into two tiers.
//!
//! [`rig_models`] is the **verified** tier: `(model, name)` pairs we've
//! sanity-checked and that cover the radios most likely to be used for Field
//! Day / digital work. It's what the default Settings dropdown shows.
//!
//! [`all_rig_models`] is the **full** catalog: the verified tier plus a much
//! broader set of common amateur transceivers so an operator whose exact rig
//! isn't in the short list can still find it (surface this behind a "show all
//! models" toggle). Rigctld actually speaks 250+ models; this covers the
//! common transmitting amateur rigs across the major makers. For anything
//! still not listed, the operator can type the raw Hamlib model number — the
//! definitive list for their installed Hamlib is `rigctl -l`.
//!
//! Every number here is anchored on Hamlib's `include/hamlib/riglist.h`
//! (`model = 1000 * backend + index`, e.g. Dummy=1, NET=2, FLRig=4,
//! IC-7300=3073). Model indices are append-only and never renumbered across
//! Hamlib releases, so a number verified in one 4.x release holds in the next.

/// `(hamlib_model_number, friendly_name)` for the curated **verified** set of
/// common rigs. This is the default UI list.
///
/// Ordered roughly Dummy/NET first, then by manufacturer. Not exhaustive —
/// for the long tail use [`all_rig_models`] or type the model number directly;
/// `rigctl -l` is the definitive list for the operator's Hamlib version.
pub fn rig_models() -> Vec<(u32, &'static str)> {
    // Numbers verified against Hamlib 4.7.1 `include/hamlib/riglist.h`:
    // model = 1000 * backend + index (RIG_MAKE_MODEL), anchored on
    // Dummy=1, NET=2, FLRig=4, IC-7300=3073. Curated to common amateur rigs;
    // `rigctl -l` is the definitive list for the operator's Hamlib version.
    vec![
        // Hamlib built-ins
        (1, "Hamlib Dummy"),
        (2, "NET rigctl (remote rigctld)"),
        (4, "FLRig (flrig)"),
        // Icom
        (3073, "Icom IC-7300"),
        (3085, "Icom IC-705"),
        (3078, "Icom IC-7610"),
        (3081, "Icom IC-9700"),
        (3070, "Icom IC-7100"),
        (3013, "Icom IC-718"),
        (3060, "Icom IC-7000"),
        (3023, "Icom IC-746"),
        (3046, "Icom IC-746PRO"),
        (3057, "Icom IC-756PROIII"),
        (3044, "Icom IC-910"),
        (3090, "Icom IC-905"),
        // Yaesu
        (1035, "Yaesu FT-991 / FT-991A"),
        (1049, "Yaesu FT-710"),
        (1042, "Yaesu FTDX10"),
        (1040, "Yaesu FTDX101D"),
        (1044, "Yaesu FTDX101MP"),
        (1036, "Yaesu FT-891"),
        (1022, "Yaesu FT-857 / FT-857D"),
        (1043, "Yaesu FT-897D"),
        (1020, "Yaesu FT-817 / FT-817ND"),
        (1041, "Yaesu FT-818 / FT-818ND"),
        (1046, "Yaesu FT-450D"),
        (1028, "Yaesu FT-950"),
        (1029, "Yaesu FT-2000"),
        (1034, "Yaesu FTDX1200"),
        (1037, "Yaesu FTDX3000"),
        (1032, "Yaesu FTDX5000"),
        (1024, "Yaesu FT-1000MP"),
        // Kenwood
        (2031, "Kenwood TS-590S"),
        (2037, "Kenwood TS-590SG"),
        (2041, "Kenwood TS-890S"),
        (2039, "Kenwood TS-990S"),
        (2028, "Kenwood TS-480 (SAT/HX)"),
        (2014, "Kenwood TS-2000"),
        (2010, "Kenwood TS-870S"),
        (2009, "Kenwood TS-850"),
        // Elecraft (Kenwood-family backend)
        (2029, "Elecraft K3"),
        (2043, "Elecraft K3S"),
        (2047, "Elecraft K4"),
        (2044, "Elecraft KX2"),
        (2045, "Elecraft KX3"),
        // FlexRadio. 2036 is the WSJT-X-proven path: it speaks the Flex dialect
        // of Kenwood CAT served by the SmartSDR CAT app's TCP/serial ports on
        // the PC. SmartSDR CAT's DEFAULT TCP port 5002 is directed at slice A;
        // its per-slice extras are B=60001, C=60002, D=60003 (60000 base).
        // 23005 talks the radio's native API directly and is alpha-grade in
        // Hamlib (failed on a real 6400M with WSAEADDRNOTAVAIL) — keep it
        // selectable, but nothing auto-picks it anymore.
        (2036, "FlexRadio FLEX-6xxx (SmartSDR CAT)"),
        (23005, "FlexRadio SmartSDR native (experimental)"),
        (2048, "FlexRadio PowerSDR (TS-2000 emul.)"),
        // Ten-Tec
        (16013, "Ten-Tec Eagle (599)"),
        (16008, "Ten-Tec Orion (565)"),
        (16011, "Ten-Tec Omni VII (588)"),
        // Xiegu (Icom-family backend)
        (3088, "Xiegu G90"),
        (3087, "Xiegu X6100"),
        (3091, "Xiegu X6200"),
        (3089, "Xiegu X5105"),
        // QRP Labs
        (2057, "QRP Labs QMX"),
        // Alinco
        (17002, "Alinco DX-SR8"),
    ]
}

/// The additional (beyond-verified) common amateur transceivers that fill out
/// the full catalog. Kept private; callers want [`all_rig_models`] (verified +
/// these) or [`rig_models`] (verified only).
///
/// Every number is anchored on Hamlib's `riglist.h` (`model = 1000 * backend +
/// index`). These are drawn from the authoritative header — no guessed numbers.
fn extended_rig_models() -> Vec<(u32, &'static str)> {
    vec![
        // Hamlib built-in bridges (backend 0)
        (5, "TRX-Manager (rig control)"),
        (7, "TCI (SunSDR / ExpertSDR)"),
        // Icom (backend 3)
        (3011, "Icom IC-706MKIIG"),
        (3010, "Icom IC-706MKII"),
        (3009, "Icom IC-706"),
        (3055, "Icom IC-703"),
        (3061, "Icom IC-7200"),
        (3067, "Icom IC-7410"),
        (3062, "Icom IC-7700"),
        (3063, "Icom IC-7600"),
        (3056, "Icom IC-7800"),
        (3068, "Icom IC-9100"),
        (3026, "Icom IC-756"),
        (3027, "Icom IC-756PRO"),
        (3047, "Icom IC-756PROII"),
        (3019, "Icom IC-735"),
        // Yaesu (backend 1)
        (1001, "Yaesu FT-847"),
        (1010, "Yaesu FT-736R"),
        (1021, "Yaesu FT-100 / FT-100D"),
        (1023, "Yaesu FT-897"),
        (1027, "Yaesu FT-450"),
        (1014, "Yaesu FT-920"),
        (1030, "Yaesu FTDX9000"),
        (1004, "Yaesu FT-1000MP Mark-V"),
        (1016, "Yaesu FT-990"),
        // Kenwood (backend 2)
        (2001, "Kenwood TS-50S"),
        (2002, "Kenwood TS-440S"),
        (2003, "Kenwood TS-450S"),
        (2004, "Kenwood TS-570D"),
        (2016, "Kenwood TS-570S"),
        (2005, "Kenwood TS-690S"),
        (2007, "Kenwood TS-790"),
        (2011, "Kenwood TS-940S"),
        (2013, "Kenwood TS-950SDX"),
        (2025, "Kenwood TS-140S"),
        (2034, "Kenwood TM-D710"),
        (2035, "Kenwood TM-V71"),
        // Kenwood-family backend: Elecraft K2 + SDRs that speak Kenwood CAT
        (2021, "Elecraft K2"),
        (2040, "OpenHPSDR / PiHPSDR"),
        (2049, "Malachite DSP SDR"),
        (2050, "Lab599 Discovery TX-500"),
        (2051, "SDRuno (SDRplay)"),
        // Ten-Tec (backend 16)
        (16001, "Ten-Tec TT-550 Pegasus"),
        (16002, "Ten-Tec TT-538 Jupiter"),
        (16007, "Ten-Tec TT-516 Argonaut V"),
        (16009, "Ten-Tec TT-585 Paragon"),
        // Xiegu (Icom-family backend)
        (3076, "Xiegu X108G"),
        // Alinco (backend 17)
        (17001, "Alinco DX-77"),
    ]
}

/// The full rig catalog: the [`rig_models`] verified set first (in curated
/// order), then [`extended_rig_models`] grouped by manufacturer. Use this for
/// the "show all models" view; the two tiers never share a model number, so a
/// caller can badge verified entries by membership in [`rig_models`].
pub fn all_rig_models() -> Vec<(u32, &'static str)> {
    rig_models()
        .into_iter()
        .chain(extended_rig_models())
        .collect()
}

/// Friendly name for a Hamlib model number, if it's anywhere in the full
/// catalog (verified or extended). Returns `None` for an unknown number, so a
/// typed-in model that Hamlib supports but we don't name still shows as blank
/// rather than mislabeled.
pub fn rig_model_name(model: u32) -> Option<&'static str> {
    all_rig_models()
        .into_iter()
        .find(|(m, _)| *m == model)
        .map(|(_, name)| name)
}

/// The kind of NATIVE spectrum stream a radio can provide — the shared capability gate for
/// the per-radio panadapter (Wave 7 Flex + Wave 8 Icom converge here). `None` (from
/// [`native_spectrum_kind`]) means the universal audio-FFT scope is the only option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpectrumKind {
    /// Icom CI-V spectrum scope (command `0x27`) — requires Nexus to own the serial CI-V
    /// port natively. Carries the rig's default CI-V bus address.
    IcomCiv { civ_addr: u8 },
    /// FlexRadio SmartSDR panadapter over VITA-49 UDP (the `sub pan` / `display pan` stream).
    FlexVita,
}

/// Map a curated Hamlib model number to the Icom rig whose native CI-V scope Nexus supports.
/// Only the 7300-family radios that actually expose the `0x27` scope stream are listed.
fn icom_scope_model(model: u32) -> Option<crate::civ::commands::IcomModel> {
    use crate::civ::commands::IcomModel::*;
    Some(match model {
        3073 => Ic7300,
        3078 => Ic7610,
        3081 => Ic9700,
        3085 => Ic705,
        3090 => Ic905,
        _ => return None,
    })
}

/// Classify what native spectrum stream a radio offers, given its Hamlib model number and
/// its connection kind (`"serial"` / `"network"`). This is the single gate both native
/// panadapter workers consult:
/// - **Flex** (SmartSDR CAT 2036 / native 23005) over a **network** connection → `FlexVita`.
/// - **Icom 7300/7610/9700/705/905** over a **serial** connection → `IcomCiv` (the scope
///   needs the native CI-V serial owner; over network rigctld it isn't reachable).
/// - Everything else (Xiegu, other Icoms, Yaesu, Kenwood, a network Icom) → `None`
///   (audio-FFT fallback).
pub fn native_spectrum_kind(model: u32, rig_conn: &str) -> Option<SpectrumKind> {
    let is_network = rig_conn.eq_ignore_ascii_case("network");
    match model {
        2036 | 23005 if is_network => Some(SpectrumKind::FlexVita),
        _ => {
            if !is_network {
                let m = icom_scope_model(model)?;
                Some(SpectrumKind::IcomCiv {
                    civ_addr: m.default_civ_addr(),
                })
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn native_spectrum_capability_gate() {
        // Flex over network → VITA panadapter.
        assert_eq!(
            native_spectrum_kind(2036, "network"),
            Some(SpectrumKind::FlexVita)
        );
        assert_eq!(
            native_spectrum_kind(23005, "network"),
            Some(SpectrumKind::FlexVita)
        );
        // IC-9700 on serial → CI-V scope at the 9700's default address 0xA2.
        assert_eq!(
            native_spectrum_kind(3081, "serial"),
            Some(SpectrumKind::IcomCiv { civ_addr: 0xA2 })
        );
        assert_eq!(
            native_spectrum_kind(3073, "serial"),
            Some(SpectrumKind::IcomCiv { civ_addr: 0x94 })
        );
        // A network Icom can't use the native serial CI-V scope → audio-FFT fallback.
        assert_eq!(native_spectrum_kind(3081, "network"), None);
        // Yaesu FTDX10 (no native spectrum stream) and an unlisted Icom → None.
        assert_eq!(native_spectrum_kind(1042, "serial"), None);
        assert_eq!(native_spectrum_kind(3013, "serial"), None); // IC-718: no scope
    }

    #[test]
    fn table_is_non_empty_and_has_builtins() {
        let models = rig_models();
        assert!(!models.is_empty());
        // Hamlib Dummy and NET rigctl are always present.
        assert!(models.iter().any(|(m, _)| *m == 1));
        assert!(models.iter().any(|(m, _)| *m == 2));
        // A representative real rig.
        assert!(models.iter().any(|(m, _)| *m == 3073));
    }

    #[test]
    fn name_lookup_resolves_known_and_unknown() {
        assert_eq!(rig_model_name(3073), Some("Icom IC-7300"));
        assert_eq!(rig_model_name(1), Some("Hamlib Dummy"));
        // An out-of-catalog model number has no name.
        assert_eq!(rig_model_name(999_999), None);
    }

    #[test]
    fn model_numbers_are_unique() {
        let models = rig_models();
        let mut seen = HashSet::new();
        for (m, _) in models {
            assert!(seen.insert(m), "duplicate model number {m}");
        }
    }

    #[test]
    fn full_catalog_supersets_verified_without_collisions() {
        let verified = rig_models();
        let all = all_rig_models();
        // The full catalog is strictly larger (verified + extended entries).
        assert!(all.len() > verified.len());
        assert_eq!(all.len(), verified.len() + extended_rig_models().len());

        // Every verified entry survives verbatim into the full catalog.
        let all_set: HashSet<(u32, &str)> = all.iter().copied().collect();
        for entry in &verified {
            assert!(
                all_set.contains(entry),
                "verified entry {entry:?} missing from full catalog"
            );
        }

        // No model number is duplicated across the whole catalog. This is the
        // load-bearing guard: an accidental collision between the verified and
        // extended tiers would silently mislabel a rig the operator selects.
        let mut seen = HashSet::new();
        for (m, _) in &all {
            assert!(
                seen.insert(*m),
                "duplicate model number {m} in full catalog"
            );
        }
    }

    #[test]
    fn verified_and_extended_tiers_are_disjoint() {
        let verified: HashSet<u32> = rig_models().into_iter().map(|(m, _)| m).collect();
        for (m, name) in extended_rig_models() {
            assert!(
                !verified.contains(&m),
                "extended model {m} ({name}) collides with the verified tier"
            );
        }
    }

    #[test]
    fn extended_only_model_resolves_via_name_lookup() {
        // 3011 (IC-706MKIIG) is only in the extended tier, not the verified one.
        assert!(rig_models().iter().all(|(m, _)| *m != 3011));
        assert_eq!(rig_model_name(3011), Some("Icom IC-706MKIIG"));
        // A representative extended entry from another backend.
        assert_eq!(rig_model_name(2050), Some("Lab599 Discovery TX-500"));
    }

    #[test]
    fn every_catalog_number_lands_in_a_known_backend() {
        // Guards against a fat-fingered number that decodes to a nonexistent
        // Hamlib backend (backend = model / 1000). These are the backends we
        // actually draw from; anything else means a typo in a model number.
        let known_backends: HashSet<u32> = [0, 1, 2, 3, 16, 17, 23].into_iter().collect();
        for (m, name) in all_rig_models() {
            let backend = m / 1000;
            assert!(
                known_backends.contains(&backend),
                "model {m} ({name}) decodes to unexpected backend {backend}"
            );
        }
    }
}
