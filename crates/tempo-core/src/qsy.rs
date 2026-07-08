//! Coordinated QSY ("move together") — a **legal, in-the-clear** low-probability-
//! of-intercept aid for off-grid nets.
//!
//! Two stations already in contact agree to step to a different calling channel
//! together, with the move **announced as plain text** in the over. This is the
//! clearly-legal subset of frequency agility: it is NOT encryption, NOT a secret
//! hop, and NOT anything that obscures meaning (which FCC §97.113(a)(4) forbids).
//! The directive, the algorithm (this file, GPLv3), and the band-plan channel
//! tokens are all public and human-readable, so a capable listener with a wide
//! receiver can still find and follow you. It only shakes a **casual** scanner
//! parked on the old frequency — an operational anti-QRM win and modest obscurity,
//! not privacy. The 10-minute station ID and all message content stay in the clear.
//!
//! ## Directive grammar (rides the existing free-text broadcast path)
//!
//! The directive is the *body* of an open broadcast — `DE <MYCALL> <directive>`
//! (see [`crate::inbox::broadcast_text`]) — chunked through [`crate::text`] like
//! any free-text message. Two forms, drawn only from the FT1 free-text charset
//! (`0-9 A-Z space + - . /`):
//!
//! - `QSY <TOKEN> <CODE>` — move to channel `<TOKEN>` (a band-plan channel id,
//!   e.g. `70CM`, `20M`, `2M-FM`) on the absolute UTC slot encoded by `<CODE>`.
//! - `QSY HOME` — return to the channel the conversation started on (re-rendezvous).
//!
//! `<CODE>` is the absolute slot index modulo [`SLOT_MOD`] in base-36 (≤3 chars).
//! Both stations share the UTC [`crate::timing::SlotClock`], so each recovers the
//! same absolute target slot (the residue class is far wider than any decode
//! latency or clock skew) and they retune on the **same** boundary — landing
//! together. See [`encode_slot`] / [`decode_slot`].
//!
//! ## Roles (no negotiation round-trip)
//!
//! Of the two stations, the lexicographically-smaller callsign is the
//! **initiator** ([`is_initiator`]); it announces moves on its own cadence. The
//! other is the **follower**: it auto-follows a directive from its partner.
//! Either operator can override (move-now / pause / stop) via the [`Roamer`].
//!
//! Everything here is inert until the operator opts in — the engine only drives
//! the [`Roamer`] while the `qsy_enabled` setting is true.

use crate::text::{self, Reassembler};

/// Keyword that opens a QSY directive.
pub const KEYWORD: &str = "QSY";
/// Token meaning "return to the home channel".
pub const HOME: &str = "HOME";

/// Modulus for the compact slot code: 36³ = 46656 slots. At FT1's 4 s slot that
/// is ~25.9 h of unambiguous range (DX1's 15 s slot → ~194 h) — vastly larger
/// than any realistic decode latency or clock skew, so the absolute target slot
/// is recovered exactly from 3 base-36 digits.
pub const SLOT_MOD: u64 = 36 * 36 * 36;

/// How far ahead (in slots) an announced move is scheduled — enough air time to
/// send the (multi-chunk) directive and for the partner to decode + retune before
/// the boundary.
pub const LEAD_SLOTS: u64 = 8;

/// Consecutive RX slots without hearing the partner that trigger a fall-back to
/// the home channel (a failed/missed move → both re-rendezvous at home).
pub const SILENCE_HOME_OVERS: u64 = 8;

/// Default announce cadence: hop every this-many of the initiator's TX overs.
/// Conservative (never per-over) so it reads as a normal QSY, not spread-spectrum.
pub const DEFAULT_CADENCE: u64 = 6;

const BASE36: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Encode a slot index as a 3-char base-36 code (the absolute slot mod [`SLOT_MOD`]).
pub fn encode_slot(slot: u64) -> String {
    let mut n = slot % SLOT_MOD;
    let mut buf = [b'0'; 3];
    for b in buf.iter_mut().rev() {
        *b = BASE36[(n % 36) as usize];
        n /= 36;
    }
    String::from_utf8(buf.to_vec()).expect("base-36 digits are ASCII")
}

