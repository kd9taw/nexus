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
    /// Unix seconds when WE received this spot (stamped at buffer insertion;
    /// 0 in raw-parse/test paths). Drives the honest "N min ago" evidence age —
    /// stamping poll-time made every cluster row read "just now" forever.
    pub received_unix: u64,
    /// OTHER spotters whose earlier spots of this DX were replaced in the
    /// buffer (dedup keeps the newest spot per DX but must not erase the
    /// corroboration evidence — VHF admission requires multiple independent
    /// near spotters). Capped small; excludes `spotter` itself.
    pub corroborators: Vec<String>,
}

impl ClusterSpot {
    /// Spot frequency in MHz (kHz / 1000).
    pub fn freq_mhz(&self) -> f64 {
        self.freq_khz / 1000.0
    }

    /// Split/QSX listening offset named in the spot comment, in kHz RELATIVE to
    /// the spot frequency: "UP 2" → +2.0, "DN 1.5" → −1.5, bare "UP" → +1.0 (the
    /// pile-up convention; single-letter "U"/"D" deliberately DON'T count — they
    /// false-positive on report fragments), "QSX 14025.5" → absolute kHz mapped
    /// to an offset (may be negative). `None` = no split named, stay simplex.
    pub fn split_offset_khz(&self) -> Option<f64> {
        let toks: Vec<&str> = self.comment.split_whitespace().collect();
        for (i, t) in toks.iter().enumerate() {
            let tl = t.to_ascii_uppercase();
            let (dir, bare) = match tl.as_str() {
                "UP" => (1.0, true),
                "DN" | "DOWN" => (-1.0, true),
                _ => {
                    // Glued forms: "UP2", "UP1.5", "DN2".
                    if let Some(rest) = tl.strip_prefix("UP") {
                        if let Ok(k) = rest.parse::<f64>() {
                            return Some(k).filter(|k| (0.1..=20.0).contains(k));
                        }
                    }
                    if let Some(rest) = tl.strip_prefix("DN") {
                        if let Ok(k) = rest.parse::<f64>() {
                            return Some(-k).filter(|k| (-20.0..=-0.1).contains(k));
                        }
                    }
                    if tl == "QSX" {
                        // "QSX 14025.5" — absolute listen frequency in kHz.
                        if let Some(abs) = toks.get(i + 1).and_then(|n| n.parse::<f64>().ok()) {
                            let off = abs - self.freq_khz;
                            if (0.1..=20.0).contains(&off.abs()) {
                                return Some(off);
                            }
                        }
                    }
                    continue;
                }
            };
            // "UP 2" / "DN 1.5" — number in the next token; bare → ±1 kHz convention.
            if let Some(k) = toks.get(i + 1).and_then(|n| n.parse::<f64>().ok()) {
                let off = dir * k;
                // An EXPLICIT but absurd offset (UP 50) is a typo/unknown — safer to
                // stay simplex than to move TX a guessed kilohertz.
                return Some(off).filter(|o| (0.1..=20.0).contains(&o.abs()));
            }
            if bare {
                return Some(dir * 1.0);
            }
        }
        None
    }

