//! Store-and-forward for off-grid nets.
//!
//! A station (or relay) queues directed messages for callsigns that may not be
//! reachable right now. When the recipient becomes **present** (heard recently
//! in the [`Roster`]), the queued message is released for transmission as a
//! burst of frames: an identifying directed frame (carrying TO+FROM, so the
//! recipient can attribute it) followed by the word-wrapped free-text chunks.
//! Sends back off between attempts and stop once delivery is confirmed.

use crate::message::Msg;
use crate::roster::Roster;
use crate::text;

/// A queued outbound message.
#[derive(Debug, Clone)]
pub struct Pending {
    pub to: String,
    pub text: String,
    pub created_slot: u64,
    pub attempts: u32,
    pub last_attempt_slot: Option<u64>,
    pub delivered: bool,
    id: char,
}

/// A presence-gated store-and-forward queue.
#[derive(Debug)]
pub struct StoreForward {
    mycall: String,
    mygrid: String,
    queue: Vec<Pending>,
    next_id: u8, // cycles 'A'..'Z' for chunk message-ids
}

impl StoreForward {
    pub fn new(mycall: &str, mygrid: &str) -> Self {
        Self {
            mycall: mycall.to_string(),
            mygrid: mygrid.to_string(),
            queue: Vec::new(),
            next_id: 0,
        }
    }

    /// Rebind the operator identity used to stamp the `DE <call>` / grid prefix on
    /// released frames, WITHOUT dropping the pending queue (keyed by recipient). For
    /// an in-place callsign/grid change in Settings (see `AppState::set_identity`).
    pub fn set_identity(&mut self, mycall: &str, mygrid: &str) {
        self.mycall = mycall.to_string();
        self.mygrid = mygrid.to_string();
    }

    /// Queue a directed message for later delivery. Returns the chunk-id char assigned to
    /// it — the caller stamps the outbound conversation bubble with it so an id-bearing ACK
    /// confirms exactly this message.
    pub fn queue(&mut self, to: &str, text: &str, slot: u64) -> char {
        let id = (b'A' + self.next_id) as char;
        self.next_id = (self.next_id + 1) % 26;
        self.queue.push(Pending {
            to: to.to_string(),
            text: text.to_string(),
            created_slot: slot,
            attempts: 0,
            last_attempt_slot: None,
            delivered: false,
            id,
        });
        id
    }

    /// Number of messages still awaiting delivery.
    pub fn pending(&self) -> usize {
        self.queue.iter().filter(|p| !p.delivered).count()
    }

    /// Frames to transmit *now*: for each undelivered message whose recipient is
    /// active (heard within `window` slots) and out of `backoff`, build the on-air burst
    /// `[identify, chunk…]` and record the attempt. Returns `(recipient, body, frames)`
    /// per releasable message — `body` is `Some` ONLY on the message's FIRST release, so
    /// the engine can record one own-TX band-activity row when the message actually goes
    /// on the air (not at compose time, and not once per resend).
    pub fn due(
        &mut self,
        roster: &Roster,
        slot: u64,
        window: u64,
        backoff: u64,
    ) -> Vec<(String, Option<String>, Vec<String>)> {
        let mut out = Vec::new();
        for p in self.queue.iter_mut().filter(|p| !p.delivered) {
            if !roster.is_active(&p.to, slot, window) {
                continue;
            }
            if let Some(last) = p.last_attempt_slot {
                if slot.saturating_sub(last) < backoff {
                    continue;
                }
            }
            let mut frames = vec![Msg::Grid {
                to: p.to.clone(),
                de: self.mycall.clone(),
                grid: self.mygrid.clone(),
            }
            .to_text()];
            frames.extend(text::chunk(&p.text, p.id));
            p.attempts += 1;
            p.last_attempt_slot = Some(slot);
            let body = if p.attempts == 1 {
                Some(p.text.clone())
            } else {
                None
            };
            out.push((p.to.clone(), body, frames));
        }
        out
    }

    /// Mark all messages for `to` delivered (e.g. on receiving an ack/roger).
    pub fn mark_delivered(&mut self, to: &str) {
        for p in self.queue.iter_mut().filter(|p| p.to == to) {
            p.delivered = true;
        }
    }

    /// Mark the OLDEST still-undelivered message for `to` delivered, returning whether one
    /// was marked. An RR73 ACK carries no message id, so each received ACK clears exactly
    /// ONE message FIFO — never the whole peer queue (which would silently drop a
    /// still-in-flight later message and falsely show it "delivered").
    pub fn mark_one_delivered(&mut self, to: &str) -> bool {
        if let Some(p) = self.queue.iter_mut().find(|p| !p.delivered && p.to == to) {
            p.delivered = true;
            true
        } else {
            false
        }
    }

    /// Drop delivered or over-attempted messages; returns how many were purged.
    pub fn purge(&mut self, max_attempts: u32) -> usize {
        let before = self.queue.len();
        self.queue
            .retain(|p| !p.delivered && p.attempts < max_attempts);
        before - self.queue.len()
    }

