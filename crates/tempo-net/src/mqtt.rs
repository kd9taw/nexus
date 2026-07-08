//! Minimal MQTT 3.1.1 QoS-0 subscriber — a pure packet codec + a thin TCP
//! transport, no external crates. Built for the PSK Reporter MQTT firehose
//! (`mqtt.pskreporter.info`), but protocol-generic. The encoders + [`Decoder`]
//! framing + [`run_session`] driver are unit-tested over in-memory streams; only
//! [`subscribe`] (the socket connect + reconnect) is live-only.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// An incoming MQTT packet we care about (others are surfaced as `Other`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// CONNACK return code (0 = accepted).
    ConnAck(u8),
    SubAck,
    /// A QoS-0 application message: topic + raw payload bytes.
    Publish {
        topic: String,
        payload: Vec<u8>,
    },
    PingResp,
    /// Any other packet type (by MQTT type number) — ignored by a subscriber.
    Other(u8),
}

/// Encode the MQTT "remaining length" varint (1–4 bytes, 7 bits each).
fn encode_remlen(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
}

/// Decode a remaining-length varint at the start of `buf`. Returns `(value,
/// bytes_consumed)`, or `None` if more bytes are needed (or it's malformed > 4B).
fn decode_remlen(buf: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    let mut mult = 1usize;
    for (i, &b) in buf.iter().take(4).enumerate() {
        value += (b & 0x7f) as usize * mult;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
        mult *= 128;
    }
    None
}

/// Append a length-prefixed (u16 BE) UTF-8 string.
fn put_str(s: &str, out: &mut Vec<u8>) {
    let b = s.as_bytes();
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(b);
}

/// CONNECT with a clean session and no will/auth (PSK Reporter allows anonymous).
pub fn encode_connect(client_id: &str, keepalive_secs: u16) -> Vec<u8> {
    let mut vh = Vec::new();
    put_str("MQTT", &mut vh); // protocol name
    vh.push(0x04); // protocol level 4 (MQTT 3.1.1)
    vh.push(0x02); // connect flags: clean session
    vh.extend_from_slice(&keepalive_secs.to_be_bytes());
    put_str(client_id, &mut vh); // payload: client id
    let mut pkt = vec![0x10]; // CONNECT
    encode_remlen(vh.len(), &mut pkt);
    pkt.extend_from_slice(&vh);
    pkt
}

/// SUBSCRIBE to one or more topic filters at QoS 0.
pub fn encode_subscribe(packet_id: u16, topics: &[&str]) -> Vec<u8> {
    let mut vh = Vec::new();
    vh.extend_from_slice(&packet_id.to_be_bytes());
    for t in topics {
        put_str(t, &mut vh);
        vh.push(0x00); // requested QoS 0
    }
    let mut pkt = vec![0x82]; // SUBSCRIBE (flags 0x02 required)
    encode_remlen(vh.len(), &mut pkt);
    pkt.extend_from_slice(&vh);
    pkt
}

pub fn encode_pingreq() -> Vec<u8> {
    vec![0xC0, 0x00]
}

/// Frames complete MQTT packets out of an incrementally-fed byte stream.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }
    /// Pop the next complete packet, or `None` if more bytes are needed.
    pub fn next_packet(&mut self) -> Option<Packet> {
        if self.buf.is_empty() {
            return None;
        }
        let header = self.buf[0];
        let (rem_len, len_bytes) = decode_remlen(&self.buf[1..])?;
        let total = 1 + len_bytes + rem_len;
        if self.buf.len() < total {
            return None; // packet not fully arrived
        }
        let ptype = header >> 4;
        let body: Vec<u8> = self.buf[1 + len_bytes..total].to_vec();
        self.buf.drain(..total);
        Some(match ptype {
            2 => Packet::ConnAck(*body.get(1).unwrap_or(&0xff)), // [ack flags, return code]
            9 => Packet::SubAck,
            3 => {
                let qos = (header >> 1) & 0x03;
                if body.len() < 2 {
                    return Some(Packet::Other(ptype));
                }
                let tlen = u16::from_be_bytes([body[0], body[1]]) as usize;
                if body.len() < 2 + tlen {
                    return Some(Packet::Other(ptype));
                }
                let topic = String::from_utf8_lossy(&body[2..2 + tlen]).into_owned();
                // QoS>0 carries a 2-byte packet id before the payload; QoS 0 doesn't.
                let payload_start = 2 + tlen + if qos > 0 { 2 } else { 0 };
                let payload = body.get(payload_start..).unwrap_or(&[]).to_vec();
                Packet::Publish { topic, payload }
            }
            13 => Packet::PingResp,
            other => Packet::Other(other),
        })
    }
}

