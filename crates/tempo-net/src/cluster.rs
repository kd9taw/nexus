//! DX-cluster / RBN connector: a pure session/parse core + a thin telnet
//! transport. [`parse_dx_spot`] decodes a `DX de …` line; [`ClusterSession`]
//! drives the login handshake + spot extraction over an incremental byte stream;
//! [`run`] is the TCP/reconnect glue; [`SpotBuffer`] holds recent spots for the
//! need-scorer. The session/buffer/`pump` logic is fully unit-tested (no socket);
//! `run` is the thin live-socket wrapper. See tasks/specs/live-feeds-phase.md §3.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One parsed cluster spot. Band derivation is left to the consumer (which owns
/// the band model); we carry the raw frequency the cluster sent.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterSpot {
    /// The station that reported the spot (uppercased).
    pub spotter: String,
    /// The DX station spotted (uppercased).
    pub dx_call: String,
    /// Spot frequency in kHz, as sent.
    pub freq_khz: f64,
    /// Free-text comment (mode / report / notes), trailing time stripped.
    pub comment: String,
    /// UTC time token ("1234Z") if the line carried one.
    pub time_utc: Option<String>,
}

impl ClusterSpot {
    /// Spot frequency in MHz (kHz / 1000).
    pub fn freq_mhz(&self) -> f64 {
        self.freq_khz / 1000.0
    }
}

/// True for a "1234Z"-style UTC time token.
fn is_time_token(t: &str) -> bool {
    t.len() == 5 && t.ends_with('Z') && t[..4].bytes().all(|b| b.is_ascii_digit())
}

/// Parse one cluster line into a [`ClusterSpot`], or `None` if it isn't a usable
/// `DX de` spot (banner / WWV / chat / malformed).
///
/// Format (universal across cluster software):
/// `DX de <spotter>:   <freq_khz>  <dx_call>  <comment…>   [HHMMZ]`
pub fn parse_dx_spot(line: &str) -> Option<ClusterSpot> {
    let line = line.trim();
    // The `DX de ` prefix marks a spot line; it's ASCII so byte-slicing is safe.
    const PREFIX: &str = "DX de ";
    if line.len() < PREFIX.len() || !line[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let after = line[PREFIX.len()..].trim_start();
    // Spotter is everything up to the first ':'.
    let (spotter, body) = after.split_once(':')?;
    let spotter = spotter.trim().to_ascii_uppercase();
    if spotter.is_empty() {
        return None;
    }

    let mut tokens = body.split_whitespace();
    let freq_khz: f64 = tokens.next()?.parse().ok()?;
    if !freq_khz.is_finite() || freq_khz <= 0.0 {
        return None;
    }
    let dx_call = tokens.next()?.trim().to_ascii_uppercase();
    if dx_call.is_empty() {
        return None;
    }

    let rest: Vec<&str> = tokens.collect();
    // A trailing HHMMZ token is the spot time; split it off the comment.
    let (comment_tokens, time_utc) = match rest.last() {
        Some(t) if is_time_token(t) => (&rest[..rest.len() - 1], Some((*t).to_string())),
        _ => (&rest[..], None),
    };
    Some(ClusterSpot {
        spotter,
        dx_call,
        freq_khz,
        comment: comment_tokens.join(" "),
        time_utc,
    })
}

/// What the session wants the transport to do in response to received bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Write this text back to the server (the callsign login).
    Send(String),
    /// A parsed DX spot.
    Spot(ClusterSpot),
}

/// Drives the cluster login handshake + spot extraction over an incremental byte
/// stream (telnet sends the login prompt WITHOUT a trailing newline, so we scan
/// the buffer for a prompt, not just complete lines).
pub struct ClusterSession {
    call: String,
    logged_in: bool,
    buf: String,
}

impl ClusterSession {
    pub fn new(call: &str) -> Self {
        Self {
            call: call.trim().to_ascii_uppercase(),
            logged_in: false,
            buf: String::new(),
        }
    }

    /// Feed a received chunk; returns the actions to take (send login, emit spots).
    pub fn feed(&mut self, chunk: &str) -> Vec<Action> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        // Pre-login: answer the first login prompt seen anywhere in the buffer.
        if !self.logged_in && is_login_prompt(&self.buf) {
            out.push(Action::Send(format!("{}\r\n", self.call)));
            self.logged_in = true;
            self.buf.clear(); // discard the pre-login banner
            return out;
        }
        // Extract complete lines; the trailing partial stays buffered.
        while let Some(nl) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=nl).collect();
            if let Some(spot) = parse_dx_spot(line.trim_end_matches(['\r', '\n'])) {
                out.push(Action::Spot(spot));
            }
        }
        // Guard against unbounded growth if the server never sends a newline.
        if self.buf.len() > 8192 {
            self.buf.clear();
        }
        out
    }
}

