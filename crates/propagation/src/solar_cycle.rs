//! Predicted-solar-cycle parsing — the 12-month **smoothed sunspot number**
//! (R12) the P.533 engine's monthly-median ionosphere runs on (daily SFI is the
//! wrong input for CCIR maps; the fallback Covington inversion in
//! [`crate::p533::engine::ssn_from_sfi`] is only an approximation).
//!
//! Pure parser over SWPC's `predicted-solar-cycle.json` (an array of
//! `{"time-tag":"YYYY-MM","predicted_ssn":…}` rows); the networked half lives
//! in [`crate::live::solar_cycle`].

use serde_json::Value;

/// (year, month 1–12) of a unix timestamp, UTC — the key the SSN table uses.
pub fn year_month(unix: i64) -> (i64, u32) {
    let (y, m, _) = crate::geo::civil_from_days(unix.div_euclid(86_400));
    (y, m)
}

/// The predicted smoothed SSN for (`year`, `month1` 1–12) from SWPC's
/// predicted-solar-cycle JSON; `None` if the month isn't in the table or the
/// document doesn't parse.
pub fn parse_predicted_ssn(json: &Value, year: i64, month1: u32) -> Option<f32> {
    let want = format!("{year:04}-{month1:02}");
    for row in json.as_array()? {
        if row.get("time-tag")?.as_str()? == want {
            return row
                .get("predicted_ssn")
                .and_then(Value::as_f64)
                .map(|v| v as f32);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_requested_month() {
        let doc: Value = serde_json::from_str(
            r#"[
                {"time-tag":"2026-06","predicted_ssn":95.2,"predicted_f10.7":148.0},
                {"time-tag":"2026-07","predicted_ssn":93.8,"predicted_f10.7":146.1}
            ]"#,
        )
        .unwrap();
        assert_eq!(parse_predicted_ssn(&doc, 2026, 7), Some(93.8));
        assert_eq!(parse_predicted_ssn(&doc, 2026, 6), Some(95.2));
        assert_eq!(parse_predicted_ssn(&doc, 2027, 1), None);
        assert_eq!(parse_predicted_ssn(&Value::Null, 2026, 7), None);
    }
}
