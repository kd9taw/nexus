//! Tempo application logic — the UI-facing layer that wraps [`tempo_core`].
//!
//! [`AppState`] owns the operator's identity, an [`inbox::Inbox`] (roster +
//! attributed inbound chat), a [`store::StoreForward`] queue for outbound
//! directed messages, per-peer [`dto::Conversation`] threads, and the current
//! [`dto::RadioStatus`] / [`dto::LinkState`]. Its public methods are the command
//! surface a Tauri shell (or a test harness) drives; they return serializable
//! [`dto`] types so the frontend renders directly from them.
//!
//! This crate is deliberately **pure logic**: no Tauri, no audio devices, no
//! threads. A real engine pumps decoded [`ft1::Decode`]s in via [`AppState::observe`]
//! and reads [`AppState::snapshot`] out; here those edges are exercised by unit
//! tests over synthetic decodes.

pub mod bandplan;
pub mod dto;
pub mod engine;

use std::collections::HashMap;

use modes::Decode;
use tempo_core::inbox::{ChatMessage as CoreChatMessage, Inbox};
use tempo_core::roster::HeardStation;
use tempo_core::store::StoreForward;

use dto::{
    AppSnapshot, ChatMessage, Conversation, LinkState, OpMode, Presence, RadioStatus, Station, Tier,
};

pub mod settings;

/// Slots within this many of "now" count as [`Presence::Active`].
const ACTIVE_WINDOW: u64 = 4;
/// Slots within this many (but past [`ACTIVE_WINDOW`]) count as [`Presence::Idle`].
const IDLE_WINDOW: u64 = 16;

/// The whole application's mutable state, owned by the shell on one thread.
pub struct AppState {
    mycall: String,
    mygrid: String,
    /// Roster + inbound-attribution engine.
    inbox: Inbox,
    /// Outbound directed-message queue (presence-gated store-and-forward).
    store: StoreForward,
    /// Per-peer conversation threads, keyed by peer callsign.
    conversations: HashMap<String, Conversation>,
    /// Monotonic slot counter — the latest slot observed/sent.
    slot: u64,
    /// The peer the operator currently has selected, if any.
    active_peer: Option<String>,
    /// Current radio / slot status.
    radio: RadioStatus,
    /// Current link state (updated from the strongest decode each slot).
    link: LinkState,
    /// Number of inbound messages already drained from the inbox into threads.
    drained: usize,
}

impl AppState {
    /// Build a fresh state for an operator.
    pub fn new(mycall: &str, mygrid: &str) -> Self {
        Self {
            mycall: mycall.to_string(),
            mygrid: mygrid.to_string(),
            inbox: Inbox::new(mycall),
            store: StoreForward::new(mycall, mygrid),
            conversations: HashMap::new(),
            slot: 0,
            active_peer: None,
            radio: RadioStatus {
                dial_mhz: 14.0905,
                band: "20m".to_string(),
                sideband: "USB".to_string(),
                transmitting: false,
                slot: 0,
                next_slot_ms: 0,
                // Optimistic until the engine has seen decodes to judge from
                // (the engine recomputes this from recent DT each snapshot).
                time_sync_ok: true,
                rx_level: 0.0,
                tx_level: 0.9,
                tx_enabled: true,
                tuning: false,
                tx_watchdog: false,
                cat_ok: None,
                cat_detail: String::new(),
                audio_error: None,
                tx_even: true,
                rx_offset_hz: 1500.0,
                tx_offset_hz: 1500.0,
                hold_tx_freq: false,
                clock_offset_ms: None,
                source: crate::dto::SourceKind::Native,
                source_label: String::new(),
            },
            link: LinkState {
                tier: Tier::Ft1,
                snr_db: 0.0,
                dt_sec: 0.0,
                freq_hz: 0.0,
                rv: -1,
                state: "idle".to_string(),
                quality: 0.0,
            },
            drained: 0,
        }
    }