/// Drive an MQTT subscriber session over a connected duplex: CONNECT → on CONNACK
/// SUBSCRIBE → deliver each PUBLISH to `on_publish`, with keepalive PINGREQs.
/// Generic over Read/Write so it's unit-testable without a socket.
#[allow(clippy::too_many_arguments)]
fn run_session<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    client_id: &str,
    topics: &[&str],
    on_publish: &mut dyn FnMut(&str, &[u8]),
    stop: &AtomicBool,
    keepalive: Duration,
    connected: &AtomicBool,
) -> std::io::Result<()> {
    writer.write_all(&encode_connect(
        client_id,
        keepalive.as_secs().min(65535) as u16,
    ))?;
    let mut dec = Decoder::new();
    let mut subscribed = false;
    let mut last_ping = Instant::now();
    let mut buf = [0u8; 8192];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        // Keepalive: ping at half the negotiated interval.
        if last_ping.elapsed() >= keepalive / 2 {
            writer.write_all(&encode_pingreq())?;
            last_ping = Instant::now();
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(e) => return Err(e),
        };
        dec.feed(&buf[..n]);
        while let Some(pkt) = dec.next_packet() {
            match pkt {
                Packet::ConnAck(code) => {
                    if code != 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!("MQTT CONNACK refused (code {code})"),
                        ));
                    }
                    // The broker accepted us — the session is genuinely up. Surfaced
                    // so the UI can show "connected (quiet)" instead of an
                    // indistinguishable-from-broken "waiting" before the first message.
                    connected.store(true, Ordering::Relaxed);
                    if !subscribed {
                        writer.write_all(&encode_subscribe(1, topics))?;
                        subscribed = true;
                    }
                }
                Packet::Publish { topic, payload } => on_publish(&topic, &payload),
                _ => {}
            }
        }
    }
}