/// Recover the absolute target slot from its [`encode_slot`] code, choosing the
/// slot in the residue class **nearest** to `now_slot` (unambiguous because
/// [`SLOT_MOD`] ≫ any lead/skew). Returns `None` if the code is malformed.
pub fn decode_slot(code: &str, now_slot: u64) -> Option<u64> {
    if code.is_empty() || code.len() > 3 {
        return None;
    }
    let mut r: u64 = 0;
    for c in code.chars() {
        r = r * 36 + c.to_digit(36)? as u64;
    }
    if r >= SLOT_MOD {
        return None;
    }
    let now = now_slot as i128;
    let m = SLOT_MOD as i128;
    let base = now - now.rem_euclid(m) + r as i128; // residue r in now's cycle
    [base - m, base, base + m]
        .into_iter()
        .filter(|&c| c >= 0)
        .min_by_key(|&c| (c - now).abs())
        .map(|c| c as u64)
}

/// A parsed coordinated-QSY directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// Move to `token` (a band-plan channel id) at absolute UTC slot `at_slot`.
    Move { token: String, at_slot: u64 },
    /// Return to the home channel.
    Home,
}

impl Directive {
    /// Render the directive body (the broadcast layer adds the `DE <MYCALL>` prefix).
    pub fn format(&self) -> String {
        match self {
            Directive::Move { token, at_slot } => {
                format!(
                    "{KEYWORD} {} {}",
                    token.to_uppercase(),
                    encode_slot(*at_slot)
                )
            }
            Directive::Home => format!("{KEYWORD} {HOME}"),
        }
    }

    /// Parse a directive *body* (a broadcast's text after the `DE <CALL>` prefix),
    /// resolving the slot code relative to `now_slot`. `None` if not a directive.
    pub fn parse(s: &str, now_slot: u64) -> Option<Directive> {
        let t: Vec<&str> = s.split_whitespace().collect();
        if t.first()
            .map(|w| !w.eq_ignore_ascii_case(KEYWORD))
            .unwrap_or(true)
        {
            return None;
        }
        match t.as_slice() {
            [_, kw] if kw.eq_ignore_ascii_case(HOME) => Some(Directive::Home),
            [_, token, code] if !token.eq_ignore_ascii_case(HOME) => {
                let at_slot = decode_slot(code, now_slot)?;
                Some(Directive::Move {
                    token: token.to_uppercase(),
                    at_slot,
                })
            }
            _ => None,
        }
    }
}

/// True if `s` starts with the QSY keyword (a directive, even if malformed) — so
/// the UI can flag it as a roam directive when the feature is off.
pub fn is_directive(s: &str) -> bool {
    s.split_whitespace()
        .next()
        .map(|w| w.eq_ignore_ascii_case(KEYWORD))
        .unwrap_or(false)
}

/// Deterministic initiator selection: the lexicographically-smaller callsign
/// initiates (announces) moves; the other follows. Both stations compute the same
/// answer from the pair — no negotiation round-trip.
pub fn is_initiator(mycall: &str, partner: &str) -> bool {
    mycall.to_uppercase() < partner.to_uppercase()
}

/// Reassembles inbound free-text frames and extracts coordinated-QSY directives.
///
/// Its own [`Reassembler`] is independent of the inbox's, so feeding it the same
/// decoded frames neither consumes nor disturbs normal chat reassembly — the
/// directive still flows through the inbox and shows in the band feed as plain
/// text (transparency); this only *additionally* surfaces it as a [`Directive`].
#[derive(Debug, Default)]
pub struct Detector {
    reasm: Reassembler,
}

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one decoded free-text frame. Returns `(sender, directive)` once a full
    /// `DE <CALL> QSY …` broadcast reassembles into a valid directive.
    pub fn observe(&mut self, frame: &str, now_slot: u64) -> Option<(String, Directive)> {
        // Mirror the inbox: a chunk reassembles; a non-chunk frame is taken whole.
        let full = if let Some(f) = self.reasm.accept(frame) {
            f
        } else if text::parse_chunk(frame).is_none() {
            frame.to_string()
        } else {
            return None; // a partial chunk — nothing complete yet
        };
        let (de, body) = crate::inbox::parse_broadcast(&full)?;
        Directive::parse(&body, now_slot).map(|d| (de, d))
    }
}