    /// Get a mutable handle to the conversation with `peer`, creating it empty
    /// if it does not yet exist.
    fn conversation_mut(&mut self, peer: &str) -> &mut Conversation {
        self.conversations
            .entry(peer.to_string())
            .or_insert_with(|| Conversation {
                peer: peer.to_string(),
                messages: Vec::new(),
            })
    }

    // ----- command surface ------------------------------------------------

    /// Queue a directed free-text message to `peer` and append it to that peer's
    /// conversation as an outbound [`dto::ChatMessage`]. The store-and-forward
    /// queue releases it on air once the peer is heard present.
    pub fn send_message(&mut self, peer: &str, text: &str) {
        self.store.queue(peer, text, self.slot);
        let msg = ChatMessage {
            from: Some(self.mycall.clone()),
            to: Some(peer.to_string()),
            text: text.to_string(),
            slot: self.slot,
            directed_to_me: false,
            outbound: true,
            snr: None,
            freq_hz: None,
            dt_sec: None,
            tier: Some(self.link.tier),
        };
        self.conversation_mut(peer).messages.push(msg);
    }

    /// Echo an outbound open broadcast into the band-activity feed (the
    /// conversation keyed `*`, where inbound broadcasts also land). The text is
    /// the human body (without the `DE <MYCALL>` wire prefix).
    pub fn note_broadcast(&mut self, text: &str) {
        let mycall = self.mycall.clone();
        let tier = self.link.tier;
        let slot = self.slot;
        let msg = ChatMessage {
            from: Some(mycall),
            to: None,
            text: text.to_string(),
            slot,
            directed_to_me: false,
            outbound: true,
            snr: None,
            freq_hz: None,
            dt_sec: None,
            tier: Some(tier),
        };
        self.conversation_mut("*").messages.push(msg);
    }

    /// Select the peer whose conversation the UI is focused on. Creates the
    /// thread if it does not exist yet so the UI has somewhere to render.
    pub fn select_peer(&mut self, peer: &str) {
        self.conversation_mut(peer);
        self.active_peer = Some(peer.to_string());
    }

    /// Switch the link/messaging tier (FT1 / FT8 / FT4 / DX1).
    ///
    /// All tiers are now live: FT1/FT8/FT4 decode+encode natively through the
    /// `modes` pipeline (see [`Tier::mode_kind`]); DX1 uses FT1's robust
    /// non-coherent path. The engine ([`crate::engine::Engine::set_tier`]) also
    /// swaps its active signal source and resizes the slot clock / capture window
    /// to match the selected mode.
    pub fn set_tier(&mut self, tier: Tier) {
        self.link.tier = tier;
    }

    /// The active link tier (selects which waveform the engine modulates/decodes).
    pub fn tier(&self) -> Tier {
        self.link.tier
    }

    // ----- engine-facing surface (driven by the live TX/RX engine) --------

    /// A presence/beacon message ("CQ MYCALL MYGRID").
    pub fn beacon_text(&self) -> String {
        format!("CQ {} {}", self.mycall, self.mygrid)
    }

    /// On-air text frames the store-and-forward queue wants to send now: for any
    /// undelivered message whose recipient is present (heard within `window`) and
    /// out of `backoff`, the identify frame + free-text chunks. Marks attempts.
    pub fn due_frames(&mut self, slot: u64, window: u64, backoff: u64) -> Vec<String> {
        // `self.store` and `self.inbox.roster` are disjoint fields, so this
        // mutable/immutable split borrow is sound.
        self.store
            .due(&self.inbox.roster, slot, window, backoff)
            .into_iter()
            .flat_map(|(_to, frames)| frames)
            .collect()
    }

    /// Mark all queued messages for `peer` delivered (e.g. on an ack).
    pub fn mark_delivered(&mut self, peer: &str) {
        self.store.mark_delivered(peer);
    }

    /// Reflect TX state in the radio status (shown in the UI).
    pub fn set_transmitting(&mut self, on: bool) {
        self.radio.transmitting = on;
    }

    /// Set the RX input audio level (0.0–1.0) shown in the UI meter.
    pub fn set_rx_level(&mut self, level: f32) {
        self.radio.rx_level = level.clamp(0.0, 1.0);
    }

