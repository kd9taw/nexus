//! The CI-V serial engine — ONE thread owns the CI-V byte stream and multiplexes three
//! traffics over it:
//!
//! 1. **Command/reply** — requests arrive on a channel ([`CivHandle::transact`]), are written
//!    to the port strictly one at a time, and the matching reply (or `FB`/`FA` ack) resolves
//!    the caller. A per-request serial deadline keeps a dead radio from wedging the queue.
//! 2. **Unsolicited transceive** — the radio pushes frequency (`00`) / mode (`01`) reports
//!    when the operator touches the front panel; they fold into the shared [`CivState`]
//!    (instant dial tracking, no polling).
//! 3. **Scope waveform** (`27`) — routed to the [`ScopeAssembler`]; each completed sweep
//!    lands in a latest-wins slot the radio loop drains into the waterfall.
//!
//! The engine is generic over [`CivIo`] (`Read + Write`), so the whole protocol path is
//! unit-tested against an in-memory fake radio — only the constructor that opens a real
//! serial port needs the `serial` feature (see [`super::broker`]).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::frame::{Frame, FrameSplitter};
use super::scope::{scope_stream_frames, ScopeAssembler, ScopeSweep};
use super::state::CivState;

/// The byte transport the engine drives: a real serial port in production (both
/// `serialport`'s port types and our test fakes implement `Read + Write`). Reads must
/// TIME OUT rather than block forever (`ErrorKind::TimedOut`/`WouldBlock` = "no data
/// yet") — the engine's loop interleaves reads with the command queue.
pub trait CivIo: Read + Write + Send {}
impl<T: Read + Write + Send> CivIo for T {}

/// How long the engine waits on the wire for one request's reply before failing it.
/// CI-V at 115200 answers in ~10–20 ms; even 19200 stays well under this.
const REQUEST_DEADLINE: Duration = Duration::from_millis(300);
/// Read chunk timeout the loop expects `CivIo` reads to observe (the real port is opened
/// with this; the loop just treats timeouts as "no data").
pub const READ_TIMEOUT: Duration = Duration::from_millis(30);