/// Is the buffer's TRAILING (incomplete) line a login prompt? Telnet prompts
/// arrive without a newline, so we look only at the text after the last `\n` and
/// require a prompt-terminal pattern — so MOTD/help body lines that merely mention
/// "login"/"callsign" can't trigger a premature, session-wedging login.
fn is_login_prompt(s: &str) -> bool {
    let tail = s
        .rsplit('\n')
        .next()
        .unwrap_or(s)
        .trim_end()
        .to_ascii_lowercase();
    tail.ends_with("login:")
        || tail.ends_with("your call:")
        || tail.ends_with("enter your call")
        || tail.ends_with("callsign:")
}

/// A bounded buffer of the most-recent spots (deduped by DX callsign, newest
/// kept) with insertion timestamps for freshness, for the need-scorer to read.
/// Thread-safety is the caller's (wrap in a Mutex).
#[derive(Debug, Clone)]
pub struct SpotBuffer {
    spots: VecDeque<(Instant, ClusterSpot)>,
    cap: usize,
}

impl SpotBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            spots: VecDeque::new(),
            cap: cap.max(1),
        }
    }
    /// Add a spot stamped now; a prior spot of the same DX call is replaced.
    pub fn push(&mut self, spot: ClusterSpot) {
        self.push_at(Instant::now(), spot);
    }
    /// As [`push`](Self::push) with an explicit timestamp (for tests).
    pub fn push_at(&mut self, at: Instant, spot: ClusterSpot) {
        self.spots.retain(|(_, s)| s.dx_call != spot.dx_call);
        self.spots.push_back((at, spot));
        while self.spots.len() > self.cap {
            self.spots.pop_front();
        }
    }
    /// All buffered spots (ignores age).
    pub fn recent(&self) -> Vec<ClusterSpot> {
        self.spots.iter().map(|(_, s)| s.clone()).collect()
    }
    /// Spots no older than `max_age` as of `now` — "heard now" means recent, so a
    /// DXpedition spotted once hours ago doesn't keep ringing the bell.
    pub fn recent_within(&self, now: Instant, max_age: Duration) -> Vec<ClusterSpot> {
        self.spots
            .iter()
            .filter(|(t, _)| now.duration_since(*t) <= max_age)
            .map(|(_, s)| s.clone())
            .collect()
    }
    pub fn len(&self) -> usize {
        self.spots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }
}

impl Default for SpotBuffer {
    fn default() -> Self {
        Self::new(200)
    }
}

/// Drive a [`ClusterSession`] over a connected duplex until EOF or `stop`. Pure
/// over any Read/Write, so it's unit-testable without a socket.
fn pump<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    call: &str,
    on_spot: &mut dyn FnMut(&ClusterSpot),
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let mut session = ClusterSession::new(call);
    let mut buf = [0u8; 4096];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(()), // connection closed
            Ok(n) => n,
            // A read timeout lets the loop re-check `stop` (the live socket sets one).
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
        let chunk = String::from_utf8_lossy(&buf[..n]);
        for action in session.feed(&chunk) {
            match action {
                Action::Send(s) => writer.write_all(s.as_bytes())?,
                Action::Spot(sp) => on_spot(&sp),
            }
        }
    }
}

/// Connect to a DX cluster / RBN telnet `addr` ("host:port"), log in with `call`,
/// and call `on_spot` for each parsed spot, reconnecting with backoff until
/// `stop` is set. The thin live-socket wrapper around [`pump`] (which holds the
/// tested logic).
pub fn run(addr: &str, call: &str, mut on_spot: impl FnMut(&ClusterSpot), stop: &AtomicBool) {
    const BASE: Duration = Duration::from_secs(2);
    const MAX: Duration = Duration::from_secs(60);
    let mut backoff = BASE;
    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        if let Some(stream) = connect(addr) {
            if let Ok(reader) = stream.try_clone() {
                let _ = pump(reader, stream, call, &mut on_spot, stop);
            }
        }
        // A real session (stayed up a while) resets backoff; a fast connect-fail /
        // instant-drop (server down, or a rejected/unregistered call) backs off
        // exponentially up to a minute, so we don't hammer the endpoint.
        backoff = if started.elapsed() > Duration::from_secs(10) {
            BASE
        } else {
            (backoff * 2).min(MAX)
        };
        if !sleep_interruptible(backoff, stop) {
            return;
        }
    }
}

