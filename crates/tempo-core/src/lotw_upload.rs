//! Pure helpers for LoTW upload via a TQSL shell-out — the offline, unit-testable
//! core (arg building, exit-code classification, binary-location candidates, and
//! stderr sanitization). The actual `std::process::Command` spawn lives in the
//! Tauri command layer (it can't run headless).
//!
//! Nexus never handles the LoTW certificate or any secret — TQSL owns all of that;
//! we pass only a non-secret Station Location name.

use std::path::PathBuf;

use crate::logbook::UploadOutcome;

/// Build the TQSL argv to SIGN + UPLOAD an ADIF file: `-d` (no date dialog), `-u`
/// (upload), `-x` (batch/exit), `-a compliant` (silently skip dupes/out-of-range),
/// then the input path. When `station_location` is `Some`, sign against that NAMED
/// Station Location (`-l <name>`); when `None`, OMIT `-l` so TQSL signs from the
/// location embedded in the ADIF (STATION_CALLSIGN/MY_GRIDSQUARE) — the traveler
/// workflow. Pure + testable; the caller prepends the resolved `tqsl` binary.
pub fn tqsl_args(station_location: Option<&str>, adif_path: &str) -> Vec<String> {
    let mut args = vec![
        "-d".into(),
        "-u".into(),
        "-x".into(),
        "-a".into(),
        "compliant".into(),
    ];
    if let Some(loc) = station_location {
        args.push("-l".into());
        args.push(loc.into());
    }
    args.push(adif_path.into());
    args
}

/// Map a TQSL process exit code (+ its stderr) to an [`UploadOutcome`], or `None`
/// when the upload should leave state untouched for a clean retry (network error)
/// — and the caller must NOT stamp anything.
///
/// Every code has a defined result: `{0,9}`→Pending, `{8}`→Duplicate (all already
/// on file — a terminal "don't re-send", NOT a no-op), `5` with a cert/station-
/// location stderr marker→AuthFail else a bare `5`→Rejected, `{2,3,4,6,7,10}` and
/// `1` (cancelled) and any **unrecognized** code→Rejected (produced no confirmed
/// upload, so never Pending/Accepted; Rejected self-heals into the next batch),
/// `11` (network)→`None` (retry, no stamp).
pub fn classify_tqsl_exit(code: i32, stderr: &str) -> Option<UploadOutcome> {
    match code {
        0 | 9 => Some(UploadOutcome::Pending),
        8 => Some(UploadOutcome::Duplicate),
        5 if stderr_is_auth(stderr) => Some(UploadOutcome::AuthFail),
        11 => None, // network — retryable, do not stamp
        _ => Some(UploadOutcome::Rejected),
    }
}

/// Does the TQSL stderr indicate a credential / certificate / station-location
/// problem (→ AuthFail) rather than a generic error?
fn stderr_is_auth(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("no certificate")
        || s.contains("no certificates")
        || s.contains("certificate")
        || s.contains("station location")
        || s.contains("callsign certificate")
}

/// Per-OS default locations to look for the `tqsl` binary, tried before a PATH
/// lookup and before a user-configured override. Pure (no fs touch here).
pub fn tqsl_candidate_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        for env in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Ok(base) = std::env::var(env) {
                v.push(PathBuf::from(base).join("TrustedQSL").join("tqsl.exe"));
            }
        }
        v.push(PathBuf::from("tqsl.exe"));
        v
    }
    #[cfg(target_os = "macos")]
    {
        let mut v = vec![PathBuf::from(
            "/Applications/TrustedQSL/tqsl.app/Contents/MacOS/tqsl",
        )];
        if let Ok(home) = std::env::var("HOME") {
            v.push(
                PathBuf::from(home).join("Applications/TrustedQSL/tqsl.app/Contents/MacOS/tqsl"),
            );
        }
        v.push(PathBuf::from("tqsl"));
        v
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        vec![
            PathBuf::from("/usr/bin/tqsl"),
            PathBuf::from("/usr/local/bin/tqsl"),
            PathBuf::from("/opt/tqsl/bin/tqsl"),
            PathBuf::from("tqsl"),
        ]
    }
}

