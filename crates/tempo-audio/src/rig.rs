//! PTT / CAT control via Hamlib's `rigctld` daemon over TCP.
//!
//! Using `rigctld` (rather than linking `libhamlib`) keeps Tempo free of a C
//! build dependency: the operator runs `rigctld -m <model> -r <port>` and Tempo
//! talks to it over a socket. The protocol is line-based — commands like `T 1`
//! (PTT on), `T 0` (PTT off), `F 14074000` (set freq), `M USB 0` (set mode); a
//! reply of `RPRT 0` means success.
//!
//! For rigs without CAT, [`PttMode::Vox`] performs no keying and relies on the
//! transceiver's VOX (audio-triggered TX).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Which serial control line keys the transmitter for [`PttMode::Serial`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialLine {
    /// Request To Send.
    Rts,
    /// Data Terminal Ready.
    Dtr,
}

/// How transmit keying is performed — INDEPENDENT of CAT control. The WSJT-X model:
/// a rig can have full CAT freq/mode control while keying via VOX or a serial line,
/// so PTT and control are separate concerns (see [`Rig`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PttMode {
    /// Key via the CAT control channel (rigctld `T`). Requires a control channel;
    /// with none configured this no-ops (like VOX).
    Cat,
    /// No CAT keying — rely on the rig's VOX (audio-triggered TX).
    #[default]
    Vox,
    /// Key directly by asserting a serial control line (RTS or DTR) on `port`.
    ///
    /// With the `serial` feature this drives the line via `serialport`; without
    /// it the keying is logged and otherwise a no-op (the build has no serial
    /// backend), so the engine still runs and can fall back to VOX.
    Serial { port: String, line: SerialLine },
}

/// rigctld command line for PTT.
pub fn ptt_line(on: bool) -> String {
    format!("T {}\n", on as u8)
}
/// rigctld command line to set the dial frequency (Hz).
pub fn freq_line(hz: u64) -> String {
    format!("F {hz}\n")
}
/// rigctld command line to set mode + passband (Hz; 0 = rig default).
pub fn mode_line(mode: &str, passband_hz: u32) -> String {
    format!("M {mode} {passband_hz}\n")
}
/// rigctld `R` — FM repeater shift: "plus"→`+`, "minus"→`-`, anything else→`None`.
pub fn rptr_shift_line(shift: &str) -> String {
    let s = match shift.trim().to_ascii_lowercase().as_str() {
        "plus" | "+" => "+",
        "minus" | "-" => "-",
        _ => "None",
    };
    format!("R {s}\n")
}
/// rigctld `O` — FM repeater offset magnitude (Hz).
pub fn rptr_offset_line(hz: i64) -> String {
    format!("O {hz}\n")
}
/// rigctld `C` — CTCSS (PL) tone. Hamlib wants TENTHS of Hz (100.0 Hz → 1000); 0 = off.
pub fn ctcss_line(tone_hz: f32) -> String {
    format!("C {}\n", (tone_hz * 10.0).round().max(0.0) as u32)
}
/// rigctld `S` — split on/off + which VFO transmits (e.g. `S 1 VFOB`).
pub fn split_line(on: bool, tx_vfo: &str) -> String {
    format!("S {} {}\n", on as u8, tx_vfo)
}
/// rigctld `I` — the split (TX) frequency in Hz.
pub fn split_freq_line(hz: u64) -> String {
    format!("I {hz}\n")
}
/// rigctld `V` — select the active VFO (e.g. `VFOA`, `VFOB`, `Main`, `Sub`).
pub fn vfo_line(vfo: &str) -> String {
    format!("V {vfo}\n")
}
/// rigctld `U` — toggle a function (RIT/XIT must be enabled this way before `J`/`Z`).
pub fn func_line(func: &str, on: bool) -> String {
    format!("U {} {}\n", func, on as u8)
}
/// rigctld `J` — RIT offset in Hz (receive incremental tuning).
pub fn rit_line(hz: i32) -> String {
    format!("J {hz}\n")
}
/// rigctld `Z` — XIT offset in Hz (transmit incremental tuning).
pub fn xit_line(hz: i32) -> String {
    format!("Z {hz}\n")
}
/// rigctld `L` — set a level by name (e.g. `RFPOWER 0.5` 0..1, `KEYSPD 25` WPM).
pub fn level_line(name: &str, value: &str) -> String {
    format!("L {name} {value}\n")
}
/// rigctld `b` — send_morse: the rig keys CW from this text (rest of the line).
pub fn morse_line(text: &str) -> String {
    format!("b {text}\n")
}
/// Parse the S-meter reading (dB relative to S9) from a rigctld `l STRENGTH` reply.
/// Hamlib reports STRENGTH as a signed integer dB value where S9 = 0 dB (S1 ≈ -48 dB,
/// S9+20 = +20 dB) — NOT the 0.0–1.0 fraction `read_level` expects. Returns `None` when
/// the rig answered with no number (RPRT/empty) or an implausible out-of-range value.
pub fn parse_smeter_db(reply: &str) -> Option<i32> {
    reply
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("RPRT"))
        .find_map(|l| l.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.round() as i32)
        // Real STRENGTH spans ~S0 (-54 dB) to S9+60; allow margin for noise-floor reads,
        // reject only clearly-garbage magnitudes (e.g. an error sentinel that parsed).
        .filter(|db| (-80..=100).contains(db))
}

