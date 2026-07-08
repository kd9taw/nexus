//! N3FJP Field Day Contest Log — real-time QSO push over its TCP API.
//!
//! The club-network story: N3FJP runs the master Field Day log; Nexus is the
//! digital (or any) station and pushes each contact in as it's logged, exactly
//! like the classic WSJT-X→JTAlert→N3FJP bridge but native. Protocol per the
//! official API docs (n3fjp.com/help/api.html): XML-ish `<CMD>…</CMD>` lines,
//! **every command terminated `\r\n`** (a bare `\r\n` closes the connection),
//! N3FJP is the TCP server (default port 1100; enabled in N3FJP under
//! Settings ▸ Application Program Interface).
//!
//! We use the ADDDIRECT path (direct DB insert — no UI-keystroke emulation, no
//! ENTER action, dupes excluded server-side) followed by CHECKLOG so the
//! N3FJP screen refreshes. Connect-per-push keeps the loop simple and robust
//! across N3FJP restarts during a chaotic Field Day.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(4);

/// Minimal XML escaping for field values (calls/sections are alnum, but a
/// comment or park name must never break the stream).
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let addr = format!("{host}:{port}");
    let stream = TcpStream::connect_timeout(
        &addr.parse().or_else(|_| {
            // Hostname: resolve via ToSocketAddrs.
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .ok_or(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "unresolvable host",
                ))
        })?,
        TIMEOUT,
    )?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    Ok(stream)
}

/// One Field Day contact for the push.
#[derive(Debug, Clone)]
pub struct N3fjpQso {
    pub call: String,
    pub class: String,
    pub section: String,
    /// Band in METERS ("20", not "14") — the N3FJP convention.
    pub band_meters: String,
    /// N3FJP mode string: "FT8" / "FT4" / "CW" / "SSB" (it buckets to
    /// CW/PH/DIG for contest scoring itself).
    pub mode: String,
    /// Dial/RF frequency in MHz, e.g. 14.074.
    pub freq_mhz: f64,
    /// QSO time, unix seconds (formatted YYYY/MM/DD + HH:MM UTC).
    pub when_unix: u64,
    pub operator: String,
}

/// Unix secs → ("YYYY/MM/DD", "HH:MM") UTC (same civil math as the Cabrillo
/// exporter — two tiny fields don't justify a date crate).
fn n3fjp_datetime(unix: u64) -> (String, String) {
    let secs_of_day = unix % 86_400;
    let days = (unix / 86_400) as i64;
    let (h, m) = (
        (secs_of_day / 3600) as u32,
        ((secs_of_day % 3600) / 60) as u32,
    );
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    (format!("{y:04}/{mo:02}/{d:02}"), format!("{h:02}:{m:02}"))
}

/// Build the ADDDIRECT command line for one QSO (without the trailing CRLF).
pub fn build_adddirect(q: &N3fjpQso) -> String {
    let (date, time) = n3fjp_datetime(q.when_unix);
    // EXCLUDEDUPES is a documented ADDDIRECT field; STAYOPEN is a separate
    // top-level command and we connect-per-push anyway — never send it here.
    let mut s = String::from("<CMD><ADDDIRECT><EXCLUDEDUPES>TRUE</EXCLUDEDUPES>");
    s.push_str(&format!("<fldCall>{}</fldCall>", esc(&q.call)));
    s.push_str(&format!("<fldDateStr>{date}</fldDateStr>"));
    s.push_str(&format!("<fldTimeOnStr>{time}</fldTimeOnStr>"));
    s.push_str(&format!("<fldBand>{}</fldBand>", esc(&q.band_meters)));
    s.push_str(&format!("<fldMode>{}</fldMode>", esc(&q.mode)));
    s.push_str(&format!("<fldFrequency>{:.4}</fldFrequency>", q.freq_mhz));
    s.push_str(&format!("<fldClass>{}</fldClass>", esc(&q.class)));
    s.push_str(&format!("<fldSection>{}</fldSection>", esc(&q.section)));
    if !q.operator.is_empty() {
        s.push_str(&format!("<fldOperator>{}</fldOperator>", esc(&q.operator)));
    }
    s.push_str("<fldComments>via Nexus</fldComments></CMD>");
    s
}

/// Push one QSO into N3FJP (connect → ADDDIRECT → CHECKLOG → close).
pub fn push_qso(host: &str, port: u16, q: &N3fjpQso) -> Result<(), String> {
    let mut stream = connect(host, port).map_err(|e| format!("N3FJP connect: {e}"))?;
    let line = build_adddirect(q);
    stream
        .write_all(format!("{line}\r\n<CMD><CHECKLOG></CMD>\r\n").as_bytes())
        .map_err(|e| format!("N3FJP send: {e}"))?;
    Ok(())
}

/// Test the connection: handshake `<CMD><PROGRAM></CMD>` and report what's on
/// the other end ("N3FJP's Field Day Contest Log v6.6").
pub fn test_connection(host: &str, port: u16) -> Result<String, String> {
    let mut stream = connect(host, port).map_err(|e| format!("connect failed: {e}"))?;
    stream
        .write_all(b"<CMD><PROGRAM></CMD>\r\n")
        .map_err(|e| format!("send failed: {e}"))?;
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).map_err(|e| {
        format!("no response: {e} (is the TCP API enabled in N3FJP ▸ Settings ▸ API?)")
    })?;
    let resp = String::from_utf8_lossy(&buf[..n]);
    let pgm = resp
        .split("<PGM>")
        .nth(1)
        .and_then(|r| r.split("</PGM>").next())
        .unwrap_or("unknown program");
    let ver = resp
        .split("<VER>")
        .nth(1)
        .and_then(|r| r.split("</VER>").next())
        .unwrap_or("?");
    Ok(format!("{pgm} v{ver}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adddirect_line_matches_the_documented_grammar() {
        let q = N3fjpQso {
            call: "W1AW".into(),
            class: "2A".into(),
            section: "CT".into(),
            band_meters: "20".into(),
            mode: "FT8".into(),
            freq_mhz: 14.074,
            when_unix: 1_782_583_500, // 2026-06-27 18:05 UTC (FD Saturday)
            operator: "KD9TAW".into(),
        };
        let line = build_adddirect(&q);
        assert!(line.starts_with("<CMD><ADDDIRECT><EXCLUDEDUPES>TRUE</EXCLUDEDUPES>"));
        assert!(line.contains("<fldCall>W1AW</fldCall>"));
        assert!(line.contains("<fldDateStr>2026/06/27</fldDateStr>"));
        assert!(line.contains("<fldTimeOnStr>18:05</fldTimeOnStr>"));
        assert!(line.contains("<fldBand>20</fldBand>"), "band in METERS");
        assert!(line.contains("<fldMode>FT8</fldMode>"));
        assert!(line.contains("<fldFrequency>14.0740</fldFrequency>"));
        assert!(line.contains("<fldClass>2A</fldClass>"));
        assert!(line.contains("<fldSection>CT</fldSection>"));
        assert!(line.ends_with("</CMD>"));
        assert!(!line.contains('\n'), "single line; CRLF added at send");
    }

    #[test]
    fn values_are_xml_escaped() {
        let q = N3fjpQso {
            call: "A&B<C>".into(),
            class: "1D".into(),
            section: "WI".into(),
            band_meters: "40".into(),
            mode: "CW".into(),
            freq_mhz: 7.0,
            when_unix: 0,
            operator: String::new(),
        };
        let line = build_adddirect(&q);
        assert!(line.contains("<fldCall>A&amp;B&lt;C&gt;</fldCall>"));
    }
}
