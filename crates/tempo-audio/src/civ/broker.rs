//! The native CI-V daemon — **what listens on the radio's rigctld TCP port** when
//! `icom_native_cat` is on.
//!
//! Nexus's entire CAT stack (`Rig`, `probe_cat`, the dual-radio monitors, the handoff)
//! talks the rigctld TEXT protocol to `127.0.0.1:<port>` and never cares what serves it.
//! [`CivDaemon`] binds that port and answers with [`CivBackend`] — every verb translated
//! to CI-V over the serial engine that owns the COM port. The prize over real rigctld:
//! the same serial stream carries the radio's **scope waveform** (a real RF panadapter)
//! and transceive pushes, which rigctld discards.
//!
//! Everything here is I/O-generic and unit-tested against the in-memory fake radio; only
//! [`CivDaemon::start`] (opening the real COM port) needs the `serial` feature.

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::commands::{self, IcomModel, Mode};
use super::engine::{CivEngine, CivError, CivHandle, Expect};
use super::frame::Frame;
use super::scope::ScopeSweep;
use crate::rigctld_server::{serve_connection, RigBackend};

/// rigctld-protocol backend that translates every verb to CI-V through the engine.
pub struct CivBackend {
    h: CivHandle,
    addr: u8,
    /// Split state the UI/`s` verb reads back (the rig's `0F` read is skipped — the
    /// last commanded state is authoritative for the session, like the Hamlib cache).
    split: AtomicBool,
}

impl CivBackend {
    pub fn new(h: CivHandle, addr: u8) -> Self {
        CivBackend {
            h,
            addr,
            split: AtomicBool::new(false),
        }
    }

    fn ack(&self, f: Frame) -> bool {
        self.h.transact(f, Expect::Ack).is_ok()
    }
    fn read(&self, f: Frame, cmd: u8, sub: Option<u8>) -> Result<Frame, CivError> {
        self.h.transact(f, Expect::Reply { cmd, sub })
    }
}

impl RigBackend for CivBackend {
    fn freq_hz(&self) -> u64 {
        self.read(commands::read_freq(self.addr), 0x03, None)
            .ok()
            .and_then(|f| commands::parse_freq(&f))
            .or(self.h.state().freq_hz) // radio busy → last transceive/reply
            .unwrap_or(0)
    }

    fn mode(&self) -> (String, u32) {
        let reply = self
            .read(commands::read_mode(self.addr), 0x04, None)
            .ok()
            .and_then(|f| commands::parse_mode(&f));
        let st = self.h.state();
        let (mode, _filter) = match reply {
            Some(m) => m,
            None => (st.mode.unwrap_or(Mode::Usb), st.filter),
        };
        // Report soundcard-digital as PKTUSB/PKTLSB, the names the rest of Nexus speaks.
        let name = match (mode, st.data_mode.unwrap_or(false)) {
            (Mode::Usb, true) => "PKTUSB".to_string(),
            (Mode::Lsb, true) => "PKTLSB".to_string(),
            (m, _) => m.name().to_string(),
        };
        (name, 0) // passband unreported (0 = unknown to Hamlib clients)
    }

    fn ptt(&self) -> bool {
        self.read(commands::read_ptt(self.addr), 0x1C, Some(0x00))
            .ok()
            .and_then(|f| commands::parse_ptt(&f))
            .or(self.h.state().ptt)
            .unwrap_or(false)
    }

    fn split(&self) -> bool {
        self.split.load(Ordering::Relaxed)
    }

    fn set_freq(&self, hz: u64) -> bool {
        self.ack(commands::set_freq(self.addr, hz))
    }

    fn set_mode(&self, mode: &str, _passband_hz: u32) -> bool {
        // PKT*/DATA-* = base sideband + DATA mode on; every plain mode turns DATA off.
        let up = mode.to_ascii_uppercase();
        let (base, data) = match up.as_str() {
            "PKTUSB" | "DATA-U" | "PKT-U" => (Mode::Usb, true),
            "PKTLSB" | "DATA-L" | "PKT-L" => (Mode::Lsb, true),
            _ => match Mode::from_name(&up) {
                Some(m) => (m, false),
                None => return false,
            },
        };
        let mode_ok = self.ack(commands::set_mode(self.addr, base, None));
        // Data-mode set: tolerate a NAK when turning it OFF (some rigs NAK a redundant
        // off) but require the ACK when turning it ON — FT8 must actually get USB-D.
        let data_ok = self.ack(commands::set_data_mode(self.addr, data, None));
        mode_ok && (data_ok || !data)
    }

