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

    /// Queue a directed message for later delivery.
    pub fn queue(&mut self, to: &str, text: &str, slot: u64) {
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
    }

    /// Number of messages still awaiting delivery.
    pub fn pending(&self) -> usize {
        self.queue.iter().filter(|p| !p.delivered).count()
    }

    /// Frames to transmit *now*: for each undelivered message whose recipient is
    /// active (heard within `window` slots) and out of `backoff`, build the
    /// on-air burst `[identify, chunk…]` and record the attempt. Returns
    /// `(recipient, frames)` per releasable message.
    pub fn due(
        &mut self,
        roster: &Roster,
        slot: u64,
        window: u64,
        backoff: u64,
    ) -> Vec<(String, Vec<String>)> {
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
            out.push((p.to.clone(), frames));
        }
        out
    }

    /// Mark all messages for `to` delivered (e.g. on receiving an ack/roger).
    pub fn mark_delivered(&mut self, to: &str) {
        for p in self.queue.iter_mut().filter(|p| p.to == to) {
            p.delivered = true;
        }
    }

    /// Drop delivered or over-attempted messages; returns how many were purged.
    pub fn purge(&mut self, max_attempts: u32) -> usize {
        let before = self.queue.len();
        self.queue
            .retain(|p| !p.delivered && p.attempts < max_attempts);
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
        // identify frame + at least one chunk.
        assert!(due[0].1.len() >= 2);
        assert!(due[0].1[0].contains("N0XYZ") && due[0].1[0].contains("W9XYZ"));

        // Within backoff → not resent.
        assert!(sf.due(&roster, 7, 10, 3).is_empty());
        // After backoff and still undelivered → resent.
        assert_eq!(sf.due(&roster, 9, 10, 3).len(), 1);

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
    fn purge_drops_delivered_and_exhausted() {
        let mut sf = StoreForward::new("W9XYZ", "EN37");
        sf.queue("N0XYZ", "ONE", 0);
        sf.queue("K2DEF", "TWO", 0);
        sf.mark_delivered("N0XYZ");
        assert_eq!(sf.purge(5), 1);
        assert_eq!(sf.pending(), 1);
    }
}