/// Sleep `dur` in 100 ms steps, returning false early if `stop` is set.
fn sleep_interruptible(dur: Duration, stop: &AtomicBool) -> bool {
    let steps = (dur.as_millis() / 100).max(1);
    for _ in 0..steps {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

fn connect(addr: &str) -> Option<TcpStream> {
    let sa = addr.to_socket_addrs().ok()?.next()?;
    let stream = TcpStream::connect_timeout(&sa, Duration::from_secs(8)).ok()?;
    // A read timeout so the pump loop can periodically observe `stop`.
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_canonical_dx_spider_line() {
        let s = parse_dx_spot("DX de W3LPL:     14025.0  UA9CDC       CW 599                1234Z")
            .unwrap();
        assert_eq!(s.spotter, "W3LPL");
        assert_eq!(s.dx_call, "UA9CDC");
        assert_eq!(s.freq_khz, 14025.0);
        assert_eq!(s.comment, "CW 599");
        assert_eq!(s.time_utc.as_deref(), Some("1234Z"));
        assert!((s.freq_mhz() - 14.025).abs() < 1e-9);
    }

    #[test]
    fn parses_an_rbn_ft8_line() {
        let s = parse_dx_spot("DX de KM3T-#:    14074.0  3Y0J         FT8  -6 dB  25 WPM  0312Z")
            .unwrap();
        assert_eq!(s.spotter, "KM3T-#");
        assert_eq!(s.dx_call, "3Y0J");
        assert_eq!(s.freq_khz, 14074.0);
        assert!(s.comment.contains("FT8"));
        assert_eq!(s.time_utc.as_deref(), Some("0312Z"));
    }

    #[test]
    fn rejects_non_spot_lines() {
        assert!(parse_dx_spot("Welcome to the DX cluster, W9XYZ").is_none());
        assert!(parse_dx_spot("WWV de W0MU <14>:   SFI=120, A=8, K=2").is_none());
        assert!(parse_dx_spot("To ALL de W1ABC: good morning").is_none());
        assert!(parse_dx_spot("").is_none());
    }

    #[test]
    fn handles_a_missing_time_and_missing_comment() {
        let s = parse_dx_spot("DX de N1XX: 7005.0 JA1ABC").unwrap();
        assert_eq!(s.dx_call, "JA1ABC");
        assert_eq!(s.comment, "");
        assert!(s.time_utc.is_none());

        let s2 = parse_dx_spot("DX de n1xx: 7005.0 ja1abc nice sig").unwrap();
        assert_eq!(s2.spotter, "N1XX"); // case-insensitive prefix + uppercased
        assert_eq!(s2.dx_call, "JA1ABC");
        assert_eq!(s2.comment, "nice sig");
        assert!(s2.time_utc.is_none());
    }

    #[test]
    fn rejects_malformed_freq_or_missing_call() {
        assert!(parse_dx_spot("DX de W3LPL: not-a-freq UA9CDC CW").is_none());
        assert!(parse_dx_spot("DX de W3LPL: 14025.0").is_none()); // no dx call
        assert!(parse_dx_spot("DX de W3LPL: -5.0 UA9CDC").is_none()); // nonsensical freq
    }

    #[test]
    fn session_answers_the_login_prompt_then_emits_spots() {
        let mut s = ClusterSession::new("w9xyz");
        // Banner + prompt (no trailing newline) → send the (uppercased) callsign.
        assert_eq!(
            s.feed("RBN CW Skimmer\r\nPlease enter your call: "),
            vec![Action::Send("W9XYZ\r\n".to_string())]
        );
        // After login, spot lines parse; non-spot chatter is ignored.
        let acts = s.feed("Hello W9XYZ\r\nDX de KM3T-#: 14074.0 3Y0J FT8 -6 dB 0312Z\r\n");
        assert_eq!(acts.len(), 1);
        match &acts[0] {
            Action::Spot(sp) => assert_eq!(sp.dx_call, "3Y0J"),
            other => panic!("expected a spot, got {other:?}"),
        }
    }

    #[test]
    fn banner_text_mentioning_login_does_not_trigger_until_the_real_prompt() {
        let mut s = ClusterSession::new("W9XYZ");
        // A MOTD line that mentions "login" (newline-terminated) must NOT log in —
        // only the trailing prompt does.
        assert!(s.feed("Type HELP for login commands.\r\n").is_empty());
        // Still not logged in → the real trailing prompt now triggers it.
        assert_eq!(
            s.feed("login: "),
            vec![Action::Send("W9XYZ\r\n".to_string())]
        );
    }

    #[test]
    fn spot_buffer_recent_within_drops_stale_spots() {
        let base = Instant::now();
        let sp = |call: &str| ClusterSpot {
            spotter: "X".into(),
            dx_call: call.into(),
            freq_khz: 14074.0,
            comment: String::new(),
            time_utc: None,
        };
        let mut b = SpotBuffer::new(10);
        b.push_at(base, sp("OLD")); // stamped at base
        b.push_at(base + Duration::from_secs(1000), sp("NEW")); // 1000s later
                                                                // As of base+1000s with a 900s window: OLD aged out, NEW kept.
        let calls: Vec<String> = b
            .recent_within(base + Duration::from_secs(1000), Duration::from_secs(900))
            .into_iter()
            .map(|s| s.dx_call)
            .collect();
        assert_eq!(calls, vec!["NEW".to_string()]);
        // recent() still returns everything (age-agnostic).
        assert_eq!(b.recent().len(), 2);
    }

    #[test]
    fn session_handles_a_spot_split_across_two_reads() {
        let mut s = ClusterSession::new("W9XYZ");
        let _ = s.feed("login: "); // log in
        assert!(s.feed("DX de W3LPL: 14025.0 UA9").is_empty()); // partial line, buffered
        let acts = s.feed("CDC CW 599 1234Z\r\n"); // completes the line
        assert_eq!(acts.len(), 1);
        match &acts[0] {
            Action::Spot(sp) => assert_eq!(sp.dx_call, "UA9CDC"),
            other => panic!("expected a spot, got {other:?}"),
        }
    }

    #[test]
    fn spot_buffer_dedups_by_call_and_caps() {
        let sp = |call: &str| ClusterSpot {
            spotter: "X".into(),
            dx_call: call.into(),
            freq_khz: 14074.0,
            comment: String::new(),
            time_utc: None,
        };
        let mut b = SpotBuffer::new(2);
        b.push(sp("A"));
        b.push(sp("B"));
        b.push(sp("A")); // dedup A (latest wins) → [B, A]
        assert_eq!(b.len(), 2);
        let calls: Vec<String> = b.recent().into_iter().map(|s| s.dx_call).collect();
        assert_eq!(calls, vec!["B".to_string(), "A".to_string()]);
        b.push(sp("C")); // cap 2 → drop oldest (B) → [A, C]
        let calls: Vec<String> = b.recent().into_iter().map(|s| s.dx_call).collect();
        assert_eq!(calls, vec!["A".to_string(), "C".to_string()]);
    }

    /// A `Read` that yields scripted chunks (one per `read`), then EOF — lets us
    /// drive `pump` deterministically without a socket.
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

    #[test]
    fn pump_logs_in_writes_the_call_and_surfaces_spots() {
        let reader = ScriptReader {
            chunks: vec![
                b"login: ".to_vec(),
                b"DX de W3LPL: 14074.0 3Y0J FT8 0312Z\r\n".to_vec(),
            ]
            .into(),
        };
        let mut writer: Vec<u8> = Vec::new();
        let stop = AtomicBool::new(false);
        let mut spots: Vec<ClusterSpot> = Vec::new();
        pump(
            reader,
            &mut writer,
            "W9XYZ",
            &mut |sp| spots.push(sp.clone()),
            &stop,
        )
        .unwrap();
        assert_eq!(writer, b"W9XYZ\r\n", "the callsign was sent at the prompt");
        assert_eq!(spots.len(), 1);
        assert_eq!(spots[0].dx_call, "3Y0J");
        assert!((spots[0].freq_mhz() - 14.074).abs() < 1e-9);
    }

    #[test]
    fn pump_stops_promptly_when_signalled() {
        // stop set before any read → returns immediately, nothing sent.
        let reader = ScriptReader {
            chunks: vec![b"login: ".to_vec()].into(),
        };
        let mut writer: Vec<u8> = Vec::new();
        let stop = AtomicBool::new(true);
        pump(reader, &mut writer, "W9XYZ", &mut |_| {}, &stop).unwrap();
        assert!(writer.is_empty());
    }
}
