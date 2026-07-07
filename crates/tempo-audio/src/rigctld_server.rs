//! A rigctld-compatible TCP **server** — the "broker, not port hog" half of CAT.
//!
//! Nexus owns one connection to the radio; this server lets WSJT-X / N1MM / Log4OM
//! (any Hamlib NET rigctl client) **share that radio through Nexus** by speaking the
//! same text protocol Hamlib's `rigctld` does, on :4532. Every command is relayed to
//! a [`RigBackend`] (Nexus's live rig state), so a logger setting the dial retunes
//! Nexus too.
//!
//! Pure protocol handling ([`handle_command`]) is unit-tested; the std-only TCP loop
//! ([`serve`]) is covered by a localhost integration test. The one piece that needs a
//! real WSJT-X to validate is the exact `\dump_state` byte layout — emitted here in
//! the classic **protocol-0** form (so the client skips the protocol-1 key=value
//! block), grounded in Hamlib's `netrigctl` parser.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// The rig state the broker serves + the setters it relays. Implemented by Nexus's
/// live rig bridge (real radio) and by a mock in tests.
pub trait RigBackend: Send + Sync {
    fn freq_hz(&self) -> u64;
    fn mode(&self) -> (String, u32); // (mode, passband Hz)
    fn ptt(&self) -> bool;
    fn vfo(&self) -> String {
        "VFOA".to_string()
    }
    fn split(&self) -> bool {
        false
    }
    /// Setters return true on success (→ `RPRT 0`), false → `RPRT -1`.
    fn set_freq(&self, hz: u64) -> bool;
    fn set_mode(&self, mode: &str, passband_hz: u32) -> bool;
    fn set_ptt(&self, on: bool) -> bool;
    fn set_vfo(&self, _vfo: &str) -> bool {
        true
    }
}

/// The classic protocol-0 `\dump_state` capability dump. Wide HF–UHF ranges, all
/// modes, so a NET-rigctl client (WSJT-X) accepts the rig and lets you set any
/// freq/mode. First line `0` = protocol version 0 → the client does NOT read the
/// protocol-1 `key=value … done` trailer. NOTE: the one part needing live WSJT-X
/// validation; built to match Hamlib's `netrigctl` field-by-field reader.
const DUMP_STATE: &str = concat!(
    "0\n",                                                // protocol version (classic)
    "2\n",                                                // rig model (NET rigctl)
    "1\n",                                                // ITU region
    "135700 1300000000 0xffffffff -1 -1 0x3 0x0\n",       // rx range (all modes)
    "0 0 0 0 0 0 0\n",                                    // rx range terminator
    "135700 1300000000 0xffffffff 5000 100000 0x3 0x0\n", // tx range
    "0 0 0 0 0 0 0\n",                                    // tx range terminator
    "0xffffffff 1\n",                                     // tuning step: all modes, 1 Hz
    "0 0\n",                                              // tuning-step terminator
    "0xffffffff 2700\n",                                  // filter: all modes, 2700 Hz
    "0xffffffff 500\n",                                   // filter: all modes, 500 Hz
    "0 0\n",                                              // filter terminator
    "0\n",                                                // max_rit
    "0\n",                                                // max_xit
    "0\n",                                                // max_ifshift
    "0\n",                                                // announces
    "0\n",                                                // preamp list (empty)
    "0\n",                                                // attenuator list (empty)
    "0x0\n",                                              // has_get_func
    "0x0\n",                                              // has_set_func
    "0x0\n",                                              // has_get_level
    "0x0\n",                                              // has_set_level
    "0x0\n",                                              // has_get_parm
    "0x0\n",                                              // has_set_parm
);

fn rprt(ok: bool) -> String {
    if ok {
        "RPRT 0\n".into()
    } else {
        "RPRT -1\n".into()
    }
}

/// Outcome of handling one request line.
pub enum Handled {
    /// Write this back to the client.
    Reply(String),
    /// Client asked to quit — write nothing and close.
    Close,
}

