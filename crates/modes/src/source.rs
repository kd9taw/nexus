//! The user-selectable [`SignalSource`] — the "native engine vs companion" switch.
//!
//! Per the product design, the operator chooses where decodes come from:
//! - [`NativeSource`] runs a [`Mode`] over locally captured audio (Nexus *is*
//!   the decoder), or
//! - [`WsjtxUdpSource`] consumes the decode stream of an upstream
//!   WSJT-X/JTDX/MSHV over UDP (Nexus is a companion).
//!
//! Both implement [`SignalSource`], so the engine drives whichever is active at
//! each slot boundary through one uniform call and the rest of the app is blind
//! to the choice.

use std::collections::VecDeque;
use std::net::{ToSocketAddrs, UdpSocket};

use crate::decode::Decode;
use crate::mode::{make_mode, Mode, ModeKind};

/// Inputs for one decode interval (slot). Native sources decode `iwave`; passive
/// sources (UDP) ignore the audio and drain queued network decodes.
pub struct DecodeRequest<'a> {
    /// Captured int16 audio @ 12 kHz (≥ the active mode's frame size).
    pub iwave: &'a [i16],
    /// Audio search band edges (Hz).
    pub nfa: i32,
    pub nfb: i32,
    /// Decode aggressiveness (≤ 0 ⇒ mode default).
    pub ndepth: i32,
    /// Callsigns for a-priori decoding (`""` if unknown).
    pub mycall: &'a str,
    pub hiscall: &'a str,
    /// QSO progress index (AP pass schedule).
    pub nqso_progress: i32,
    /// QSO/RX audio frequency (Hz) being worked — WSJT-X's nfqso; centers the
    /// deep AP passes + sync for FT8/FT4. 0 / out-of-band ⇒ band center.
    pub nfqso: i32,
    /// Monotonic ms timestamp for this frame (cross-frame IR-HARQ keying; FT1).
    /// 0 disables cross-frame combining.
    pub frame_time_ms: i64,
}

impl<'a> DecodeRequest<'a> {
    /// A plain full-band decode request over `iwave` with no AP / QSO context.
    pub fn full_band(iwave: &'a [i16]) -> Self {
        Self {
            iwave,
            nfa: 200,
            nfb: 2900,
            ndepth: 3,
            mycall: "",
            hiscall: "",
            nqso_progress: 0,
            nfqso: 0, // band center (no QSO freq)
            frame_time_ms: 0,
        }
    }
}

/// A source of [`Decode`]s, driven once per slot boundary by the engine.
pub trait SignalSource: Send {
    /// Human-readable label for the UI (e.g. `"Native (FT8)"`, `"WSJT-X UDP"`).
    fn label(&self) -> String;

    /// Mode identity of this source's decodes, if known. `None` for a UDP source
    /// (it carries whatever the upstream app is running).
    fn mode_kind(&self) -> Option<ModeKind>;

    /// Produce the decodes available for this interval.
    fn decode(&mut self, req: &DecodeRequest) -> Vec<Decode>;
}

/// Native decode: run the active [`Mode`] over locally captured audio.
pub struct NativeSource {
    mode: Box<dyn Mode>,
}

impl NativeSource {
    /// Wrap an explicit boxed mode.
    pub fn new(mode: Box<dyn Mode>) -> Self {
        Self { mode }
    }

    /// Build from a [`ModeKind`].
    pub fn from_kind(kind: ModeKind) -> Self {
        Self {
            mode: make_mode(kind),
        }
    }

    /// The active mode.
    pub fn mode(&self) -> &dyn Mode {
        self.mode.as_ref()
    }

    /// Switch modes at runtime (e.g. the user picks FT4 instead of FT8).
    pub fn set_mode(&mut self, mode: Box<dyn Mode>) {
        self.mode = mode;
    }
}

impl SignalSource for NativeSource {
    fn label(&self) -> String {
        format!("Native ({})", self.mode.name())
    }

    fn mode_kind(&self) -> Option<ModeKind> {
        Some(self.mode.kind())
    }

    fn decode(&mut self, req: &DecodeRequest) -> Vec<Decode> {
        let kind = self.mode.kind();
        let mut decs = self.mode.decode_frame(
            req.iwave,
            req.nfa,
            req.nfb,
            req.ndepth,
            req.mycall,
            req.hiscall,
            req.nqso_progress,
            req.nfqso,
            req.frame_time_ms,
        );
        // Tag each decode with the mode that produced it (the conversion can't
        // know; we do).
        for d in &mut decs {
            d.mode = Some(kind);
        }
        decs
    }
}

