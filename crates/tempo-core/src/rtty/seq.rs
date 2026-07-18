//! RTTY auto-sequencer — a text-pattern QSO state machine over the
//! free-running decoded character stream.
//!
//! RTTY has no slot clock, so unlike the FT8 sequencer this machine is driven
//! by tolerant pattern matchers over a rolling window of decoded characters.
//! Garbled copy is NORMAL on the RTTY bands: callsigns match with one
//! character of fuzz gated by the per-char ATC confidence, and RST fields
//! normalize the classic lost-FIGS garble (`599` printed on the letters plane
//! is `TOO`) plus the `5NN` cut-number convention.
//!
//! The machine is PURE: it consumes [`DecodedChar`]s plus a millisecond clock
//! the caller supplies, and emits [`Action`]s (send this text / log this QSO /
//! abort) that the engine wires to the actual TX path and logbook. Exchange
//! content is table-driven by an [`ExchangeSchema`] ([`CASUAL`] RST/name/QTH
//! and [`FIELD_DAY`] class/section ship today — a contest serial is a schema
//! entry ([`FieldKind::Serial`]), not an engine change), and every transmitted
//! line is built from operator-editable [`Templates`] with
//! `{CALL}`/`{MYCALL}`/`{RST}`/`{EXCH}` substitution — the same template layer
//! a manual F-key presses.
//!
//! HUMAN-INITIATE GATE (ARRL FD rule 6.4 bans fully-automated contacts): the
//! machine never starts a session on its own. In [`SeqState::Idle`] decoded
//! text only accumulates — [`RttySeq::start_cq`] and [`RttySeq::answer`] are
//! the only doors in, and both are operator actions. [`find_cq`] exists so the
//! UI can SURFACE a heard CQ for the operator to click; it never transitions
//! the machine.
//!
//! Timeout discipline: while waiting on the peer, every `timeout_ms` of
//! silence sends `AGN` (or repeats the answering call); after `max_repeats`
//! cycles the session aborts — a runner falls back to calling CQ, a
//! search-and-pounce station returns to [`SeqState::Idle`].

use super::demod::DecodedChar;
use crate::fd_rules::valid_section;

// ---- Tunables -------------------------------------------------------------

/// A fuzzed callsign character is only forgiven when the demodulator was NOT
/// confident about it — a confidently-decoded different character means a
/// genuinely different callsign.
const HIGH_CONF: f32 = 0.75;

/// Tokens whose mean per-char confidence sits below this are noise-floor
/// garbage and never participate in a state transition (the blueprint's
/// "confidence gate").
const MIN_TOKEN_CONF: f32 = 0.25;

/// Rolling decoded-text window, in characters (~1 minute at 45.45 baud).
const WINDOW_CAP: usize = 400;

/// Timing knobs. `timeout_ms` runs from the last emitted action (or the last
/// [`RttySeq::on_tx_complete`], when the engine reports it — an RTTY
/// over takes ~10 s of TX time, which the machine cannot see on its own).
#[derive(Debug, Clone, Copy)]
pub struct SeqConfig {
    pub timeout_ms: u64,
    /// Timeout cycles before the session aborts; cycles `1..max_repeats` send
    /// AGN / repeat the call, cycle `max_repeats` emits [`Action::Abort`].
    pub max_repeats: u32,
}

impl Default for SeqConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            max_repeats: 3,
        }
    }
}

// ---- Exchange schemas -----------------------------------------------------

/// How one exchange field is recognized in garbled copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Signal report: `599`, `5NN` (cut), or full lost-FIGS garble (`TOO`).
    Rst,
    /// Field Day class — digits + one class letter (`3A`), lost-FIGS garble on
    /// the digit part tolerated (`EA` → `3A`). `letters` is the legal class
    /// letter set (SFD `A-F`; a WFD schema would pass `HIOM`).
    FdClass { letters: &'static str },
    /// ARRL/RAC section code, validated against the master list (plus the
    /// `MX`/`DX` extensions DX stations send).
    Section,
    /// Contest serial number, 1–4 digits after garble normalization. No
    /// shipped schema uses it yet — it exists so serials are a schema, not an
    /// engine change.
    Serial,
    /// A free word (name, QTH), located by its on-air label.
    Word,
}

/// One field of an exchange: its ADIF-ish key, the on-air label that precedes
/// it (`NAME SETH`), whether the QSO can complete without it, and how to
/// recognize it.
#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: Option<&'static str>,
    pub required: bool,
    pub kind: FieldKind,
}

/// A table-driven exchange: the fields, in the order they are parsed. The
/// contest-later seam — new events add a schema, never engine code.
#[derive(Debug, Clone, Copy)]
pub struct ExchangeSchema {
    pub name: &'static str,
    pub fields: &'static [FieldSpec],
}

/// Casual ragchew: RST required, name and QTH picked up when labeled.
pub const CASUAL: ExchangeSchema = ExchangeSchema {
    name: "casual",
    fields: &[
        FieldSpec {
            key: "RST",
            label: None,
            required: true,
            kind: FieldKind::Rst,
        },
        FieldSpec {
            key: "NAME",
            label: Some("NAME"),
            required: false,
            kind: FieldKind::Word,
        },
        FieldSpec {
            key: "QTH",
            label: Some("QTH"),
            required: false,
            kind: FieldKind::Word,
        },
    ],
};

/// ARRL Field Day: class + section, both required.
pub const FIELD_DAY: ExchangeSchema = ExchangeSchema {
    name: "fieldday",
    fields: &[
        FieldSpec {
            key: "CLASS",
            label: None,
            required: true,
            kind: FieldKind::FdClass { letters: "ABCDEF" },
        },
        FieldSpec {
            key: "SECTION",
            label: None,
            required: true,
            kind: FieldKind::Section,
        },
    ],
};

// ---- Templates ------------------------------------------------------------

/// Operator-editable message templates. Placeholders: `{CALL}` (the peer),
/// `{MYCALL}`, `{RST}` (my sent report), `{EXCH}` (my full exchange string,
/// labels included), `{SERIAL}` (next serial, zero-padded), plus `{KEY}` for
/// any key in my exchange values.
#[derive(Debug, Clone)]
pub struct Templates {
    pub cq: String,
    pub answer: String,
    pub exchange: String,
    pub agn: String,
    pub sign_off: String,
}