    /// Drop every queued message for `to`, delivered or not; returns how many were dropped.
    /// The operator deleted the conversation, so nothing further for that peer may go on the
    /// air. Without this, deleting a thread leaves its messages transmitting for up to
    /// `MAX_SEND_ATTEMPTS` releases — and a message to a never-heard peer stays at
    /// `attempts == 0`, which `purge` never collects, so it would queue indefinitely.
    pub fn drop_for(&mut self, to: &str) -> usize {
        let before = self.queue.len();
        self.queue.retain(|p| p.to != to);
        before - self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modes::Decode;

    fn dec(msg: &str) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr: -7,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn holds_until_recipient_present_then_releases() {
        let mut sf = StoreForward::new("W9XYZ", "EN37");
        sf.queue("N0XYZ", "QSY TO 40M AT 0200Z PSE", 0);
        assert_eq!(sf.pending(), 1);

        let mut roster = Roster::new();
        // Recipient not heard yet → nothing to send.
        assert!(sf.due(&roster, 1, 10, 3).is_empty());

        // Recipient appears on the air.
        roster.observe(&dec("CQ N0XYZ EN52"), 5);
        let due = sf.due(&roster, 6, 10, 3);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, "N0XYZ");
        assert_eq!(
            due[0].1.as_deref(),
            Some("QSY TO 40M AT 0200Z PSE"),
            "body on first release"
        );
        // identify frame + at least one chunk.
        assert!(due[0].2.len() >= 2);
        assert!(due[0].2[0].contains("N0XYZ") && due[0].2[0].contains("W9XYZ"));

        // Within backoff → not resent.
        assert!(sf.due(&roster, 7, 10, 3).is_empty());
        // After backoff and still undelivered → resent, but NO body this time (the own-TX
        // row was already recorded on the first release — resends mustn't spam it).
        let resend = sf.due(&roster, 9, 10, 3);
        assert_eq!(resend.len(), 1);
        assert_eq!(resend[0].1, None, "no body on a resend");

        // Delivered → no longer due, pending drops.
        sf.mark_delivered("N0XYZ");
        assert_eq!(sf.pending(), 0);
        assert!(sf.due(&roster, 20, 10, 3).is_empty());
    }

    #[test]
    fn presence_window_expires() {
        let mut sf = StoreForward::new("W9XYZ", "EN37");
        sf.queue("N0XYZ", "TEST", 0);
        let mut roster = Roster::new();
        roster.observe(&dec("CQ N0XYZ EN52"), 5);
        // Heard at slot 5; at slot 100 with window 10 it is stale → not due.
        assert!(sf.due(&roster, 100, 10, 3).is_empty());
        // Within window → due.
        assert_eq!(sf.due(&roster, 12, 10, 3).len(), 1);
    }

    #[test]
    fn one_ack_clears_only_the_oldest_message() {
        // Regression: an RR73 ACK has no message id, so it must clear exactly ONE queued
        // message FIFO — never the whole peer queue (which silently dropped a later
        // still-in-flight message and falsely marked it delivered).
        let mut sf = StoreForward::new("W9XYZ", "EN37");
        sf.queue("N0XYZ", "FIRST", 0);
        sf.queue("N0XYZ", "SECOND", 1);
        assert_eq!(sf.pending(), 2);
        assert!(sf.mark_one_delivered("N0XYZ"));
        assert_eq!(sf.pending(), 1, "one ACK clears exactly one message");
        assert!(sf.mark_one_delivered("N0XYZ"));
        assert_eq!(sf.pending(), 0);
        assert!(!sf.mark_one_delivered("N0XYZ"), "nothing left to clear");
    }

    #[test]
    fn purge_drops_delivered_and_exhausted() {
        let mut sf = StoreForward::new("W9XYZ", "EN37");
        sf.queue("N0XYZ", "ONE", 0);
        sf.queue("K2DEF", "TWO", 0);
        sf.mark_delivered("N0XYZ");
        assert_eq!(sf.purge(5), 1);
        assert_eq!(sf.pending(), 1);
    }

    #[test]
    fn drop_for_cancels_one_peers_queue_including_what_purge_would_never_collect() {
        let mut sf = StoreForward::new("W9XYZ", "EN37");
        sf.queue("N0XYZ", "ONE", 0);
        sf.queue("N0XYZ", "TWO", 0);
        sf.queue("K2DEF", "THREE", 0);

        // Never released (peer unheard) → attempts stays 0, which `purge` NEVER collects:
        // without drop_for these would sit queued forever.
        assert_eq!(
            sf.purge(8),
            0,
            "purge cannot reach a never-released message"
        );

        assert_eq!(
            sf.drop_for("N0XYZ"),
            2,
            "both of that peer's messages dropped"
        );
        assert_eq!(sf.pending(), 1, "the other peer is untouched");
        assert_eq!(sf.drop_for("N0XYZ"), 0, "idempotent");
    }
}