/// Handle one rigctld request line against `backend`. Pure (apart from the backend
/// calls) so the protocol is unit-testable. Implements the subset a NET-rigctl
/// client (WSJT-X) uses: get/set freq (`f`/`F`), mode (`m`/`M`), PTT (`t`/`T`),
/// VFO (`v`/`V`), split (`s`), plus `\dump_state`, `\chk_vfo`, `\get_powerstat`, `q`.
pub fn handle_command(line: &str, backend: &dyn RigBackend) -> Handled {
    let line = line.trim();
    match line {
        "" => Handled::Reply(String::new()),
        "\\dump_state" => Handled::Reply(DUMP_STATE.to_string()),
        // No VFO mode → the client sends commands without an explicit VFO argument.
        "\\chk_vfo" => Handled::Reply("CHKVFO 0\n".into()),
        "\\get_powerstat" => Handled::Reply("1\n".into()), // powered on
        "q" | "Q" => Handled::Close,
        _ => {
            let mut p = line.split_whitespace();
            let reply = match p.next() {
                Some("f") => format!("{}\n", backend.freq_hz()),
                // Hamlib sends freq as printf %lf ("F 14074000.000000"), so parse
                // as f64 and round to Hz — a u64 parse rejects every real client.
                Some("F") => rprt(
                    p.next()
                        .and_then(|s| s.parse::<f64>().ok())
                        // Reject NaN/±inf and absurd magnitudes: `f.round() as u64`
                        // saturates inf/huge to u64::MAX (a garbage dial with a
                        // false RPRT 0). Cap at 1 THz — far above any ham band.
                        .filter(|f| f.is_finite() && (0.0..=1e12).contains(f))
                        .map(|f| backend.set_freq(f.round() as u64))
                        .unwrap_or(false),
                ),
                Some("m") => {
                    let (mode, pbw) = backend.mode();
                    format!("{mode}\n{pbw}\n")
                }
                Some("M") => {
                    let mode = p.next().unwrap_or("");
                    let pbw = p.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                    rprt(!mode.is_empty() && backend.set_mode(mode, pbw))
                }
                Some("v") => format!("{}\n", backend.vfo()),
                Some("V") => rprt(p.next().map(|v| backend.set_vfo(v)).unwrap_or(false)),
                Some("t") => format!("{}\n", backend.ptt() as u8),
                // RIG_PTT_ON=1, RIG_PTT_ON_MIC=2, RIG_PTT_ON_DATA=3 are all key-down;
                // only 0 (RIG_PTT_OFF) is key-up. WSJT-X with a Rear/Data audio source
                // sends `T 3`, so keying on `s == "1"` would silently never TX.
                Some("T") => rprt(
                    p.next()
                        .and_then(|s| s.parse::<i32>().ok())
                        .map(|v| backend.set_ptt(v != 0))
                        .unwrap_or(false),
                ),
                Some("s") => format!("{}\n{}\n", backend.split() as u8, backend.vfo()),
                // Unknown command → Hamlib's "not implemented".
                _ => "RPRT -11\n".into(),
            };
            Handled::Reply(reply)
        }
    }
}

/// Serve one client connection until EOF or `q`. Each line is one request.
pub fn serve_connection(stream: TcpStream, backend: Arc<dyn RigBackend>) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        match handle_command(&line, backend.as_ref()) {
            Handled::Reply(r) => {
                if !r.is_empty() && writer.write_all(r.as_bytes()).is_err() {
                    break;
                }
            }
            Handled::Close => break,
        }
    }
}

/// Run the broker accept loop, a thread per client. Blocks; spawn it on its own
/// thread. Backed by `backend` (shared with Nexus's rig).
pub fn serve(listener: TcpListener, backend: Arc<dyn RigBackend>) {
    for stream in listener.incoming().flatten() {
        let b = Arc::clone(&backend);
        std::thread::spawn(move || serve_connection(stream, b));
    }
}