/// True if a rigctld reply indicates success (`RPRT 0`).
pub fn reply_ok(reply: &str) -> bool {
    reply.lines().any(|l| l.trim() == "RPRT 0")
}

/// Parse a rigctld `u <FUNC>` (get-function) reply. In the default protocol a SUCCESSFUL get
/// returns the value ONLY — `0` or `1` on its own line, with NO `RPRT` — while an error returns
/// `RPRT <negative>` (e.g. `-11` ENAVAIL = the rig doesn't have this func). So an `RPRT` line
/// means unavailable/errored → `None`; otherwise the first `0`/`1` → `Some(off/on)`.
pub fn parse_func_reply(reply: &str) -> Option<bool> {
    for l in reply.lines() {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        if l.starts_with("RPRT") {
            return None; // error / not available — the caller keeps the last known state
        }
        match l {
            "0" => return Some(false),
            "1" => return Some(true),
            _ => {}
        }
    }
    None
}

/// Parse a rigctld `m` (get_mode) reply into (mode, passband_hz): the mode name on one line and
/// the RX passband width (Hz) on the next. Either may be absent on a given read (a networked
/// chain can split the two lines). Ignores `RPRT`/blank lines; a 0 width (rig's "default filter")
/// is treated as no-value.
pub fn parse_mode_passband(reply: &str) -> (Option<String>, Option<u32>) {
    let mut mode = None;
    let mut passband = None;
    for l in reply
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("RPRT"))
    {
        if let Ok(hz) = l.parse::<u32>() {
            if passband.is_none() && hz > 0 {
                passband = Some(hz);
            }
        } else if mode.is_none() {
            mode = Some(l.to_string());
        }
    }
    (mode, passband)
}

/// A handle to the rig's keying + CAT tuning.
///
/// CAT control (freq/mode/split/RIT/power/morse) and PTT keying are SEPARATE concerns.
/// The `control` channel is present whenever a CAT rig is configured — independent of
/// how PTT is keyed — so a rig keyed by VOX or a serial line still receives freq/mode
/// commands (the WSJT-X model). `control == None` means no CAT (every CAT verb is a
/// quiet no-op); PTT then keys via [`PttMode`].
pub struct Rig {
    /// CAT control channel — rigctld `host:port` (e.g. `127.0.0.1:4532`). `Some` =
    /// a CAT rig is configured; drives all freq/mode/CAT verbs AND `PttMode::Cat`.
    control: Option<String>,
    /// Lazily-opened TCP connection to `control`.
    stream: Option<TcpStream>,
    /// How PTT is keyed (independent of `control`).
    ptt_mode: PttMode,
    /// Lazily-opened serial port for [`PttMode::Serial`] (feature `serial`).
    #[cfg(feature = "serial")]
    serial: Option<Box<dyn serialport::SerialPort>>,
    /// Last PTT state we commanded (also lets callers/tests observe keying).
    pub keyed: bool,
}

impl Rig {
    /// General constructor: an optional CAT control channel + a PTT method. This is
    /// the seam that decouples control from keying — pass `Some(addr)` for a CAT rig
    /// regardless of whether `ptt_mode` is `Cat`, `Vox`, or `Serial`.
    pub fn with_control(control: Option<String>, ptt_mode: PttMode) -> Self {
        Self {
            control,
            stream: None,
            ptt_mode,
            #[cfg(feature = "serial")]
            serial: None,
            keyed: false,
        }
    }
    /// No CAT control, no keying — rely on the rig's VOX.
    pub fn vox() -> Self {
        Self::with_control(None, PttMode::Vox)
    }
    /// A CAT rig keyed via CAT: control + PTT both over rigctld at `addr`.
    pub fn rigctld(addr: &str) -> Self {
        Self::with_control(Some(addr.to_string()), PttMode::Cat)
    }
    /// Serial-line PTT (RTS/DTR) with NO CAT control. For serial PTT alongside CAT,
    /// use [`with_control`](Self::with_control) and pass a control address.
    pub fn serial(port: &str, line: SerialLine) -> Self {
        Self::with_control(
            None,
            PttMode::Serial {
                port: port.to_string(),
                line,
            },
        )
    }