    fn set_ptt(&self, on: bool) -> bool {
        self.ack(commands::set_ptt(self.addr, on))
    }

    fn set_vfo(&self, vfo: &str) -> bool {
        match commands::select_vfo(self.addr, vfo) {
            Some(f) => self.ack(f),
            None => false,
        }
    }

    fn level(&self, name: &str) -> Option<String> {
        match name {
            "STRENGTH" => {
                let f = self
                    .read(commands::read_smeter(self.addr), 0x15, Some(0x02))
                    .ok()?;
                let raw = commands::parse_smeter_raw(&f)?;
                Some(format!(
                    "{}",
                    commands::smeter_db_rel_s9(raw).round() as i32
                ))
            }
            "RFPOWER" => {
                let f = self
                    .read(commands::read_rf_power(self.addr), 0x14, Some(0x0A))
                    .ok()?;
                let raw = commands::parse_rf_power_raw(&f)?;
                Some(format!("{:.2}", f64::from(raw) / 255.0))
            }
            _ => None,
        }
    }

    fn set_level(&self, name: &str, value: &str) -> Option<bool> {
        match name {
            "RFPOWER" => {
                let frac: f64 = value.parse().ok()?;
                let percent = (frac.clamp(0.0, 1.0) * 100.0).round() as u8;
                Some(self.ack(commands::set_rf_power(self.addr, percent)))
            }
            "KEYSPD" => {
                let wpm: u32 = value.parse().ok()?;
                Some(self.ack(commands::set_keyer_speed_wpm(self.addr, wpm)))
            }
            _ => None,
        }
    }

    fn set_func(&self, token: &str, on: bool) -> Option<bool> {
        match token {
            "RIT" => Some(self.ack(commands::set_rit_on(self.addr, on))),
            "XIT" => Some(self.ack(commands::set_dtx_on(self.addr, on))),
            _ => None,
        }
    }

    fn send_morse(&self, text: &str) -> Option<bool> {
        // Chunk to the rig's per-frame CW text limit; all chunks must ack.
        let bytes: Vec<u8> = text.bytes().filter(u8::is_ascii).collect();
        if bytes.is_empty() {
            return Some(false);
        }
        let ok = bytes.chunks(commands::MORSE_CHUNK).all(|c| {
            let chunk = String::from_utf8_lossy(c);
            self.ack(commands::send_morse(self.addr, &chunk))
        });
        Some(ok)
    }

    fn stop_morse(&self) -> Option<bool> {
        Some(self.ack(commands::stop_morse(self.addr)))
    }

    fn set_split(&self, on: bool, _tx_vfo: &str) -> Option<bool> {
        let ok = self.ack(commands::set_split(self.addr, on));
        if ok {
            self.split.store(on, Ordering::Relaxed);
        }
        Some(ok)
    }

    fn set_split_freq(&self, hz: u64) -> Option<bool> {
        Some(self.ack(commands::set_unselected_freq(self.addr, hz)))
    }

    fn set_rit(&self, hz: i32) -> Option<bool> {
        Some(self.ack(commands::set_rit_offset(self.addr, hz)))
    }

    fn set_xit(&self, hz: i32) -> Option<bool> {
        // Icom's ΔTX shares the RIT offset register.
        Some(self.ack(commands::set_rit_offset(self.addr, hz)))
    }

    fn set_rptr_shift(&self, shift: &str) -> Option<bool> {
        Some(self.ack(commands::set_duplex(self.addr, shift)))
    }

    fn set_ctcss(&self, tenths: u32) -> Option<bool> {
        if tenths == 0 {
            return Some(self.ack(commands::set_tone_func(self.addr, false)));
        }
        let tone = self.ack(commands::set_repeater_tone(self.addr, tenths));
        let func = self.ack(commands::set_tone_func(self.addr, true));
        Some(tone && func)
    }
    // set_rptr_offset stays unimplemented (RPRT -11): the offset command needs on-rig
    // verification; the 9700's auto-repeater supplies the offset in practice.
}