    /// Reflect transmit-enable / tuning / watchdog flags into the radio status.
    pub fn set_tx_flags(&mut self, tx_enabled: bool, tuning: bool, tx_watchdog: bool) {
        self.radio.tx_enabled = tx_enabled;
        self.radio.tuning = tuning;
        self.radio.tx_watchdog = tx_watchdog;
    }

    /// Set the time-sync health flag (engine computes it from recent decode DT).
    pub fn set_time_sync_ok(&mut self, ok: bool) {
        self.radio.time_sync_ok = ok;
    }

    /// Update the slot-timing readout the TopBar renders: milliseconds until the
    /// next T/R boundary. Driven by the radio loop's slot clock.
    pub fn set_slot_timing(&mut self, next_slot_ms: u64) {
        self.radio.next_slot_ms = next_slot_ms;
    }

    /// Feed a slot's worth of decodes through the core: updates the roster and
    /// inbound attribution, folds newly-attributed inbound messages into the
    /// right conversation threads, advances the slot counter, and refreshes the
    /// [`LinkState`] from the strongest decode in the batch.
    pub fn observe(&mut self, decodes: &[Decode], slot: u64) {
        self.slot = slot;
        self.radio.slot = slot;

        // Update the link from the strongest (highest-SNR) decode this slot.
        if let Some(best) = decodes.iter().max_by_key(|d| d.snr) {
            self.link.snr_db = best.snr as f32;
            self.link.dt_sec = best.dt;
            self.link.freq_hz = best.freq;
            // DTO wire contract keeps rv as i32 (-1 = N/A); modes::Decode.rv is
            // Option<i32>, so collapse None -> -1 at this boundary.
            self.link.rv = best.rv.unwrap_or(-1);
            self.link.quality = best.qual;
            self.link.state = "rx".to_string();
        }

        self.inbox.observe(decodes, slot);
        self.drain_inbox();
    }

    /// Fold any inbound messages the inbox has newly attributed into per-peer
    /// conversation threads. Idempotent across calls via the `drained` cursor.
    fn drain_inbox(&mut self) {
        let new: Vec<CoreChatMessage> = self.inbox.messages[self.drained..].to_vec();
        self.drained = self.inbox.messages.len();
        for m in new {
            // Directed traffic (a named recipient) threads under its sender —
            // the peer we're conversing with. Non-directed free text (no
            // recipient, e.g. an open `DE <CALL>` broadcast) lands in the
            // band-activity feed, the conversation keyed `*`.
            let peer = match &m.to {
                Some(_) => m
                    .from
                    .clone()
                    .or_else(|| m.to.clone())
                    .unwrap_or_else(|| "*".to_string()),
                None => "*".to_string(),
            };
            let msg = ChatMessage {
                from: m.from,
                to: m.to,
                text: m.text,
                slot: m.slot,
                directed_to_me: m.directed_to_me,
                outbound: false,
                snr: None,
                freq_hz: None,
                dt_sec: None,
                tier: Some(self.link.tier),
            };
            self.conversation_mut(&peer).messages.push(msg);
        }
    }

    // ----- projection -----------------------------------------------------

    /// Project a core [`HeardStation`] into a DTO [`Station`] with a bucketed
    /// presence relative to the current slot.
    fn station_dto(&self, h: &HeardStation) -> Station {
        let age = self.slot.saturating_sub(h.last_heard_slot);
        let presence = if age <= ACTIVE_WINDOW {
            Presence::Active
        } else if age <= IDLE_WINDOW {
            Presence::Idle
        } else {
            Presence::Stale
        };
        Station {
            call: h.call.clone(),
            grid: h.grid.clone(),
            snr: h.snr,
            last_heard_slot: h.last_heard_slot,
            heard_count: h.heard_count,
            presence,
            // Set by the engine from the logbook (worked-before); default false here.
            worked: false,
        }
    }

