//! Presence: a roster of recently-heard stations, built passively from decodes.
//!
//! Any decoded frame that carries a sender callsign (CQ/beacon or a standard
//! directed frame) updates the roster with the station's last-heard time (as a
//! slot index), signal report, and grid when available. This is the off-grid
//! "who's out there" view.

use crate::message::Msg;
use modes::Decode;
use std::collections::HashMap;

/// A station we have heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeardStation {
    pub call: String,
    pub grid: Option<String>,
    pub snr: i32,
    /// Slot index when last heard (monotonic; avoids wall-clock for testability).
    pub last_heard_slot: u64,
    pub heard_count: u32,
}

/// Roster of heard stations, keyed by callsign.
#[derive(Debug, Default)]
pub struct Roster {
    stations: HashMap<String, HeardStation>,
}

impl Roster {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the roster from a decode heard at `slot`.
    pub fn observe(&mut self, d: &Decode, slot: u64) {
        let m = Msg::parse(&d.message);
        let Some(sender) = m.sender() else { return };
        let grid = match &m {
            // A non-empty grid only — an i3=4 compound CQ/call carries none (empty), and
            // a roster grid of Some("") would be a phantom empty grid.
            Msg::Cq { grid, .. } | Msg::Grid { grid, .. } if !grid.is_empty() => {
                Some(grid.clone())
            }
            _ => None,
        };
        let entry = self
            .stations
            .entry(sender.to_string())
            .or_insert_with(|| HeardStation {
                call: sender.to_string(),
                grid: None,
                snr: d.snr,
                last_heard_slot: slot,
                heard_count: 0,
            });
        entry.snr = d.snr;
        entry.last_heard_slot = slot;
        entry.heard_count += 1;
        if grid.is_some() {
            entry.grid = grid;
        }
    }

    pub fn get(&self, call: &str) -> Option<&HeardStation> {
        self.stations.get(call)
    }

    /// True if `call` was heard within `window` slots of `now_slot`.
    pub fn is_active(&self, call: &str, now_slot: u64, window: u64) -> bool {
        self.stations
            .get(call)
            .is_some_and(|s| now_slot.saturating_sub(s.last_heard_slot) <= window)
    }

    /// Heard stations, most-recently-heard first.
    pub fn by_recent(&self) -> Vec<&HeardStation> {
        let mut v: Vec<&HeardStation> = self.stations.values().collect();
        v.sort_by(|a, b| b.last_heard_slot.cmp(&a.last_heard_slot));
        v
    }

    pub fn len(&self) -> usize {
        self.stations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }
}

/// Build a presence/beacon message (a CQ carrying my grid).
pub fn beacon(mycall: &str, grid: &str) -> Msg {
    Msg::Cq {
        de: mycall.to_string(),
        grid: grid.to_string(),
        dir: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(msg: &str, snr: i32) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn tracks_senders_and_grids() {
        let mut r = Roster::new();
        r.observe(&dec("CQ W9XYZ EN37", -5), 0);
        r.observe(&dec("W9XYZ K2DEF FN31", -12), 1);
        r.observe(&dec("CQ W9XYZ EN37", -3), 4); // heard again, newer + stronger

        assert_eq!(r.len(), 2);
        let w = r.get("W9XYZ").unwrap();
        assert_eq!(w.grid.as_deref(), Some("EN37"));
        assert_eq!(w.snr, -3);
        assert_eq!(w.last_heard_slot, 4);
        assert_eq!(w.heard_count, 2);
        assert_eq!(r.get("K2DEF").unwrap().grid.as_deref(), Some("FN31"));

        // Most-recent first.
        assert_eq!(r.by_recent()[0].call, "W9XYZ");
        assert!(r.is_active("K2DEF", 4, 10));
        assert!(!r.is_active("K2DEF", 100, 10));
    }
}