/// Sanitize a TQSL stderr tail for storage/display: redact any absolute-path run
/// (Windows drive `X:\…` / UNC `\\…`, or a POSIX `/…`) to its last component, flatten
/// whitespace, and truncate. Avoids leaking the cert path, a custom tqsl path, or
/// the temp `.adi` path echoed on file errors. Returns `None` for empty input.
pub fn sanitize_detail(stderr: &str) -> Option<String> {
    let flat = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    let redacted = flat
        .split(' ')
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ");
    let out: String = redacted.chars().take(200).collect();
    Some(out)
}

/// If a whitespace-delimited token looks like an absolute path, reduce it to its
/// basename; otherwise return it unchanged.
fn redact_token(tok: &str) -> String {
    let looks_abs = tok.starts_with('/')
        || tok.starts_with("\\\\")
        || (tok.len() >= 3
            && tok.as_bytes()[1] == b':'
            && (tok.as_bytes()[2] == b'\\' || tok.as_bytes()[2] == b'/'));
    if !looks_abs {
        return tok.to_string();
    }
    tok.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(tok)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_sign_and_upload() {
        let a = tqsl_args(Some("Home FT8"), "/tmp/x.adi");
        assert_eq!(
            a,
            vec![
                "-d",
                "-u",
                "-x",
                "-a",
                "compliant",
                "-l",
                "Home FT8",
                "/tmp/x.adi"
            ]
        );
    }

    #[test]
    fn args_omit_location_for_adif_signing() {
        // Traveler mode: no `-l`, so TQSL signs from the location embedded in the ADIF.
        let a = tqsl_args(None, "/tmp/x.adi");
        assert_eq!(a, vec!["-d", "-u", "-x", "-a", "compliant", "/tmp/x.adi"]);
        assert!(!a.iter().any(|s| s == "-l"), "no -l in ADIF-location mode");
    }

    #[test]
    fn exit_codes_map_exhaustively() {
        use UploadOutcome::*;
        assert_eq!(classify_tqsl_exit(0, ""), Some(Pending));
        assert_eq!(classify_tqsl_exit(9, ""), Some(Pending)); // partial = success
        assert_eq!(classify_tqsl_exit(8, ""), Some(Duplicate)); // all dupes (terminal!)
        assert_eq!(classify_tqsl_exit(2, "rejected"), Some(Rejected));
        assert_eq!(classify_tqsl_exit(3, ""), Some(Rejected));
        assert_eq!(classify_tqsl_exit(4, ""), Some(Rejected));
        assert_eq!(classify_tqsl_exit(6, ""), Some(Rejected));
        assert_eq!(classify_tqsl_exit(7, ""), Some(Rejected));
        assert_eq!(classify_tqsl_exit(10, ""), Some(Rejected));
        assert_eq!(classify_tqsl_exit(1, ""), Some(Rejected)); // cancelled → Rejected (never Pending)
        assert_eq!(classify_tqsl_exit(99, ""), Some(Rejected)); // unknown → Rejected
        assert_eq!(classify_tqsl_exit(11, ""), None); // network → no stamp / retry
    }

    #[test]
    fn exit_5_discriminates_auth_vs_generic() {
        use UploadOutcome::*;
        assert_eq!(
            classify_tqsl_exit(5, "Error: No certificate for KD9TAW"),
            Some(AuthFail)
        );
        assert_eq!(
            classify_tqsl_exit(5, "Error: station location 'Home' not found"),
            Some(AuthFail)
        );
        assert_eq!(
            classify_tqsl_exit(5, "internal tqsllib error"),
            Some(Rejected)
        );
    }

    #[test]
    fn sanitize_redacts_paths_and_truncates() {
        let s = sanitize_detail("Unable to open C:\\Users\\seth\\tmp\\up.adi for reading").unwrap();
        assert!(!s.contains("Users"), "windows path redacted: {s}");
        assert!(s.contains("up.adi"));
        let p = sanitize_detail("cannot read /home/seth/.tqsl/cert.p12 now").unwrap();
        assert!(!p.contains("/home/seth"), "posix path redacted: {p}");
        assert!(p.contains("cert.p12"));
        assert_eq!(sanitize_detail("   ").map(|s| s.len()), None);
        let long = "x ".repeat(300);
        assert!(sanitize_detail(&long).unwrap().chars().count() <= 200);
    }

    #[test]
    fn candidate_paths_nonempty_and_end_with_tqsl() {
        let v = tqsl_candidate_paths();
        assert!(!v.is_empty());
        assert!(v
            .iter()
            .all(|p| p.to_string_lossy().to_lowercase().contains("tqsl")));
    }
}
