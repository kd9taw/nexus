//! Directed messaging inbox: turns a stream of decodes into roster updates and
//! attributed chat messages.
//!
//! Because FT1 free-text frames carry no callsign, Tempo attributes free-text to
//! a sender by **temporal association**: a standard frame (CQ/beacon or a
//! directed `TO FROM …` frame) identifies the current talker, and subsequent
//! free-text chunks are attributed to that station until another identifies
//! itself. A station therefore precedes a free-text message with an identifying
//! frame (its beacon, or a directed frame naming the recipient). This is the
//! pragmatic session model for a 13-char, callsign-less free-text substrate.

use crate::message::{looks_like_call, Msg};
use crate::roster::Roster;
use crate::text::{self, Reassembler};
use modes::Decode;

/// Prefix that marks free text as an open broadcast (sender embedded).
pub const BROADCAST_PREFIX: &str = "DE";

/// If `text` is a `DE <CALL> <body>` open broadcast, return `(call, body)`.
///
/// The sender call is the first token after `DE`; the body is everything after
/// it (must be non-empty — `DE <CALL>` alone is just an identify, not a message).
pub fn parse_broadcast(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix(BROADCAST_PREFIX)?;
    // Require a separating space so "DESK" isn't mistaken for a broadcast.
    let rest = rest.strip_prefix(' ')?;
    let mut parts = rest.splitn(2, ' ');
    let call = parts.next()?.trim();
    let body = parts.next().unwrap_or("").trim();
    if call.is_empty() || body.is_empty() {
        return None;
    }
    Some((call.to_string(), body.to_string()))
}

/// Render an open-broadcast free-text string: `DE <MYCALL> <body>`.
pub fn broadcast_text(mycall: &str, body: &str) -> String {
    format!("{BROADCAST_PREFIX} {mycall} {body}")
}

/// A received chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Attributed sender (most-recently-identified station), if known.
    pub from: Option<String>,
    /// Recipient, if the preceding directed frame named one (else broadcast).
    pub to: Option<String>,
    pub text: String,
    pub slot: u64,
    /// True if `to` matched our callsign.
    pub directed_to_me: bool,
}

/// Processes decodes into a roster + chat messages.
pub struct Inbox {
    pub mycall: String,
    pub roster: Roster,
    reasm: Reassembler,
    /// Most recently identified talker (sender of the last standard frame).
    current_from: Option<String>,
    /// Recipient named by the last directed frame, if any.
    current_to: Option<String>,
    pub messages: Vec<ChatMessage>,
    /// ACKs we owe: each directed-to-me message we just fully received, as `(sender,
    /// chunk-id)`. Recorded on EVERY completion (including a resend we suppress from the
    /// display), so a lost ACK recovers when the sender resends. Drained by the engine.
    owed_acks: Vec<(String, char)>,
}

impl Inbox {
    pub fn new(mycall: &str) -> Self {
        Self {
            mycall: mycall.to_string(),
            roster: Roster::new(),
            reasm: Reassembler::new(),
            current_from: None,
            current_to: None,
            messages: Vec::new(),
            owed_acks: Vec::new(),
        }
    }

    /// Drain the id-bearing ACKs we owe (directed-to-me messages we received). The engine
    /// sends one `"<sender> <me> RR73 <id>"` per entry.
    pub fn take_owed_acks(&mut self) -> Vec<(String, char)> {
        std::mem::take(&mut self.owed_acks)
    }