/// A scheduled move: where to go and the absolute UTC slot to go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingQsy {
    /// Target channel token, or [`HOME`].
    pub token: String,
    /// Absolute UTC slot index to retune on.
    pub at_slot: u64,
}

/// The coordinated-QSY state machine for one station.
///
/// Pure and token-based (no band-plan dependency): it decides *when* to announce,
/// *what* channel token to move to, *when* to execute, and *when* to fall back
/// home. The engine maps tokens ↔ real dial frequencies and performs the retune.
/// Everything is inert while [`Roamer::enabled`] is false.
#[derive(Debug)]
pub struct Roamer {
    /// Operator's QSY set (channel tokens, uppercased), cycled round-robin.
    set: Vec<String>,
    /// Announce cadence (initiator hops every this-many of its TX overs, ≥1).
    cadence: u64,
    /// Master opt-in. When false the engine never drives the Roamer.
    pub enabled: bool,
    /// Held on the current channel (no announcing / no following).
    pub paused: bool,
    /// Home channel token (where the conversation started).
    pub home: Option<String>,
    /// Channel token we believe we're currently on.
    pub current: Option<String>,
    /// The station we're roaming with (sets the initiator/follower roles).
    pub partner: Option<String>,
    /// A scheduled-but-not-yet-executed move.
    pub pending: Option<PendingQsy>,
    /// True once a "lost sync → home" fall-back has fired (surfaced to the UI).
    pub lost_sync: bool,
    set_idx: usize,
    tx_overs: u64,
    rx_silence: u64,
    heard_partner: bool,
    force_move: bool,
    detector: Detector,
}

impl Default for Roamer {
    fn default() -> Self {
        Self {
            set: Vec::new(),
            cadence: DEFAULT_CADENCE,
            enabled: false,
            paused: false,
            home: None,
            current: None,
            partner: None,
            pending: None,
            lost_sync: false,
            set_idx: 0,
            tx_overs: 0,
            rx_silence: 0,
            heard_partner: false,
            force_move: false,
            detector: Detector::new(),
        }
    }
}