impl Templates {
    pub fn casual() -> Self {
        Self {
            cq: "CQ CQ CQ DE {MYCALL} {MYCALL} {MYCALL} K".into(),
            answer: "{CALL} DE {MYCALL} {MYCALL} K".into(),
            exchange: "{CALL} DE {MYCALL} UR RST {RST} {RST} {EXCH} {CALL} DE {MYCALL} K".into(),
            agn: "{CALL} DE {MYCALL} AGN AGN PSE K".into(),
            sign_off: "{CALL} QSL TU 73 DE {MYCALL} SK".into(),
        }
    }

    pub fn field_day() -> Self {
        Self {
            cq: "CQ FD CQ FD DE {MYCALL} {MYCALL} K".into(),
            answer: "{CALL} DE {MYCALL} {MYCALL} K".into(),
            exchange: "{CALL} DE {MYCALL} {EXCH} {EXCH} K".into(),
            agn: "{CALL} DE {MYCALL} AGN AGN PSE K".into(),
            sign_off: "{CALL} QSL TU 73 DE {MYCALL} K".into(),
        }
    }

    /// The default set for a schema (falls back to the casual set).
    pub fn for_schema(schema: &ExchangeSchema) -> Self {
        if schema.name == "fieldday" {
            Self::field_day()
        } else {
            Self::casual()
        }
    }
}

// ---- Actions and states ---------------------------------------------------

/// What the machine wants the engine to do. The machine never touches TX or
/// the logbook itself — this crate stays pure.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Key this text (already template-rendered).
    SendText(String),
    /// Both exchanges are validated — log the contact. `exchange` is the
    /// peer's fields in schema order, `(key, value)`.
    LogQso {
        call: String,
        exchange: Vec<(String, String)>,
    },
    /// The session gave up (max repeats reached). A runner is already back in
    /// `CallingCq` when this is observed; S&P is back in `Idle`.
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqState {
    /// Monitoring only. Decoded text accumulates for the UI, but no pattern
    /// ever transitions out of Idle — the operator must start a session.
    Idle,
    /// (Run) my CQ is out; watching for a directed answer or a doubled call.
    CallingCq,
    /// (S&P) I called a station; awaiting their reply with exchange.
    Answering,
    /// My exchange is out; awaiting the peer's exchange (run) or their
    /// TU/QSL confirmation (S&P).
    ExchangeSent,
    /// Contact logged and my closing sent; lingering for the peer's 73.
    Confirmed,
    /// Terminal. The operator (or engine policy) starts the next session.
    Done,
}

// ---- Token layer ----------------------------------------------------------

/// One whitespace-delimited token of the decoded window, uppercased, edge
/// punctuation stripped, per-char confidences kept aligned.
#[derive(Debug, Clone)]
struct Tok {
    text: String,
    conf: Vec<f32>,
}

impl Tok {
    fn mean_conf(&self) -> f32 {
        if self.conf.is_empty() {
            0.0
        } else {
            self.conf.iter().sum::<f32>() / self.conf.len() as f32
        }
    }
}

const EDGE_PUNCT: &[char] = &['?', '.', ',', ':', ';', '!', '"', '\'', '(', ')'];

fn flush_tok(toks: &mut Vec<Tok>, mut t: Tok) {
    while t.text.ends_with(EDGE_PUNCT) {
        t.text.pop();
        t.conf.pop();
    }
    while t.text.starts_with(EDGE_PUNCT) {
        t.text.remove(0);
        t.conf.remove(0);
    }
    if !t.text.is_empty() {
        toks.push(t);
    }
}

/// Tokenize the window. A trailing token NOT followed by whitespace is still
/// mid-decode (`W1A` may become `W1AW` two characters from now) and is
/// deliberately dropped until its terminator arrives — RTTY lines end in
/// CR/LF, so complete traffic always terminates.
fn tokenize(window: &[(char, f32)]) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut cur = Tok {
        text: String::new(),
        conf: Vec::new(),
    };
    for &(c, f) in window {
        if c.is_whitespace() {
            if !cur.text.is_empty() {
                flush_tok(&mut toks, std::mem::replace(&mut cur, Tok {
                    text: String::new(),
                    conf: Vec::new(),
                }));
            }
        } else if c.is_ascii_graphic() {
            cur.text.push(c.to_ascii_uppercase());
            cur.conf.push(f);
        }
        // other control chars (BEL from FIGS-S garble, NUL) are dropped
    }
    toks
}

/// Prosigns/abbreviations that are never callsigns or exchange values.
const KEYWORDS: &[&str] = &[
    "DE", "CQ", "QRZ", "UR", "RST", "TU", "QSL", "PSE", "AGN", "RPT", "K", "KN", "SK", "BK", "R",
    "ES", "HW", "FB", "OM", "GL", "GM", "GA", "GE", "73", "88", "TEST", "FD", "WFD", "NR", "NAME",
    "QTH", "BTU", "EE", "UE", "NW", "NOW",
];

fn is_keyword(t: &str) -> bool {
    KEYWORDS.contains(&t)
}

/// `TU`/`QSL`/`73`/`SK` — plus `UE`, which is `73` printed on the letters
/// plane after a lost FIGS.
const SIGN_OFF: &[&str] = &["TU", "QSL", "73", "SK", "UE"];

const AGN_WORDS: &[&str] = &["AGN", "RPT"];

/// Could this token be a callsign? At least one digit, two letters, 3–10
/// chars of alnum or `/`, not a known prosign, and not an RST (`5NN` is a
/// doubled contest report, never a doubled callsign).
fn plausible_call(t: &str) -> bool {
    let n = t.len();
    (3..=10).contains(&n)
        && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '/')
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2
        && !is_keyword(t)
        && normalize_rst(t).is_none()
}

/// Index in `long` whose removal yields `short` (`long.len() == short.len()+1`).
fn one_deleted(long: &[char], short: &[char]) -> Option<usize> {
    let mut i = 0;
    while i < short.len() && long[i] == short[i] {
        i += 1;
    }
    if long[i + 1..] == short[i..] {
        Some(i)
    } else {
        None
    }
}