/// The running native daemon: the CI-V serial engine + a stoppable rigctld TCP server.
pub struct CivDaemon {
    engine: CivEngine,
    tcp_stop: Arc<AtomicBool>,
    tcp_thread: Option<JoinHandle<()>>,
}

impl CivDaemon {
    /// Start on an already-open transport (tests use the in-memory fake radio).
    pub fn start_with_io(
        io: Box<dyn super::engine::CivIo>,
        civ_addr: u8,
        tcp_port: u16,
    ) -> std::io::Result<CivDaemon> {
        let engine = CivEngine::start(io, civ_addr);
        let listener = TcpListener::bind(("127.0.0.1", tcp_port))?;
        listener.set_nonblocking(true)?;
        let backend: Arc<dyn RigBackend> = Arc::new(CivBackend::new(engine.handle(), civ_addr));
        let tcp_stop = Arc::new(AtomicBool::new(false));
        let tcp_thread = {
            let stop = tcp_stop.clone();
            std::thread::Builder::new()
                .name("civ-daemon-tcp".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let _ = stream.set_nodelay(true);
                                let b = Arc::clone(&backend);
                                std::thread::spawn(move || serve_connection(stream, b));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            Err(_) => break,
                        }
                    }
                })
                .expect("spawn civ-daemon-tcp")
        };
        Ok(CivDaemon {
            engine,
            tcp_stop,
            tcp_thread: Some(tcp_thread),
        })
    }

    /// Open the real COM port and start the daemon (the production entry).
    #[cfg(feature = "serial")]
    pub fn start(
        port_name: &str,
        baud: u32,
        civ_addr: u8,
        tcp_port: u16,
    ) -> std::io::Result<CivDaemon> {
        let port = serialport::new(port_name, baud)
            .timeout(super::engine::READ_TIMEOUT)
            .open()
            .map_err(std::io::Error::other)?;
        Self::start_with_io(Box::new(port), civ_addr, tcp_port)
    }

    /// The CI-V address to drive `model_name` at, when it's a native-capable Icom.
    pub fn civ_addr_for(model_name: &str) -> Option<u8> {
        IcomModel::from_name(model_name).map(IcomModel::default_civ_addr)
    }

    /// False once the serial engine died (port unplugged / denied).
    pub fn is_alive(&self) -> bool {
        self.engine.is_alive()
    }

    /// Newest completed scope sweep (latest-wins; `None` until the next arrives).
    pub fn take_scope_row(&self) -> Option<ScopeSweep> {
        self.engine.take_scope_row()
    }

    /// Stream the radio's scope waveform (on for the ACTIVE radio, off for monitors —
    /// the stream would otherwise crowd a monitor's slow poll off the serial link).
    pub fn set_scope_enabled(&self, on: bool) {
        self.engine.set_scope_enabled(on);
    }
}

impl Drop for CivDaemon {
    fn drop(&mut self) {
        self.tcp_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.tcp_thread.take() {
            let _ = t.join();
        }
        // engine's Drop stops the serial thread (and closes the port).
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::tests_support::FakeRadio;
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    fn daemon() -> (CivDaemon, u16) {
        // Race-free enough for tests: bind :0 to learn a free port, drop, rebind.
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let (radio, _push) = FakeRadio::new(0xA2);
        let d = CivDaemon::start_with_io(Box::new(radio), 0xA2, port).unwrap();
        (d, port)
    }

    #[test]
    fn a_rigctld_client_drives_the_fake_radio_end_to_end() {
        let (_d, port) = daemon();
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut rd = BufReader::new(c.try_clone().unwrap());
        let mut line = String::new();

        // Exactly what Rig/probe_cat do: read freq, set freq, read back.
        c.write_all(b"f\n").unwrap();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "145000000\n");

        c.write_all(b"F 144200000\n").unwrap();
        line.clear();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "RPRT 0\n");

        c.write_all(b"f\n").unwrap();
        line.clear();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "144200000\n");

        // S-meter through the extended verb (the fake reports raw 120 = S9 = 0 dB).
        c.write_all(b"l STRENGTH\n").unwrap();
        line.clear();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "0\n");
    }

    #[test]
    fn chk_vfo_answers_so_open_cats_probe_finds_us() {
        let (_d, port) = daemon();
        assert!(crate::rigctld_server::probe_rigctld(
            &format!("127.0.0.1:{port}"),
            Duration::from_millis(800),
        ));
    }
}