/// What reply resolves a request.
#[derive(Debug, Clone, Copy)]
pub enum Expect {
    /// A set command → bare `FB` (ok) / `FA` (rejected).
    Ack,
    /// A read → a frame with this command byte (and this first data byte, for
    /// sub-commanded reads like `15 02`).
    Reply { cmd: u8, sub: Option<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CivError {
    /// No matching reply within the deadline (radio off / wrong address / wrong baud).
    Timeout,
    /// The radio rejected the command (`FA`).
    Nak,
    /// The engine thread is gone.
    Gone,
}

struct CivRequest {
    frame: Frame,
    expect: Expect,
    /// `None` for the engine's own housekeeping commands (scope enable/disable): they
    /// still occupy the pending slot — every write MUST, or their acks would resolve a
    /// later caller's command on the half-duplex bus — but nobody awaits the result.
    reply_to: Option<mpsc::SyncSender<Result<Frame, CivError>>>,
}

impl CivRequest {
    fn resolve(self, r: Result<Frame, CivError>) {
        if let Some(tx) = self.reply_to {
            let _ = tx.try_send(r);
        }
    }
}

/// Cloneable client handle to the engine: transact commands, read the live state.
#[derive(Clone)]
pub struct CivHandle {
    tx: mpsc::Sender<CivRequest>,
    state: Arc<Mutex<CivState>>,
    alive: Arc<AtomicBool>,
}

impl CivHandle {
    /// Send one CI-V command and wait for its reply/ack. Serialized with every other
    /// caller — the engine owns the half-duplex bus.
    pub fn transact(&self, frame: Frame, expect: Expect) -> Result<Frame, CivError> {
        // Fail in microseconds when the engine thread is dead — a wedged daemon must
        // never serialize callers behind full recv timeouts (the UI-hang convoy).
        if !self.alive.load(Ordering::Relaxed) {
            return Err(CivError::Gone);
        }
        let (rtx, rrx) = mpsc::sync_channel(1);
        self.tx
            .send(CivRequest {
                frame,
                expect,
                reply_to: Some(rtx),
            })
            .map_err(|_| CivError::Gone)?;
        // The engine enforces REQUEST_DEADLINE per request; the extra headroom here covers
        // requests queued behind others.
        rrx.recv_timeout(REQUEST_DEADLINE * 4 + Duration::from_millis(100))
            .map_err(|_| CivError::Timeout)?
    }

    /// A snapshot of the live state (freq/mode/PTT/meters folded from replies + transceive).
    pub fn state(&self) -> CivState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// The running engine. Dropping it stops the thread (and the port closes with it).
pub struct CivEngine {
    handle: CivHandle,
    scope_row: Arc<Mutex<Option<ScopeSweep>>>,
    scope_enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CivEngine {
    /// Start the engine on `io`, talking to the radio at CI-V address `radio_addr`.
    pub fn start(io: Box<dyn CivIo>, radio_addr: u8) -> CivEngine {
        let (tx, rx) = mpsc::channel::<CivRequest>();
        let state = Arc::new(Mutex::new(CivState::default()));
        let scope_row = Arc::new(Mutex::new(None));
        let scope_enabled = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let thread = {
            let state = state.clone();
            let scope_row = scope_row.clone();
            let scope_enabled = scope_enabled.clone();
            let stop = stop.clone();
            let alive = alive.clone();
            std::thread::Builder::new()
                .name("civ-engine".into())
                .spawn(move || {
                    engine_loop(
                        io,
                        radio_addr,
                        rx,
                        state,
                        scope_row,
                        scope_enabled,
                        stop,
                        alive,
                    )
                })
                .expect("spawn civ-engine")
        };
        CivEngine {
            handle: CivHandle {
                tx,
                state,
                alive: alive.clone(),
            },
            scope_row,
            scope_enabled,
            stop,
            alive,
            thread: Some(thread),
        }
    }

    pub fn handle(&self) -> CivHandle {
        self.handle.clone()
    }

    /// Take the newest completed scope sweep, if one arrived since the last take.
    pub fn take_scope_row(&self) -> Option<ScopeSweep> {
        self.scope_row.lock().ok().and_then(|mut s| s.take())
    }

    /// Enable/disable the radio's scope waveform stream. The engine sends the CI-V
    /// enable/disable commands on the transition (idempotent per state).
    pub fn set_scope_enabled(&self, on: bool) {
        self.scope_enabled.store(on, Ordering::Relaxed);
    }

    /// False once the engine thread has exited (I/O error — port unplugged/denied).
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

impl Drop for CivEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One frame write with bounded retries for transient stalls.
enum WriteOutcome {
    Ok,
    /// Timed out repeatedly — the request fails, the engine lives.
    Transient,
    /// Hard I/O error — the port is gone.
    Fatal,
}

fn write_frame(io: &mut Box<dyn CivIo>, frame: &Frame) -> WriteOutcome {
    let bytes = frame.to_bytes();
    for _ in 0..3 {
        match io.write_all(&bytes).and_then(|_| io.flush()) {
            Ok(()) => return WriteOutcome::Ok,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {} // retry
            Err(_) => return WriteOutcome::Fatal,
        }
    }
    WriteOutcome::Transient
}

/// True when `f` resolves a request expecting `expect`. `FA` rejects both kinds.
fn resolves(expect: Expect, f: &Frame) -> Option<Result<Frame, CivError>> {
    if f.is_nak() {
        return Some(Err(CivError::Nak));
    }
    match expect {
        Expect::Ack => f.is_ack().then(|| Ok(f.clone())),
        Expect::Reply { cmd, sub } => {
            let sub_ok = sub.is_none_or(|s| f.data.first() == Some(&s));
            (f.cmd == cmd && sub_ok).then(|| Ok(f.clone()))
        }
    }
}

#[allow(clippy::too_many_arguments)] // one private loop, one call site
fn engine_loop(
    mut io: Box<dyn CivIo>,
    radio_addr: u8,
    rx: mpsc::Receiver<CivRequest>,
    state: Arc<Mutex<CivState>>,
    scope_row: Arc<Mutex<Option<ScopeSweep>>>,
    scope_enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
) {
    let mut splitter = FrameSplitter::new();
    let mut assembler = ScopeAssembler::new();
    let mut pending: Option<(CivRequest, Instant)> = None;
    // The engine's own housekeeping commands, queued ahead of caller traffic. They flow
    // through the SAME pending slot as user requests — every write must, or their acks
    // would resolve a later caller's command on the half-duplex bus.
    let mut internal: std::collections::VecDeque<CivRequest> = std::collections::VecDeque::new();
    let mut scope_sent: Option<bool> = None; // last commanded waveform-output state
    let mut buf = [0u8; 512];
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Keep the radio's waveform-output state in sync with the wanted flag.
        let want_scope = scope_enabled.load(Ordering::Relaxed);
        if scope_sent != Some(want_scope) {
            for f in scope_stream_frames(radio_addr, want_scope) {
                internal.push_back(CivRequest {
                    frame: f,
                    expect: Expect::Ack,
                    reply_to: None,
                });
            }
            scope_sent = Some(want_scope);
        }
        // Start the next queued request when idle (housekeeping first, then callers).
        if pending.is_none() {
            let next = internal.pop_front().map(Ok).unwrap_or_else(|| {
                rx.try_recv().map_err(|e| match e {
                    mpsc::TryRecvError::Empty => false,
                    mpsc::TryRecvError::Disconnected => true,
                })
            });
            match next {
                Ok(req) => {
                    // A transient write stall (USB hiccup) fails THIS REQUEST, never the
                    // engine — killing the engine over one stall would take down all
                    // native CAT including the ability to unkey a keyed radio.
                    match write_frame(&mut io, &req.frame) {
                        WriteOutcome::Ok => {
                            pending = Some((req, Instant::now() + REQUEST_DEADLINE));
                        }
                        WriteOutcome::Transient => {
                            req.resolve(Err(CivError::Timeout));
                        }
                        WriteOutcome::Fatal => {
                            req.resolve(Err(CivError::Gone));
                            break; // port gone for real
                        }
                    }
                }
                Err(true) => break, // all handles dropped
                Err(false) => {}
            }
        }
        // Read whatever arrived (short timeout keeps the loop responsive).
        let frames = match io.read(&mut buf) {
            Ok(0) => {
                // A pipe-like fake returns Ok(0) at EOF; a serial port never does. Treat
                // as "no data" so tests can drain, but yield so we don't spin.
                std::thread::sleep(Duration::from_millis(1));
                Vec::new()
            }
            Ok(n) => splitter.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Vec::new()
            }
            Err(_) => break, // hard I/O error — port unplugged
        };
        for f in frames {
            // Scope waveform frames go to the assembler, never to request matching.
            if f.cmd == 0x27 {
                if let Some(sweep) = assembler.push(&f) {
                    if let Ok(mut slot) = scope_row.lock() {
                        *slot = Some(sweep); // latest wins
                    }
                }
                continue;
            }
            // Everything else refreshes the live state (replies AND transceive pushes).
            if let Ok(mut s) = state.lock() {
                s.apply(&f);
            }
            // Resolve the in-flight request if this frame answers it.
            if let Some((req, _)) = &pending {
                if let Some(result) = resolves(req.expect, &f) {
                    let (req, _) = pending.take().unwrap();
                    req.resolve(result);
                }
            }
        }
        // Fail a request the radio never answered.
        if let Some((_, deadline)) = &pending {
            if Instant::now() > *deadline {
                let (req, _) = pending.take().unwrap();
                req.resolve(Err(CivError::Timeout));
            }
        }
    }
    // Fail everything still queued so callers unblock immediately.
    if let Some((req, _)) = pending.take() {
        req.resolve(Err(CivError::Gone));
    }
    while let Ok(req) = rx.try_recv() {
        req.resolve(Err(CivError::Gone));
    }
    alive.store(false, Ordering::Relaxed);
}

/// The in-memory fake IC-9700 the engine + daemon tests drive — kept out of `mod tests`
/// so the broker's end-to-end test reuses it.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::super::frame::{bcd_to_freq, freq_to_bcd, Frame, CONTROLLER};
    use std::collections::VecDeque;
    use std::io::{self, Read, Write};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// An in-memory fake IC-9700: scripted replies keyed by (cmd, first data byte).
    /// Reads time out when nothing is queued, like a real serial port.
    pub struct FakeRadio {
        addr: u8,
        outgoing: VecDeque<u8>,
        /// Unsolicited bytes injected before the next read (transceive, scope).
        push_next: Arc<Mutex<Vec<u8>>>,
        /// Frequency register the fake maintains (echoes set, answers read).
        freq: u64,
        /// When true, drop every command silently (a dead radio).
        pub mute: bool,
        /// When true, every read/write fails hard (a yanked cable) — the engine
        /// classifies it Fatal and exits, driving `is_alive()` false.
        pub dead: bool,
    }
    impl FakeRadio {
        pub fn new(addr: u8) -> (Self, Arc<Mutex<Vec<u8>>>) {
            let push = Arc::new(Mutex::new(Vec::new()));
            (
                FakeRadio {
                    addr,
                    outgoing: VecDeque::new(),
                    push_next: push.clone(),
                    freq: 145_000_000,
                    mute: false,
                    dead: false,
                },
                push,
            )
        }
        fn reply(&mut self, cmd: u8, data: &[u8]) {
            let f = Frame {
                to: CONTROLLER,
                from: self.addr,
                cmd,
                data: data.to_vec(),
            };
            self.outgoing.extend(f.to_bytes());
        }
        fn ack(&mut self) {
            self.reply(0xFB, &[]);
        }
    }
    impl Write for FakeRadio {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.dead {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "dead"));
            }
            // FrameSplitter drops controller-originated frames as "echo", so parse raw.
            let mut raw = Vec::new();
            let mut cur = Vec::new();
            for &b in buf {
                cur.push(b);
                if b == 0xFD {
                    raw.push(std::mem::take(&mut cur));
                }
            }
            for bytes in raw {
                let Some(f) = Frame::parse(&bytes) else {
                    continue;
                };
                if self.mute {
                    continue;
                }
                match (f.cmd, f.data.first().copied()) {
                    (0x03, _) => {
                        let bcd = freq_to_bcd(self.freq).to_vec();
                        self.reply(0x03, &bcd);
                    }
                    (0x05, _) => {
                        self.freq = bcd_to_freq(&f.data);
                        self.ack();
                    }
                    (0x15, Some(0x02)) => self.reply(0x15, &[0x02, 0x01, 0x20]), // raw 120 = S9
                    (0x27, _) => self.ack(),    // scope enable/disable
                    _ => self.reply(0xFA, &[]), // NAK anything unknown
                }
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Read for FakeRadio {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.dead {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "dead"));
            }
            if let Ok(mut p) = self.push_next.lock() {
                if !p.is_empty() {
                    self.outgoing.extend(p.drain(..));
                }
            }
            if self.outgoing.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
                return Err(io::Error::new(io::ErrorKind::TimedOut, "no data"));
            }
            let n = buf.len().min(self.outgoing.len());
            for (i, b) in self.outgoing.drain(..n).enumerate() {
                buf[i] = b;
            }
            Ok(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::commands::{read_freq, read_smeter, set_freq};
    use super::super::frame::{freq_to_bcd, Frame};
    use super::tests_support::FakeRadio;
    use super::*;

    #[test]
    fn transact_read_and_set_against_a_fake_radio() {
        let (radio, _push) = FakeRadio::new(0xA2);
        let eng = CivEngine::start(Box::new(radio), 0xA2);
        let h = eng.handle();
        // Read the frequency.
        let f = h
            .transact(
                read_freq(0xA2),
                Expect::Reply {
                    cmd: 0x03,
                    sub: None,
                },
            )
            .expect("freq read");
        assert_eq!(super::super::commands::parse_freq(&f), Some(145_000_000));
        // Set a new one (ack), read it back.
        h.transact(set_freq(0xA2, 144_200_000), Expect::Ack)
            .expect("freq set acked");
        let f = h
            .transact(
                read_freq(0xA2),
                Expect::Reply {
                    cmd: 0x03,
                    sub: None,
                },
            )
            .expect("freq re-read");
        assert_eq!(super::super::commands::parse_freq(&f), Some(144_200_000));
        // The engine folded replies into the shared state too.
        assert_eq!(h.state().freq_hz, Some(144_200_000));
    }

    #[test]
    fn sub_commanded_read_matches_on_the_sub_byte() {
        let (radio, _push) = FakeRadio::new(0xA2);
        let eng = CivEngine::start(Box::new(radio), 0xA2);
        let f = eng
            .handle()
            .transact(
                read_smeter(0xA2),
                Expect::Reply {
                    cmd: 0x15,
                    sub: Some(0x02),
                },
            )
            .expect("smeter read");
        assert_eq!(super::super::commands::parse_smeter_raw(&f), Some(120));
    }

    #[test]
    fn a_dead_radio_times_out_instead_of_wedging() {
        let (mut radio, _push) = FakeRadio::new(0xA2);
        radio.mute = true;
        let eng = CivEngine::start(Box::new(radio), 0xA2);
        let t0 = Instant::now();
        let r = eng.handle().transact(
            read_freq(0xA2),
            Expect::Reply {
                cmd: 0x03,
                sub: None,
            },
        );
        assert_eq!(r.unwrap_err(), CivError::Timeout);
        assert!(t0.elapsed() < Duration::from_secs(3), "bounded, not wedged");
        // And the engine still answers later requests (a NAK-ing radio here).
        // (mute stays on — a second request also times out but doesn't panic.)
        let r = eng.handle().transact(
            read_freq(0xA2),
            Expect::Reply {
                cmd: 0x03,
                sub: None,
            },
        );
        assert_eq!(r.unwrap_err(), CivError::Timeout);
    }

    #[test]
    fn unsolicited_transceive_folds_into_state_without_a_request() {
        let (radio, push) = FakeRadio::new(0xA2);
        let eng = CivEngine::start(Box::new(radio), 0xA2);
        // The operator turns the knob: the radio pushes cmd 00 with the new freq.
        let f = Frame {
            to: 0x00, // transceive broadcasts to address 00
            from: 0xA2,
            cmd: 0x00,
            data: freq_to_bcd(146_520_000).to_vec(),
        };
        push.lock().unwrap().extend(f.to_bytes());
        // Wait for the engine to pick it up.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if eng.handle().state().freq_hz == Some(146_520_000) {
                break;
            }
            assert!(Instant::now() < deadline, "transceive folded into state");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn nak_resolves_as_nak_not_timeout() {
        let (radio, _push) = FakeRadio::new(0xA2);
        let eng = CivEngine::start(Box::new(radio), 0xA2);
        // The fake NAKs unknown commands — 0x1C PTT isn't scripted.
        let r = eng
            .handle()
            .transact(super::super::commands::set_ptt(0xA2, true), Expect::Ack);
        assert_eq!(r.unwrap_err(), CivError::Nak);
    }
}