    /// The operating mode named in the spot comment, when one is ("CW 599", "FT8
    /// -6 dB", "RTTY", "SSB 59"…). RBN comments lead with the mode; human spots
    /// often carry one too. `None` when the comment doesn't say — callers fall
    /// back to band/frequency heuristics. Token-matched (not substring) so e.g.
    /// a callsign fragment can't read as a mode.
    pub fn mode(&self) -> Option<&'static str> {
        const MODES: &[&str] = &[
            // "AM" is deliberately ABSENT: it false-positives on time-of-day comments
            // ("9 AM EST") far more often than real AM-mode spots occur on HF.
            "CW", "FT8", "FT4", "RTTY", "SSB", "USB", "LSB", "PSK31", "PSK", "JS8", "FM",
            "MSK144", "Q65", "JT65", "FT1", "DX1",
        ];
        self.comment
            .split_whitespace()
            .find_map(|tok| MODES.iter().find(|m| tok.eq_ignore_ascii_case(m)).copied())
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
        received_unix: 0, // stamped at buffer insertion
        corroborators: Vec::new(),
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
        // Extract complete lines; the trailing partial stays buffered. Pre-login
        // these are banner lines (parse_dx_spot ignores them).
        while let Some(nl) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=nl).collect();
            if let Some(spot) = parse_dx_spot(line.trim_end_matches(['\r', '\n'])) {
                out.push(Action::Spot(spot));
            }
        }
        // A login prompt as the trailing (newline-less) line — answer it EVERY time
        // it appears, not just once: some DXSpider/RBN hosts keep the socket open
        // and RE-prompt after a rejected/unregistered call, and answering only the
        // first prompt wedged the session forever (open TCP, no spots, no retry).
        if is_login_prompt(&self.buf) {
            out.push(Action::Send(format!("{}\r\n", self.call)));
            self.logged_in = true;
            self.buf.clear(); // discard the prompt/banner
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

/// A buffer of recent spots, deduped by (DX callsign, ~frequency) so a station
/// spotted on CW *and* SSB keeps BOTH opportunities (a single dedup-by-call would
/// let the high-rate RBN CW firehose overwrite the rarer human SSB entry). Retention
/// is AGE-based (`max_age`) so sparse sources survive the need-scorer's read window
/// regardless of how fast the firehose churns; `cap` is only a memory safety ceiling.
/// Thread-safety is the caller's (wrap in a Mutex).
#[derive(Debug, Clone)]
pub struct SpotBuffer {
    spots: VecDeque<(Instant, ClusterSpot)>,
    cap: usize,
    /// Spots older than this are dropped on push. Set ≥ the consumer's read window
    /// (`get_need_alerts` reads 900 s) so the read never misses a still-valid spot.
    max_age: Duration,
}

impl SpotBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            spots: VecDeque::new(),
            cap: cap.max(1),
            max_age: Duration::from_secs(1200),
        }
    }
    /// Add a spot stamped now; a prior spot of the same DX call is replaced.
    pub fn push(&mut self, mut spot: ClusterSpot) {
        if spot.received_unix == 0 {
            spot.received_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
        }
        self.push_at(Instant::now(), spot);
    }
    /// As [`push`](Self::push) with an explicit timestamp (for tests).
    pub fn push_at(&mut self, at: Instant, mut spot: ClusterSpot) {
        // Dedup key: same call AND ~same frequency (rounded to 1 kHz). CW (band bottom)
        // and SSB (band top) of one call differ in freq → both kept; true repeats collapse.
        let fk = spot.freq_khz.round() as i64;
        // Carry the replaced spot's spotter (and ITS corroborators) forward —
        // "who else reported this DX" is the multi-endpoint evidence the VHF
        // gate needs; plain replacement silently reduced every DX to one voice.
        if let Some((_, old)) = self
            .spots
            .iter()
            .find(|(_, s)| s.dx_call == spot.dx_call && s.freq_khz.round() as i64 == fk)
        {
            let mut set: Vec<String> = old
                .corroborators
                .iter()
                .cloned()
                .chain(std::iter::once(old.spotter.clone()))
                .filter(|c| {
                    !c.eq_ignore_ascii_case(&spot.spotter)
                        && !spot
                            .corroborators
                            .iter()
                            .any(|x| x.eq_ignore_ascii_case(c))
                })
                .collect();
            spot.corroborators.append(&mut set);
            spot.corroborators.truncate(8);
        }
        self.spots
            .retain(|(_, s)| !(s.dx_call == spot.dx_call && s.freq_khz.round() as i64 == fk));
        self.spots.push_back((at, spot));
        // Age-based retention is PRIMARY: drop spots older than `max_age` so a sparse
        // source (human SSB) survives the read window even while the RBN firehose floods.
        while let Some((t, _)) = self.spots.front() {
            if at.saturating_duration_since(*t) > self.max_age {
                self.spots.pop_front();
            } else {
                break;
            }
        }
        // Count ceiling: a memory safety net only (cap is high; age trimming does the work).
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
        // High ceiling so the count-cap never evicts inside the read window even under a
        // full RBN firehose (~300 calls/min × 20 min ≈ 6k); age trimming bounds memory.
        Self::new(8000)
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
    connected: &AtomicBool,
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
                Action::Send(s) => {
                    writer.write_all(s.as_bytes())?;
                    // The login prompt was answered — the session is genuinely up.
                    // (NOT on bare TCP-establish: a host that prompts and rejects
                    // must not read as "connected" before we've even logged in.)
                    connected.store(true, Ordering::Relaxed);
                }
                Action::Spot(sp) => on_spot(&sp),
            }
        }
    }
}

