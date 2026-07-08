//! POTA / SOTA reference handling — pure validation + normalization.
//!
//! Two "on-the-air" programs share one shape in the log (see [`crate::logbook::Ota`]):
//! Parks On The Air uses a park reference like `K-1234`; Summits On The Air uses a
//! summit reference like `W7A/MN-001`. The ADIF mapping differs (POTA → `SIG`/
//! `SIG_INFO`, SOTA → `SOTA_REF`), so the program is tracked alongside the reference.
//! Live activator-spot parsing + fetch live in the `propagation` crate (it has the
//! JSON + HTTP deps); this module is the dependency-free reference core.

/// An on-the-air program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaProgram {
    /// Parks On The Air.
    Pota,
    /// Summits On The Air.
    Sota,
}

impl OtaProgram {
    /// ADIF / API program code: "POTA" / "SOTA".
    pub fn code(self) -> &'static str {
        match self {
            OtaProgram::Pota => "POTA",
            OtaProgram::Sota => "SOTA",
        }
    }

    /// Parse a program code (case-insensitive). WWFF is treated as POTA-shaped here
    /// (same `SIG`/`SIG_INFO` ADIF mapping) but only POTA/SOTA are first-class.
    pub fn from_code(s: &str) -> Option<OtaProgram> {
        match s.trim().to_ascii_uppercase().as_str() {
            "POTA" => Some(OtaProgram::Pota),
            "SOTA" => Some(OtaProgram::Sota),
            _ => None,
        }
    }
}

/// Validate + normalize a POTA park reference (e.g. `"k-1234"` → `"K-1234"`).
/// Format: a 1–4 char alphanumeric prefix, a hyphen, then 4–5 digits (POTA uses
/// 4 historically, 5 for newer parks). Returns `None` if it doesn't match.
pub fn normalize_pota_ref(s: &str) -> Option<String> {
    let t = s.trim().to_ascii_uppercase();
    let (prefix, digits) = t.split_once('-')?;
    let prefix_ok =
        (1..=4).contains(&prefix.len()) && prefix.bytes().all(|b| b.is_ascii_alphanumeric());
    let digits_ok = (4..=5).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit());
    (prefix_ok && digits_ok).then_some(t)
}

/// Validate + normalize a SOTA summit reference (e.g. `"w7a/mn-001"` → `"W7A/MN-001"`).
/// Format: `<association>/<region>-<number>` — association 1–8 alphanumerics, region
/// 1–2 letters, number exactly 3 digits. Returns `None` if it doesn't match.
pub fn normalize_sota_ref(s: &str) -> Option<String> {
    let t = s.trim().to_ascii_uppercase();
    let (assoc, rest) = t.split_once('/')?;
    let (region, number) = rest.split_once('-')?;
    let assoc_ok =
        (1..=8).contains(&assoc.len()) && assoc.bytes().all(|b| b.is_ascii_alphanumeric());
    let region_ok =
        (1..=2).contains(&region.len()) && region.bytes().all(|b| b.is_ascii_alphabetic());
    let number_ok = number.len() == 3 && number.bytes().all(|b| b.is_ascii_digit());
    (assoc_ok && region_ok && number_ok).then_some(t)
}

/// Validate + normalize a reference for the given program.
pub fn normalize_ref(program: OtaProgram, s: &str) -> Option<String> {
    match program {
        OtaProgram::Pota => normalize_pota_ref(s),
        OtaProgram::Sota => normalize_sota_ref(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_codes_round_trip() {
        assert_eq!(OtaProgram::from_code("pota"), Some(OtaProgram::Pota));
        assert_eq!(OtaProgram::from_code(" SOTA "), Some(OtaProgram::Sota));
        assert_eq!(OtaProgram::Pota.code(), "POTA");
        assert_eq!(OtaProgram::from_code("WWFF"), None);
    }

    #[test]
    fn pota_refs_validate_and_normalize() {
        assert_eq!(normalize_pota_ref("k-1234").as_deref(), Some("K-1234"));
        assert_eq!(normalize_pota_ref("VE-5678").as_deref(), Some("VE-5678"));
        assert_eq!(normalize_pota_ref("US-12345").as_deref(), Some("US-12345")); // 5-digit
                                                                                 // Rejects junk.
        assert_eq!(normalize_pota_ref("K1234"), None); // no hyphen
        assert_eq!(normalize_pota_ref("K-12"), None); // too few digits
        assert_eq!(normalize_pota_ref("LONGPFX-1234"), None); // prefix too long
        assert_eq!(normalize_pota_ref("K-12AB"), None); // non-digit suffix
        assert_eq!(normalize_pota_ref(""), None);
    }

    #[test]
    fn sota_refs_validate_and_normalize() {
        assert_eq!(
            normalize_sota_ref("w7a/mn-001").as_deref(),
            Some("W7A/MN-001")
        );
        assert_eq!(normalize_sota_ref("G/LD-001").as_deref(), Some("G/LD-001"));
        assert_eq!(
            normalize_sota_ref("VK3/VN-012").as_deref(),
            Some("VK3/VN-012")
        );
        // Rejects junk.
        assert_eq!(normalize_sota_ref("W7A-MN-001"), None); // no slash
        assert_eq!(normalize_sota_ref("W7A/MN-01"), None); // 2-digit number
        assert_eq!(normalize_sota_ref("W7A/M1-001"), None); // region not letters
        assert_eq!(normalize_sota_ref("W7A/MN-0012"), None); // 4-digit number
    }

    #[test]
    fn normalize_ref_dispatches_by_program() {
        assert_eq!(normalize_ref(OtaProgram::Pota, "k-1").as_deref(), None);
        assert_eq!(
            normalize_ref(OtaProgram::Pota, "k-1234").as_deref(),
            Some("K-1234")
        );
        assert_eq!(
            normalize_ref(OtaProgram::Sota, "g/ld-001").as_deref(),
            Some("G/LD-001")
        );
    }
}
