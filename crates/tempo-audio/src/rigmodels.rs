//! A curated subset of Hamlib rig model numbers for the rig-control UI.
//!
//! These `(model, name)` pairs cover the radios most likely to be used for
//! Field Day / digital work. The model numbers are **best-effort**: Hamlib's
//! catalog is large and occasionally renumbered between releases, so the
//! definitive list for an operator's installed Hamlib is `rigctl -l`. Model
//! `1` is Hamlib's "Dummy" rig and `2` is "NET rigctl" (talk to another
//! rigctld); both are always present.

/// `(hamlib_model_number, friendly_name)` for a curated set of common rigs.
///
/// Ordered roughly Dummy/NET first, then by manufacturer. Not exhaustive —
/// verify against `rigctl -l` for the operator's Hamlib version.
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
        // the PC (127.0.0.1:5004 by convention — 5002 is the DDUtil port).
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

/// Friendly name for a Hamlib model number, if it's in the curated table.
pub fn rig_model_name(model: u32) -> Option<&'static str> {
    rig_models()
        .into_iter()
        .find(|(m, _)| *m == model)
        .map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // An out-of-table model number has no curated name.
        assert_eq!(rig_model_name(999_999), None);
    }

    #[test]
    fn model_numbers_are_unique() {
        let models = rig_models();
        let mut seen = std::collections::HashSet::new();
        for (m, _) in models {
            assert!(seen.insert(m), "duplicate model number {m}");
        }
    }
}
