//! WSJT-X-compatible ALL.TXT decode-log lines — the running record of every decode (and
//! transmission), in the exact format GridTracker / JTAlert / loggers tail.
//!
//! Format verified against WSJT-X `MainWindow::write_all` + the jt9 decoder line layout:
//! `yyMMdd_hhmmss{%10.3f dialMHz} {Rx|Tx} {mode:<6}{snr:>4}{dt:>5.1}{audioHz:>5} {message}`.
//! WSJT-X strips its `~` sync marker (the `msg.mid(0,15)+msg.mid(18,-1)` surgery) before
//! writing, so ALL.TXT carries no `~`. Example:
//! `231114_221320    14.074 Rx FT8    -10  0.2 1500 CQ W1ABC FN42`

use tempo_core::logbook::datetime_utc;

/// One ALL.TXT line for a decode (`is_tx=false`) or a transmission (`is_tx=true`).
/// `dial_mhz` is the VFO dial frequency; `audio_hz` the in-passband carrier offset.
#[allow(clippy::too_many_arguments)]
pub fn all_txt_line(
    unix: u64,
    dial_mhz: f64,
    is_tx: bool,
    mode: &str,
    snr: i32,
    dt: f32,
    audio_hz: f32,
    message: &str,
) -> String {
    let (y, mo, d, h, mi, se) = datetime_utc(unix);
    format!(
        "{:02}{:02}{:02}_{:02}{:02}{:02}{:>10.3} {} {:<6}{:>4}{:>5.1}{:>5} {}",
        (y as u32) % 100,
        mo,
        d,
        h,
        mi,
        se,
        dial_mhz,
        if is_tx { "Tx" } else { "Rx" },
        mode,
        snr,
        dt,
        audio_hz.round() as i32,
        message.trim(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // unix 1_700_000_000 == 2023-11-14 22:13:20 UTC → "231114_221320".
    const T: u64 = 1_700_000_000;

    #[test]
    fn rx_line_matches_wsjtx_layout_exactly() {
        let line = all_txt_line(T, 14.074, false, "FT8", -10, 0.2, 1500.0, "CQ W1ABC FN42");
        assert_eq!(
            line,
            "231114_221320    14.074 Rx FT8    -10  0.2 1500 CQ W1ABC FN42"
        );
    }

    #[test]
    fn tx_line_uses_tx_marker_and_zeroed_metrics() {
        let line = all_txt_line(T, 14.074, true, "FT8", 0, 0.0, 1200.0, "W1ABC KD9TAW R-09");
        assert_eq!(
            line,
            "231114_221320    14.074 Tx FT8      0  0.0 1200 W1ABC KD9TAW R-09"
        );
    }

    #[test]
    fn positive_snr_and_ft4_and_message_trim() {
        let line = all_txt_line(T, 7.074, false, "FT4", 12, -0.3, 800.0, "  CQ DX VK3ABC  ");
        assert_eq!(
            line,
            "231114_221320     7.074 Rx FT4     12 -0.3  800 CQ DX VK3ABC"
        );
    }
}
