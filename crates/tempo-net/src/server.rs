//! UDP transport for the WSJT-X-compatible protocol.
//!
//! [`WsjtxServer`] binds a local UDP socket and emits [`crate::wsjtx`] datagrams
//! to a configured target (the WSJT-X default is `127.0.0.1:2237`; a multicast
//! group also works for several consumers at once). It also polls the socket
//! non-blocking for inbound control datagrams (Reply / HaltTx / FreeText).
//!
//! All datagrams carry the sender id `"Tempo"`.

use crate::wsjtx::{self, Decode, Inbound, QsoLogged, Status};
use std::io;
use std::net::{SocketAddr, UdpSocket};

/// The sender id ("key") Tempo announces itself with.
pub const APP_ID: &str = "Tempo";

/// A bound UDP socket that speaks the WSJT-X protocol to a target address.
pub struct WsjtxServer {
    socket: UdpSocket,
    target: SocketAddr,
    id: String,
}

impl WsjtxServer {
    /// Bind a UDP socket at `bind` and send to `target`.
    ///
    /// - `bind` is typically `0.0.0.0:0` (ephemeral) for pure output, or a fixed
    ///   port if you also want to receive control datagrams at a known address.
    /// - `target` is where datagrams go; if it is a multicast group address the
    ///   socket is configured to allow multicast send.
    ///
    /// The socket is set non-blocking so [`WsjtxServer::poll`] never stalls the
    /// caller's loop.
    pub fn new(bind: SocketAddr, target: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        if target.ip().is_multicast() {
            // Permit sending to a multicast group (and loop it back locally so a
            // consumer on the same host still hears us).
            socket.set_multicast_loop_v4(true).ok();
            if let SocketAddr::V4(v4) = target {
                socket
                    .join_multicast_v4(v4.ip(), &std::net::Ipv4Addr::UNSPECIFIED)
                    .ok();
            }
        }
        Ok(Self {
            socket,
            target,
            id: APP_ID.to_string(),
        })
    }

    /// The local address the socket is bound to (useful in tests to learn the
    /// ephemeral port).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The configured target address.
    pub fn target(&self) -> SocketAddr {
        self.target
    }

    fn send(&self, bytes: &[u8]) -> io::Result<()> {
        self.socket.send_to(bytes, self.target).map(|_| ())
    }

    /// Send a Heartbeat. `version`/`revision` describe this Tempo build.
    pub fn send_heartbeat(&self, max_schema: u32, version: &str, revision: &str) -> io::Result<()> {
        self.send(&wsjtx::encode_heartbeat(
            &self.id, max_schema, version, revision,
        ))
    }

    /// Send a Status update.
    pub fn send_status(&self, status: &Status) -> io::Result<()> {
        self.send(&wsjtx::encode_status(&self.id, status))
    }

    /// Send a Decode (a heard signal).
    pub fn send_decode(&self, decode: &Decode) -> io::Result<()> {
        self.send(&wsjtx::encode_decode(&self.id, decode))
    }

    /// Send a QSOLogged record (a completed contact).
    pub fn send_qso_logged(&self, qso: &QsoLogged) -> io::Result<()> {
        self.send(&wsjtx::encode_qso_logged(&self.id, qso))
    }

    /// Send a Close (Tempo is shutting down).
    /// **Clear (type 3)** — tell consumers (JTAlert/GridTracker) we erased a
    /// decode window so they clear theirs: 0 = Band Activity, 1 = Rx Frequency,
    /// 2 = both.
    pub fn send_clear(&self, window: u8) -> io::Result<()> {
        self.send(&wsjtx::encode_clear(&self.id, window))
    }

    pub fn send_close(&self) -> io::Result<()> {
        self.send(&wsjtx::encode_close(&self.id))
    }

    /// Non-blocking poll for one inbound control datagram.
    ///
    /// Returns:
    /// - `Ok(Some(inbound))` when a recognized WSJT-X datagram arrived,
    /// - `Ok(None)` when nothing is waiting (`WouldBlock`) or the datagram was
    ///   not a valid WSJT-X frame (silently ignored),
    /// - `Err(_)` only on a genuine socket error.
    pub fn poll(&self) -> io::Result<Option<Inbound>> {
        let mut buf = [0u8; 4096];
        match self.socket.recv_from(&mut buf) {
            Ok((n, _from)) => Ok(wsjtx::parse_inbound(&buf[..n])),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qds::QdsWriter;
    use std::net::{IpAddr, Ipv4Addr};

    fn loopback(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    /// End-to-end over loopback: a server sends to a listener socket, which
    /// decodes the datagram. No real network, just 127.0.0.1.
    #[test]
    fn status_roundtrips_over_loopback() {
        let listener = UdpSocket::bind(loopback(0)).unwrap();
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let target = listener.local_addr().unwrap();

        let server = WsjtxServer::new(loopback(0), target).unwrap();
        let status = Status {
            dial_freq: 14_074_000,
            mode: "FT8",
            de_call: "KD9TAW",
            de_grid: "EN52",
            special_op: 3,
            tr_period: 15,
            config_name: "Default",
            ..Default::default()
        };
        server.send_status(&status).unwrap();

        let mut buf = [0u8; 4096];
        let (n, _) = listener.recv_from(&mut buf).unwrap();
        // It parses as a WSJT-X frame and is the Status type ("Other" since the
        // inbound parser doesn't model Status payloads — but header is valid).
        match wsjtx::parse_inbound(&buf[..n]).unwrap() {
            Inbound::Other { id, message_type } => {
                assert_eq!(id, APP_ID);
                assert_eq!(message_type, wsjtx::msg_type::STATUS);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn poll_returns_none_when_idle() {
        let server = WsjtxServer::new(loopback(0), loopback(2237)).unwrap();
        assert_eq!(server.poll().unwrap(), None);
    }

    #[test]
    fn server_receives_inbound_reply() {
        // The server binds a fixed-ish port; a "consumer" sends a Reply to it.
        let server = WsjtxServer::new(loopback(0), loopback(0)).unwrap();
        let server_addr = server.local_addr().unwrap();

        let consumer = UdpSocket::bind(loopback(0)).unwrap();
        // A correctly-framed Reply datagram, as a controlling app would send.
        let reply = {
            let mut w = QdsWriter::new();
            w.put_u32(wsjtx::MAGIC)
                .put_u32(wsjtx::SCHEMA)
                .put_u32(wsjtx::msg_type::REPLY)
                .put_utf8(Some("GridTracker"))
                .put_u32(1000) // time_ms
                .put_i32(-5) // snr
                .put_f64(0.1) // delta_time
                .put_u32(1500) // delta_freq
                .put_utf8(Some("FT8"))
                .put_utf8(Some("CQ W1AW FN31"))
                .put_bool(false) // low_confidence
                .put_u8(0); // modifiers
            w.into_bytes()
        };
        consumer.send_to(&reply, server_addr).unwrap();

        // Give the datagram a moment to arrive, polling non-blocking.
        let mut got = None;
        for _ in 0..100 {
            if let Some(inb) = server.poll().unwrap() {
                got = Some(inb);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        match got.expect("no inbound received") {
            Inbound::Reply {
                id,
                message,
                snr,
                delta_freq,
                ..
            } => {
                assert_eq!(id, "GridTracker");
                assert_eq!(message, "CQ W1AW FN31");
                assert_eq!(snr, -5);
                assert_eq!(delta_freq, 1500);
            }
            other => panic!("expected Reply, got {other:?}"),
        }
    }
}