/// Case-insensitive callsign match with one character of fuzz, gated by the
/// demodulator's per-char confidence:
/// * exact match — always;
/// * one substituted char — only if that char decoded with LOW confidence
///   (a high-confidence different character is a different station);
/// * one dropped char — allowed (nothing decoded, nothing to gate on);
/// * one inserted char — only if the inserted char is low-confidence.
///
/// Fuzz needs 4+ characters of expected call to bite on.
fn call_matches(expected: &str, tok: &Tok) -> bool {
    let e: Vec<char> = expected.trim().to_ascii_uppercase().chars().collect();
    let g: Vec<char> = tok.text.chars().collect();
    if e == g {
        return true;
    }
    if e.len() < 4 {
        return false;
    }
    if e.len() == g.len() {
        let diffs: Vec<usize> = (0..e.len()).filter(|&i| e[i] != g[i]).collect();
        return diffs.len() == 1
            && tok.conf.get(diffs[0]).is_none_or(|&c| c < HIGH_CONF);
    }
    if g.len() + 1 == e.len() {
        return one_deleted(&e, &g).is_some();
    }
    if g.len() == e.len() + 1 {
        if let Some(pos) = one_deleted(&g, &e) {
            return tok.conf.get(pos).is_none_or(|&c| c < HIGH_CONF);
        }
    }
    false
}

// ---- Garble normalization -------------------------------------------------

/// The digit a letter garbles FROM when a FIGS shift is lost — the ITA2
/// shared-code pairs (the QWERTYUIOP row maps to 1234567890).
fn garble_digit(c: char) -> Option<char> {
    match c {
        'Q' => Some('1'),
        'W' => Some('2'),
        'E' => Some('3'),
        'R' => Some('4'),
        'T' => Some('5'),
        'Y' => Some('6'),
        'U' => Some('7'),
        'I' => Some('8'),
        'O' => Some('9'),
        'P' => Some('0'),
        _ => None,
    }
}

/// Normalize a token to an RST if possible: digits pass through, `N` is the 9
/// cut number (`5NN`), and the QWERTYUIOP letters map back through the lost-
/// FIGS garble (`TOO` → `599`). Valid RST = 3 digits, readability 1–5,
/// strength/tone 1–9.
fn normalize_rst(t: &str) -> Option<String> {
    if t.len() != 3 {
        return None;
    }
    let mut out = String::new();
    for c in t.chars() {
        let d = if c.is_ascii_digit() {
            c
        } else if c == 'N' {
            '9'
        } else {
            garble_digit(c)?
        };
        out.push(d);
    }
    let b = out.as_bytes();
    if (b'1'..=b'5').contains(&b[0]) && b[1] != b'0' && b[2] != b'0' {
        Some(out)
    } else {
        None
    }
}

/// Normalize a token to a Field Day class: 1–2 digits (garble tolerated) +
/// one legal class letter. `garble` enables the lost-FIGS letter→digit map on
/// the digit part (`EA` → `3A`) — the caller tries literal digits first.
fn normalize_class(t: &str, letters: &str, garble: bool) -> Option<String> {
    let ch: Vec<char> = t.chars().collect();
    if !(2..=3).contains(&ch.len()) {
        return None;
    }
    let last = ch[ch.len() - 1];
    if !letters.contains(last) {
        return None;
    }
    let mut digits = String::new();
    for &c in &ch[..ch.len() - 1] {
        let d = if c.is_ascii_digit() {
            c
        } else if garble {
            garble_digit(c)?
        } else {
            return None;
        };
        digits.push(d);
    }
    Some(format!("{digits}{last}"))
}

/// Normalize a token to a contest serial: 1–4 digits after garble mapping.
fn normalize_serial(t: &str) -> Option<String> {
    if !(1..=4).contains(&t.len()) {
        return None;
    }
    t.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                Some(c)
            } else {
                garble_digit(c)
            }
        })
        .collect()
}

// ---- CQ detection (UI surface) --------------------------------------------

/// Find the newest `CQ ... DE <CALL>` in a stretch of decoded text — for the
/// UI to surface a clickable CQ. This NEVER drives the machine (the operator
/// clicking the surfaced call and the UI calling [`RttySeq::answer`] is the
/// human gate).
pub fn find_cq(text: &str) -> Option<String> {
    let mut window: Vec<(char, f32)> = text.chars().map(|c| (c, 1.0)).collect();
    window.push((' ', 1.0)); // explicit text is complete — terminate the tail
    let toks = tokenize(&window);
    let mut found = None;
    for i in 0..toks.len() {
        if toks[i].text != "CQ" {
            continue;
        }
        // "CQ CQ CQ DE W1AW", "CQ FD DE W1AW", "CQ TEST DE W1AW"
        for j in i + 1..(i + 4).min(toks.len()) {
            if toks[j].text == "DE" {
                if let Some(t) = toks.get(j + 1) {
                    if plausible_call(&t.text) {
                        found = Some(t.text.clone());
                    }
                }
                break;
            }
        }
    }
    found
}

// ---- The machine ----------------------------------------------------------

/// The RTTY QSO auto-sequencer. Feed it decoded characters and a clock; drain
/// [`Action`]s. See the module docs for the state flow.
#[derive(Debug)]
pub struct RttySeq {
    mycall: String,
    schema: ExchangeSchema,
    pub templates: Templates,
    pub cfg: SeqConfig,
    /// My own exchange values, `(key, value)` — e.g. `[("RST","599"),
    /// ("NAME","SETH"),("QTH","MADISON")]` or `[("CLASS","2A"),
    /// ("SECTION","WI")]`. Feeds `{EXCH}`/`{RST}`/`{KEY}` substitution.
    my_exchange: Vec<(String, String)>,
    state: SeqState,
    /// true = this session started with my CQ (run); false = S&P.
    running: bool,
    peer: Option<String>,
    peer_fields: Vec<(String, String)>,
    window: Vec<(char, f32)>,
    actions: Vec<Action>,
    last_sent: String,
    wait_since: u64,
    repeats: u32,
    /// Next contest serial. Present per the blueprint but unexposed — no
    /// shipped schema sends it; `{SERIAL}` in a template reads it and each
    /// logged QSO advances it.
    next_serial: u32,
}

impl RttySeq {
    pub fn new(mycall: &str, schema: ExchangeSchema, my_exchange: &[(&str, &str)]) -> Self {
        Self {
            mycall: mycall.trim().to_ascii_uppercase(),
            templates: Templates::for_schema(&schema),
            schema,
            cfg: SeqConfig::default(),
            my_exchange: my_exchange
                .iter()
                .map(|(k, v)| (k.to_string(), v.trim().to_ascii_uppercase()))
                .collect(),
            state: SeqState::Idle,
            running: false,
            peer: None,
            peer_fields: Vec::new(),
            window: Vec::new(),
            actions: Vec::new(),
            last_sent: String::new(),
            wait_since: 0,
            repeats: 0,
            next_serial: 1,
        }
    }