/// Probe whether a rigctld (or compatible broker — maybe another Nexus) is already
/// listening on `addr`, so we can connect THROUGH it instead of fighting for the
/// serial port. Sends `\chk_vfo` and checks for any reply. Short timeout; never
/// blocks startup long.
pub fn probe_rigctld(addr: &str, timeout: std::time::Duration) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return false;
    };
    let Some(sa) = addrs.next() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&sa, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if stream.write_all(b"\\chk_vfo\n").is_err() {
        return false;
    }
    let mut buf = [0u8; 16];
    use std::io::Read;
    matches!(stream.read(&mut buf), Ok(n) if n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockRig {
        freq: Mutex<u64>,
        ptt: Mutex<bool>,
        mode: Mutex<(String, u32)>,
    }
    impl Default for MockRig {
        fn default() -> Self {
            MockRig {
                freq: Mutex::new(14_074_000),
                ptt: Mutex::new(false),
                mode: Mutex::new(("USB".into(), 2700)),
            }
        }
    }
    impl RigBackend for MockRig {
        fn freq_hz(&self) -> u64 {
            *self.freq.lock().unwrap()
        }
        fn mode(&self) -> (String, u32) {
            self.mode.lock().unwrap().clone()
        }
        fn ptt(&self) -> bool {
            *self.ptt.lock().unwrap()
        }
        fn set_freq(&self, hz: u64) -> bool {
            *self.freq.lock().unwrap() = hz;
            true
        }
        fn set_mode(&self, mode: &str, pbw: u32) -> bool {
            *self.mode.lock().unwrap() = (mode.to_string(), pbw);
            true
        }
        fn set_ptt(&self, on: bool) -> bool {
            *self.ptt.lock().unwrap() = on;
            true
        }
    }

    fn reply(line: &str, b: &dyn RigBackend) -> String {
        match handle_command(line, b) {
            Handled::Reply(r) => r,
            Handled::Close => "\0CLOSE".into(),
        }
    }

    #[test]
    fn protocol_get_set_commands() {
        let b = MockRig::default();
        assert_eq!(reply("f", &b), "14074000\n");
        assert_eq!(reply("F 7035000", &b), "RPRT 0\n");
        assert_eq!(*b.freq.lock().unwrap(), 7_035_000);
        assert_eq!(reply("f", &b), "7035000\n");
        assert_eq!(reply("m", &b), "USB\n2700\n");
        assert_eq!(reply("M FT8 3000", &b), "RPRT 0\n");
        assert_eq!(reply("m", &b), "FT8\n3000\n");
        assert_eq!(reply("t", &b), "0\n");
        assert_eq!(reply("T 1", &b), "RPRT 0\n");
        assert_eq!(reply("t", &b), "1\n");
        assert_eq!(reply("v", &b), "VFOA\n");
        // Malformed set → error; unknown command → not-implemented.
        assert_eq!(reply("F notanumber", &b), "RPRT -1\n");
        assert_eq!(reply("Z", &b), "RPRT -11\n");
        assert!(matches!(handle_command("q", &b), Handled::Close));
    }

    #[test]
    fn set_freq_accepts_hamlib_float_wire_form() {
        // Hamlib's netrigctl sends freq as printf %lf, e.g. `F 14074000.000000`.
        // A u64 parse rejected this and every real client's band change failed.
        let b = MockRig::default();
        assert_eq!(reply("F 14074000.000000", &b), "RPRT 0\n");
        assert_eq!(*b.freq.lock().unwrap(), 14_074_000);
        // Fractional Hz rounds to nearest.
        assert_eq!(reply("F 7035000.6", &b), "RPRT 0\n");
        assert_eq!(*b.freq.lock().unwrap(), 7_035_001);
        // Integer form (loggers that send it) and negatives/garbage still handled.
        assert_eq!(reply("F 21074000", &b), "RPRT 0\n");
        assert_eq!(*b.freq.lock().unwrap(), 21_074_000);
        assert_eq!(reply("F -1", &b), "RPRT -1\n");
        assert_eq!(reply("F notanumber", &b), "RPRT -1\n");
        // inf / NaN / absurd magnitudes must be rejected, not saturate the cast
        // to u64::MAX and set a garbage dial with a false RPRT 0.
        let last = *b.freq.lock().unwrap();
        assert_eq!(reply("F inf", &b), "RPRT -1\n");
        assert_eq!(reply("F 1e30", &b), "RPRT -1\n");
        assert_eq!(reply("F NaN", &b), "RPRT -1\n");
        assert_eq!(
            *b.freq.lock().unwrap(),
            last,
            "a rejected F must not move the dial"
        );
    }

    #[test]
    fn set_ptt_keys_on_any_nonzero_state() {
        // RIG_PTT_ON_MIC(2)/RIG_PTT_ON_DATA(3) are key-down; WSJT-X with a Rear/Data
        // audio source sends `T 3`. Keying only on "1" left the rig un-keyed on TX.
        let b = MockRig::default();
        assert_eq!(reply("T 3", &b), "RPRT 0\n");
        assert!(*b.ptt.lock().unwrap(), "T 3 (ON_DATA) must key the rig");
        assert_eq!(reply("T 0", &b), "RPRT 0\n");
        assert!(!*b.ptt.lock().unwrap(), "T 0 (OFF) must un-key");
        assert_eq!(reply("T 2", &b), "RPRT 0\n");
        assert!(*b.ptt.lock().unwrap(), "T 2 (ON_MIC) must key the rig");
        assert_eq!(reply("T 1", &b), "RPRT 0\n");
        assert!(*b.ptt.lock().unwrap(), "T 1 (ON) must key the rig");
        // Malformed PTT arg → error, no key change.
        assert_eq!(reply("T x", &b), "RPRT -1\n");
    }

    #[test]
    fn dump_state_and_chk_vfo_shape() {
        let b = MockRig::default();
        assert_eq!(reply("\\chk_vfo", &b), "CHKVFO 0\n");
        let ds = reply("\\dump_state", &b);
        let lines: Vec<&str> = ds.lines().collect();
        assert_eq!(lines[0], "0", "protocol version 0 (skips proto-1 trailer)");
        assert_eq!(lines[2], "1", "ITU region");
        // Range rows are 7 fields; terminators are all-zero.
        assert!(lines.contains(&"0 0 0 0 0 0 0"), "range terminator present");
        assert!(lines.contains(&"0 0"), "ts/filter terminator present");
        // Every line parses as the expected token shape (no stray text).
        assert!(lines[3].split_whitespace().count() == 7);
    }

    #[test]
    fn broker_serves_a_localhost_client_end_to_end() {
        let backend: Arc<dyn RigBackend> = Arc::new(MockRig::default());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let b = Arc::clone(&backend);
        std::thread::spawn(move || serve(listener, b));

        let mut client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut rd = BufReader::new(client.try_clone().unwrap());

        let mut line = String::new();
        client.write_all(b"f\n").unwrap();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "14074000\n");

        // A logger sets the dial through the broker → Nexus's backend updates.
        client.write_all(b"F 21074000\n").unwrap();
        line.clear();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "RPRT 0\n");
        assert_eq!(backend.freq_hz(), 21_074_000);

        // And it reads back the new frequency.
        client.write_all(b"f\n").unwrap();
        line.clear();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "21074000\n");
    }

    #[test]
    fn probe_detects_a_running_broker_and_absence() {
        // A bound broker is detected (connect-through path); a dead port is not.
        let backend: Arc<dyn RigBackend> = Arc::new(MockRig::default());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let b = Arc::clone(&backend);
        std::thread::spawn(move || serve(listener, b));
        let to = std::time::Duration::from_millis(500);
        assert!(
            probe_rigctld(&addr.to_string(), to),
            "running broker detected"
        );
        // An unused high port: nothing there.
        assert!(
            !probe_rigctld("127.0.0.1:1", to),
            "no broker on a dead port"
        );
    }
}