/// Map an upstream WSJT-X/JTDX/MSHV `Decode` mode field to a [`ModeKind`].
/// WSJT-X reports the mode as a single-character code in the Decode message
/// (`"~"` = FT8, `"+"` = FT4); some apps send the full name. Unknown modes
/// (FST4/JT65/Q65/…) map to `None`.
fn wsjtx_mode_to_kind(m: &str) -> Option<ModeKind> {
    match m.trim() {
        "~" => Some(ModeKind::Ft8),
        "+" => Some(ModeKind::Ft4),
        other if other.eq_ignore_ascii_case("FT8") => Some(ModeKind::Ft8),
        other if other.eq_ignore_ascii_case("FT4") => Some(ModeKind::Ft4),
        _ => None,
    }
}

/// Companion decode: ingest the decode stream of an upstream WSJT-X/JTDX/MSHV
/// over UDP. Inbound datagrams are parsed with
/// [`tempo_net::wsjtx::parse_inbound`]; `Decode` messages are mapped to the
/// unified [`Decode`] and queued, then drained each interval.
///
/// Either [`bind`](WsjtxUdpSource::bind) a non-blocking UDP socket (the real
/// path), or feed raw datagrams via
/// [`ingest_datagram`](WsjtxUdpSource::ingest_datagram) (tests / an external
/// receive loop).
pub struct WsjtxUdpSource {
    socket: Option<UdpSocket>,
    queue: VecDeque<Decode>,
}

impl WsjtxUdpSource {
    /// A queue-only source (no socket); feed it via [`ingest_datagram`].
    pub fn new() -> Self {
        Self {
            socket: None,
            queue: VecDeque::new(),
        }
    }

    /// Bind a non-blocking UDP socket on `addr` (e.g. `"127.0.0.1:2237"`), the
    /// default sink WSJT-X/JTDX/MSHV transmit their telemetry to.
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket: Some(socket),
            queue: VecDeque::new(),
        })
    }

    /// The bound local address, if a socket is bound (diagnostics / tests).
    pub fn local_addr(&self) -> Option<std::net::SocketAddr> {
        self.socket.as_ref().and_then(|s| s.local_addr().ok())
    }

    /// Parse one inbound datagram; if it is a WSJT-X `Decode`, map it to the
    /// unified [`Decode`] and queue it. Returns `true` if a decode was queued.
    pub fn ingest_datagram(&mut self, bytes: &[u8]) -> bool {
        match tempo_net::wsjtx::parse_inbound(bytes) {
            Some(tempo_net::wsjtx::Inbound::Decode {
                snr,
                delta_time,
                delta_freq,
                mode,
                message,
                low_confidence,
                ..
            }) => {
                self.queue.push_back(Decode {
                    message,
                    sync: 0.0,
                    snr,
                    // The upstream `Decode` reports dt already in the WSJT-X
                    // `xdt = t - 0.5` convention and freq as the audio offset.
                    dt: delta_time as f32,
                    freq: delta_freq as f32,
                    nap: 0,
                    qual: if low_confidence { 0.0 } else { 1.0 },
                    rv: None,
                    // Carry the upstream app's mode so the feed labels it truly,
                    // not as our selected tier.
                    mode: wsjtx_mode_to_kind(&mode),
                });
                true
            }
            _ => false,
        }
    }

    /// Drain all datagrams currently pending on the bound socket into the queue.
    fn drain_socket(&mut self) {
        // Collect first (immutable socket borrow), then parse (mutable self).
        let mut packets: Vec<Vec<u8>> = Vec::new();
        if let Some(sock) = &self.socket {
            let mut buf = [0u8; 4096];
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, _)) => packets.push(buf[..n].to_vec()),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }
        for p in packets {
            self.ingest_datagram(&p);
        }
    }
}

impl Default for WsjtxUdpSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalSource for WsjtxUdpSource {
    fn label(&self) -> String {
        "WSJT-X UDP".to_string()
    }

    fn mode_kind(&self) -> Option<ModeKind> {
        None
    }

    fn decode(&mut self, _req: &DecodeRequest) -> Vec<Decode> {
        // The audio in `_req` is irrelevant: decodes come from the network.
        self.drain_socket();
        self.queue.drain(..).collect()
    }
}