    pub fn state(&self) -> SeqState {
        self.state
    }

    pub fn peer(&self) -> Option<&str> {
        self.peer.as_deref()
    }

    pub fn peer_exchange(&self) -> &[(String, String)] {
        &self.peer_fields
    }

    /// The rolling decoded window as text (for the UI RX pane / [`find_cq`]).
    pub fn window_text(&self) -> String {
        self.window.iter().map(|&(c, _)| c).collect()
    }

    /// Drain the pending actions.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    // -- Operator entry points (the human-initiate gate) --

    /// Operator starts running: send CQ and watch for answers.
    pub fn start_cq(&mut self, now_ms: u64) {
        self.running = true;
        self.peer = None;
        self.peer_fields.clear();
        self.window.clear();
        self.repeats = 0;
        let tpl = self.templates.cq.clone();
        let text = self.render(&tpl);
        self.send(text, now_ms);
        self.state = SeqState::CallingCq;
    }

    /// Operator answers a heard CQ (search & pounce).
    pub fn answer(&mut self, call: &str, now_ms: u64) {
        self.running = false;
        self.peer = Some(call.trim().to_ascii_uppercase());
        self.peer_fields.clear();
        self.window.clear();
        self.repeats = 0;
        let tpl = self.templates.answer.clone();
        let text = self.render(&tpl);
        self.send(text, now_ms);
        self.state = SeqState::Answering;
    }

    /// Operator kills the session. Silent — no [`Action::Abort`] (they know).
    pub fn abort(&mut self) {
        self.state = SeqState::Idle;
        self.peer = None;
        self.peer_fields.clear();
        self.window.clear();
        self.repeats = 0;
    }

    // -- Engine hooks --

    /// Decoded characters arrived.
    pub fn feed(&mut self, chars: &[DecodedChar], now_ms: u64) {
        for d in chars {
            self.window.push((d.ch, d.confidence));
        }
        if self.window.len() > WINDOW_CAP {
            let excess = self.window.len() - WINDOW_CAP;
            self.window.drain(..excess);
        }
        self.advance(now_ms);
    }

    /// Convenience for text without per-char confidence (treated as certain).
    pub fn feed_text(&mut self, text: &str, now_ms: u64) {
        let chars: Vec<DecodedChar> = text
            .chars()
            .map(|c| DecodedChar {
                ch: c,
                confidence: 1.0,
            })
            .collect();
        self.feed(&chars, now_ms);
    }

    /// The engine finished keying the last `SendText` — restart the reply
    /// timer from now (an RTTY over takes many seconds of TX the machine
    /// cannot see). In `Confirmed`, the closing went out: the QSO is done.
    pub fn on_tx_complete(&mut self, now_ms: u64) {
        self.wait_since = now_ms;
        if self.state == SeqState::Confirmed {
            self.state = SeqState::Done;
        }
    }

    /// Clock tick: timeout → AGN/repeat, then abort after `max_repeats`.
    pub fn tick(&mut self, now_ms: u64) {
        if matches!(self.state, SeqState::Idle | SeqState::Done) {
            return;
        }
        if now_ms.saturating_sub(self.wait_since) < self.cfg.timeout_ms {
            return;
        }
        match self.state {
            SeqState::CallingCq => {
                // Nobody answered — CQ again. Never aborts; the operator owns
                // stopping a run.
                let tpl = self.templates.cq.clone();
                let text = self.render(&tpl);
                self.send(text, now_ms);
            }
            SeqState::Confirmed => self.state = SeqState::Done,
            SeqState::Answering | SeqState::ExchangeSent => {
                self.repeats += 1;
                if self.repeats >= self.cfg.max_repeats {
                    self.actions.push(Action::Abort);
                    if self.running {
                        self.start_cq(now_ms);
                    } else {
                        self.abort();
                    }
                } else if self.state == SeqState::Answering && !self.heard_mycall() {
                    // They never came back to us — call again.
                    let tpl = self.templates.answer.clone();
                    let text = self.render(&tpl);
                    self.send(text, now_ms);
                } else {
                    // In-QSO no-copy — ask for a repeat.
                    let tpl = self.templates.agn.clone();
                    let text = self.render(&tpl);
                    self.send(text, now_ms);
                }
            }
            SeqState::Idle | SeqState::Done => unreachable!(),
        }
    }

    // -- Internals --

    fn send(&mut self, text: String, now_ms: u64) {
        self.last_sent = text.clone();
        self.actions.push(Action::SendText(text));
        self.wait_since = now_ms;
    }

    fn heard_mycall(&self) -> bool {
        tokenize(&self.window)
            .iter()
            .any(|t| call_matches(&self.mycall, t))
    }

