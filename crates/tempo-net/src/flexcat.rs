//! FlexRadio SmartSDR TCP API client (port 4992) — the command/status channel used to create and
//! steer a *panadapter* display object so the radio streams its FFT to us (the actual FFT frames
//! arrive over VITA-49 UDP, parsed in [`crate::flexvita`]).
//!
//! The wire protocol is line-based ASCII:
//!   * the radio greets with `V<version>` then `H<handle-hex>`;
//!   * the client sends commands `C<seq>|<command>`;
//!   * the radio replies `R<seq>|<code-hex>|<response>` and pushes async `S<handle>|<body>` status
//!     and `M<code-hex>|<text>` messages.
//!
//! The PURE protocol layer (line + pan-status parsing, command encoding) is unit-tested here; the
//! socket orchestration ([`FlexCat`]) needs a real radio and is validated on-air.
//!
//! HONESTY NOTE: written to the published SmartSDR Ethernet API + the open-source FlexLib, and
//! unit-tested against synthetic frames — NOT yet confirmed against live hardware (no Flex on the
//! dev LAN). The UI gates the native panadapter behind this until an operator confirms it.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

/// The SmartSDR API TCP port.
pub const FLEX_API_PORT: u16 = 4992;

/// One decoded line from the radio's TCP stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlexMsg {
    /// `V<version>` — the API version greeting.
    Version(String),
    /// `H<handle-hex>` — our client handle.
    Handle(u32),
    /// `R<seq>|<code-hex>|<response>` — a reply to command `seq` (`code` 0 = success).
    Reply { seq: u32, code: u32, msg: String },
    /// `S<handle-hex>|<body>` — an async status update (e.g. a `display pan …` object changed).
    Status { handle: u32, body: String },
    /// `M<code-hex>|<text>` — an async human-readable message from the radio.
    Message { code: u32, text: String },
    /// Anything we don't recognize (kept for logging, never fatal).
    Unknown(String),
}

/// Parse one newline-stripped line from the SmartSDR TCP stream. Pure — no I/O.
pub fn parse_line(line: &str) -> FlexMsg {
    let line = line.trim_end_matches(['\r', '\n']);
    let Some((tag, rest)) = line.split_at_checked(1) else {
        return FlexMsg::Unknown(line.to_string());
    };
    match tag {
        "V" => FlexMsg::Version(rest.to_string()),
        "H" => u32::from_str_radix(rest.trim(), 16)
            .map(FlexMsg::Handle)
            .unwrap_or_else(|_| FlexMsg::Unknown(line.to_string())),
        "R" => {
            // R<seq>|<code-hex>|<response>
            let mut it = rest.splitn(3, '|');
            let seq = it.next().and_then(|s| s.trim().parse::<u32>().ok());
            let code = it
                .next()
                .and_then(|s| u32::from_str_radix(s.trim(), 16).ok());
            match (seq, code) {
                (Some(seq), Some(code)) => FlexMsg::Reply {
                    seq,
                    code,
                    msg: it.next().unwrap_or("").to_string(),
                },
                _ => FlexMsg::Unknown(line.to_string()),
            }
        }
        "S" => {
            // S<handle-hex>|<body>
            let (h, body) = rest.split_once('|').unwrap_or((rest, ""));
            u32::from_str_radix(h.trim(), 16)
                .map(|handle| FlexMsg::Status {
                    handle,
                    body: body.to_string(),
                })
                .unwrap_or_else(|_| FlexMsg::Unknown(line.to_string()))
        }
        "M" => {
            let (c, text) = rest.split_once('|').unwrap_or((rest, ""));
            let code = u32::from_str_radix(c.trim(), 16).unwrap_or(0);
            FlexMsg::Message {
                code,
                text: text.to_string(),
            }
        }
        _ => FlexMsg::Unknown(line.to_string()),
    }
}

/// The fields we care about from a `display pan 0x<id> …` status body: the panadapter's stream id
/// (which the UDP FFT packets are tagged with) plus its current center/bandwidth so we can label
/// the display. All optional — a status update carries only the keys that changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanStatus {
    /// Panadapter object id (`0x40000000`-style), present on the create reply / first status.
    pub pan_id: Option<u32>,
    /// The VITA stream id the FFT UDP packets carry (may differ from `pan_id`).
    pub stream_id: Option<u32>,
    pub center_mhz: Option<f64>,
    pub bandwidth_mhz: Option<f64>,
    pub x_pixels: Option<u32>,
}

/// Parse a `display pan 0x<id> key=value …` status body into the fields we track. Pure.
/// Returns `None` when the body isn't a `display pan` line.
pub fn parse_pan_status(body: &str) -> Option<PanStatus> {
    let rest = body.strip_prefix("display pan ")?;
    let mut it = rest.split_whitespace();
    let mut st = PanStatus::default();
    // First token after "display pan " is the object id (0x…), the rest are key=value.
    if let Some(first) = it.next() {
        st.pan_id = parse_hex_id(first);
    }
    for tok in it {
        let Some((k, v)) = tok.split_once('=') else {
            continue;
        };
        match k {
            "center" => st.center_mhz = v.parse().ok(),
            "bandwidth" => st.bandwidth_mhz = v.parse().ok(),
            "x_pixels" | "xpixels" => st.x_pixels = v.parse().ok(),
            "stream_id" | "client_handle" => {} // handled below via explicit keys if present
            _ => {}
        }
        if k == "stream_id" {
            st.stream_id = parse_hex_id(v);
        }
    }
    Some(st)
}

