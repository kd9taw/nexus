//! Real-time solar-wind adapter (pure parser half) — the LEADING geomagnetic indicator.
//!
//! Kp and the A-index (from `swpc`) lag real conditions by hours; the DSCOVR/ACE
//! solar-wind stream is the early warning. A southward interplanetary magnetic field
//! (Bz negative) couples energy into the magnetosphere → polar/high-latitude HF paths
//! degrade and aurora can light up VHF, typically 1–2 h before Kp catches up. A fast
//! wind stream (high speed) does the same on a slower fuse.
//!
//! NOAA SWPC serves these as the well-known "array of arrays" products: row 0 is the
//! column headers, every later row is all-STRING values, newest last:
//!   - `services.swpc.noaa.gov/products/solar-wind/mag-1-day.json`
//!     headers include `bz_gsm` (nT) and `bt` (total field, nT)
//!   - `services.swpc.noaa.gov/products/solar-wind/plasma-1-day.json`
//!     headers include `speed` (km/s) and `density` (p/cm³)
//!
//! Pure (`&Value` in, `Option<...>` out) so it is unit-testable offline; the networked
//! fetcher lives in `live::solar_wind`. Columns are located BY NAME (not fixed index) so
//! a reordered/extended product doesn't silently misread.

use serde::Serialize;
use serde_json::Value;

/// Current solar-wind conditions (most recent valid sample in the product window).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolarWind {
    /// Bz (GSM), nT. Negative = southward = the geoeffective case.
    pub bz_nt: f32,
    /// Total field magnitude Bt, nT.
    pub bt_nt: f32,
    /// Bulk speed, km/s.
    pub speed_kms: f32,
    /// Proton density, p/cm³.
    pub density: f32,
}

/// Index of a named column in a SWPC product's header row (row 0).
fn col(header: &Value, name: &str) -> Option<usize> {
    header
        .as_array()?
        .iter()
        .position(|c| c.as_str() == Some(name))
}

/// The newest data row (scanning from the end) whose `idx` column parses as a float —
/// so a trailing `null`/blank sample doesn't blank the readout. Returns that whole row.
fn newest_row_with(rows: &[Value], idx: usize) -> Option<&[Value]> {
    rows.iter().rev().find_map(|r| {
        let a = r.as_array()?;
        cell(a, idx)?; // require the anchor column to parse
        Some(a.as_slice())
    })
}

/// Parse cell `idx` of a data row as f32 (values arrive as strings; `null`/blank → None).
fn cell(row: &[Value], idx: usize) -> Option<f32> {
    row.get(idx)?.as_str()?.trim().parse::<f32>().ok()
}

/// Parse the `mag-1-day` product → (Bz, Bt) from the newest row with a valid Bz.
pub fn parse_mag(v: &Value) -> Option<(f32, f32)> {
    let arr = v.as_array()?;
    let header = arr.first()?;
    let bz_i = col(header, "bz_gsm")?;
    let bt_i = col(header, "bt");
    let row = newest_row_with(&arr[1..], bz_i)?;
    Some((
        cell(row, bz_i)?,
        bt_i.and_then(|i| cell(row, i)).unwrap_or(0.0),
    ))
}

/// Parse the `plasma-1-day` product → (speed, density) from the newest row with a speed.
pub fn parse_plasma(v: &Value) -> Option<(f32, f32)> {
    let arr = v.as_array()?;
    let header = arr.first()?;
    let speed_i = col(header, "speed")?;
    let dens_i = col(header, "density");
    let row = newest_row_with(&arr[1..], speed_i)?;
    Some((
        cell(row, speed_i)?,
        dens_i.and_then(|i| cell(row, i)).unwrap_or(0.0),
    ))
}

/// Assemble a [`SolarWind`] from the two products. Bz/Bt are required (the leading
/// signal); speed/density are best-effort (0 when the plasma feed is unreachable).
pub fn assemble(mag: &Value, plasma: &Value) -> Option<SolarWind> {
    let (bz_nt, bt_nt) = parse_mag(mag)?;
    let (speed_kms, density) = parse_plasma(plasma).unwrap_or((0.0, 0.0));
    Some(SolarWind {
        bz_nt,
        bt_nt,
        speed_kms,
        density,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_bz_and_bt_from_newest_valid_row() {
        let v = json!([
            ["time_tag", "bx_gsm", "by_gsm", "bz_gsm", "lon_gsm", "lat_gsm", "bt"],
            [
                "2024-01-01 00:00:00.000",
                "1.0",
                "2.0",
                "-3.5",
                "180",
                "10",
                "5.1"
            ],
            [
                "2024-01-01 00:01:00.000",
                "1.1",
                "2.1",
                "-8.2",
                "181",
                "11",
                "9.3"
            ],
            [
                "2024-01-01 00:02:00.000",
                null,
                null,
                null,
                null,
                null,
                null
            ]
        ]);
        let (bz, bt) = parse_mag(&v).unwrap();
        assert!((bz - -8.2).abs() < 1e-3); // skipped the trailing null row
        assert!((bt - 9.3).abs() < 1e-3);
    }

    #[test]
    fn parses_speed_and_density_by_column_name() {
        let v = json!([
            ["time_tag", "density", "speed", "temperature"],
            ["2024-01-01 00:00:00.000", "5.2", "420", "100000"]
        ]);
        let (speed, density) = parse_plasma(&v).unwrap();
        assert!((speed - 420.0).abs() < 1e-3);
        assert!((density - 5.2).abs() < 1e-3);
    }

    #[test]
    fn assemble_survives_missing_plasma() {
        let mag = json!([["time_tag", "bz_gsm", "bt"], ["t", "-6.0", "7.0"]]);
        let plasma = json!(null);
        let sw = assemble(&mag, &plasma).unwrap();
        assert!((sw.bz_nt - -6.0).abs() < 1e-3);
        assert_eq!(sw.speed_kms, 0.0);
    }

    #[test]
    fn empty_or_headerless_is_none() {
        assert!(parse_mag(&json!([])).is_none());
        assert!(parse_mag(&json!(null)).is_none());
        assert!(parse_plasma(&json!([["time_tag", "density"]])).is_none()); // no speed col
    }
}