    /// Process all decodes heard in a slot.
    pub fn observe(&mut self, decodes: &[Decode], slot: u64) {
        for d in decodes {
            self.roster.observe(d, slot);
            let m = Msg::parse(&d.message);
            // A FREE-TEXT frame can masquerade as a structured one: a chunk ending in "73"
            // ("A22NOON ES 73") and a bare broadcast ("DE KD9TAW 73") both parse as `Bye73`,
            // and a Letter-Digit-Digit DX call ("A79AA K2DEF 73") is a REAL roger. The
            // distinguisher: a genuine standard frame is a recognized variant whose every
            // callsign field is a plausible call — `looks_like_call` (callsign OR compound
            // OR i3=4 hashed `<...>`), NOT plain `is_callsign`, so a compound/hashed QSO
            // frame ("<W9XYZ> PJ4/K1ABC R-10") stays standard instead of leaking into chat.
            // Anything else is CONTENT — route a chunk to the reassembler and everything
            // else (bare broadcast, plain line) to push_text, which sorts a "DE <CALL>
            // <body>" broadcast from a directed line.
            let is_standard = !matches!(m, Msg::Other(_))
                && m.sender().map_or(true, looks_like_call)
                && m.addressee().map_or(true, looks_like_call);
            if !is_standard {
                if let Some((id, ..)) = text::parse_chunk(&d.message) {
                    if let Some(full) = self.reasm.accept(&d.message) {
                        // A directed-to-me message completed → owe an id-ACK to its sender,
                        // recorded EVERY time we hear it complete (so the sender's resend
                        // re-triggers the ACK if the first was lost), independent of the
                        // display resend-dedup inside push_text.
                        if self.current_to.as_deref() == Some(self.mycall.as_str()) {
                            if let Some(from) = self.current_from.clone() {
                                self.owed_acks.push((from, id));
                            }
                        }
                        self.push_text(full, slot);
                    }
                    // else: a partial chunk — buffered, nothing to emit yet.
                } else {
                    self.push_text(d.message.clone(), slot);
                }
                continue;
            }
            // A standard frame identifies the current talker (and maybe a directed
            // recipient), establishing attribution context.
            if let Some(sender) = m.sender() {
                self.current_from = Some(sender.to_string());
            }
            // A sign-off (RR73 / RRR / 73) ENDS the directed exchange — clear the recipient
            // context so a following courtesy free-text line isn't tagged directed-to-me
            // (which would emit a spurious ACK and misattribute a following `DE` broadcast).
            self.current_to = match &m {
                Msg::Rr73 { .. } | Msg::Rrr { .. } | Msg::Bye73 { .. } => None,
                _ => m.addressee().map(|s| s.to_string()),
            };
        }
    }

    fn push_text(&mut self, text: String, slot: u64) {
        // An open broadcast embeds its sender as a `DE <CALL> ` prefix (FT8-style "to
        // everyone"). Route it to the band-activity bucket (to = None) when we're NOT mid
        // directed exchange, OR when its embedded sender is a DIFFERENT station than the
        // current directed talker — i.e. a THIRD party broadcasting mid-QSO, which must
        // not be mis-tagged as coming from the talker and addressed to me (a stale
        // `current_to == mycall` would otherwise flag it directed-to-me + owe a stray ACK).
        let msg = match parse_broadcast(&text) {
            Some((de, body))
                if self.current_to.is_none()
                    || self.current_from.as_deref() != Some(de.as_str()) =>
            {
                ChatMessage {
                    from: Some(de),
                    to: None,
                    text: body,
                    slot,
                    directed_to_me: false,
                }
            }
            _ => self.directed_message(text, slot),
        };
        // Resend dedup: store-and-forward retransmits the same message every few cycles
        // until it's ACKed, and the reassembler re-emits each completed copy. Skip an
        // identical message (same sender + recipient + text) within a short window so it
        // isn't double-shown or double-ACKed (which would over-mark the send queue).
        const RESEND_DEDUP_SLOTS: u64 = 40;
        let dup = self.messages.iter().rev().take(24).any(|m| {
            m.from == msg.from
                && m.to == msg.to
                && m.text == msg.text
                && slot.saturating_sub(m.slot) < RESEND_DEDUP_SLOTS
        });
        if !dup {
            self.messages.push(msg);
        }
    }

    /// Build a directed/attributed message from the current talker context.
    fn directed_message(&self, text: String, slot: u64) -> ChatMessage {
        let to = self.current_to.clone();
        let directed_to_me = to.as_deref() == Some(self.mycall.as_str());
        ChatMessage {
            from: self.current_from.clone(),
            to,
            text,
            slot,
            directed_to_me,
        }
    }