/// Parse a `0x…`-or-decimal object/stream id.
fn parse_hex_id(s: &str) -> Option<u32> {
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Encode a command line `C<seq>|<command>` (newline included).
pub fn encode_command(seq: u32, command: &str) -> String {
    format!("C{seq}|{command}\n")
}

/// A live TCP connection to a Flex's SmartSDR API. Spawns a reader thread that parses lines into a
/// channel; `command` sends a `C…` line and awaits the matching `R…` reply.
pub struct FlexCat {
    stream: TcpStream,
    rx: mpsc::Receiver<FlexMsg>,
    seq: u32,
}

impl FlexCat {
    /// Connect to `ip:4992` and start the reader thread. Reads the `V`/`H` greeting is left to the
    /// caller (they arrive as the first messages on the channel).
    pub fn connect(ip: &str) -> std::io::Result<FlexCat> {
        let stream = TcpStream::connect((ip, FLEX_API_PORT))?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        let (tx, rx) = mpsc::channel();
        let reader = stream.try_clone()?;
        std::thread::spawn(move || {
            let mut buf = BufReader::new(reader);
            let mut line = String::new();
            loop {
                line.clear();
                match buf.read_line(&mut line) {
                    Ok(0) => break, // peer closed
                    Ok(_) => {
                        if tx.send(parse_line(&line)).is_err() {
                            break; // consumer dropped
                        }
                    }
                    Err(ref e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(FlexCat { stream, rx, seq: 1 })
    }

    /// Receive the next parsed message (blocks up to `timeout`).
    pub fn recv(&self, timeout: Duration) -> Option<FlexMsg> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// Send a command and wait for its `R<seq>|…` reply (up to `timeout`). Returns the reply
    /// `(code, msg)` — `code == 0` is success. Intervening status/message frames are ignored here
    /// (a full client would fan them out; the panadapter path only needs the create/set replies).
    pub fn command(&mut self, command: &str, timeout: Duration) -> std::io::Result<(u32, String)> {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1).max(1);
        self.stream
            .write_all(encode_command(seq, command).as_bytes())?;
        let deadline = std::time::Instant::now() + timeout;
        while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
            if let Ok(FlexMsg::Reply { seq: rseq, code, msg }) = self.rx.recv_timeout(remaining) {
                if rseq == seq {
                    return Ok((code, msg));
                }
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "no reply from Flex",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_greeting_and_replies() {
        assert_eq!(parse_line("V1.4.0.0\n"), FlexMsg::Version("1.4.0.0".to_string()));
        assert_eq!(parse_line("H2ABC\n"), FlexMsg::Handle(0x2ABC));
        assert_eq!(
            parse_line("R3|0|0x40000000\n"),
            FlexMsg::Reply {
                seq: 3,
                code: 0,
                msg: "0x40000000".to_string()
            }
        );
        // A non-zero reply code (error) still parses.
        assert_eq!(
            parse_line("R7|50000015|bad\n"),
            FlexMsg::Reply {
                seq: 7,
                code: 0x5000_0015,
                msg: "bad".to_string()
            }
        );
    }

    #[test]
    fn parses_status_and_message_lines() {
        assert_eq!(
            parse_line("S2ABC|display pan 0x40000000 center=14.1\n"),
            FlexMsg::Status {
                handle: 0x2ABC,
                body: "display pan 0x40000000 center=14.1".to_string()
            }
        );
        assert!(matches!(parse_line("M10000000|hello\n"), FlexMsg::Message { .. }));
        assert!(matches!(parse_line("garbage"), FlexMsg::Unknown(_)));
    }

    #[test]
    fn parses_a_pan_status_body() {
        let st = parse_pan_status(
            "display pan 0x40000000 wnb=0 center=14.100 bandwidth=0.200 x_pixels=1200 stream_id=0x42000000",
        )
        .unwrap();
        assert_eq!(st.pan_id, Some(0x4000_0000));
        assert_eq!(st.stream_id, Some(0x4200_0000));
        assert_eq!(st.center_mhz, Some(14.1));
        assert_eq!(st.bandwidth_mhz, Some(0.2));
        assert_eq!(st.x_pixels, Some(1200));
        // Not a pan line → None.
        assert!(parse_pan_status("slice 0 in_use=1").is_none());
    }

    #[test]
    fn encodes_a_command() {
        assert_eq!(
            encode_command(5, "display pan create x=1200 y=512"),
            "C5|display pan create x=1200 y=512\n"
        );
    }
}
