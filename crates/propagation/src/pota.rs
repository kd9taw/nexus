//! Parks/Summits On The Air activator-spot parsing (the "who's on the air now"
//! hunter feed). Pure JSON→[`OtaSpot`] mapping for the two live APIs; the HTTP
//! fetch lives in [`crate::live::pota`]. Reference validation is in
//! `tempo_core::pota`.

use serde::{Deserialize, Serialize};

/// One activator currently on the air (POTA or SOTA), normalized across both feeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtaSpot {
    /// "POTA" | "SOTA".
    pub program: String,
    /// Park/summit id, e.g. "K-1234" / "W7A/MN-001".
    pub reference: String,
    /// Park/summit name (may be empty for RBN-sourced POTA spots).
    pub name: String,
    pub activator: String,
    /// Frequency in kHz (POTA reports kHz; SOTA reports MHz — normalized here).
    pub freq_khz: f64,
    pub mode: String,
    pub spotter: Option<String>,
    pub comment: Option<String>,
    pub grid: Option<String>,
    /// Spot time (unix seconds, UTC) from the feed's timestamp — POTA `spotTime`,
    /// SOTA `timeStamp`. `None` if absent/unparseable. Lets a consumer drop STALE
    /// activations, which matters for SOTA: its `spots/<n>/all` returns the last `n`
    /// spots by COUNT, not by recency, so an old summit can ride along on a quiet day.
    pub spot_time_unix: Option<i64>,
}

fn s(v: &serde_json::Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .filter(|x| !x.is_empty())
}

/// Parse the POTA activator-spots JSON (`https://api.pota.app/spot/activator`): an
/// array of objects with `activator`, `reference`, `frequency` (kHz string), `mode`,
/// `name`, `spotTime`, `spotter`, `comments`, `grid6`/`grid4`. Rows missing the
/// required call/reference/frequency are skipped. Malformed JSON → empty.
pub fn parse_pota_spots(json: &str) -> Vec<OtaSpot> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json).unwrap_or_default();
    arr.iter()
        .filter_map(|v| {
            let activator = s(v, "activator")?;
            let reference = s(v, "reference")?;
            let freq_khz = freq_field(v, "frequency")?;
            Some(OtaSpot {
                program: "POTA".into(),
                reference,
                name: s(v, "name").unwrap_or_default(),
                activator,
                freq_khz,
                mode: s(v, "mode").unwrap_or_default(),
                spotter: s(v, "spotter"),
                comment: s(v, "comments"),
                grid: s(v, "grid6").or_else(|| s(v, "grid4")),
                spot_time_unix: time_field(v, "spotTime"),
            })
        })
        .collect()
}

/// Parse the SOTAwatch v2 spots JSON
/// (`https://api-db2.sota.org.uk/api/spots/<n>/all`): an array with
/// `activatorCallsign`, `associationCode` + `summitCode` (joined as `ASSOC/SUMMIT`),
/// `frequency` (**MHz** string → kHz here), `mode`, `summitDetails`, `callsign`
/// (spotter), `comments`. Rows missing call/summit/frequency are skipped.
pub fn parse_sota_spots(json: &str) -> Vec<OtaSpot> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json).unwrap_or_default();
    arr.iter()
        .filter_map(|v| {
            let activator = s(v, "activatorCallsign")?;
            let assoc = s(v, "associationCode")?;
            let summit = s(v, "summitCode")?;
            let mhz = freq_field(v, "frequency")?;
            Some(OtaSpot {
                program: "SOTA".into(),
                reference: format!("{assoc}/{summit}"),
                name: s(v, "summitDetails").unwrap_or_default(),
                activator,
                freq_khz: mhz * 1000.0,
                mode: s(v, "mode").unwrap_or_default(),
                spotter: s(v, "callsign"),
                comment: s(v, "comments"),
                grid: None,
                spot_time_unix: time_field(v, "timeStamp"),
            })
        })
        .collect()
}