    /// Messages directed specifically to me.
    pub fn for_me(&self) -> Vec<&ChatMessage> {
        self.messages.iter().filter(|m| m.directed_to_me).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(msg: &str) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr: -8,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn attributes_freetext_to_identified_sender() {
        let mut inbox = Inbox::new("K2DEF");
        // W9XYZ identifies via a directed frame to me, then sends a 2-chunk msg.
        inbox.observe(&[dec("K2DEF W9XYZ EN37")], 0); // directed grid to me (identify)
        let frames = text::chunk("MEET AT THE REPEATER AT NOON", 'A');
        for (i, f) in frames.iter().enumerate() {
            inbox.observe(&[dec(f)], (i as u64) + 1);
        }
        let mine = inbox.for_me();
        assert_eq!(mine.len(), 1, "messages: {:?}", inbox.messages);
        assert_eq!(mine[0].from.as_deref(), Some("W9XYZ"));
        assert!(mine[0].directed_to_me);
        assert_eq!(
            mine[0].text,
            text::normalize("MEET AT THE REPEATER AT NOON")
        );
        assert!(inbox.roster.get("W9XYZ").is_some());
    }

    #[test]
    fn reassembles_a_message_whose_chunk_looks_like_a_standard_frame() {
        // Regression: a chat line ending in "73" produces a final chunk like
        // "A22NOON ES 73" that Msg::parse reads as `Bye73`. If chunk-detection didn't run
        // first, that chunk went to the attribution branch and the message never
        // reassembled. Chunks must be detected BEFORE Msg::parse.
        let mut inbox = Inbox::new("K2DEF");
        inbox.observe(&[dec("K2DEF W9XYZ EN37")], 0); // identify (directed to me)
        let frames = text::chunk("MEET AT NOON ES 73", 'A');
        assert!(
            frames.iter().any(|f| f.ends_with("73")),
            "test premise: a chunk ends in 73 ({frames:?})"
        );
        for (i, f) in frames.iter().enumerate() {
            inbox.observe(&[dec(f)], (i as u64) + 1);
        }
        let mine = inbox.for_me();
        assert_eq!(mine.len(), 1, "the '...73' message reassembled: {:?}", inbox.messages);
        assert_eq!(mine[0].from.as_deref(), Some("W9XYZ"));
        assert_eq!(mine[0].text, text::normalize("MEET AT NOON ES 73"));
    }

    #[test]
    fn dx_call_roger_is_not_swallowed_as_a_chunk() {
        // "A79AA K2DEF 73" (a Qatar A7 call signing off) matches parse_chunk's shape
        // (A-uppercase, 7, 9...) but is a REAL standard frame — both A79AA and K2DEF are
        // valid calls — so it must set attribution, not feed the reassembler and corrupt
        // an in-flight chat buffer.
        let mut inbox = Inbox::new("K2DEF");
        inbox.observe(&[dec("A79AA K2DEF 73")], 0);
        assert!(
            inbox.messages.is_empty(),
            "a DX roger must not produce a phantom chat: {:?}",
            inbox.messages
        );
    }

    #[test]
    fn re_owes_the_ack_on_a_resend_so_a_lost_ack_recovers() {
        // The ACK obligation is recorded on EVERY directed-to-me completion — including a
        // resend whose DISPLAY we dedup — so if the first RR73 is lost on air, the next
        // resend re-owes it and the ACK goes out again (M2 lost-ACK recovery).
        let mut inbox = Inbox::new("K2DEF");
        let id_frame = "K2DEF W9XYZ EN37";
        let frames = text::chunk("HELLO", 'A');
        inbox.observe(&[dec(id_frame)], 0);
        for (i, f) in frames.iter().enumerate() {
            inbox.observe(&[dec(f)], (i as u64) + 1);
        }
        assert_eq!(inbox.take_owed_acks().len(), 1, "owe an ACK on first receipt");
        // Sender resends (our ACK was lost). The display dedups, but we MUST re-owe.
        inbox.observe(&[dec(id_frame)], 10);
        for (i, f) in frames.iter().enumerate() {
            inbox.observe(&[dec(f)], (i as u64) + 11);
        }
        assert_eq!(
            inbox.take_owed_acks().len(),
            1,
            "re-owe the ACK on a resend so a lost ACK recovers"
        );
    }

    #[test]
    fn compound_hashed_qso_frame_is_not_leaked_as_chat() {
        // Regression: an i3=4 frame renders ONE call hashed (`<W9XYZ>`). is_callsign rejects
        // the angle brackets, so a gate using it mis-routed this real standard frame into
        // chat (the task-#21 phantom-leak class) and lost attribution. looks_like_call
        // accepts hashed/compound, so it stays a standard frame → attribution, not a message.
        let mut inbox = Inbox::new("K2DEF");
        inbox.observe(&[dec("<W9XYZ> PJ4/K1ABC R-10")], 0);
        assert!(
            inbox.messages.is_empty(),
            "no phantom chat from a compound QSO frame: {:?}",
            inbox.messages
        );
    }

    #[test]
    fn bare_short_broadcast_threads_as_a_broadcast() {
        // S3 fast-path: a short broadcast goes out as ONE bare frame "DE W9XYZ 73" (no
        // chunk header). It parses as Bye73, but it is NOT a real two-call exchange, so it
        // must thread as an OPEN broadcast (from W9XYZ, body "73") — not vanish into
        // attribution as it would if "...73" frames were treated as standard rogers.
        let mut inbox = Inbox::new("K2DEF");
        inbox.observe(&[dec("DE W9XYZ 73")], 0);
        let m = inbox.messages.last().expect("the bare broadcast threaded");
        assert_eq!(m.from.as_deref(), Some("W9XYZ"));
        assert_eq!(m.to, None, "open broadcast, not directed");
        assert!(m.text.contains("73"), "carried the body: {:?}", m.text);
    }

    #[test]
    fn third_party_broadcast_mid_exchange_is_not_misattributed() {
        let mut inbox = Inbox::new("W1AW");
        // Mid directed exchange: W9XYZ is talking to me (current_from=W9XYZ, current_to=W1AW).
        inbox.observe(&[dec("W1AW W9XYZ EN37")], 0);
        // A THIRD station broadcasts. It must thread as an open broadcast FROM K2DEF, not a
        // directed-to-me message attributed to W9XYZ (which would owe a stray ACK).
        inbox.observe(&[dec("DE K2DEF CQ POTA")], 1);
        let m = inbox.messages.last().expect("the broadcast threaded");
        assert_eq!(m.from.as_deref(), Some("K2DEF"));
        assert_eq!(m.to, None, "open broadcast");
        assert!(!m.directed_to_me);
    }

    #[test]
    fn signoff_ends_the_directed_context() {
        // A roger to me ends the exchange: a following courtesy free-text line must NOT be
        // tagged directed-to-me (which would owe a spurious ACK and re-key us).
        let mut inbox = Inbox::new("K2DEF");
        inbox.observe(&[dec("K2DEF W9XYZ RR73")], 0); // roger addressed to me
        inbox.observe(&[dec("GE TNX QSO ALL")], 1); // following courtesy free text
        let last = inbox.messages.last().expect("the courtesy line threaded");
        assert!(!last.directed_to_me, "sign-off cleared the directed-to-me context");
    }

    #[test]
    fn parse_broadcast_splits_sender_and_body() {
        assert_eq!(
            parse_broadcast("DE W9XYZ HELLO ALL"),
            Some(("W9XYZ".to_string(), "HELLO ALL".to_string()))
        );
        // `DE <CALL>` with no body is an identify, not a broadcast.
        assert_eq!(parse_broadcast("DE W9XYZ"), None);
        // Must have the `DE ` prefix (with separating space).
        assert_eq!(parse_broadcast("DESK W9XYZ HI"), None);
        assert_eq!(parse_broadcast("HELLO ALL"), None);
    }

    #[test]
    fn broadcast_freetext_routes_as_broadcast_to_embedded_sender() {
        let mut inbox = Inbox::new("K2DEF");
        // A bare `DE W9XYZ HELLO ALL` free-text frame, with no prior directed
        // context, is an open broadcast attributed to W9XYZ (to = None).
        inbox.observe(&[dec("DE W9XYZ HELLO ALL")], 0);
        assert_eq!(inbox.messages.len(), 1, "messages: {:?}", inbox.messages);
        let m = &inbox.messages[0];
        assert_eq!(m.from.as_deref(), Some("W9XYZ"));
        assert_eq!(m.to, None, "broadcast has no recipient");
        assert_eq!(m.text, "HELLO ALL");
        assert!(!m.directed_to_me);
    }

    #[test]
    fn chunked_broadcast_reassembles_and_routes() {
        let mut inbox = Inbox::new("K2DEF");
        // A longer broadcast is chunked; reassembled text keeps the DE prefix.
        let frames = text::chunk(&broadcast_text("W9XYZ", "NET ON 7130 AT 0200Z"), 'A');
        for (i, f) in frames.iter().enumerate() {
            inbox.observe(&[dec(f)], i as u64);
        }
        assert_eq!(inbox.messages.len(), 1, "messages: {:?}", inbox.messages);
        let m = &inbox.messages[0];
        assert_eq!(m.from.as_deref(), Some("W9XYZ"));
        assert_eq!(m.to, None);
        assert_eq!(m.text, text::normalize("NET ON 7130 AT 0200Z"));
    }
}
