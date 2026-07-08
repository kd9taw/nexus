//! FlexRadio 6000-series LAN discovery — the "Find my Flex" button.
//!
//! A powered-on Flex broadcasts a VITA-49 discovery packet on UDP 4992 about
//! once a second. We don't need full VITA parsing: the payload carries a plain
//! ASCII key=value section (`model=FLEX-6400 serial=... ip=192.168.1.20
//! port=4992 nickname=Shack ...`), so listening briefly and scanning datagrams
//! for those tokens finds every radio on the segment. Read-only: nothing is
//! ever sent to the radio.
//!
//! HONESTY NOTE: written to the published discovery format and unit-tested
//! against a synthetic payload — not yet verified against live hardware (no
//! Flex on the dev LAN). The UI labels the button accordingly until an
//! operator confirms it against a real 6xxx.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

/// One discovered radio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlexRadio {
    /// e.g. "FLEX-6400".
    pub model: String,
    /// The operator's radio nickname, if set.
    pub nickname: String,
    /// The radio's IP (the CAT/API host to connect to).
    pub ip: String,
}

/// Scan a discovery datagram's bytes for the ASCII key=value section and pull
/// the fields we need. Pure — unit-testable without a radio. `None` when the
/// datagram carries no `ip=` token (not a discovery packet).
pub fn parse_discovery(datagram: &[u8]) -> Option<FlexRadio> {
    // The VITA header is binary; the payload keys are plain ASCII. Lossily
    // decode and split on whitespace/NULs — key=value tokens survive intact.
    let text = String::from_utf8_lossy(datagram);
    let mut ip = None;
    let mut model = None;
    let mut nickname = None;
    for tok in text.split(|c: char| c.is_whitespace() || c == '\0') {
        if let Some(v) = tok.strip_prefix("ip=") {
            // Sanity: dotted-quad only (the token scan must not accept junk
            // that happens to contain "ip=").
            if v.split('.').count() == 4 && v.split('.').all(|o| o.parse::<u8>().is_ok()) {
                ip = Some(v.to_string());
            }
        } else if let Some(v) = tok.strip_prefix("model=") {
            model = Some(v.to_string());
        } else if let Some(v) = tok.strip_prefix("nickname=") {
            nickname = Some(v.to_string());
        }
    }
    Some(FlexRadio {
        model: model.unwrap_or_else(|| "FLEX".to_string()),
        nickname: nickname.unwrap_or_default(),
        ip: ip?,
    })
}

/// Listen on UDP 4992 for up to `secs` and return every distinct radio heard.
/// Empty = the port was bound fine but nothing announced (no Flex powered up
/// on this segment).
///
/// The socket sets `SO_REUSEADDR` before binding: SmartSDR's own discovery
/// listener (FlexLib) opts into sharing too, so a co-hosted SmartSDR and Nexus
/// can both hear the broadcast — on Windows, sharing works only when BOTH
/// sockets opt in. If the bind still fails because some non-sharing app holds
/// 4992 exclusively, we return a distinctive AddrInUse error naming the likely
/// culprit so the UI can show it verbatim instead of a raw os-error string.
pub fn discover(secs: u64) -> std::io::Result<Vec<FlexRadio>> {
    let raw = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    raw.set_reuse_address(true)?;
    let addr: std::net::SocketAddr = ([0, 0, 0, 0], 4992).into();
    raw.bind(&addr.into()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "discovery port 4992 is held exclusively by another app (SmartSDR?) — \
                 close it and retry, or type the radio's IP manually",
            )
        } else {
            e
        }
    })?;
    let sock: UdpSocket = raw.into();
    sock.set_read_timeout(Some(Duration::from_millis(400)))?;
    let deadline = Instant::now() + Duration::from_secs(secs.clamp(1, 10));
    let mut found: Vec<FlexRadio> = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        // Err = the 400 ms read timeout ticking — keep listening until the deadline.
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            if let Some(r) = parse_discovery(&buf[..n]) {
                if !found.iter().any(|f| f.ip == r.ip) {
                    found.push(r);
                }
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_ascii_kv_section_out_of_a_binary_datagram() {
        // Synthetic: VITA-ish binary header bytes, then the documented ASCII
        // key=value payload a real 6xxx broadcasts.
        let mut d = vec![0x38u8, 0x5b, 0x2f, 0x02, 0x00, 0x00, 0x01, 0x1c];
        d.extend_from_slice(
            b"discovery_protocol_version=3.0.0.2 model=FLEX-6400 serial=0621-1104-6400-0001 \
              version=3.5.9 nickname=Shack callsign=KD9TAW ip=192.168.1.20 port=4992 \
              status=Available",
        );
        let r = parse_discovery(&d).expect("parsed");
        assert_eq!(r.model, "FLEX-6400");
        assert_eq!(r.nickname, "Shack");
        assert_eq!(r.ip, "192.168.1.20");
    }

    #[test]
    fn non_discovery_traffic_yields_none() {
        assert_eq!(parse_discovery(b"GET / HTTP/1.1\r\nHost: x\r\n"), None);
        assert_eq!(parse_discovery(&[0u8; 64]), None);
        // An ip= token with junk must not pass the dotted-quad sanity check.
        assert_eq!(parse_discovery(b"ip=not.an.addr.zz model=FAKE"), None);
    }
}