/// Parse a naive-UTC ISO timestamp field (POTA `spotTime` / SOTA `timeStamp`, both
/// UTC) to unix seconds. Reuses the crate's tolerant parser (handles the `Z` suffix +
/// fractional seconds SOTA sometimes emits). `None` if absent/unparseable.
fn time_field(v: &serde_json::Value, k: &str) -> Option<i64> {
    v.get(k)
        .and_then(|x| x.as_str())
        .and_then(crate::kc2g::parse_naive_utc_unix)
}

/// Read a frequency that may be a JSON string or number.
fn freq_field(v: &serde_json::Value, k: &str) -> Option<f64> {
    let f = v.get(k)?;
    f.as_str()
        .and_then(|x| x.trim().parse::<f64>().ok())
        .or_else(|| f.as_f64())
        .filter(|x| *x > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pota_activator_spots() {
        // Trimmed real-shape payload (api.pota.app/spot/activator).
        let json = r#"[
          {"spotId":51518793,"activator":"F4MOJ/P","frequency":"3573.0","mode":"FT8",
           "reference":"FR-11086","parkName":null,"spotTime":"2026-06-07T05:34:30",
           "spotter":"OE9GHV-#","comments":"RBN 1 dB via OE9GHV-#","source":"RBN",
           "name":"Belledonne Reserve","locationDesc":"FR-ARA","grid4":"JN35","grid6":"JN35bh"},
          {"activator":"K1ABC","frequency":"14074.0","mode":"FT8","reference":"K-1234",
           "name":"Acadia NP","spotter":"W9XYZ","comments":""}
        ]"#;
        let spots = parse_pota_spots(json);
        assert_eq!(spots.len(), 2);
        assert_eq!(spots[0].program, "POTA");
        assert_eq!(spots[0].reference, "FR-11086");
        assert_eq!(spots[0].activator, "F4MOJ/P");
        assert_eq!(spots[0].freq_khz, 3573.0);
        assert_eq!(spots[0].grid.as_deref(), Some("JN35bh")); // grid6 preferred
        assert_eq!(spots[0].spotter.as_deref(), Some("OE9GHV-#"));
        // Empty comments → None.
        assert_eq!(spots[1].comment, None);
        assert_eq!(spots[1].reference, "K-1234");
        // spotTime → unix (2026-06-07T05:34:30 UTC).
        assert!(spots[0]
            .spot_time_unix
            .is_some_and(|t| (1_767_000_000..1_800_000_000).contains(&t)));
    }

    #[test]
    fn parses_sota_spots_and_converts_mhz() {
        let json = r#"[
          {"id":324998,"userID":8546,"timeStamp":"2026-06-07T05:32:14",
           "comments":"[SOTA Activator] last calls","callsign":"VK3HN",
           "associationCode":"VK3","summitCode":"VN-012","activatorCallsign":"VK3KR",
           "activatorName":"David","frequency":"7.033","mode":"CW",
           "summitDetails":"Mt Mitchell, 935m, 6 points","highlightColor":null}
        ]"#;
        let spots = parse_sota_spots(json);
        assert_eq!(spots.len(), 1);
        let s = &spots[0];
        assert_eq!(s.program, "SOTA");
        assert_eq!(s.reference, "VK3/VN-012");
        assert_eq!(s.activator, "VK3KR");
        assert_eq!(s.freq_khz, 7033.0); // 7.033 MHz → kHz
        assert_eq!(s.mode, "CW");
        assert_eq!(s.spotter.as_deref(), Some("VK3HN"));
        assert!(s.name.contains("Mt Mitchell"));
        // timeStamp → unix (recency filtering for sparse SOTA).
        assert!(s
            .spot_time_unix
            .is_some_and(|t| (1_767_000_000..1_800_000_000).contains(&t)));
    }

    #[test]
    fn malformed_or_incomplete_rows_are_skipped() {
        assert!(parse_pota_spots("not json").is_empty());
        assert!(parse_sota_spots("{}").is_empty());
        // A row missing the activator is dropped, the valid one kept.
        let json = r#"[{"reference":"K-1","frequency":"14074"},
                       {"activator":"K1ABC","reference":"K-1234","frequency":"14074"}]"#;
        assert_eq!(parse_pota_spots(json).len(), 1);
    }
}
