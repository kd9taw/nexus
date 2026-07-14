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
    /// True while Nexus itself intends to transmit. Shared with the owning [`CivDaemon`] so the
    /// broker's disconnect fail-safe unkey can skip while Nexus is on the air (its own Rig is a
    /// client here, and a transient reconnect must not steal the over). See `owner_transmitting`.
    tx_intent: Arc<AtomicBool>,
}

impl CivBackend {
    pub fn new(h: CivHandle, addr: u8, tx_intent: Arc<AtomicBool>) -> Self {
        CivBackend {
            h,
            addr,
            split: AtomicBool::new(false),
            tx_intent,
        }
    }

    fn ack(&self, f: Frame) -> bool {
        self.h.transact(f, Expect::Ack).is_ok()
    }
    fn read(&self, f: Frame, cmd: u8, sub: Option<u8>) -> Result<Frame, CivError> {
        self.h.transact(f, Expect::Reply { cmd, sub })
    }

    /// Read a `15 <sub>` transmit meter and format it through `cal` (raw 0–255 → engineering
    /// unit) to `decimals` places. Returns None if the rig doesn't answer (e.g. not keyed).
    fn tx_meter(&self, sub: u8, cal: fn(u16) -> f32, decimals: usize) -> Option<String> {
        let f = self
            .read(commands::read_meter(self.addr, sub), 0x15, Some(sub))
            .ok()?;
        let raw = commands::parse_meter_raw(&f, sub)?;
        Some(format!("{:.*}", decimals, cal(raw)))
    }

    /// Read a `14 <sub>` DSP level as a 0..1 fraction string (the rigctld level convention).
    fn dsp_level(&self, sub: u8) -> Option<String> {
        let f = self
            .read(commands::read_dsp_level(self.addr, sub), 0x14, Some(sub))
            .ok()?;
        let raw = commands::parse_dsp_level_raw(&f, sub)?;
        Some(format!("{:.2}", f64::from(raw) / 255.0))
    }
    /// Set a `14 <sub>` DSP level from a 0..1 fraction string.
    fn set_dsp_level_pct(&self, sub: u8, value: &str) -> Option<bool> {
        let frac: f64 = value.parse().ok()?;
        let percent = (frac.clamp(0.0, 1.0) * 100.0).round() as u8;
        Some(self.ack(commands::set_dsp_level(self.addr, sub, percent)))
    }
}

impl RigBackend for CivBackend {
    fn owner_transmitting(&self) -> bool {
        self.tx_intent.load(Ordering::Relaxed)
    }

    fn freq_hz(&self) -> u64 {
        match self.read(commands::read_freq(self.addr), 0x03, None) {
            Ok(f) => commands::parse_freq(&f)
                .or(self.h.state().freq_hz)
                .unwrap_or(0),
            // Radio busy (a timeout can be one crowded moment): the last transceive/
            // reply is honest recent truth. A DEAD engine gets no such grace — serving
            // the frozen cache would paint a zombie green with a frozen dial.
            Err(CivError::Timeout) => self.h.state().freq_hz.unwrap_or(0),
            Err(_) => 0,
        }
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
            "MICGAIN" => {
                let f = self
                    .read(commands::read_mic_gain(self.addr), 0x14, Some(0x0B))
                    .ok()?;
                let raw = commands::parse_mic_gain_raw(&f)?;
                Some(format!("{:.2}", f64::from(raw) / 255.0))
            }
            // Transmit meters (0x15 read family). Values are already in engineering units:
            // SWR ratio, ALC 0..1, Po in watts, COMP in dB. Meaningful only while keyed.
            "SWR" => self.tx_meter(commands::METER_SWR, commands::swr_from_raw, 2),
            "ALC" => self.tx_meter(commands::METER_ALC, commands::alc_frac_from_raw, 3),
            // Answer BOTH tokens with true watts: Hamlib's plain RFPOWER_METER is a normalized
            // 0..1 fraction while _WATTS is watts, and Nexus polls _WATTS so the reading is watts
            // on any rig. The native daemon has only the one calibrated Po meter, so it serves
            // watts for either name (a Hamlib rig lacking _WATTS returns None → the row hides).
            "RFPOWER_METER" | "RFPOWER_METER_WATTS" => {
                self.tx_meter(commands::METER_PO, commands::po_watts_from_raw, 1)
            }
            "COMP_METER" => self.tx_meter(commands::METER_COMP, commands::comp_db_from_raw, 1),
            // RX DSP levels — 0..1 like mic gain (distinct from the NR/NB on/off funcs).
            "NR" => self.dsp_level(commands::LVL_NR),
            "NB" => self.dsp_level(commands::LVL_NB),
            // AGC as the Hamlib enum int (OFF=0/FAST=2/SLOW=3/MEDIUM=5), translated from the rig's
            // Icom byte so the rigctld side stays Hamlib-native.
            "AGC" => {
                let f = self
                    .read(commands::read_agc(self.addr), 0x16, Some(0x12))
                    .ok()?;
                let civ = commands::parse_agc_civ(&f)?;
                Some(format!("{}", commands::agc_hamlib_from_civ(civ)))
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
            "MICGAIN" => {
                let frac: f64 = value.parse().ok()?;
                let percent = (frac.clamp(0.0, 1.0) * 100.0).round() as u8;
                Some(self.ack(commands::set_mic_gain(self.addr, percent)))
            }
            "NR" => self.set_dsp_level_pct(commands::LVL_NR, value),
            "NB" => self.set_dsp_level_pct(commands::LVL_NB, value),
            "AGC" => {
                // Value is the Hamlib AGC enum int; translate to the rig's Icom byte.
                let hamlib: u8 = value.parse().ok()?;
                Some(self.ack(commands::set_agc(
                    self.addr,
                    commands::agc_civ_from_hamlib(hamlib),
                )))
            }
            "KEYSPD" => {
                let wpm: u32 = value.parse().ok()?;
                Some(self.ack(commands::set_keyer_speed_wpm(self.addr, wpm)))
            }
            _ => None,
        }
    }

