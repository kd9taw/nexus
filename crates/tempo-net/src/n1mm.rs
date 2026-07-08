//! N1MM+ native contact broadcast — the `<contactinfo>` UDP XML datagram
//! (official: n1mmwp.hamdocs.com/appendices/external-udp-broadcasts/).
//!
//! N1MM-networked clubs run aggregation dashboards (n1mm_view-style) that
//! consume these datagrams; emitting them per Field Day QSO makes Nexus a
//! first-class station on that network. Emit-only: N1MM itself never accepts
//! inbound contactinfo (its UDP intake is spectrum/freq data only).

use std::net::UdpSocket;

/// One contact for the broadcast.
#[derive(Debug, Clone)]
pub struct N1mmContact {
    pub mycall: String,
    pub call: String,
    /// Band as the meter string the dashboards bucket by: "20" / "40" / "80"
    /// ("0.7" for 70 cm). NOT MHz.
    pub band: String,
    /// "CW" | "USB" | "FT8" | "FT4" …
    pub mode: String,
    /// "YYYY-MM-DD HH:MM:SS" UTC.
    pub timestamp: String,
    pub section: String,
    pub points: u32,
    /// "ARRL-FIELD-DAY" | "WFD".
    pub contestname: String,
    /// RX/TX frequency in units of 10 Hz (N1MM convention).
    pub freq_10hz: u64,
    /// Our sent exchange, e.g. "3A WI".
    pub sent_exchange: String,
    pub operator: String,
    /// 32-hex unique id (consumers dedup on it).
    pub id: String,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build the `<contactinfo>` datagram (the mandatory-for-consumers field set
/// plus the commonly displayed extras; absent fields are simply omitted —
/// consumers treat them as empty).
pub fn build_contactinfo(c: &N1mmContact) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            "<contactinfo>",
            "<app>NEXUS</app>",
            "<contestname>{contest}</contestname>",
            "<contestnr>1</contestnr>",
            "<timestamp>{ts}</timestamp>",
            "<mycall>{my}</mycall>",
            "<band>{band}</band>",
            "<rxfreq>{f}</rxfreq><txfreq>{f}</txfreq>",
            "<operator>{op}</operator>",
            "<mode>{mode}</mode>",
            "<call>{call}</call>",
            "<section>{sect}</section>",
            "<points>{pts}</points>",
            "<radionr>1</radionr>",
            "<IsRunQSO>0</IsRunQSO>",
            "<StationName>NEXUS</StationName>",
            "<ID>{id}</ID>",
            "<IsClaimedQso>1</IsClaimedQso>",
            "<SentExchange>{sent}</SentExchange>",
            "</contactinfo>"
        ),
        contest = esc(&c.contestname),
        ts = esc(&c.timestamp),
        my = esc(&c.mycall),
        band = esc(&c.band),
        f = c.freq_10hz,
        op = esc(&c.operator),
        mode = esc(&c.mode),
        call = esc(&c.call),
        sect = esc(&c.section),
        pts = c.points,
        id = esc(&c.id),
        sent = esc(&c.sent_exchange),
    )
}

/// Fire one datagram at `addr` ("host:port", default port 12060 when absent).
/// Best-effort: a down dashboard must never block a Field Day QSO.
pub fn send_contact(addr: &str, c: &N1mmContact) -> Result<(), String> {
    let target = if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:12060")
    };
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    sock.send_to(build_contactinfo(c).as_bytes(), &target)
        .map_err(|e| format!("N1MM send to {target}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contactinfo_carries_the_consumer_mandatory_fields() {
        let c = N1mmContact {
            mycall: "W9XYZ".into(),
            call: "W1AW".into(),
            band: "20".into(),
            mode: "FT8".into(),
            timestamp: "2026-06-27 18:05:00".into(),
            section: "CT".into(),
            points: 2,
            contestname: "ARRL-FIELD-DAY".into(),
            freq_10hz: 1_407_400,
            sent_exchange: "3A WI".into(),
            operator: "KD9TAW".into(),
            id: "0123456789abcdef0123456789abcdef".into(),
        };
        let xml = build_contactinfo(&c);
        for needle in [
            "<mycall>W9XYZ</mycall>",
            "<call>W1AW</call>",
            "<band>20</band>",
            "<mode>FT8</mode>",
            "<timestamp>2026-06-27 18:05:00</timestamp>",
            "<section>CT</section>",
            "<points>2</points>",
            "<StationName>NEXUS</StationName>",
            "<ID>0123456789abcdef0123456789abcdef</ID>",
            "<contestname>ARRL-FIELD-DAY</contestname>",
            "<contestnr>1</contestnr>",
            "<SentExchange>3A WI</SentExchange>",
            "<rxfreq>1407400</rxfreq>",
        ] {
            assert!(xml.contains(needle), "missing {needle} in {xml}");
        }
        assert!(xml.starts_with("<?xml"));
        assert!(xml.ends_with("</contactinfo>"));
    }
}
