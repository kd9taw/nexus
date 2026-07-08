//! Antenna rotator control via Hamlib's `rotctld` daemon over TCP — the same
//! daemon-over-TCP pattern as the `rigctld` CAT path, so Nexus needs no C dependency.
//! The operator runs `rotctld -m <model> -r <port> -t <tcp>` (or points a rig with a
//! built-in rotor); Nexus connects and sends `P <az> <el>` to turn the antenna and `p`
//! to read where it is. Azimuth-only (elevation fixed at 0).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_millis(800);

/// rotctld `P` — point to `az_deg` (normalized to [0,360)), elevation 0.
pub fn point_line(az_deg: f64) -> String {
    format!("P {:.1} 0\n", az_deg.rem_euclid(360.0))
}

/// rotctld `P` — point to `az_deg` (normalized to [0,360)) and `el_deg` (clamped
/// to [0,90], an az/el rotor's mechanical elevation range). The elevation-capable
/// form used to track a satellite pass.
pub fn point_line_azel(az_deg: f64, el_deg: f64) -> String {
    format!(
        "P {:.1} {:.1}\n",
        az_deg.rem_euclid(360.0),
        el_deg.clamp(0.0, 90.0)
    )
}

fn connect(addr: &str) -> std::io::Result<TcpStream> {
    let s = TcpStream::connect(addr)?;
    s.set_read_timeout(Some(IO_TIMEOUT))?;
    s.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(s)
}

/// Point the rotator at `az_deg` via rotctld at `addr` (host:port). `Ok` on `RPRT 0`
/// (or an empty ack), else the daemon's error text.
pub fn point(addr: &str, az_deg: f64) -> std::io::Result<()> {
    let mut s = connect(addr)?;
    s.write_all(point_line(az_deg).as_bytes())?;
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).unwrap_or(0);
    let reply = String::from_utf8_lossy(&buf[..n]);
    if reply.contains("RPRT 0") || reply.trim().is_empty() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "rotctld error: {}",
            reply.trim()
        )))
    }
}

/// Point the rotator at `az_deg`/`el_deg` via rotctld at `addr` (host:port). `Ok`
/// on `RPRT 0` (or an empty ack), else the daemon's error text. The az/el twin of
/// [`point`] for satellite tracking on an elevation-capable rotor.
pub fn point_azel(addr: &str, az_deg: f64, el_deg: f64) -> std::io::Result<()> {
    let mut s = connect(addr)?;
    s.write_all(point_line_azel(az_deg, el_deg).as_bytes())?;
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).unwrap_or(0);
    let reply = String::from_utf8_lossy(&buf[..n]);
    if reply.contains("RPRT 0") || reply.trim().is_empty() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "rotctld error: {}",
            reply.trim()
        )))
    }
}

/// Stop rotation immediately (rotctld `S`).
pub fn stop(addr: &str) -> std::io::Result<()> {
    let mut s = connect(addr)?;
    s.write_all(b"S\n")?;
    let mut buf = [0u8; 64];
    let _ = s.read(&mut buf); // drain the RPRT ack; stop is best-effort
    Ok(())
}

/// Read the current azimuth (degrees) from rotctld (`p` → `az\nel`). `None` on any
/// failure (daemon down, timeout, unparsable) — the UI shows "—" rather than erroring.
pub fn read_azimuth(addr: &str) -> Option<f64> {
    let mut s = connect(addr).ok()?;
    s.write_all(b"p\n").ok()?;
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).ok()?;
    let reply = String::from_utf8_lossy(&buf[..n]);
    reply.lines().next()?.trim().parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_line_formats_and_normalizes_azimuth() {
        assert_eq!(point_line(90.0), "P 90.0 0\n");
        assert_eq!(point_line(0.0), "P 0.0 0\n");
        assert_eq!(point_line(359.4), "P 359.4 0\n");
        // wrap negatives and ≥360 into [0,360)
        assert_eq!(point_line(-90.0), "P 270.0 0\n");
        assert_eq!(point_line(450.0), "P 90.0 0\n");
    }

    #[test]
    fn point_line_azel_formats_normalizes_and_clamps() {
        assert_eq!(point_line_azel(90.0, 45.0), "P 90.0 45.0\n");
        assert_eq!(point_line_azel(0.0, 0.0), "P 0.0 0.0\n");
        // azimuth wraps into [0,360)
        assert_eq!(point_line_azel(-90.0, 30.0), "P 270.0 30.0\n");
        assert_eq!(point_line_azel(450.0, 10.0), "P 90.0 10.0\n");
        // elevation clamps into [0,90]
        assert_eq!(point_line_azel(180.0, -5.0), "P 180.0 0.0\n");
        assert_eq!(point_line_azel(180.0, 120.0), "P 180.0 90.0\n");
    }
}