    fn func(&self, token: &str) -> Option<bool> {
        // DSP / audio funcs share CI-V command 0x16; the token → sub-command map lives in
        // commands::func_sub. RIT/XIT are separate registers with no simple read here.
        let sub = commands::func_sub(token)?;
        let f = self
            .read(commands::read_dsp_func(self.addr, sub), 0x16, Some(sub))
            .ok()?;
        commands::parse_dsp_func(&f, sub)
    }

    fn set_func(&self, token: &str, on: bool) -> Option<bool> {
        match token {
            "RIT" => Some(self.ack(commands::set_rit_on(self.addr, on))),
            "XIT" => Some(self.ack(commands::set_dtx_on(self.addr, on))),
            // NB / NR / ANF / COMP / MON / VOX → the 0x16 DSP-function table.
            _ => commands::func_sub(token)
                .map(|sub| self.ack(commands::set_dsp_func(self.addr, sub, on))),
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

    fn set_rptr_offset(&self, hz: i64) -> Option<bool> {
        // Cmd 0D, 3-byte BCD in 100 Hz units (confirmed IC-9700 ref: 600 kHz → 00 60 00).
        // The offset magnitude is unsigned; direction comes from the duplex shift (`R`).
        Some(self.ack(commands::set_rptr_offset(self.addr, hz.unsigned_abs())))
    }

    fn set_ctcss(&self, tenths: u32) -> Option<bool> {
        if tenths == 0 {
            return Some(self.ack(commands::set_tone_func(self.addr, false)));
        }
        let tone = self.ack(commands::set_repeater_tone(self.addr, tenths));
        let func = self.ack(commands::set_tone_func(self.addr, true));
        Some(tone && func)
    }
}

/// The running native daemon: the CI-V serial engine + a stoppable rigctld TCP server.
pub struct CivDaemon {
    engine: CivEngine,
    /// The radio's CI-V address — kept for the Drop-time safety key-up.
    civ_addr: u8,
    tcp_stop: Arc<AtomicBool>,
    tcp_thread: Option<JoinHandle<()>>,
    /// Shared with the broker backend: set true while Nexus is transmitting so the disconnect
    /// fail-safe unkey doesn't fire on Nexus's own Rig reconnect (the CI-V PTT-flicker fix).
    tx_intent: Arc<AtomicBool>,
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
        let tx_intent = Arc::new(AtomicBool::new(false));
        let backend: Arc<dyn RigBackend> = Arc::new(CivBackend::new(
            engine.handle(),
            civ_addr,
            tx_intent.clone(),
        ));
        let tcp_stop = Arc::new(AtomicBool::new(false));
        let tcp_thread = {
            let stop = tcp_stop.clone();
            std::thread::Builder::new()
                .name("civ-daemon-tcp".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                // WINDOWS GOTCHA: WinSock accept() INHERITS the listener's
                                // non-blocking mode (Linux does not — so tests never saw this).
                                // Our listener is non-blocking (the loop polls tcp_stop), so
                                // without this reset every accepted connection's first idle
                                // read hit WouldBlock, serve_connection's line loop treated it
                                // as an error and closed the connection after ~one command.
                                // Nexus's own Rig client then churned reconnects (os error
                                // 10053) — and when the dropped connection had just asserted
                                // PTT (`T 1`), the disconnect fail-safe unkeyed the radio: the
                                // IC-9700 native-CI-V "PTT flicker".
                                let _ = stream.set_nonblocking(false);
                                let _ = stream.set_nodelay(true);
                                let b = Arc::clone(&backend);
                                std::thread::spawn(move || serve_connection(stream, b));
                            }
                            // Transient accept errors (an aborted pending connection —
                            // WSAECONNRESET on Windows) must NOT kill the listener: a
                            // healthy daemon would turn permanently connection-refused.
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            Err(_) => {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                        }
                    }
                })
                .expect("spawn civ-daemon-tcp")
        };
        super::diag::note("CivDaemon created (new serial engine + rigctld TCP)");
        Ok(CivDaemon {
            engine,
            civ_addr,
            tcp_stop,
            tcp_thread: Some(tcp_thread),
            tx_intent,
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

    /// Tell the broker whether Nexus itself is transmitting, so the disconnect fail-safe unkey
    /// stands down while we're on the air (a reconnect of Nexus's own Rig must not drop the over).
    /// The service loop calls this each tick with its keyed state.
    pub fn set_tx_intent(&self, on: bool) {
        self.tx_intent.store(on, Ordering::Relaxed);
    }

    /// Flip the rig's DATA mode (`1A 06`) — the TUNE path uses this so a plain-USB Icom
    /// modulates the tune tone from the USB codec (data OFF = mic source = zero RF).
    /// Best-effort single transact; NAKs (rig already there) are fine.
    pub fn set_data_mode(&self, on: bool) {
        let _ = self.engine.handle().transact(
            commands::set_data_mode(self.civ_addr, on, None),
            Expect::Ack,
        );
    }

    /// Main/Sub selector byte for the scope-CONTROL commands: `Some(0x00)` (Main) on dual-scope
    /// rigs, `None` (omit) on single-scope rigs. The stream is already pinned to Main by
    /// `scope_stream_frames`, so controlling the Main scope is what the operator sees.
    fn scope_ms(&self) -> Option<u8> {
        super::scope::scope_is_dual(self.civ_addr).then_some(0x00)
    }

    /// Set the rig's scope SPAN (`27 15`) — the ± half-width in Hz (rig table 2.5k..500k).
    /// Best-effort transact; a NAK (unsupported / in fixed mode) is fine.
    pub fn set_scope_span(&self, span_hz: u32) {
        let _ = self.engine.handle().transact(
            commands::set_scope_span(self.civ_addr, self.scope_ms(), span_hz),
            Expect::Ack,
        );
    }

    /// Set the rig's scope REFERENCE level (`27 19`), in tenths of a dB (−200..+200).
    pub fn set_scope_ref(&self, ref_tenths_db: i32) {
        let _ = self.engine.handle().transact(
            commands::set_scope_ref(self.civ_addr, self.scope_ms(), ref_tenths_db),
            Expect::Ack,
        );
    }

    /// Set the rig's scope CENTER/FIXED mode (`27 14`): `true` = fixed (band-edge), `false` =
    /// center (follow the dial).
    pub fn set_scope_center_mode(&self, fixed: bool) {
        let _ = self.engine.handle().transact(
            commands::set_scope_center_mode(self.civ_addr, self.scope_ms(), fixed),
            Expect::Ack,
        );
    }
}

impl Drop for CivDaemon {
    fn drop(&mut self) {
        // TX SAFETY: a radio keyed via CI-V stays keyed when the port merely closes —
        // send a best-effort key-up FIRST, while the serial engine is still alive.
        // Idempotent (an already-RX radio just acks); one choke point covers every
        // native teardown path: rig rebuilds, monitor recycles, handoff drops, app exit.
        if self.engine.is_alive() {
            super::diag::note("CivDaemon::Drop — safety key-up (a daemon is being torn down)");
            let _ = self
                .engine
                .handle()
                .transact(commands::set_ptt(self.civ_addr, false), Expect::Ack);
        }
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