impl Roamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the QSY channel set (tokens, uppercased) and announce cadence (≥1).
    pub fn configure(&mut self, set: Vec<String>, cadence: u64) {
        self.set = set.into_iter().map(|t| t.to_uppercase()).collect();
        self.cadence = cadence.max(1);
        if self.set_idx >= self.set.len() {
            self.set_idx = 0;
        }
    }

    /// Turn the feature on: capture the home/current channel + partner and reset
    /// the running counters. (No-op effect on the radio — the engine stays put
    /// until a move is scheduled.)
    pub fn enable(&mut self, home: String, partner: Option<String>) {
        let home = home.to_uppercase();
        self.enabled = true;
        self.paused = false;
        self.home = Some(home.clone());
        self.current = Some(home);
        self.partner = partner.map(|p| p.to_uppercase());
        self.pending = None;
        self.lost_sync = false;
        self.set_idx = 0;
        self.tx_overs = 0;
        self.rx_silence = 0;
        self.heard_partner = false;
        self.force_move = false;
    }

    /// Turn the feature off. Returns the home token to retune to if we're not
    /// already there (the engine returns the operator home on disable / stop).
    pub fn disable(&mut self) -> Option<String> {
        self.enabled = false;
        self.paused = false;
        self.pending = None;
        self.lost_sync = false;
        let go_home = match (&self.current, &self.home) {
            (Some(c), Some(h)) if c != h => Some(h.clone()),
            _ => None,
        };
        if go_home.is_some() {
            self.current = self.home.clone();
        }
        go_home
    }

    /// Update the roaming partner (e.g. the operator selected a different peer).
    pub fn set_partner(&mut self, partner: Option<String>) {
        self.partner = partner.map(|p| p.to_uppercase());
    }

    /// Hold on / release the current channel (manual override).
    pub fn set_paused(&mut self, on: bool) {
        self.paused = on;
    }

    /// Force the initiator to announce a move on its next over (manual override).
    pub fn request_move_now(&mut self) {
        self.force_move = true;
    }

    /// Whether this station is the initiator for the current partner.
    pub fn is_initiator_now(&self, mycall: &str) -> bool {
        self.partner
            .as_deref()
            .map(|p| is_initiator(mycall, p))
            .unwrap_or(false)
    }

    fn partner_present(&self) -> bool {
        self.heard_partner && self.rx_silence <= SILENCE_HOME_OVERS
    }

    /// Pick the next channel token (round-robin), skipping the one we're on.
    fn next_token(&mut self) -> Option<String> {
        if self.set.is_empty() {
            return None;
        }
        for _ in 0..self.set.len() {
            let t = self.set[self.set_idx].clone();
            self.set_idx = (self.set_idx + 1) % self.set.len();
            if Some(t.as_str()) != self.current.as_deref() {
                return Some(t);
            }
        }
        None // the set is just the current channel — nowhere to move
    }

    /// Called once per OUR TX over (initiator path). Returns a [`Directive`] to
    /// announce when a hop is due (cadence reached or move-now), having scheduled
    /// the matching [`pending`](Roamer::pending) move. `None` otherwise.
    pub fn poll_announce(&mut self, mycall: &str, now_slot: u64) -> Option<Directive> {
        if !self.enabled || self.paused || !self.is_initiator_now(mycall) {
            return None;
        }
        self.tx_overs = self.tx_overs.saturating_add(1);
        if self.pending.is_some() {
            return None;
        }
        let due = self.force_move || (self.partner_present() && self.tx_overs >= self.cadence);
        if !due {
            return None;
        }
        let token = self.next_token()?;
        self.force_move = false;
        self.tx_overs = 0;
        let at_slot = now_slot + LEAD_SLOTS;
        self.pending = Some(PendingQsy {
            token: token.clone(),
            at_slot,
        });
        Some(Directive::Move { token, at_slot })
    }

    /// Feed one decoded free-text frame (RX). If it completes a directive from our
    /// **partner**, schedule the matching move (follower) or a home return.
    /// Returns the parsed `(sender, directive)` regardless, so the engine can log
    /// / surface it. Acting is gated by `enabled && !paused`.
    pub fn accept_decode(
        &mut self,
        frame: &str,
        now_slot: u64,
        mycall: &str,
    ) -> Option<(String, Directive)> {
        let (de, dir) = self.detector.observe(frame, now_slot)?;
        if self.enabled && !self.paused && self.partner.as_deref() == Some(de.as_str()) {
            self.heard_partner = true;
            self.rx_silence = 0;
            self.lost_sync = false;
            match &dir {
                // Only the follower schedules from an announced move; the
                // initiator already holds its own pending.
                Directive::Move { token, at_slot } if !is_initiator(mycall, &de) => {
                    self.pending = Some(PendingQsy {
                        token: token.clone(),
                        at_slot: *at_slot,
                    });
                }
                Directive::Home => {
                    self.pending = Some(PendingQsy {
                        token: HOME.to_string(),
                        at_slot: now_slot,
                    });
                }
                _ => {}
            }
        }
        Some((de, dir))
    }

    /// Note that we heard `sender` this slot (any frame). Keeps the partner-present
    /// estimate fresh so the initiator keeps hopping and we don't fall back home
    /// while the QSO is alive.
    pub fn note_heard(&mut self, sender: &str) {
        if self.enabled && self.partner.as_deref() == Some(sender.to_uppercase().as_str()) {
            self.heard_partner = true;
            self.rx_silence = 0;
            self.lost_sync = false;
        }
    }

    /// Advance the RX-silence counter once per RX slot. When the partner has gone
    /// quiet for [`SILENCE_HOME_OVERS`] slots and we're off the home channel,
    /// schedule an immediate return home (both sides do this → re-rendezvous).
    pub fn on_rx_slot(&mut self, now_slot: u64) {
        if !self.enabled || self.paused {
            return;
        }
        self.rx_silence = self.rx_silence.saturating_add(1);
        let off_home = match (&self.current, &self.home) {
            (Some(c), Some(h)) => c != h,
            _ => false,
        };
        if self.pending.is_none() && off_home && self.rx_silence >= SILENCE_HOME_OVERS {
            self.lost_sync = true;
            self.pending = Some(PendingQsy {
                token: HOME.to_string(),
                at_slot: now_slot,
            });
        }
    }

    /// If a scheduled move is due (`now_slot >= at_slot`), commit it and return the
    /// **resolved** target token (HOME → the home channel) for the engine to tune.
    pub fn take_due(&mut self, now_slot: u64) -> Option<String> {
        let due = self.pending.as_ref().is_some_and(|p| now_slot >= p.at_slot);
        if !due {
            return None;
        }
        let pending = self.pending.take()?;
        let token = if pending.token == HOME {
            self.home.clone()?
        } else {
            pending.token
        };
        self.current = Some(token.clone());
        self.tx_overs = 0;
        self.rx_silence = 0;
        Some(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::broadcast_text;

    #[test]
    fn directive_roundtrips() {
        let now = 100_000;
        let m = Directive::Move {
            token: "70CM".into(),
            at_slot: now + LEAD_SLOTS,
        };
        assert_eq!(Directive::parse(&m.format(), now), Some(m.clone()));
        assert_eq!(
            Directive::parse(&Directive::Home.format(), now),
            Some(Directive::Home)
        );
        // case-insensitive keyword + lowercase token normalize to uppercase token
        assert_eq!(
            Directive::parse("qsy 2m-fm 0A8", now),
            Directive::parse("QSY 2M-FM 0A8", now)
        );
    }

    #[test]
    fn bad_directives_rejected() {
        let now = 0;
        assert!(Directive::parse("HELLO WORLD", now).is_none());
        assert!(Directive::parse("CQ W9XYZ EN37", now).is_none());
        assert!(Directive::parse("QSY", now).is_none());
        assert!(Directive::parse("QSY 70CM", now).is_none()); // missing slot code
        assert!(Directive::parse("QSY HOME EXTRA", now).is_none());
        assert!(Directive::parse("QSY 70CM ZZZZ", now).is_none()); // code too long
        assert!(is_directive("QSY anything"));
        assert!(!is_directive("HELLO"));
    }

    #[test]
    fn slot_code_roundtrips_near_now() {
        for &(target, now) in &[
            (100u64, 90u64),
            (1_000_000, 999_990),
            (SLOT_MOD - 1, SLOT_MOD - 5), // just below a cycle boundary
            (SLOT_MOD + 3, SLOT_MOD - 2), // straddling a cycle boundary
            (476_851_200_123, 476_851_200_118), // realistic 2026-era FT1 slot
        ] {
            let code = encode_slot(target);
            assert_eq!(
                decode_slot(&code, now),
                Some(target),
                "code {code} now {now}"
            );
        }
    }

    #[test]
    fn initiator_is_deterministic_and_asymmetric() {
        assert!(is_initiator("KA9AAA", "KB9BBB"));
        assert!(!is_initiator("KB9BBB", "KA9AAA"));
        assert!(is_initiator("ka9aaa", "KB9BBB")); // case-insensitive
    }

    #[test]
    fn detector_extracts_chunked_directive() {
        let dir = Directive::Move {
            token: "40M".into(),
            at_slot: 108,
        };
        let frames = text::chunk(&broadcast_text("KA9AAA", &dir.format()), 'A');
        assert!(frames.len() > 1, "directive broadcast is multi-chunk");
        let mut det = Detector::new();
        let mut got = None;
        for f in &frames {
            if let Some(x) = det.observe(f, 101) {
                got = Some(x);
            }
        }
        assert_eq!(got, Some(("KA9AAA".to_string(), dir)));
    }

    /// Two stations move together: the initiator announces, the follower decodes
    /// the chunked directive, and BOTH resolve the same target on the same slot.
    #[test]
    fn two_stations_move_together() {
        let set = vec!["20M".to_string(), "40M".to_string()];
        let mut a = Roamer::new(); // KA9AAA — initiator (lexicographically smaller)
        a.configure(set.clone(), 1);
        a.enable("20M".into(), Some("KB9BBB".into()));
        a.note_heard("KB9BBB"); // partner is present

        let mut b = Roamer::new(); // KB9BBB — follower
        b.configure(set, 1);
        b.enable("20M".into(), Some("KA9AAA".into()));

        // A announces a hop on its over.
        let dir = a.poll_announce("KA9AAA", 100).expect("initiator announces");
        let at = match &dir {
            Directive::Move { at_slot, .. } => *at_slot,
            _ => panic!("expected a Move directive"),
        };
        assert_eq!(at, 100 + LEAD_SLOTS);
        assert_eq!(a.pending.as_ref().unwrap().token, "40M");

        // The directive goes out as a broadcast; B decodes it a slot later.
        let frames = text::chunk(&broadcast_text("KA9AAA", &dir.format()), 'B');
        for f in &frames {
            b.accept_decode(f, 101, "KB9BBB");
        }
        assert_eq!(b.pending.as_ref().expect("follower scheduled").token, "40M");
        assert_eq!(b.pending.as_ref().unwrap().at_slot, at);

        // Neither moves before the target slot; both move ON it, to the same place.
        assert_eq!(a.take_due(at - 1), None);
        assert_eq!(b.take_due(at - 1), None);
        assert_eq!(a.take_due(at).as_deref(), Some("40M"));
        assert_eq!(b.take_due(at).as_deref(), Some("40M"));
        assert_eq!(a.current.as_deref(), Some("40M"));
        assert_eq!(b.current.as_deref(), Some("40M"));
    }

    /// After a move, a partner that goes silent for SILENCE_HOME_OVERS slots
    /// triggers a fall-back to the home channel.
    #[test]
    fn lost_sync_falls_back_home() {
        let mut r = Roamer::new();
        r.configure(vec!["20M".into(), "40M".into()], 1);
        r.enable("20M".into(), Some("KB9BBB".into()));
        // Pretend we hopped to 40M.
        r.pending = Some(PendingQsy {
            token: "40M".into(),
            at_slot: 200,
        });
        assert_eq!(r.take_due(200).as_deref(), Some("40M"));
        assert_eq!(r.current.as_deref(), Some("40M"));

        // Silence on the new channel — climb toward the home fall-back.
        for s in 0..(SILENCE_HOME_OVERS - 1) {
            r.on_rx_slot(300 + s);
            assert!(r.pending.is_none(), "no fall-back before the limit");
        }
        r.on_rx_slot(300 + SILENCE_HOME_OVERS);
        assert!(r.lost_sync, "marked lost-sync");
        let go = r.pending.as_ref().expect("home return scheduled");
        assert_eq!(go.token, HOME);
        // Executing it resolves HOME to the home channel.
        assert_eq!(r.take_due(go.at_slot).as_deref(), Some("20M"));
        assert_eq!(r.current.as_deref(), Some("20M"));
    }

    #[test]
    fn disabled_roamer_is_inert() {
        let mut r = Roamer::new();
        r.configure(vec!["20M".into(), "40M".into()], 1);
        // Not enabled: no announce, no follow, no fall-back.
        assert_eq!(r.poll_announce("KA9AAA", 100), None);
        let frames = text::chunk(
            &broadcast_text(
                "KA9AAA",
                &Directive::Move {
                    token: "40M".into(),
                    at_slot: 108,
                }
                .format(),
            ),
            'A',
        );
        for f in &frames {
            // It still *parses* the directive (for UI display) but never schedules.
            r.accept_decode(f, 101, "KB9BBB");
        }
        assert!(r.pending.is_none(), "disabled roamer schedules nothing");
        r.on_rx_slot(200);
        assert!(r.pending.is_none());
    }

    #[test]
    fn pause_and_move_now_overrides() {
        let mut r = Roamer::new();
        r.configure(vec!["20M".into(), "40M".into()], 100); // long cadence
        r.enable("20M".into(), Some("KB9BBB".into()));
        r.note_heard("KB9BBB");
        // Paused: even move-now does nothing.
        r.set_paused(true);
        r.request_move_now();
        assert_eq!(r.poll_announce("KA9AAA", 10), None);
        // Resumed + move-now: announces immediately despite the long cadence.
        r.set_paused(false);
        assert!(matches!(
            r.poll_announce("KA9AAA", 10),
            Some(Directive::Move { .. })
        ));
    }
}