    fn ensure_connected(&mut self) -> std::io::Result<&mut TcpStream> {
        if self.stream.is_none() {
            if let Some(addr) = &self.control {
                let s = TcpStream::connect(addr)?;
                s.set_read_timeout(Some(Duration::from_millis(500)))?;
                s.set_write_timeout(Some(Duration::from_millis(500)))?;
                self.stream = Some(s);
            }
        }
        self.stream
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotConnected, "no rig stream"))
    }

    /// Send one command line and read its COMPLETE (newline-terminated) reply.
    ///
    /// On ANY failure — incomplete reply by the deadline, a closed connection,
    /// or a hard I/O error — the stream is dropped so the next command
    /// reconnects from a clean protocol state. This is a TX-safety invariant:
    /// if a slow reply were left in the socket buffer, the *next* command would
    /// read it as its own answer and every command after that would be judged on
    /// the previous one's reply (a keyed rig read as "PTT ok", a rejected T read
    /// as success). A dropped stream can never desync; a stale byte can.
    fn command(&mut self, line: &str) -> std::io::Result<String> {
        match self.command_inner(line) {
            Ok(reply) => Ok(reply),
            Err(e) => {
                self.stream = None; // force a clean reconnect on the next call
                Err(e)
            }
        }
    }

    fn command_inner(&mut self, line: &str) -> std::io::Result<String> {
        let stream = self.ensure_connected()?;
        // Discard any STALE bytes left in the socket by a prior MULTI-LINE reply — `m` (get_mode)
        // returns the mode line AND a passband line, and on a networked chain the 2nd line can
        // arrive AFTER we already returned the 1st. A lingering byte would be read as THIS
        // command's answer and desync every command after it (the exact hazard the drop-on-failure
        // guards, but a successful multi-line reply slips past that). Non-blocking → free when clean.
        stream.set_nonblocking(true)?;
        let mut scratch = [0u8; 256];
        while let Ok(n) = stream.read(&mut scratch) {
            if n == 0 {
                break; // peer closed — the real read below surfaces it
            }
        }
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?; // restore blocking-with-timeout
        stream.write_all(line.as_bytes())?;
        // Read until a COMPLETE reply (newline-terminated), not one 500 ms
        // gulp: a networked chain (rigctld → SmartSDR CAT → radio) can take
        // longer than one read window and can split a reply across reads.
        // The per-read timeout (500 ms) bounds each wait; a 2.5 s overall
        // deadline bounds the whole reply. An incomplete reply is an ERROR
        // (not a silently-truncated "") so callers never treat a partial or
        // timed-out answer as success — and `command` drops the stream.
        let deadline = std::time::Instant::now() + Duration::from_millis(2_500);
        let mut out = Vec::with_capacity(64);
        let mut buf = [0u8; 256];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "rigctld closed the connection",
                    ));
                }
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if out.ends_with(b"\n") {
                        return Ok(String::from_utf8_lossy(&out).to_string());
                    }
                }
                Err(ref e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {} // per-read timeout tick — keep waiting to the deadline
                Err(e) => return Err(e), // hard error — caller drops the stream
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "rig reply incomplete after 2.5 s (got {:?})",
                        String::from_utf8_lossy(&out)
                    ),
                ));
            }
        }
    }

    /// Key (true) or unkey (false) the transmitter. No-op under VOX (and under CAT
    /// keying when no control channel is configured — degrades to VOX).
    pub fn ptt(&mut self, on: bool) -> std::io::Result<()> {
        self.keyed = on;
        match &self.ptt_mode {
            PttMode::Vox => Ok(()),
            PttMode::Serial { .. } => self.serial_ptt(on),
            PttMode::Cat => {
                if self.control.is_none() {
                    return Ok(()); // CAT keying chosen but no CAT channel → VOX fallback
                }
                let reply = self.command(&ptt_line(on))?;
                if reply_ok(&reply) || reply.is_empty() {
                    Ok(())
                } else {
                    Err(std::io::Error::other(format!(
                        "rigctld PTT error: {reply:?}"
                    )))
                }
            }
        }
    }

    /// Assert/deassert the configured serial PTT line.
    ///
    /// With the `serial` feature this lazily opens the port and drives RTS/DTR;
    /// without it, keying is logged and treated as a no-op so the engine can
    /// still run (effectively VOX) on a build with no serial backend.
    #[cfg(feature = "serial")]
    fn serial_ptt(&mut self, on: bool) -> std::io::Result<()> {
        let (port, line) = match &self.ptt_mode {
            PttMode::Serial { port, line } => (port.clone(), *line),
            _ => return Ok(()),
        };
        if self.serial.is_none() {
            // 1200 baud is arbitrary — we only toggle control lines, not data.
            let opened = serialport::new(&port, 1200)
                .timeout(Duration::from_millis(200))
                .open()?;
            self.serial = Some(opened);
        }
        let sp = self.serial.as_mut().unwrap();
        match line {
            SerialLine::Rts => sp.write_request_to_send(on)?,
            SerialLine::Dtr => sp.write_data_terminal_ready(on)?,
        }
        Ok(())
    }

    /// Serial PTT no-op fallback when built without the `serial` feature.
    #[cfg(not(feature = "serial"))]
    fn serial_ptt(&mut self, on: bool) -> std::io::Result<()> {
        if let PttMode::Serial { port, line } = &self.ptt_mode {
            eprintln!(
                "tempo-audio: serial PTT requested ({line:?} on {port}, key={on}) but the \
                 `serial` feature is not enabled — treating as VOX (no-op)."
            );
        }
        Ok(())
    }

    /// Set the dial frequency (Hz). No-op unless a CAT control channel is configured.
    pub fn set_freq(&mut self, hz: u64) -> std::io::Result<()> {
        if self.control.is_none() {
            return Ok(());
        }
        self.command(&freq_line(hz)).map(|_| ())
    }

    /// Set the operating mode (e.g. "USB") + passband. A BLANK mode is a no-op —
    /// the caller is choosing to OBEY the radio's current mode (max compatibility),
    /// so Nexus sends no `M` command. Also a no-op unless a CAT control channel is
    /// configured (works even when PTT is keyed by VOX/serial — control is separate).
    /// Surfaces a rig REJECTION (`RPRT -1`, e.g. a rig with no DATA/PKT submode) as
    /// an `Err`, so the radio loop's bounded retry can give up instead of looping.
    pub fn set_mode(&mut self, mode: &str, passband_hz: u32) -> std::io::Result<()> {
        if mode.trim().is_empty() {
            return Ok(());
        }
        if self.control.is_none() {
            return Ok(());
        }
        let reply = self.command(&mode_line(mode, passband_hz))?;
        if reply_ok(&reply) || reply.is_empty() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "rigctld mode error: {reply:?}"
            )))
        }
    }

    /// Apply FM repeater settings: shift direction (`R`), offset magnitude (`O`), and
    /// CTCSS tone (`C`). Best-effort — a rig that supports shift but not CTCSS (or has no
    /// repeater support) rejects individual commands harmlessly, so each is sent and its
    /// per-command error swallowed. No-op without a CAT control channel; `tone_hz` 0
    /// disables CTCSS. Call after a successful FM `set_mode` (the connection is live).
    pub fn set_fm_repeater(
        &mut self,
        shift: &str,
        offset_hz: i64,
        tone_hz: f32,
    ) -> std::io::Result<()> {
        if self.control.is_none() {
            return Ok(());
        }
        let _ = self.command(&rptr_shift_line(shift));
        if offset_hz > 0 {
            let _ = self.command(&rptr_offset_line(offset_hz));
        }
        let _ = self.command(&ctcss_line(tone_hz));
        Ok(())
    }

    /// Whether a CAT control channel is configured (so the freq/mode/CAT verbs are
    /// live). Lets callers distinguish "no CAT" from "CAT present but the rig is mute".
    pub fn has_control(&self) -> bool {
        self.control.is_some()
    }

    /// Probe the rig by reading its current dial frequency (Hz) over CAT — the
    /// basis of a WSJT-X-style "Test CAT". Connects to rigctld and sends `f`,
    /// which replies with the frequency on its own line. Returns a descriptive
    /// error when rigctld is unreachable (connection refused) or the rig itself
    /// doesn't answer (bad baud / serial port / CAT disabled → no numeric reply).
    /// Only valid with a CAT control channel.
    /// Read a rig LEVEL (e.g. "RFPOWER" → 0.0–1.0) via rigctld `l NAME`.
    /// CAT-only; errors on FakeIt/none like `read_freq`.
    pub fn read_level(&mut self, name: &str) -> std::io::Result<f32> {
        if self.control.is_none() {
            return Err(std::io::Error::other("not a CAT rig"));
        }
        let reply = self.command(&format!("l {name}\n"))?;
        reply
            .lines()
            .find_map(|l| l.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .ok_or_else(|| std::io::Error::other("no level in reply"))
    }

    /// Read the rig's S-meter (dB relative to S9) via rigctld `l STRENGTH`. CAT-only;
    /// `None` on VOX/serial or no numeric reply. Unlike [`Rig::read_level`], STRENGTH is a
    /// signed dB value, not a 0.0–1.0 fraction (parsing/bounds live in [`parse_smeter_db`]).
    pub fn read_smeter_db(&mut self) -> Option<i32> {
        self.control.as_ref()?;
        let reply = self.command("l STRENGTH\n").ok()?;
        parse_smeter_db(&reply)
    }

    /// Read a rig CAT function state (e.g. "NB", "NR", "ANF", "COMP", "VOX") via rigctld
    /// `u FUNC`. CAT-only; `None` on VOX/serial, an unsupported func, or a link hiccup — the
    /// caller keeps the last known state rather than flickering the toggle.
    pub fn read_func(&mut self, token: &str) -> Option<bool> {
        self.control.as_ref()?;
        let reply = self.command(&format!("u {token}\n")).ok()?;
        parse_func_reply(&reply)
    }

    /// Enable/disable a rig CAT function via rigctld `U FUNC <0|1>`. CAT-only; `Ok(())` on
    /// `RPRT 0`, else an error (unsupported func or link failure).
    pub fn set_func(&mut self, token: &str, on: bool) -> std::io::Result<()> {
        if self.control.is_none() {
            return Err(std::io::Error::other("not a CAT rig"));
        }
        let reply = self.command(&format!("U {token} {}\n", u8::from(on)))?;
        if reply_ok(&reply) {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "set_func {token} rejected: {reply:?}"
            )))
        }
    }

    pub fn read_freq(&mut self) -> std::io::Result<u64> {
        if self.control.is_none() {
            return Err(std::io::Error::other("not a CAT rig"));
        }
        let reply = self.command("f\n")?;
        reply
            .lines()
            .find_map(|l| l.trim().parse::<u64>().ok())
            .filter(|hz| *hz > 0)
            .ok_or_else(|| {
                std::io::Error::other(format!(
                    "rig did not return a frequency (reply {reply:?}) — check the serial port, \
                     baud rate, and that CAT/CI-V is enabled on the rig"
                ))
            })
    }

    /// Read the rig's current mode (e.g. "USB"/"CW"). `None` if not a CAT rig or the
    /// rig didn't answer. The `m` reply is the mode on one line, passband on the next.
    pub fn read_mode(&mut self) -> Option<String> {
        self.read_mode_passband().0
    }

    /// Read the rig's mode + RX passband (Hz) from ONE `m` reply. CAT-only. The passband is
    /// opportunistic: present when both reply lines arrive in one read (the common path); a
    /// networked chain that splits them just surfaces the width on a later poll (the pre-command
    /// drain flushes the stray line so it never poisons the next command).
    pub fn read_mode_passband(&mut self) -> (Option<String>, Option<u32>) {
        if self.control.is_none() {
            return (None, None);
        }
        match self.command("m\n") {
            Ok(reply) => parse_mode_passband(&reply),
            Err(_) => (None, None),
        }
    }

    /// Set the RX passband width (Hz) by re-issuing the current mode with the new width — Hamlib
    /// carries filter width as the 2nd arg of set_mode (there is no portable bandwidth level).
    /// CAT-only. The caller passes the current mode (the loop tracks it).
    pub fn set_passband(&mut self, mode: &str, hz: u32) -> std::io::Result<()> {
        self.set_mode(mode, hz)
    }

    /// Send a RAW CAT command string straight to the rig via rigctld's `w` (send_cmd) and
    /// return the rig's reply. This BYPASSES Hamlib's mode abstraction AND its mode cache —
    /// `read_mode` (the `m` command) can return the mode Hamlib *thinks* it set even when the
    /// rig never moved, whereas e.g. raw Yaesu `MD0;` returns the rig's TRUE current mode code
    /// off the wire. Diagnostic-only; `None` if not a CAT rig or no reply.
    pub fn send_raw(&mut self, raw: &str) -> Option<String> {
        self.control.as_ref()?;
        let reply = self.command(&format!("w {raw}\n")).ok()?;
        let trimmed = reply.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    // --- the all-mode (phone/CW) control surface. All CAT-only (no-op otherwise). ---

    /// Set split on/off and which VFO transmits (DX pileups). `tx_vfo` e.g. "VFOB".
    pub fn set_split(&mut self, on: bool, tx_vfo: &str) -> std::io::Result<()> {
        self.cat(&split_line(on, tx_vfo))
    }
    /// Set the split (TX) frequency in Hz.
    pub fn set_split_freq(&mut self, hz: u64) -> std::io::Result<()> {
        self.cat(&split_freq_line(hz))
    }
    /// Select the active VFO (e.g. "VFOA"/"VFOB").
    pub fn set_vfo(&mut self, vfo: &str) -> std::io::Result<()> {
        self.cat(&vfo_line(vfo))
    }
    /// Set RIT (receive incremental tuning) offset in Hz; enabling RIT first (0 = off).
    pub fn set_rit(&mut self, hz: i32) -> std::io::Result<()> {
        self.cat(&func_line("RIT", hz != 0))?;
        self.cat(&rit_line(hz))
    }
    /// Set XIT (transmit incremental tuning) offset in Hz (0 = off).
    pub fn set_xit(&mut self, hz: i32) -> std::io::Result<()> {
        self.cat(&func_line("XIT", hz != 0))?;
        self.cat(&xit_line(hz))
    }
    /// Set RF output power as a 0.0–1.0 fraction (Hamlib `RFPOWER`).
    pub fn set_power(&mut self, frac: f32) -> std::io::Result<()> {
        self.cat(&level_line(
            "RFPOWER",
            &format!("{:.3}", frac.clamp(0.0, 1.0)),
        ))
    }
    /// Set the rig's internal CW keyer speed in WPM (Hamlib `KEYSPD`).
    pub fn set_keyspd(&mut self, wpm: u32) -> std::io::Result<()> {
        self.cat(&level_line("KEYSPD", &wpm.to_string()))
    }
    /// Key CW from text via the rig's own keyer (Hamlib `send_morse`). Set the speed
    /// first with [`set_keyspd`](Self::set_keyspd). Best for canned/keyboard macros;
    /// CAT latency makes it poor for live paddle feel (use WinKeyer for that).
    pub fn send_morse(&mut self, text: &str) -> std::io::Result<()> {
        self.cat(&morse_line(text))
    }
    /// Abort CW in progress. Newer Hamlib exposes `\stop_morse`; older builds vary by
    /// manufacturer (the WinKeyer path has a reliable Clear-Buffer abort instead).
    pub fn stop_morse(&mut self) -> std::io::Result<()> {
        self.cat("\\stop_morse\n")
    }

    /// Send a rigctld command, succeeding on `RPRT 0` (or an empty reply); no-op when
    /// no CAT control channel is configured. Shared by the all-mode control verbs above.
    fn cat(&mut self, line: &str) -> std::io::Result<()> {
        if self.control.is_none() {
            return Ok(());
        }
        let reply = self.command(line)?;
        if reply_ok(&reply) || reply.is_empty() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "rigctld error for {line:?}: {reply:?}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_match_rigctld_protocol() {
        assert_eq!(ptt_line(true), "T 1\n");
        assert_eq!(ptt_line(false), "T 0\n");
        assert_eq!(freq_line(14_074_000), "F 14074000\n");
        assert_eq!(mode_line("USB", 0), "M USB 0\n");
        assert_eq!(mode_line("FM", 0), "M FM 0\n");
        assert!(reply_ok("RPRT 0\n"));
        assert!(!reply_ok("RPRT -1\n"));
    }

    #[test]
    fn fm_repeater_lines_match_rigctld_protocol() {
        assert_eq!(rptr_shift_line("plus"), "R +\n");
        assert_eq!(rptr_shift_line("minus"), "R -\n");
        assert_eq!(rptr_shift_line("simplex"), "R None\n");
        assert_eq!(rptr_offset_line(600_000), "O 600000\n");
        assert_eq!(ctcss_line(100.0), "C 1000\n"); // Hamlib wants tenths of Hz
        assert_eq!(ctcss_line(0.0), "C 0\n"); // off
        assert_eq!(ctcss_line(88.5), "C 885\n");
    }

    #[test]
    fn smeter_strength_parses_db_relative_to_s9() {
        assert_eq!(parse_smeter_db("-54\n"), Some(-54)); // ~S0, no signal
        assert_eq!(parse_smeter_db("-36\n"), Some(-36)); // ~S3
        assert_eq!(parse_smeter_db("0\n"), Some(0)); // S9
        assert_eq!(parse_smeter_db("20\n"), Some(20)); // S9+20
        assert_eq!(parse_smeter_db("-5.0\n"), Some(-5)); // float form rounds to int dB
        assert_eq!(parse_smeter_db("RPRT -1\n"), None); // error reply is not a reading
        assert_eq!(parse_smeter_db(""), None); // rig didn't answer
        assert_eq!(parse_smeter_db("9999\n"), None); // garbage magnitude → rejected
    }

    #[test]
    fn func_get_reply_branches_on_value_vs_rprt() {
        // Default protocol: a successful get is value-only (no RPRT); an error is RPRT<negative>.
        assert_eq!(parse_func_reply("1\n"), Some(true));
        assert_eq!(parse_func_reply("0\n"), Some(false));
        assert_eq!(parse_func_reply("RPRT -11\n"), None); // ENAVAIL — rig lacks the func
        assert_eq!(parse_func_reply("RPRT -5\n"), None); // transient — caller keeps last state
        assert_eq!(parse_func_reply(""), None); // no answer
    }

    #[test]
    fn mode_passband_parse_splits_the_m_reply() {
        assert_eq!(parse_mode_passband("USB\n2400\n"), (Some("USB".into()), Some(2400)));
        assert_eq!(parse_mode_passband("CW\n500\n"), (Some("CW".into()), Some(500)));
        assert_eq!(parse_mode_passband("USB\n"), (Some("USB".into()), None)); // split → width later
        assert_eq!(parse_mode_passband("USB\n0\n"), (Some("USB".into()), None)); // 0 = rig default
        assert_eq!(parse_mode_passband("RPRT -1\n"), (None, None));
    }

    #[test]
    fn all_mode_control_lines_match_rigctld_protocol() {
        assert_eq!(split_line(true, "VFOB"), "S 1 VFOB\n");
        assert_eq!(split_line(false, "VFOA"), "S 0 VFOA\n");
        assert_eq!(split_freq_line(14_205_000), "I 14205000\n");
        assert_eq!(vfo_line("VFOA"), "V VFOA\n");
        assert_eq!(func_line("RIT", true), "U RIT 1\n");
        assert_eq!(func_line("XIT", false), "U XIT 0\n");
        assert_eq!(rit_line(-200), "J -200\n");
        assert_eq!(xit_line(500), "Z 500\n");
        assert_eq!(level_line("RFPOWER", "0.500"), "L RFPOWER 0.500\n");
        assert_eq!(level_line("KEYSPD", "25"), "L KEYSPD 25\n");
        // send_morse takes the rest of the line as the CW text (spaces preserved).
        assert_eq!(morse_line("CQ CQ DE W9XYZ"), "b CQ CQ DE W9XYZ\n");
    }

    #[test]
    fn all_mode_control_is_a_no_op_under_vox() {
        // Every new verb is CAT-only — under VOX they must not attempt a connection.
        let mut rig = Rig::vox();
        rig.set_split(true, "VFOB").unwrap();
        rig.set_split_freq(14_205_000).unwrap();
        rig.set_vfo("VFOA").unwrap();
        rig.set_rit(-200).unwrap();
        rig.set_xit(0).unwrap();
        rig.set_power(0.5).unwrap();
        rig.set_keyspd(25).unwrap();
        rig.send_morse("TEST").unwrap();
        rig.stop_morse().unwrap();
        assert_eq!(rig.read_mode(), None);
    }

    #[test]
    fn vox_mode_keys_without_a_socket() {
        let mut rig = Rig::vox();
        rig.ptt(true).unwrap();
        assert!(rig.keyed);
        rig.ptt(false).unwrap();
        assert!(!rig.keyed);
        // freq/mode are also no-ops under VOX (no connection attempted).
        rig.set_freq(14_074_000).unwrap();
        rig.set_mode("USB", 0).unwrap();
    }

    // Without the `serial` feature, Serial PTT must fall back to a no-op (like
    // VOX) so the engine can run with no serial backend and no real port.
    #[cfg(not(feature = "serial"))]
    #[test]
    fn serial_mode_falls_back_to_vox_without_a_port() {
        let mut rig = Rig::serial("COM_DOES_NOT_EXIST", SerialLine::Rts);
        rig.ptt(true).unwrap();
        assert!(rig.keyed);
        rig.ptt(false).unwrap();
        assert!(!rig.keyed);
        // freq/mode are no-ops outside rigctld CAT — no connection attempted.
        rig.set_freq(14_074_000).unwrap();
        rig.set_mode("USB", 0).unwrap();
    }

    #[test]
    fn serial_constructor_sets_mode() {
        let rig = Rig::serial("COM5", SerialLine::Dtr);
        assert!(matches!(
            rig.ptt_mode,
            PttMode::Serial { ref port, line: SerialLine::Dtr } if port == "COM5"
        ));
        assert!(rig.control.is_none(), "serial PTT alone has no CAT control");
        assert!(!rig.keyed);
    }

    // ---- Mock-rigctld round-trip harness (no hardware, runs in CI) ----------
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A throwaway mock rigctld: binds an ephemeral port, accepts one connection,
    /// replies to each command line via `reply`, and records every command for
    /// assertions. Models the rigctl line protocol (`f`→freq, `F`/`M`/`T`→RPRT).
    fn mock_rigctld(
        reply: impl Fn(&str) -> String + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let log_w = log.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 256];
                loop {
                    let n = match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    for line in text.lines() {
                        log_w.lock().unwrap().push(line.to_string());
                        if stream.write_all(reply(line).as_bytes()).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        (addr, log)
    }

    /// Healthy rig: `f`→`freq`, everything else→`RPRT 0`.
    fn ok_reply(freq: u64) -> impl Fn(&str) -> String + Send + 'static {
        move |line: &str| {
            if line.starts_with('f') {
                format!("{freq}\n")
            } else {
                "RPRT 0\n".to_string()
            }
        }
    }

    #[test]
    fn slow_fragmented_reply_still_reads_whole_line() {
        // SmartSDR CAT chains answer slower than a local rigctld and TCP can
        // fragment: 700 ms in, byte-at-a-time. The old single-500ms-read
        // returned "" here (operator report: green connect, dead control).
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            use std::io::Read as _;
            let _ = sock.read(&mut buf); // consume the "f\n"
            std::thread::sleep(std::time::Duration::from_millis(700));
            for b in b"14074000\n" {
                let _ = sock.write_all(&[*b]);
                let _ = sock.flush();
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        });
        let mut rig = Rig::rigctld(&addr.to_string());
        assert_eq!(rig.read_freq().expect("whole reply assembled"), 14_074_000);
    }

    #[test]
    fn timed_out_reply_errors_and_drops_stream_so_the_next_command_cannot_desync() {
        // The C3 desync: a reply that lands after the 2.5 s deadline must NOT be
        // left in the socket for the next command to read as its own answer.
        // Command 1's reply arrives late (past the deadline) → the command must
        // error AND drop the stream; command 2 reconnects and reads its OWN
        // fresh reply, never the stale one.
        use std::io::{Read as _, Write as _};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let n = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let mut sock = match conn {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let idx = n.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let mut buf = [0u8; 64];
                    let _ = sock.read(&mut buf); // consume the command line
                    if idx == 0 {
                        // Reply LATE — past the 2.5 s deadline. The client has
                        // already abandoned this command; this write is stale.
                        std::thread::sleep(std::time::Duration::from_millis(2_800));
                        let _ = sock.write_all(b"RPRT 0\n");
                    } else {
                        let _ = sock.write_all(b"14074000\n"); // fresh, immediate
                    }
                });
            }
        });
        let mut rig = Rig::rigctld(&addr.to_string());
        // Slow reply must be an ERROR, never a silent success.
        assert!(
            rig.set_mode("USB", 0).is_err(),
            "reply past the 2.5 s deadline must error, not succeed"
        );
        // Stream was dropped; the next command reconnects and reads its OWN reply.
        assert_eq!(
            rig.read_freq().expect("reconnect + fresh reply"),
            14_074_000,
            "next command must read its own reply, not the stale RPRT 0"
        );
    }

    #[test]
    fn read_freq_parses_the_dial_over_tcp() {
        let (addr, log) = mock_rigctld(ok_reply(14_074_000));
        let mut rig = Rig::rigctld(&addr);
        assert_eq!(rig.read_freq().unwrap(), 14_074_000);
        assert_eq!(log.lock().unwrap().as_slice(), &["f".to_string()]);
    }

    #[test]
    fn set_freq_mode_ptt_send_correct_lines() {
        let (addr, log) = mock_rigctld(ok_reply(7_074_000));
        let mut rig = Rig::rigctld(&addr);
        rig.set_freq(7_074_000).unwrap();
        rig.set_mode("USB", 0).unwrap();
        rig.ptt(true).unwrap();
        assert!(rig.keyed);
        rig.ptt(false).unwrap();
        assert!(!rig.keyed);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["F 7074000", "M USB 0", "T 1", "T 0"]
        );
    }

    #[test]
    fn cat_control_works_with_vox_ptt() {
        // The keystone decoupling (WSJT-X model): a CAT rig keyed by VOX still receives
        // freq + mode commands over the control channel — but PTT no-ops (VOX keys it).
        // This is exactly the case that was silently broken: CAT rig + non-CAT PTT meant
        // the rig never got an M/F command, so the mode never switched per section.
        let (addr, log) = mock_rigctld(ok_reply(14_074_000));
        let mut rig = Rig::with_control(Some(addr), PttMode::Vox);
        rig.set_freq(14_074_000).unwrap();
        rig.set_mode("PKTUSB", 0).unwrap();
        rig.ptt(true).unwrap(); // VOX → no T command sent, but state is tracked
        assert!(rig.keyed);
        rig.ptt(false).unwrap();
        assert!(!rig.keyed);
        assert_eq!(*log.lock().unwrap(), vec!["F 14074000", "M PKTUSB 0"]);
    }

    #[test]
    fn cat_ptt_without_a_control_channel_degrades_to_vox() {
        // PttMode::Cat but no control configured must not panic or attempt a connection —
        // it degrades to VOX (no-op keying) so the engine still runs.
        let mut rig = Rig::with_control(None, PttMode::Cat);
        rig.ptt(true).unwrap();
        assert!(rig.keyed);
        rig.ptt(false).unwrap();
        assert!(!rig.keyed);
    }

    #[test]
    fn set_mode_errors_when_rig_rejects_the_mode() {
        // A rig with no DATA/PKT submode replies RPRT -1 to `M PKTUSB` — set_mode must
        // surface that as Err so the radio loop's bounded retry can give up (not loop
        // an `M` command every tick). A mode the rig accepts still returns Ok.
        let (addr, _log) = mock_rigctld(|l| {
            if l.starts_with('M') {
                "RPRT -1\n".to_string()
            } else {
                "RPRT 0\n".to_string()
            }
        });
        let mut rig = Rig::rigctld(&addr);
        assert!(rig.set_mode("PKTUSB", 0).is_err());

        let (addr2, _l2) = mock_rigctld(ok_reply(14_074_000));
        let mut rig2 = Rig::rigctld(&addr2);
        assert!(rig2.set_mode("USB", 0).is_ok());
    }

    #[test]
    fn ptt_errors_when_rig_reports_failure() {
        // rigctld answers RPRT -1 (e.g. CAT not ready) → ptt must surface an error.
        let (addr, _log) = mock_rigctld(|_l| "RPRT -1\n".to_string());
        let mut rig = Rig::rigctld(&addr);
        assert!(rig.ptt(true).is_err());
    }

    #[test]
    fn read_freq_errors_on_non_numeric_reply() {
        let (addr, _log) = mock_rigctld(|_l| "RPRT -1\n".to_string());
        let mut rig = Rig::rigctld(&addr);
        assert!(rig.read_freq().is_err());
    }

    #[test]
    fn read_freq_errors_when_rigctld_unreachable() {
        // Grab then drop a port so nothing is listening → connection refused.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        drop(l);
        let mut rig = Rig::rigctld(&addr);
        assert!(rig.read_freq().is_err());
    }
}
