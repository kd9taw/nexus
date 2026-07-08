//! Minimal SNTP (RFC 4330) client — a single UDP query to estimate the local
//! clock's offset from UTC. `std`-only, keeping `tempo-net` dependency-free.
//!
//! FT1/DX1 are slot-timed to UTC, so an accurate PC clock is essential.
//! [`query`] returns the **local-clock-minus-UTC** offset in milliseconds
//! (positive = the PC clock is ahead / fast). It is best-effort: any network or
//! parse failure is an `Err`, which callers should treat as "unknown / offline".

use std::net::{ToSocketAddrs, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Seconds between the NTP epoch (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_OFFSET: f64 = 2_208_988_800.0;
/// 2^32, for converting the NTP fractional-seconds field.
const TWO_POW_32: f64 = 4_294_967_296.0;

/// Query one NTP server (`host`, e.g. `"pool.ntp.org:123"`) and return the local
/// clock's offset from UTC in milliseconds (positive = local clock is ahead).
pub fn query(host: &str, timeout: Duration) -> std::io::Result<i64> {
    let addr = host
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no NTP address"))?;
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(timeout))?;
    sock.set_write_timeout(Some(timeout))?;

    // Client request: LI=0, VN=4, Mode=3 (client) in byte 0; the rest zeroed.
    let mut req = [0u8; 48];
    req[0] = 0x23;

    let t1 = unix_now();
    sock.send_to(&req, addr)?;
    let mut resp = [0u8; 48];
    let n = sock.recv(&mut resp)?;
    let t4 = unix_now();
    if n < 48 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "short NTP reply",
        ));
    }
    // Server receive (T2, bytes 32..40) and transmit (T3, bytes 40..48) stamps.
    let t2 = ntp_to_unix(&resp[32..40]);
    let t3 = ntp_to_unix(&resp[40..48]);
    if t3 <= 0.0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid NTP timestamp",
        ));
    }
    // NTP offset = ((T2−T1)+(T3−T4))/2 = UTC − local; report local − UTC.
    let offset_secs = ((t2 - t1) + (t3 - t4)) / 2.0;
    Ok((-offset_secs * 1000.0).round() as i64)
}

/// Try several servers in order, returning the first success.
pub fn query_any(hosts: &[&str], timeout: Duration) -> std::io::Result<i64> {
    let mut last = std::io::Error::other("no NTP servers");
    for h in hosts {
        match query(h, timeout) {
            Ok(ms) => return Ok(ms),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Parse an 8-byte NTP timestamp (u32 seconds + u32 fraction, big-endian) into
/// Unix seconds.
fn ntp_to_unix(b: &[u8]) -> f64 {
    let secs = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64;
    let frac = u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as f64 / TWO_POW_32;
    secs + frac - NTP_UNIX_OFFSET
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntp_timestamp_parses_to_unix() {
        // NTP seconds for 2024-01-01T00:00:00Z = 3_913_056_000 → Unix 1_704_067_200.
        let secs: u32 = 3_913_056_000;
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&secs.to_be_bytes());
        let unix = ntp_to_unix(&b);
        assert!((unix - 1_704_067_200.0).abs() < 1.0, "got {unix}");
    }

    #[test]
    fn fraction_field_is_half_a_second() {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&(NTP_UNIX_OFFSET as u32).to_be_bytes()); // unix 0
        b[4] = 0x80; // top bit of the fraction = 0.5 s
        let unix = ntp_to_unix(&b);
        assert!((unix - 0.5).abs() < 1e-6, "got {unix}");
    }
}