    /// Build the full UI snapshot. Stations are ordered most-recently-heard
    /// first; conversations are sorted by peer for stable rendering.
    pub fn snapshot(&self) -> AppSnapshot {
        let stations: Vec<Station> = self
            .inbox
            .roster
            .by_recent()
            .into_iter()
            .map(|h| self.station_dto(h))
            .collect();

        let mut conversations: Vec<Conversation> = self.conversations.values().cloned().collect();
        conversations.sort_by(|a, b| a.peer.cmp(&b.peer));

        AppSnapshot {
            mycall: self.mycall.clone(),
            mygrid: self.mygrid.clone(),
            // Mode + per-mode status are owned by the Engine; default here and
            // let `engine::Engine::snapshot` fill them in.
            mode: OpMode::Chat,
            radio: self.radio.clone(),
            link: self.link.clone(),
            stations,
            conversations,
            active_peer: self.active_peer.clone(),
            qso: None,
            field_day: None,
            // Filled by the engine from its last decodes; empty at the AppState layer.
            recent_decodes: Vec::new(),
            // Filled by the engine while coordinated QSY is enabled; None here.
            qsy: None,
            // Filled by the engine from its session HARQ tally; 0 at this layer.
            harq_rescues: 0,
        }
    }

    /// Update the dial frequency / band / sideband shown in the UI.
    pub fn set_radio(&mut self, dial_mhz: f64, band: &str, sideband: &str) {
        self.radio.dial_mhz = dial_mhz;
        self.radio.band = band.to_string();
        self.radio.sideband = sideband.to_string();
    }

    /// Operator callsign / grid (for sequencers and beacons).
    pub fn identity(&self) -> (&str, &str) {
        (&self.mycall, &self.mygrid)
    }

    /// The peer the operator currently has selected, if any (the coordinated-QSY
    /// partner defaults to this).
    pub fn active_peer(&self) -> Option<&str> {
        self.active_peer.as_deref()
    }

    /// Serialize [`AppState::snapshot`] to a `serde_json::Value` for shells that
    /// prefer to hand the UI an opaque JSON blob.
    pub fn snapshot_json(&self) -> serde_json::Value {
        serde_json::to_value(self.snapshot()).expect("AppSnapshot serializes")
    }

    /// The conversation with `peer`, if one exists.
    pub fn conversation(&self, peer: &str) -> Option<&Conversation> {
        self.conversations.get(peer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_core::text;

    fn dec(msg: &str, snr: i32) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr,
            dt: 0.1,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn send_message_queues_outbound_into_right_conversation() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.send_message("W9XYZ", "MEET AT THE REPEATER AT NOON");

        let conv = app
            .conversation("W9XYZ")
            .expect("conversation created for peer");
        assert_eq!(conv.peer, "W9XYZ");
        assert_eq!(conv.messages.len(), 1);

        let m = &conv.messages[0];
        assert!(m.outbound, "queued message must be outbound");
        assert_eq!(m.from.as_deref(), Some("K2DEF"));
        assert_eq!(m.to.as_deref(), Some("W9XYZ"));
        assert_eq!(m.text, "MEET AT THE REPEATER AT NOON");
        assert!(!m.directed_to_me);
        // It also landed in the store-and-forward queue.
        assert_eq!(app.store.pending(), 1);
    }

    #[test]
    fn observe_directed_decode_updates_roster_and_creates_attributed_inbound() {
        let mut app = AppState::new("K2DEF", "FN31");

        // W9XYZ identifies via a directed grid frame to me (establishes context),
        // then sends a chunked free-text message.
        app.observe(&[dec("K2DEF W9XYZ EN37", -8)], 0);
        let frames = text::chunk("MEET AT THE REPEATER AT NOON", 'A');
        for (i, f) in frames.iter().enumerate() {
            app.observe(&[dec(f, -8)], (i as u64) + 1);
        }

        // Roster learned about W9XYZ (and its grid).
        let station = app.inbox.roster.get("W9XYZ").expect("roster knows W9XYZ");
        assert_eq!(station.grid.as_deref(), Some("EN37"));

        // The inbound message was attributed to W9XYZ and threaded under it.
        let conv = app
            .conversation("W9XYZ")
            .expect("inbound conversation created");
        assert_eq!(conv.messages.len(), 1);
        let m = &conv.messages[0];
        assert!(!m.outbound, "received message must be inbound");
        assert!(m.directed_to_me, "message was addressed to me");
        assert_eq!(m.from.as_deref(), Some("W9XYZ"));
        assert_eq!(m.to.as_deref(), Some("K2DEF"));
        assert_eq!(m.text, text::normalize("MEET AT THE REPEATER AT NOON"));

        // LinkState picked up the decode parameters.
        assert_eq!(app.link.snr_db, -8.0);
        assert_eq!(app.link.freq_hz, 1500.0);
    }

    #[test]
    fn observe_picks_strongest_decode_for_link() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.observe(
            &[
                dec("CQ W9XYZ EN37", -15),
                dec("CQ N0ABC EM48", -3), // strongest
                dec("CQ K1AAA FN42", -10),
            ],
            0,
        );
        assert_eq!(app.link.snr_db, -3.0);
    }