/// Connect to an MQTT broker `addr` ("host:port"), subscribe to `topics`, and
/// call `on_publish(topic, payload)` for each message, reconnecting with capped
/// exponential backoff until `stop`. The thin live-socket wrapper around
/// [`run_session`] (which holds the tested logic).
pub fn subscribe(
    addr: &str,
    client_id: &str,
    topics: &[&str],
    mut on_publish: impl FnMut(&str, &[u8]),
    stop: &AtomicBool,
    connected: &AtomicBool,
) {
    const BASE: Duration = Duration::from_secs(2);
    const MAX: Duration = Duration::from_secs(60);
    const KEEPALIVE: Duration = Duration::from_secs(60);
    let mut backoff = BASE;
    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        if let Some(stream) = connect(addr) {
            if let Ok(reader) = stream.try_clone() {
                let _ = run_session(
                    reader,
                    stream,
                    client_id,
                    topics,
                    &mut on_publish,
                    stop,
                    KEEPALIVE,
                    connected,
                );
                // Session over (broker drop / error / stop) — no longer connected.
                connected.store(false, Ordering::Relaxed);
            }
        }
        backoff = if started.elapsed() > Duration::from_secs(10) {
            BASE
        } else {
            (backoff * 2).min(MAX)
        };
        let steps = (backoff.as_millis() / 100).max(1);
        for _ in 0..steps {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn connect(addr: &str) -> Option<TcpStream> {
    let sa = addr.to_socket_addrs().ok()?.next()?;
    let stream = TcpStream::connect_timeout(&sa, Duration::from_secs(8)).ok()?;
    // A read timeout so the session loop can fire keepalives + observe `stop`.
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn remlen_round_trips_across_the_varint_boundaries() {
        for n in [0usize, 1, 127, 128, 16383, 16384, 2_097_151] {
            let mut out = Vec::new();
            encode_remlen(n, &mut out);
            let (v, used) = decode_remlen(&out).unwrap();
            assert_eq!(v, n);
            assert_eq!(used, out.len());
        }
        // 127 fits in one byte, 128 needs two (continuation bit).
        let mut one = Vec::new();
        encode_remlen(127, &mut one);
        assert_eq!(one, vec![0x7f]);
        let mut two = Vec::new();
        encode_remlen(128, &mut two);
        assert_eq!(two, vec![0x80, 0x01]);
    }

    #[test]
    fn encode_connect_and_subscribe_have_correct_headers() {
        let c = encode_connect("nexus", 60);
        assert_eq!(c[0], 0x10); // CONNECT
        assert_eq!(&c[2..8], b"\x00\x04MQTT"); // protocol name field
        assert_eq!(c[8], 0x04); // level 4
        assert_eq!(c[9], 0x02); // clean session
        let s = encode_subscribe(1, &["pskr/filter/v2/#"]);
        assert_eq!(s[0], 0x82); // SUBSCRIBE + flags
        assert_eq!(*s.last().unwrap(), 0x00); // trailing requested QoS 0
    }

    #[test]
    fn decoder_frames_a_publish_and_handles_partial_then_complete() {
        // Build a QoS-0 PUBLISH for topic "t/x" payload "hi".
        let mut pkt = vec![0x30]; // PUBLISH, QoS 0
        let mut body = Vec::new();
        put_str("t/x", &mut body);
        body.extend_from_slice(b"hi");
        encode_remlen(body.len(), &mut pkt);
        pkt.extend_from_slice(&body);

        let mut d = Decoder::new();
        d.feed(&pkt[..4]); // partial
        assert_eq!(d.next_packet(), None);
        d.feed(&pkt[4..]); // rest
        assert_eq!(
            d.next_packet(),
            Some(Packet::Publish {
                topic: "t/x".to_string(),
                payload: b"hi".to_vec()
            })
        );
        assert_eq!(d.next_packet(), None);
    }

    #[test]
    fn decoder_reads_connack_and_pingresp() {
        let mut d = Decoder::new();
        d.feed(&[0x20, 0x02, 0x00, 0x00]); // CONNACK accepted
        assert_eq!(d.next_packet(), Some(Packet::ConnAck(0)));
        d.feed(&[0xD0, 0x00]); // PINGRESP
        assert_eq!(d.next_packet(), Some(Packet::PingResp));
    }

    struct ScriptReader {
        chunks: VecDeque<Vec<u8>>,
    }
    impl Read for ScriptReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.chunks.pop_front() {
                Some(c) => {
                    let n = c.len().min(buf.len());
                    buf[..n].copy_from_slice(&c[..n]);
                    Ok(n)
                }
                None => Ok(0),
            }
        }
    }

    fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0x30];
        let mut body = Vec::new();
        put_str(topic, &mut body);
        body.extend_from_slice(payload);
        encode_remlen(body.len(), &mut pkt);
        pkt.extend_from_slice(&body);
        pkt
    }

    #[test]
    fn run_session_connects_subscribes_then_delivers_publishes() {
        let reader = ScriptReader {
            chunks: vec![
                vec![0x20, 0x02, 0x00, 0x00], // CONNACK accepted
                // Real mqtt.pskreporter.info v2 layout (verified against the
                // official broker): 11 segments, trailing fields are ADIF DXCC
                // numbers (291=USA, 339=Japan), NO frequency in the topic.
                publish_packet(
                    "pskr/filter/v2/20m/FT8/W1AW/JA1XYZ/FN31/PM95/291/339",
                    b"{}",
                ),
            ]
            .into(),
        };
        let mut writer: Vec<u8> = Vec::new();
        let stop = AtomicBool::new(false);
        let connected = AtomicBool::new(false);
        let mut topics_seen: Vec<String> = Vec::new();
        run_session(
            reader,
            &mut writer,
            "nexus",
            &["pskr/filter/v2/#"],
            &mut |topic, _payload| topics_seen.push(topic.to_string()),
            &stop,
            Duration::from_secs(60),
            &connected,
        )
        .unwrap();
        // The accepted CONNACK flips the session-up flag (the UI's "connected" state).
        assert!(connected.load(Ordering::Relaxed), "CONNACK 0 → connected");
        // CONNECT then SUBSCRIBE were written.
        assert_eq!(writer[0], 0x10, "CONNECT first");
        assert!(
            writer.windows(1).any(|w| w[0] == 0x82) || writer.contains(&0x82),
            "SUBSCRIBE sent"
        );
        // The PUBLISH was delivered.
        assert_eq!(topics_seen.len(), 1);
        assert!(topics_seen[0].starts_with("pskr/filter/v2/"));
    }

    #[test]
    fn run_session_errors_on_refused_connack() {
        let reader = ScriptReader {
            chunks: vec![vec![0x20, 0x02, 0x00, 0x05]].into(), // CONNACK code 5 (refused)
        };
        let mut writer: Vec<u8> = Vec::new();
        let stop = AtomicBool::new(false);
        let connected = AtomicBool::new(false);
        let err = run_session(
            reader,
            &mut writer,
            "nexus",
            &["x"],
            &mut |_, _| {},
            &stop,
            Duration::from_secs(60),
            &connected,
        );
        assert!(err.is_err(), "a refused CONNACK is an error");
        assert!(
            !connected.load(Ordering::Relaxed),
            "a refused CONNACK must NOT read as connected"
        );
    }
}