    fn my_field(&self, key: &str) -> Option<&str> {
        self.my_exchange
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// My exchange as sent on the air: labeled fields as `LABEL VALUE`, bare
    /// fields as `VALUE`, RST excluded (it has its own `{RST}` slot).
    fn my_exch_string(&self) -> String {
        let mut parts = Vec::new();
        for f in self.schema.fields {
            if f.kind == FieldKind::Rst {
                continue;
            }
            if let Some(v) = self.my_field(f.key) {
                match f.label {
                    Some(l) => parts.push(format!("{l} {v}")),
                    None => parts.push(v.to_string()),
                }
            }
        }
        parts.join(" ")
    }

    fn render(&self, tpl: &str) -> String {
        let mut s = tpl.to_string();
        s = s.replace("{MYCALL}", &self.mycall);
        s = s.replace("{CALL}", self.peer.as_deref().unwrap_or("?"));
        s = s.replace("{RST}", self.my_field("RST").unwrap_or("599"));
        s = s.replace("{EXCH}", &self.my_exch_string());
        s = s.replace("{SERIAL}", &format!("{:03}", self.next_serial));
        for (k, v) in &self.my_exchange {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }

    fn advance(&mut self, now_ms: u64) {
        let toks = tokenize(&self.window);
        match self.state {
            // Never self-start (ARRL 6.4): Idle only accumulates text.
            SeqState::Idle | SeqState::Done => {}
            SeqState::CallingCq => {
                if let Some(call) = self.detect_answer(&toks) {
                    self.peer = Some(call);
                    self.window.clear();
                    self.repeats = 0;
                    let tpl = self.templates.exchange.clone();
                    let text = self.render(&tpl);
                    self.send(text, now_ms);
                    self.state = SeqState::ExchangeSent;
                }
            }
            SeqState::Answering => {
                // Only lines addressed to us count — the runner may be
                // working someone else.
                if !toks.iter().any(|t| call_matches(&self.mycall, t)) {
                    return;
                }
                if let Some(fields) = self.parse_exchange(&toks) {
                    self.peer_fields = fields;
                    self.window.clear();
                    self.repeats = 0;
                    let tpl = self.templates.exchange.clone();
                    let text = self.render(&tpl);
                    self.send(text, now_ms);
                    self.state = SeqState::ExchangeSent;
                }
            }
            SeqState::ExchangeSent => {
                if self.heard_word(&toks, AGN_WORDS) {
                    // Peer wants a repeat — resend, don't count it against
                    // the abort budget (they are alive).
                    let text = self.last_sent.clone();
                    self.window.clear();
                    self.send(text, now_ms);
                    return;
                }
                if self.running {
                    // Awaiting the caller's exchange.
                    if let Some(fields) = self.parse_exchange(&toks) {
                        self.peer_fields = fields;
                        self.log_and_close(now_ms);
                    }
                } else {
                    // Awaiting the runner's confirmation.
                    if self.heard_word(&toks, SIGN_OFF) {
                        self.log_and_close(now_ms);
                    }
                }
            }
            SeqState::Confirmed => {
                if self.heard_word(&toks, SIGN_OFF) {
                    self.state = SeqState::Done;
                }
            }
        }
    }

    fn heard_word(&self, toks: &[Tok], set: &[&str]) -> bool {
        toks.iter()
            .any(|t| set.contains(&t.text.as_str()) && t.mean_conf() >= MIN_TOKEN_CONF)
    }

    /// Both exchanges validated: log, close, linger for the peer's 73.
    fn log_and_close(&mut self, now_ms: u64) {
        let call = self.peer.clone().unwrap_or_default();
        self.actions.push(Action::LogQso {
            call,
            exchange: self.peer_fields.clone(),
        });
        self.next_serial += 1;
        self.window.clear();
        self.repeats = 0;
        let tpl = self.templates.sign_off.clone();
        let text = self.render(&tpl);
        self.send(text, now_ms);
        self.state = SeqState::Confirmed;
    }

    fn acceptable_call(&self, t: &Tok) -> bool {
        plausible_call(&t.text) && t.mean_conf() >= MIN_TOKEN_CONF
    }

    /// Who answered my CQ? Two forms, newest match wins:
    /// (a) directed — `<MYCALL> DE <CALL>` (my call fuzzy-matched);
    /// (b) pileup style — a doubled bare call (`W1XYZ W1XYZ`), as long as it
    ///     is not part of someone ELSE's CQ or directed answer.
    fn detect_answer(&self, toks: &[Tok]) -> Option<String> {
        let mut found = None;
        for i in 0..toks.len() {
            if call_matches(&self.mycall, &toks[i])
                && i + 2 < toks.len()
                && toks[i + 1].text == "DE"
                && self.acceptable_call(&toks[i + 2])
            {
                found = Some(toks[i + 2].text.clone());
                continue;
            }
            if i + 1 < toks.len()
                && self.acceptable_call(&toks[i])
                && toks[i + 1].text == toks[i].text
                && !call_matches(&self.mycall, &toks[i])
            {
                let near_cq = toks[i.saturating_sub(3)..i].iter().any(|t| t.text == "CQ");
                let answering_other = i >= 2
                    && toks[i - 1].text == "DE"
                    && !call_matches(&self.mycall, &toks[i - 2]);
                if !near_cq && !answering_other {
                    found = Some(toks[i].text.clone());
                }
            }
        }
        found
    }

    /// A token may carry an exchange value if it is unclaimed, confident
    /// enough, and not a prosign or either station's callsign.
    fn usable(&self, toks: &[Tok], claimed: &[bool], i: usize) -> bool {
        !claimed[i]
            && !is_keyword(&toks[i].text)
            && toks[i].mean_conf() >= MIN_TOKEN_CONF
            && !call_matches(&self.mycall, &toks[i])
            && self.peer.as_deref() != Some(toks[i].text.as_str())
    }

    /// Tolerant table-driven exchange parse: each schema field claims the
    /// first token that matches its kind (labeled Word fields claim the first
    /// acceptable word AFTER their label). Repeats of a claimed value
    /// (`599 599`) are claimed with it so a later Serial field cannot steal
    /// one. `None` until every REQUIRED field has been copied — fields
    /// accumulate across transmissions because the window survives an AGN
    /// cycle.
    fn parse_exchange(&self, toks: &[Tok]) -> Option<Vec<(String, String)>> {
        let mut claimed = vec![false; toks.len()];
        let mut out = Vec::new();
        for f in self.schema.fields {
            let mut hit: Option<(usize, String)> = None;
            match f.kind {
                FieldKind::Rst => {
                    // Pass A: tokens that still contain a digit (599, 5NN).
                    // Pass B: full letters-plane garble (TOO).
                    for garbled in [false, true] {
                        if hit.is_some() {
                            break;
                        }
                        for i in 0..toks.len() {
                            if !self.usable(toks, &claimed, i) {
                                continue;
                            }
                            let has_digit = toks[i].text.chars().any(|c| c.is_ascii_digit());
                            if has_digit == garbled {
                                continue;
                            }
                            if let Some(v) = normalize_rst(&toks[i].text) {
                                hit = Some((i, v));
                                break;
                            }
                        }
                    }
                }
                FieldKind::FdClass { letters } => {
                    // Literal digits first; garble mapping only as a fallback
                    // so real copy is never mis-normalized.
                    for garbled in [false, true] {
                        if hit.is_some() {
                            break;
                        }
                        for i in 0..toks.len() {
                            if !self.usable(toks, &claimed, i) {
                                continue;
                            }
                            if let Some(v) = normalize_class(&toks[i].text, letters, garbled) {
                                hit = Some((i, v));
                                break;
                            }
                        }
                    }
                }
                FieldKind::Section => {
                    for i in 0..toks.len() {
                        if !self.usable(toks, &claimed, i) {
                            continue;
                        }
                        let t = toks[i].text.as_str();
                        if valid_section(t) || t == "MX" || t == "DX" {
                            hit = Some((i, t.to_string()));
                            break;
                        }
                    }
                }
                FieldKind::Serial => {
                    for i in 0..toks.len() {
                        if !self.usable(toks, &claimed, i) {
                            continue;
                        }
                        if let Some(v) = normalize_serial(&toks[i].text) {
                            hit = Some((i, v));
                            break;
                        }
                    }
                }
                FieldKind::Word => {
                    let label = f.label.unwrap_or(f.key);
                    if let Some(li) = toks.iter().position(|t| t.text == label) {
                        for i in li + 1..toks.len() {
                            if !self.usable(toks, &claimed, i) {
                                continue;
                            }
                            let t = &toks[i].text;
                            if t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()) {
                                hit = Some((i, t.clone()));
                                break;
                            }
                        }
                    }
                }
            }
            match hit {
                Some((idx, v)) => {
                    let text = toks[idx].text.clone();
                    for (i, t) in toks.iter().enumerate() {
                        if t.text == text {
                            claimed[i] = true;
                        }
                    }
                    out.push((f.key.to_string(), v));
                }
                None => {
                    if f.required {
                        return None;
                    }
                }
            }
        }
        Some(out)
    }
}

// ---- Tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MYCALL: &str = "KD9TAW";