/// Connect to a DX cluster / RBN telnet `addr` ("host:port"), log in with `call`,
/// and call `on_spot` for each parsed spot, reconnecting with backoff until
/// `stop` is set. The thin live-socket wrapper around [`pump`] (which holds the
/// tested logic). `connected` mirrors the session state (true while a TCP session
/// is up) so the UI can tell "connected but quiet" from "can't connect" — a
/// spotless session previously read as an indistinguishable-from-broken "waiting".
pub fn run(
    addr: &str,
    call: &str,
    mut on_spot: impl FnMut(&ClusterSpot),
    stop: &AtomicBool,
    connected: &AtomicBool,
) {
    const BASE: Duration = Duration::from_secs(2);
    const MAX: Duration = Duration::from_secs(60);
    let mut backoff = BASE;
    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        if let Some(stream) = connect(addr) {
            if let Ok(reader) = stream.try_clone() {
                // `connected` flips true inside pump when the login prompt is
                // ANSWERED (not on bare TCP-establish), and clears on session end.
                let _ = pump(reader, stream, call, &mut on_spot, stop, connected);
                connected.store(false, Ordering::Relaxed);
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
    fn comment_mode_is_token_matched() {
        let mk = |comment: &str| ClusterSpot {
            spotter: "W3LPL".into(),
            dx_call: "UA9CDC".into(),
            freq_khz: 14025.0,
            comment: comment.into(),
            time_utc: None,
            received_unix: 0,
            corroborators: Vec::new(),
        };
        assert_eq!(mk("CW 599").mode(), Some("CW"));
        assert_eq!(mk("FT8  -6 dB  25 WPM").mode(), Some("FT8"));
        assert_eq!(mk("up 2 big pile").mode(), None, "no mode named");
        // Token match, not substring: a fragment can't read as a mode.
        assert_eq!(mk("CWO net").mode(), None);
        assert_eq!(mk("ssb 59 nice sig").mode(), Some("SSB"), "case-insensitive");
    }

    #[test]
    fn session_reanswers_a_login_reprompt() {
        // Some DXSpider/RBN hosts keep the socket open and prompt AGAIN after a
        // rejected/unregistered call. Answering only the first prompt wedged the
        // session forever (open TCP, no spots). The session must re-answer.
        let mut sess = ClusterSession::new("W9XYZ");
        let first = sess.feed("login: ");
        assert!(matches!(&first[..], [Action::Send(s)] if s == "W9XYZ\r\n"));
        // Server rejects and re-prompts.
        let again = sess.feed("\rsorry, unknown station\r\nlogin: ");
        assert!(
            matches!(&again[..], [Action::Send(s)] if s == "W9XYZ\r\n"),
            "re-prompt must be re-answered, got {again:?}"
        );
    }

    #[test]
    fn split_parses_pileup_conventions() {
        let mk = |comment: &str| ClusterSpot {
            spotter: "W3LPL".into(),
            dx_call: "3Y0J".into(),
            freq_khz: 14023.0,
            comment: comment.into(),
            time_utc: None,
            received_unix: 0,
            corroborators: Vec::new(),
        };
        assert_eq!(mk("CW UP 2").split_offset_khz(), Some(2.0));
        assert_eq!(mk("UP2 big pile").split_offset_khz(), Some(2.0));
        assert_eq!(mk("DN 1.5").split_offset_khz(), Some(-1.5));
        assert_eq!(mk("CW UP").split_offset_khz(), Some(1.0), "bare UP → +1 convention");
        assert_eq!(mk("QSX 14025.5").split_offset_khz(), Some(2.5));
        assert_eq!(mk("loud in NJ").split_offset_khz(), None);
        assert_eq!(mk("UP 50").split_offset_khz(), None, "absurd explicit offset → stay simplex");
        assert_eq!(mk("5UP9").split_offset_khz(), None, "report fragments don't parse");
        assert_eq!(mk("U 2").split_offset_khz(), None, "single-letter U doesn't count");
        assert_eq!(mk("59 D").split_offset_khz(), None, "single-letter D doesn't count");
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
            received_unix: 0,
            corroborators: Vec::new(),
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

    fn mk_spot(dx: &str, freq_khz: f64, comment: &str, spotter: &str) -> ClusterSpot {
        ClusterSpot {
            spotter: spotter.into(),
            dx_call: dx.into(),
            freq_khz,
            comment: comment.into(),
            time_utc: None,
            received_unix: 0,
            corroborators: Vec::new(),
        }
    }

    #[test]
    fn ssb_spot_survives_an_rbn_flood_within_the_read_window() {
        // THE phone bug: the old 200-cap buffer FIFO-evicted a sparse SSB spot within
        // ~2 min under the RBN firehose, before the 900 s need-scan could read it.
        let mut b = SpotBuffer::default(); // cap 8000, max_age 1200 s
        let t0 = Instant::now();
        b.push_at(t0, mk_spot("EA8DX", 14250.0, "SSB 59", "W1AAA")); // one human SSB spot
        for i in 0..1000u64 {
            // 1000 unique RBN CW spots (far more than the old 200 cap) over the next minute
            b.push_at(
                t0 + Duration::from_millis(i),
                mk_spot(&format!("R{i}"), 14025.0, "CW", "RBNX"),
            );
        }
        let recent = b.recent_within(t0 + Duration::from_secs(60), Duration::from_secs(900));
        assert!(
            recent.iter().any(|s| s.dx_call == "EA8DX"),
            "SSB spot must survive the RBN flood within the 900 s read window"
        );
    }

    #[test]
    fn cw_and_ssb_of_same_call_both_retained() {
        let mut b = SpotBuffer::new(100);
        let t = Instant::now();
        b.push_at(t, mk_spot("DL1ABC", 14025.0, "CW 599", "W1AAA")); // CW (band bottom)
        b.push_at(t + Duration::from_secs(1), mk_spot("DL1ABC", 14250.0, "SSB 59", "W2BBB")); // SSB (top)
        assert_eq!(
            b.recent().iter().filter(|s| s.dx_call == "DL1ABC").count(),
            2,
            "CW and SSB of the same call are distinct opportunities — both kept"
        );
        // A true repeat (same call, ~same freq within 1 kHz) still collapses.
        b.push_at(t + Duration::from_secs(2), mk_spot("DL1ABC", 14025.4, "CW 599", "W3CCC"));
        assert_eq!(
            b.recent().iter().filter(|s| s.dx_call == "DL1ABC").count(),
            2,
            "repeat CW (same ~freq) dedups — still 2 distinct entries"
        );
    }

    #[test]
    fn age_trim_drops_spots_older_than_max_age() {
        let mut b = SpotBuffer::new(8000); // max_age 1200 s
        let t = Instant::now();
        b.push_at(t, mk_spot("OLD1", 14025.0, "CW", "W1AAA"));
        // A push 1300 s later (> max_age) trims the now-stale OLD1 off the front.
        b.push_at(t + Duration::from_secs(1300), mk_spot("NEW1", 14026.0, "CW", "W2BBB"));
        let calls: Vec<String> = b.recent().into_iter().map(|s| s.dx_call).collect();
        assert!(!calls.contains(&"OLD1".to_string()), "spot older than max_age trimmed");
        assert!(calls.contains(&"NEW1".to_string()));
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
    fn dedup_replacement_carries_corroborating_spotters() {
        // VHF needs >= 2 independent near spotters — the buffer's newest-wins
        // dedup must carry the replaced spot's spotter forward as evidence,
        // not silently reduce every DX to one voice.
        let mut b = SpotBuffer::new(10);
        let mk = |spotter: &str| ClusterSpot {
            spotter: spotter.into(),
            dx_call: "4U1UN".into(),
            freq_khz: 50313.0,
            comment: "CW".into(),
            time_utc: None,
            received_unix: 0,
            corroborators: Vec::new(),
        };
        b.push(mk("K9LC"));
        b.push(mk("N9CO"));
        b.push(mk("K9IMM"));
        let spots = b.recent();
        assert_eq!(spots.len(), 1, "still deduped to one row per DX");
        let s = &spots[0];
        assert_eq!(s.spotter, "K9IMM", "newest spot wins");
        assert!(
            s.corroborators.contains(&"K9LC".to_string())
                && s.corroborators.contains(&"N9CO".to_string()),
            "replaced spotters carried as corroborators: {:?}",
            s.corroborators
        );
        // Re-spot by an existing voice must not duplicate it.
        b.push(mk("K9LC"));
        let s = &b.recent()[0];
        let n = s
            .corroborators
            .iter()
            .filter(|c| *c == "K9IMM" || *c == "N9CO")
            .count();
        assert_eq!(n, 2, "no dupes, prior voices kept: {:?}", s.corroborators);
        assert!(!s.corroborators.contains(&"K9LC".to_string()), "self excluded");
    }

    #[test]
    fn spot_buffer_dedups_by_call_and_caps() {
        let sp = |call: &str| ClusterSpot {
            spotter: "X".into(),
            dx_call: call.into(),
            freq_khz: 14074.0,
            comment: String::new(),
            time_utc: None,
            received_unix: 0,
            corroborators: Vec::new(),
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
        let connected = AtomicBool::new(false);
        pump(
            reader,
            &mut writer,
            "W9XYZ",
            &mut |sp| spots.push(sp.clone()),
            &stop,
            &connected,
        )
        .unwrap();
        assert_eq!(writer, b"W9XYZ\r\n", "the callsign was sent at the prompt");
        assert!(connected.load(Ordering::Relaxed), "answering the login prompt = connected");
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
        let connected = AtomicBool::new(false);
        pump(reader, &mut writer, "W9XYZ", &mut |_| {}, &stop, &connected).unwrap();
        assert!(writer.is_empty());
        assert!(!connected.load(Ordering::Relaxed), "never answered → never connected");
    }
}