    #[test]
    fn set_tier_selects_all_modes() {
        // All tiers are now live-selectable: FT1/FT8/FT4 native, DX1 robust.
        let mut app = AppState::new("K2DEF", "FN31");
        for t in [Tier::Dx1, Tier::Ft8, Tier::Ft4, Tier::Ft1] {
            app.set_tier(t);
            assert_eq!(app.tier(), t);
        }
    }

    #[test]
    fn snapshot_serializes_with_camelcase_contract_fields() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.observe(&[dec("CQ W9XYZ EN37", -5)], 0);
        app.send_message("W9XYZ", "HELLO");
        app.select_peer("W9XYZ");

        let v = app.snapshot_json();

        // Top-level AppSnapshot camelCase keys.
        for key in [
            "mycall",
            "mygrid",
            "radio",
            "link",
            "stations",
            "conversations",
            "activePeer",
        ] {
            assert!(v.get(key).is_some(), "missing top-level key {key}: {v}");
        }
        assert_eq!(v["mycall"], "K2DEF");
        assert_eq!(v["activePeer"], "W9XYZ");

        // RadioStatus camelCase keys.
        let radio = &v["radio"];
        for key in [
            "dialMhz",
            "band",
            "sideband",
            "transmitting",
            "slot",
            "nextSlotMs",
            "timeSyncOk",
        ] {
            assert!(radio.get(key).is_some(), "missing radio.{key}: {radio}");
        }

        // LinkState camelCase keys.
        let link = &v["link"];
        for key in ["tier", "snrDb", "dtSec", "freqHz", "rv", "state", "quality"] {
            assert!(link.get(key).is_some(), "missing link.{key}: {link}");
        }
        assert_eq!(link["tier"], "FT1");

        // Station camelCase keys + presence enum value.
        let station = &v["stations"][0];
        for key in [
            "call",
            "grid",
            "snr",
            "lastHeardSlot",
            "heardCount",
            "presence",
        ] {
            assert!(
                station.get(key).is_some(),
                "missing station.{key}: {station}"
            );
        }
        assert_eq!(station["call"], "W9XYZ");
        assert_eq!(station["presence"], "active");

        // Conversation + ChatMessage camelCase keys.
        let conv = &v["conversations"][0];
        assert_eq!(conv["peer"], "W9XYZ");
        let msg = &conv["messages"][0];
        for key in [
            "from",
            "to",
            "text",
            "slot",
            "directedToMe",
            "outbound",
            "snr",
            "freqHz",
            "dtSec",
            "tier",
        ] {
            assert!(msg.get(key).is_some(), "missing message.{key}: {msg}");
        }
        assert_eq!(msg["outbound"], true);
        assert_eq!(msg["tier"], "FT1");

        // Full round-trip back into the typed snapshot.
        let back: AppSnapshot = serde_json::from_value(v).expect("round-trips");
        assert_eq!(back.mycall, "K2DEF");
        assert_eq!(back.active_peer.as_deref(), Some("W9XYZ"));
    }
}