    fn casual_seq() -> RttySeq {
        RttySeq::new(
            MYCALL,
            CASUAL,
            &[("RST", "599"), ("NAME", "SETH"), ("QTH", "MADISON")],
        )
    }

    fn fd_seq() -> RttySeq {
        RttySeq::new(MYCALL, FIELD_DAY, &[("CLASS", "2A"), ("SECTION", "WI")])
    }

    fn sends(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::SendText(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn logs(actions: &[Action]) -> Vec<(String, Vec<(String, String)>)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::LogQso { call, exchange } => Some((call.clone(), exchange.clone())),
                _ => None,
            })
            .collect()
    }

    fn field<'a>(exch: &'a [(String, String)], key: &str) -> Option<&'a str> {
        exch.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// DecodedChars at a uniform confidence.
    fn chars(text: &str, conf: f32) -> Vec<DecodedChar> {
        text.chars()
            .map(|ch| DecodedChar {
                ch,
                confidence: conf,
            })
            .collect()
    }

    // -- Human-initiate gate --

    #[test]
    fn never_self_starts() {
        let mut seq = casual_seq();
        seq.feed_text("CQ CQ CQ DE W1AW W1AW K\n", 0);
        seq.feed_text("KD9TAW DE W1AW UR RST 599 599 NAME BOB K\n", 1_000);
        seq.feed_text("W1AW W1AW\n", 2_000);
        assert_eq!(seq.state(), SeqState::Idle);
        assert!(seq.take_actions().is_empty(), "Idle must emit nothing");
        // ...but the UI can still surface the CQ for the operator to click.
        assert_eq!(find_cq(&seq.window_text()).as_deref(), Some("W1AW"));
    }

    #[test]
    fn tick_in_idle_is_silent() {
        let mut seq = casual_seq();
        seq.feed_text("CQ DE W1AW W1AW K\n", 0);
        seq.tick(120_000);
        assert_eq!(seq.state(), SeqState::Idle);
        assert!(seq.take_actions().is_empty());
    }

    // -- Happy paths, both schemas, both roles --

    #[test]
    fn runner_casual_full_qso() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        let a = seq.take_actions();
        assert_eq!(seq.state(), SeqState::CallingCq);
        let s = sends(&a);
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("CQ") && s[0].contains(MYCALL), "cq: {}", s[0]);

        // W1AW answers, directed.
        seq.feed_text("KD9TAW DE W1AW W1AW K\n", 10_000);
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        assert_eq!(seq.peer(), Some("W1AW"));
        let s = sends(&seq.take_actions());
        assert_eq!(s.len(), 1);
        assert!(
            s[0].contains("W1AW") && s[0].contains("599") && s[0].contains("NAME SETH"),
            "exchange: {}",
            s[0]
        );

        // His exchange comes back.
        seq.feed_text(
            "KD9TAW DE W1AW R UR RST 599 599 NAME BOB QTH BOSTON K\n",
            30_000,
        );
        assert_eq!(seq.state(), SeqState::Confirmed);
        let a = seq.take_actions();
        let l = logs(&a);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0, "W1AW");
        assert_eq!(field(&l[0].1, "RST"), Some("599"));
        assert_eq!(field(&l[0].1, "NAME"), Some("BOB"));
        assert_eq!(field(&l[0].1, "QTH"), Some("BOSTON"));
        let s = sends(&a);
        assert!(s[0].contains("TU") && s[0].contains("73"), "close: {}", s[0]);

        // His 73 ends it.
        seq.feed_text("73 SK\n", 45_000);
        assert_eq!(seq.state(), SeqState::Done);
    }

    #[test]
    fn runner_fieldday_full_qso() {
        let mut seq = fd_seq();
        seq.start_cq(0);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("CQ FD"), "fd cq: {}", s[0]);

        // Pileup-style doubled bare call (no DE).
        seq.feed_text("W1XYZ W1XYZ K\n", 10_000);
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        assert_eq!(seq.peer(), Some("W1XYZ"));
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("2A WI 2A WI"), "fd exchange: {}", s[0]);

        seq.feed_text("KD9TAW DE W1XYZ R 4A WI 4A WI K\n", 30_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let a = seq.take_actions();
        let l = logs(&a);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0, "W1XYZ");
        assert_eq!(field(&l[0].1, "CLASS"), Some("4A"));
        assert_eq!(field(&l[0].1, "SECTION"), Some("WI"));

        seq.feed_text("TU 73\n", 45_000);
        assert_eq!(seq.state(), SeqState::Done);
    }

    #[test]
    fn sp_casual_full_qso() {
        let mut seq = casual_seq();
        seq.answer("W1AW", 0);
        assert_eq!(seq.state(), SeqState::Answering);
        let s = sends(&seq.take_actions());
        assert!(
            s[0].contains("W1AW DE KD9TAW"),
            "answer: {}",
            s[0]
        );

        // The runner comes back with his exchange.
        seq.feed_text(
            "KD9TAW DE W1AW UR RST 599 599 NAME BOB QTH BOSTON HW KD9TAW DE W1AW KN\n",
            15_000,
        );
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("NAME SETH"), "my exchange: {}", s[0]);

        // He confirms; we log and send our closing.
        seq.feed_text("KD9TAW QSL TU 73 DE W1AW SK\n", 35_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let a = seq.take_actions();
        let l = logs(&a);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0, "W1AW");
        assert_eq!(field(&l[0].1, "RST"), Some("599"));
        assert_eq!(field(&l[0].1, "NAME"), Some("BOB"));
        assert!(!sends(&a).is_empty(), "a closing 73 goes out");

        // Closing keyed → done.
        seq.on_tx_complete(40_000);
        assert_eq!(seq.state(), SeqState::Done);
    }

    #[test]
    fn sp_fieldday_full_qso() {
        let mut seq = fd_seq();
        seq.answer("W9ABC", 0);
        seq.take_actions();

        seq.feed_text("KD9TAW DE W9ABC 3A IL 3A IL K\n", 15_000);
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("2A WI"), "my fd exchange: {}", s[0]);

        seq.feed_text("KD9TAW QSL TU DE W9ABC K\n", 35_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let l = logs(&seq.take_actions());
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0, "W9ABC");
        assert_eq!(field(&l[0].1, "CLASS"), Some("3A"));
        assert_eq!(field(&l[0].1, "SECTION"), Some("IL"));
    }

    // -- Garbled copy --

    #[test]
    fn figs_garble_rst_too_normalizes_to_599() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.feed_text("KD9TAW DE W1AW W1AW K\n", 10_000);
        seq.take_actions();
        // Lost FIGS: 599 prints as TOO on the letters plane.
        seq.feed_text("KD9TAW DE W1AW R UR RST TOO TOO NAME BOB K\n", 30_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let l = logs(&seq.take_actions());
        assert_eq!(field(&l[0].1, "RST"), Some("599"));
    }

    #[test]
    fn cut_number_5nn_normalizes_to_599() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.feed_text("KD9TAW DE W1AW W1AW K\n", 10_000);
        seq.take_actions();
        seq.feed_text("KD9TAW DE W1AW R 5NN 5NN NAME BOB K\n", 30_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let l = logs(&seq.take_actions());
        assert_eq!(field(&l[0].1, "RST"), Some("599"));
    }

    #[test]
    fn figs_garble_class_ea_normalizes_to_3a() {
        let mut seq = fd_seq();
        seq.start_cq(0);
        seq.feed_text("W5DEF W5DEF K\n", 10_000);
        seq.take_actions();
        // "3A" with a lost FIGS prints as "EA".
        seq.feed_text("KD9TAW DE W5DEF R EA WI EA WI K\n", 30_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let l = logs(&seq.take_actions());
        assert_eq!(field(&l[0].1, "CLASS"), Some("3A"));
        assert_eq!(field(&l[0].1, "SECTION"), Some("WI"));
    }

    #[test]
    fn fuzzed_mycall_low_confidence_matches() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        // "KD9TAW" copied as "KD9TAQ" — but the demod flagged the Q shaky.
        let mut line = chars("KD9TAQ DE W1AW K\n", 1.0);
        line[5].confidence = 0.3; // the Q
        seq.feed(&line, 10_000);
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        assert_eq!(seq.peer(), Some("W1AW"));
    }

    #[test]
    fn fuzzed_mycall_high_confidence_is_a_different_station() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        // Same garble, but every char decoded solid: that really is KD9TAQ —
        // someone else's QSO. Stay on CQ.
        seq.feed(&chars("KD9TAQ DE W1AW K\n", 1.0), 10_000);
        assert_eq!(seq.state(), SeqState::CallingCq);
        assert!(seq.take_actions().is_empty());
    }

    #[test]
    fn dropped_char_in_mycall_matches() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        // A dropped character never printed, so there is no confidence to
        // consult — one deletion is always forgiven.
        seq.feed_text("KD9TW DE W1AW K\n", 10_000);
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        assert_eq!(seq.peer(), Some("W1AW"));
    }

    #[test]
    fn noise_floor_junk_is_confidence_gated() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        // A doubled plausible call, but decoded at the noise floor.
        seq.feed(&chars("W1XXX W1XXX K\n", 0.1), 10_000);
        assert_eq!(seq.state(), SeqState::CallingCq);
        assert!(seq.take_actions().is_empty());
    }

    #[test]
    fn another_stations_cq_is_not_an_answer() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        seq.feed_text("CQ CQ DE W5XYZ W5XYZ K\n", 10_000);
        assert_eq!(seq.state(), SeqState::CallingCq, "his CQ is not my caller");
        // Nor is someone answering a third station.
        seq.feed_text("W9AAA DE W5BBB W5BBB K\n", 12_000);
        assert_eq!(seq.state(), SeqState::CallingCq);
        assert!(seq.take_actions().is_empty());
    }

    #[test]
    fn sp_ignores_runner_working_someone_else() {
        let mut seq = casual_seq();
        seq.answer("W1AW", 0);
        seq.take_actions();
        seq.feed_text("W9XYZ DE W1AW UR RST 599 599 NAME BOB K\n", 15_000);
        assert_eq!(seq.state(), SeqState::Answering, "that exchange was not for us");
        assert!(seq.take_actions().is_empty());
    }

    #[test]
    fn exchange_accumulates_across_transmissions() {
        let mut seq = fd_seq();
        seq.start_cq(0);
        seq.feed_text("W1XYZ W1XYZ K\n", 10_000);
        seq.take_actions();
        // First over: class copied, section lost in a burst of static.
        seq.feed_text("KD9TAW DE W1XYZ R 4A K\n", 30_000);
        assert_eq!(seq.state(), SeqState::ExchangeSent, "incomplete: keep waiting");
        // Timeout → AGN, but the window (and the copied 4A) survives.
        seq.tick(65_000);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("AGN"), "agn: {}", s[0]);
        // Second over fills in the section — the QSO completes.
        seq.feed_text("4A WI K\n", 80_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let l = logs(&seq.take_actions());
        assert_eq!(field(&l[0].1, "CLASS"), Some("4A"));
        assert_eq!(field(&l[0].1, "SECTION"), Some("WI"));
    }

    // -- Timeouts, AGN, abort --

    #[test]
    fn runner_timeout_sends_agn_then_aborts_back_to_cq() {
        let mut seq = fd_seq();
        seq.start_cq(0);
        seq.feed_text("W1XYZ W1XYZ K\n", 1_000);
        seq.take_actions();
        assert_eq!(seq.state(), SeqState::ExchangeSent);

        // Cycles 1 and 2: AGN. Cycle 3 (max_repeats): abort → back to CQ.
        seq.tick(40_000);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("AGN"), "first agn: {}", s[0]);
        seq.tick(80_000);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("AGN"), "second agn: {}", s[0]);
        seq.tick(120_000);
        let a = seq.take_actions();
        assert!(a.contains(&Action::Abort), "third cycle aborts");
        assert_eq!(seq.state(), SeqState::CallingCq, "runner falls back to CQ");
        assert!(
            sends(&a).iter().any(|t| t.contains("CQ")),
            "a fresh CQ goes out"
        );
        assert_eq!(seq.peer(), None);
    }

    #[test]
    fn sp_timeout_repeats_call_then_aborts_to_idle() {
        let mut seq = casual_seq();
        seq.answer("W1AW", 0);
        seq.take_actions();

        // Nothing heard at all → repeat the answering call, not AGN.
        seq.tick(40_000);
        let s = sends(&seq.take_actions());
        assert!(
            s[0].contains("W1AW DE KD9TAW"),
            "repeat the call: {}",
            s[0]
        );
        seq.tick(80_000);
        seq.take_actions();
        seq.tick(120_000);
        let a = seq.take_actions();
        assert!(a.contains(&Action::Abort));
        assert_eq!(seq.state(), SeqState::Idle, "S&P gives up to Idle");
    }

    #[test]
    fn peer_agn_request_resends_exchange_without_burning_budget() {
        let mut seq = fd_seq();
        seq.answer("W9ABC", 0);
        seq.feed_text("KD9TAW DE W9ABC 3A IL 3A IL K\n", 15_000);
        let sent = sends(&seq.take_actions());
        let my_exchange = sent.last().unwrap().clone();

        seq.feed_text("KD9TAW DE W9ABC AGN AGN PSE K\n", 40_000);
        let s = sends(&seq.take_actions());
        assert_eq!(s, vec![my_exchange], "the same exchange goes out again");
        assert_eq!(seq.state(), SeqState::ExchangeSent);
    }

    #[test]
    fn cq_repeats_forever_without_abort() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        for n in 1..=10u64 {
            seq.tick(n * 40_000);
            let a = seq.take_actions();
            assert!(!a.contains(&Action::Abort), "CQ never aborts");
            assert!(sends(&a)[0].contains("CQ"));
        }
        assert_eq!(seq.state(), SeqState::CallingCq);
    }

    #[test]
    fn on_tx_complete_restarts_the_reply_timer() {
        let mut seq = fd_seq();
        seq.start_cq(0);
        seq.feed_text("W1XYZ W1XYZ K\n", 0);
        seq.take_actions();
        // The exchange took 25 s to key; the engine reports TX end.
        seq.on_tx_complete(25_000);
        seq.tick(30_500); // 30.5 s after emission, but only 5.5 s after TX end
        assert!(seq.take_actions().is_empty(), "timer runs from tx end");
        seq.tick(56_000);
        let s = sends(&seq.take_actions());
        assert!(s[0].contains("AGN"));
    }

    #[test]
    fn operator_abort_is_silent() {
        let mut seq = casual_seq();
        seq.answer("W1AW", 0);
        seq.take_actions();
        seq.abort();
        assert_eq!(seq.state(), SeqState::Idle);
        assert!(seq.take_actions().is_empty());
    }

    // -- The contest seam --

    #[test]
    fn serial_schema_parses_without_engine_changes() {
        static SERIAL_SCHEMA: ExchangeSchema = ExchangeSchema {
            name: "test-serial",
            fields: &[
                FieldSpec {
                    key: "RST",
                    label: None,
                    required: true,
                    kind: FieldKind::Rst,
                },
                FieldSpec {
                    key: "SERIAL",
                    label: None,
                    required: true,
                    kind: FieldKind::Serial,
                },
            ],
        };
        let mut seq = RttySeq::new(MYCALL, SERIAL_SCHEMA, &[("RST", "599")]);
        seq.start_cq(0);
        seq.feed_text("KD9TAW DE W1AW W1AW K\n", 10_000);
        seq.take_actions();
        // Both 599 repeats are claimed by RST, so the serial takes 001.
        seq.feed_text("KD9TAW DE W1AW 599 599 001 001 K\n", 30_000);
        assert_eq!(seq.state(), SeqState::Confirmed);
        let l = logs(&seq.take_actions());
        assert_eq!(field(&l[0].1, "RST"), Some("599"));
        assert_eq!(field(&l[0].1, "SERIAL"), Some("001"));
    }

    // -- Pattern helpers --

    #[test]
    fn find_cq_forms() {
        assert_eq!(find_cq("CQ CQ CQ DE W1AW W1AW K").as_deref(), Some("W1AW"));
        assert_eq!(find_cq("CQ FD DE W9XYZ W9XYZ").as_deref(), Some("W9XYZ"));
        assert_eq!(
            find_cq("CQ DE W1AW K\nCQ CQ DE K5LLL K").as_deref(),
            Some("K5LLL"),
            "newest CQ wins"
        );
        assert_eq!(find_cq("KD9TAW DE W1AW UR 599"), None);
        assert_eq!(find_cq("CQ CQ 599 599"), None, "prosigns are not calls");
    }

    #[test]
    fn partial_trailing_token_is_not_matched_early() {
        let mut seq = casual_seq();
        seq.start_cq(0);
        seq.take_actions();
        // The caller's line is still mid-decode: "W1A" must not be taken as a
        // (fuzzed) call before its last characters arrive.
        seq.feed_text("KD9TAW DE W1A", 10_000);
        assert_eq!(seq.state(), SeqState::CallingCq);
        seq.feed_text("W K\n", 10_500);
        assert_eq!(seq.state(), SeqState::ExchangeSent);
        assert_eq!(seq.peer(), Some("W1AW"));
    }

    #[test]
    fn rst_normalization_table() {
        assert_eq!(normalize_rst("599").as_deref(), Some("599"));
        assert_eq!(normalize_rst("5NN").as_deref(), Some("599"));
        assert_eq!(normalize_rst("TOO").as_deref(), Some("599"));
        assert_eq!(normalize_rst("T99").as_deref(), Some("599"));
        assert_eq!(normalize_rst("579").as_deref(), Some("579"));
        assert_eq!(normalize_rst("PET"), None, "035 is not a valid RST");
        assert_eq!(normalize_rst("899"), None, "readability tops out at 5");
        assert_eq!(normalize_rst("59"), None);
        assert_eq!(normalize_rst("BOB"), None, "B is not a garble digit");
    }
}
