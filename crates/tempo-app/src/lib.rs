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

pub mod alltxt;
pub mod bandplan;
pub mod dto;
pub mod engine;
pub mod privileges;
pub mod update;

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

/// Max store-and-forward send attempts before a message is purged (so a message to a
/// station that never rogers doesn't resend forever — the resend-loop backstop).
const MAX_SEND_ATTEMPTS: u32 = 8;

/// Conversation caps — bound both the IN-MEMORY hot state and the on-disk file so an
/// always-on station can't grow without limit. The in-memory threads are deep-cloned in
/// full on every ~300 ms snapshot poll, so the hot path needs the same bound the
/// persistence path has always had: at most [`MAX_THREAD_MSGS`] messages per thread (keep
/// the most-recent tail) and at most [`MAX_THREADS`] directed threads (most-recently-active
/// win). Shared so what's in memory is exactly what gets persisted (see
/// [`AppState::bound_conversations`] / [`AppState::export_conversations`]).
const MAX_THREAD_MSGS: usize = 200;
const MAX_THREADS: usize = 200;

/// If `msg` is a directed roger (RR73 / RRR) addressed to `mycall`, return the SENDER's
/// call — a delivery ACK for a message we queued to them (Tempo-to-Tempo chat).
fn ack_sender_for(msg: &str, mycall: &str) -> Option<String> {
    use tempo_core::message::{same_call, Msg};
    let (to, de) = match Msg::parse(msg) {
        Msg::Rr73 { to, de } | Msg::Rrr { to, de } => (to, de),
        _ => return None,
    };
    if !de.is_empty() && same_call(&to, mycall) {
        Some(de.to_uppercase())
    } else {
        None
    }
}

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
    /// ACKs we owe: each directed-to-me message we fully received, as `(sender, chunk-id,
    /// incurred-slot)`. The engine drains this each poll and sends one id-bearing
    /// `"<sender> <me> RR73 <id>"` per entry (dropping any past their TTL so ACKs banked
    /// while TX was off don't flush as a stale burst). Re-incurred on a heard resend, so a
    /// lost ACK recovers; the id lets the sender confirm exactly that message.
    pending_acks: Vec<(String, char, u64)>,
}

impl AppState {
    /// Build a fresh state for an operator.
    /// Update the displayed grid (UDP Location / GPS feeders) without
    /// rebuilding the whole app state.
    pub fn set_mygrid(&mut self, grid: &str) {
        self.mygrid = grid.to_uppercase();
    }

    /// Rebind the operator identity (callsign + grid) IN PLACE, preserving
    /// conversation history, the roster, the slot counter, and the active peer.
    /// Used when the operator edits their call/grid in Settings — a typo fix or a
    /// grid update must NOT blank the chat or the `*` band feed (contrast
    /// [`AppState::new`], which starts fresh). Future decodes attribute against the
    /// new call; already-threaded messages keep their historical directed-to-me flag
    /// (correct — they were/weren't addressed to who you were at the time).
    pub fn set_identity(&mut self, mycall: &str, mygrid: &str) {
        self.mycall = mycall.to_string();
        self.mygrid = mygrid.to_string();
        self.inbox.mycall = mycall.to_string();
        self.store.set_identity(mycall, mygrid);
    }

    /// Forget the heard-stations roster (band QSY: the old band's stations aren't
    /// on the new frequency). Conversations, the inbox threads, the `*` band feed,
    /// and the active peer are all preserved — only presence resets.
    pub fn clear_stations(&mut self) {
        self.inbox.roster.clear();
    }

    /// Delete a conversation thread — drops it from the in-memory thread map, cancels any
    /// still-queued outbound messages for that peer, and clears the active peer if it was
    /// selected. A deliberate operator action (the recents-list ✕).
    ///
    /// Dropping the store-and-forward entries is the part that matters ON THE AIR: without
    /// it, deleting a thread left the radio transmitting its queued messages for up to
    /// [`MAX_SEND_ATTEMPTS`] releases, and a message to a never-heard peer (`attempts == 0`,
    /// which `purge` never collects) would have queued indefinitely.
    ///
    /// The thread is not suppressed: the `*` band feed re-creates on the next broadcast, and
    /// a directed peer re-creates if that station transmits to us again. That is intended —
    /// this deletes history, it does not block a station.
    ///
    /// (Wire name kept as `archive_conversation` across the Tauri/TS boundary; renaming it
    /// would touch six files for zero behavior change.)
    pub fn archive_conversation(&mut self, peer: &str) {
        self.conversations.remove(peer);
        self.store.drop_for(peer);
        if self.active_peer.as_deref() == Some(peer) {
            self.active_peer = None;
        }
    }

    /// Snapshot the conversation threads for on-disk persistence — BOUNDED two ways
    /// so the file can't grow without limit on an always-on station: at most
    /// [`MAX_THREAD_MSGS`] messages per thread, and at most [`MAX_THREADS`] directed
    /// threads (the most-recently-active win) plus the `*` band feed. The in-memory
    /// state is already held to these same caps by [`AppState::bound_conversations`],
    /// so this is a stable projection. Pairs with [`AppState::load_conversations`].
    pub fn export_conversations(&self) -> Vec<Conversation> {
        let trim = |c: &Conversation| {
            let start = c.messages.len().saturating_sub(MAX_THREAD_MSGS);
            Conversation {
                peer: c.peer.clone(),
                messages: c.messages[start..].to_vec(),
            }
        };
        let mut out: Vec<Conversation> = Vec::new();
        // Always keep the band feed.
        if let Some(band) = self.conversations.get("*") {
            out.push(trim(band));
        }
        // Then the most-recently-active directed threads (by last-message slot), capped.
        let mut directed: Vec<&Conversation> = self
            .conversations
            .values()
            .filter(|c| c.peer != "*")
            .collect();
        directed.sort_by_key(|c| std::cmp::Reverse(c.messages.last().map(|m| m.slot).unwrap_or(0)));
        out.extend(directed.into_iter().take(MAX_THREADS).map(trim));
        out
    }

    /// Restore persisted conversation threads at startup (the in-memory map is empty
    /// then), so chat history survives an app restart. Keyed by peer.
    pub fn load_conversations(&mut self, convs: Vec<Conversation>) {
        for mut c in convs {
            // Drop PHANTOM threads persisted before the FT1 gate above: a directed
            // conversation whose messages are ALL inbound and explicitly tagged with a
            // non-FT1 tier (folded FT8/FT4 QSO fragments — never a real chat). Keep the
            // `*` band feed and any thread with operator participation (an outbound
            // message) or a genuine FT1 message, so a real unanswered inbound chat
            // survives. An untagged message (`tier: None`, written before ChatMessage
            // carried a tier) is KEPT — we can't prove it was a decode leak, and dropping
            // a real old chat is the worse error. This cleans an operator's existing
            // conversations.json of the leaked decode "chats" on next launch.
            let phantom = c.peer != "*"
                && !c.messages.is_empty()
                && c.messages
                    .iter()
                    .all(|m| !m.outbound && matches!(m.tier, Some(t) if t != Tier::Ft1));
            if phantom {
                continue;
            }
            // The store-and-forward queue IS persisted now (pending_msgs.json journal;
            // restore it BEFORE conversations). A held bubble stays HELD when its message
            // really is still queued — it will transmit when the peer is next heard, which
            // is the whole point of store-and-forward. Only a bubble whose message is NOT
            // in the restored queue (pre-journal sessions, a missing/corrupt journal) is
            // marked ABANDONED, so it can never render as a false "Sent". Messages already
            // on the air (`stored == false`) are untouched: those genuinely were sent.
            for m in &mut c.messages {
                if m.stored && !self.store.has_pending(&c.peer, &m.text) {
                    m.stored = false;
                    m.abandoned = true;
                }
            }
            self.conversations.insert(c.peer.clone(), c);
        }
    }

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
                rig_confirmed: false,
                dial_mhz: 14.074, // FT8 20m (default mode)
                band: "20m".to_string(),
                sideband: "USB".to_string(),
                transmitting: false,
                slot: 0,
                next_slot_ms: 0,
                // Optimistic until the engine has seen decodes to judge from
                // (the engine recomputes this from recent DT each snapshot).
                time_sync_ok: true,
                rf_power: None,  // engine fills from command/read-back
                mic_gain: None,  // engine fills from command/read-back
                nr_level: None,  // engine fills from CAT NR-level read-back
                agc: None,       // engine fills from CAT AGC read-back
                smeter_db: None, // engine fills from CAT STRENGTH read-back
                tx_swr: None,    // engine fills from the keyed-only TX-meter poll
                tx_alc: None,
                tx_po_w: None,
                tx_comp_db: None,
                rig_mode: None, // engine fills from CAT mode read-back (display-only)
                sideband_override: None, // engine fills from the cockpit mode picker (transient)
                phone_seg_lo: None, // engine fills from license privileges for the current band
                phone_seg_hi: None,
                nb: None, // engine fills from the CAT func probe + read-back
                nr: None,
                notch: None,
                comp: None,
                vox: None,
                filter_width_hz: None, // engine fills from the CAT `m` passband read-back
                rit_hz: 0,
                xit_hz: 0,
                active_vfo: String::new(), // engine fills ("A"/"B")
                rx_level: 0.0,
                tx_level: 0.9,
                tx_enabled: true,
                tx_allowed: true,
                tuning: false,
                tx_watchdog: false,
                qso_recording: false,
                cat_ok: None,
                cat_detail: String::new(),
                cw_keyer: "cat".to_string(),
                cw_wpm: 25,
                split_tx_mhz: None,
                audio_error: None,
                radio_config_warning: None,
                tx_even: true,
                tx_cycle_auto: true,
                tr_period_secs: 0.0,
                beacon: false,
                rx_offset_hz: 1500.0,
                tx_offset_hz: 1500.0,
                hold_tx_freq: false,
                clock_offset_ms: None,
                source: crate::dto::SourceKind::Native,
                source_label: String::new(),
            },
            link: LinkState {
                // Default to FT8 on startup — the mode the overwhelming majority of
                // operators use. (Tier is runtime state, not persisted, so every
                // launch starts on FT8.)
                tier: Tier::Ft8,
                snr_db: 0.0,
                dt_sec: 0.0,
                freq_hz: 0.0,
                rv: -1,
                state: "idle".to_string(),
                quality: 0.0,
            },
            drained: 0,
            pending_acks: Vec::new(),
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
        let ack_id = self.store.queue(peer, text, self.slot);
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
            delivered: false, // flips true when the recipient's id-bearing ACK arrives
            ack_id: Some(ack_id), // confirm THIS message by its store chunk-id
            stored: true,     // HELD until the peer is heard and the queue releases it
            abandoned: false,
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
            delivered: false, // broadcasts have no per-recipient ACK
            ack_id: None,
            stored: false, // broadcasts go out directly, never via the store queue
            abandoned: false,
        };
        self.conversation_mut("*").messages.push(msg);
    }

    /// Select the peer whose conversation the UI is focused on. Creates the
    /// thread if it does not exist yet so the UI has somewhere to render.
    pub fn select_peer(&mut self, peer: &str) {
        self.conversation_mut(peer);
        self.active_peer = Some(peer.to_string());
    }

    /// Clear the active peer (deselect) — the UI's null selection must round-trip
    /// so backend consumers of `active_peer` never act on a stale one.
    pub fn clear_peer(&mut self) {
        self.active_peer = None;
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
    /// Frames to transmit now PLUS the logical bodies of any messages released for the
    /// FIRST time this call — the engine records one own-TX band-activity row per released
    /// message (so a store-and-forward message shows when it actually goes on the air, not
    /// at compose time, and not once per resend).
    pub fn due_frames(
        &mut self,
        slot: u64,
        window: u64,
        backoff: u64,
    ) -> (Vec<String>, Vec<String>) {
        // `self.store` and `self.inbox.roster` are disjoint fields, so this
        // mutable/immutable split borrow is sound.
        let mut frames = Vec::new();
        let mut bodies = Vec::new();
        // A `Some(body)` is the message's FIRST release — it just went on the air, so the
        // held bubble stops saying "waiting". Collected here and applied AFTER the loop:
        // calling a `&mut self` helper inside would break the split borrow above.
        let mut released_to = Vec::new();
        for (to, body, fs) in self.store.due(&self.inbox.roster, slot, window, backoff) {
            if let Some(b) = body {
                bodies.push(b);
                released_to.push(to);
            }
            frames.extend(fs);
        }
        for peer in released_to {
            self.mark_conversation_on_air(&peer);
        }
        (frames, bodies)
    }

    /// Mark all queued messages for `peer` delivered (e.g. on an ack).
    /// Undelivered-message count (drives the engine's persist-on-change check).
    pub fn pending_count(&self) -> usize {
        self.store.pending()
    }

    /// The undelivered outbound queue, for the engine's restart journal.
    pub fn export_pending(&self) -> Vec<tempo_core::store::Pending> {
        self.store.export()
    }

    /// Restore the journaled outbound queue at startup — call BEFORE
    /// [`Self::load_conversations`], whose held-vs-abandoned decision reads it.
    pub fn restore_pending(&mut self, items: Vec<tempo_core::store::Pending>) {
        self.store.restore(items);
    }

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
        // ACK obligations the inbox just incurred: each directed-to-me message we received
        // (incl. a heard resend) → owe an id-bearing ACK to its sender. Bounded so a long
        // monitor session can't grow the list. (An incoming ACK is consumed by the inbox as
        // attribution, never folded as chat, so it can't itself owe an ACK — no loop.)
        const MAX_PENDING_ACKS: usize = 32;
        for (from, id) in self.inbox.take_owed_acks() {
            self.pending_acks.push((from.to_uppercase(), id, self.slot));
        }
        if self.pending_acks.len() > MAX_PENDING_ACKS {
            let drop = self.pending_acks.len() - MAX_PENDING_ACKS;
            self.pending_acks.drain(0..drop);
        }
        // ACK-in: a directed RR73 addressed to us confirms a message we queued to that
        // sender arrived → mark it delivered (stops its resend) + stamp the conversation
        // for a real "Delivered ✓"; then drop spent queue entries. Chat (FT1) only.
        if self.link.tier == Tier::Ft1 {
            let mut acked: Vec<String> = decodes
                .iter()
                .filter_map(|d| ack_sender_for(&d.message, &self.mycall))
                .collect();
            acked.sort();
            acked.dedup(); // one RR73 may be decoded twice in a slot
            for from in acked {
                if self.store.mark_one_delivered(&from) {
                    self.mark_conversation_delivered(&from);
                }
            }
            self.store.purge(MAX_SEND_ATTEMPTS);
        }
        // Conversation threads are a Tempo (FT1) feature. FT8 / FT4 / DX1 decodes are
        // QSO traffic — folding their CQ/exchange/free-text fragments into per-peer
        // threads turned the recents list into a feed of phantom "chats" the operator
        // never started. Keep `inbox.observe` above (it feeds the shared roster used by
        // Operate too), but only FOLD into conversations in FT1/Tempo; otherwise advance
        // the cursor so this slot's frames are discarded (and can't replay on an FT1
        // switch). Outbound chats (send_message) bypass this — they push directly.
        if self.link.tier == Tier::Ft1 {
            self.drain_inbox();
        } else {
            self.drained = self.inbox.messages.len();
        }
        // Keep the in-memory threads bounded so the hot state (deep-cloned in full on
        // every ~300 ms snapshot poll) can't grow without limit over a long session.
        self.bound_conversations();
    }

    /// Drain the id-bearing ACKs we owe. The engine sends one `"<sender> <me> RR73 <id>"`
    /// per entry. Cleared each poll.
    pub fn take_pending_acks(&mut self) -> Vec<(String, char, u64)> {
        std::mem::take(&mut self.pending_acks)
    }

    /// Stamp the OLDEST still-undelivered outbound message to `peer` as delivered (one
    /// RR73 confirms one message, FIFO) — drives the real "Delivered ✓".
    fn mark_conversation_delivered(&mut self, peer: &str) {
        if let Some(conv) = self.conversations.get_mut(peer) {
            if let Some(m) = conv
                .messages
                .iter_mut()
                .find(|m| m.outbound && !m.delivered)
            {
                m.delivered = true;
            }
        }
    }

    /// Clear `stored` on the OLDEST still-held outbound message to `peer` — it just went on
    /// the air for the first time (one release clears one bubble, FIFO, mirroring
    /// `mark_conversation_delivered`).
    fn mark_conversation_on_air(&mut self, peer: &str) {
        if let Some(conv) = self.conversations.get_mut(peer) {
            if let Some(m) = conv.messages.iter_mut().find(|m| m.outbound && m.stored) {
                m.stored = false;
            }
        }
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
            // NOTE: the ACK obligation is no longer derived here — it's incurred by the
            // inbox on EVERY directed-to-me completion (incl. resends we dedup from the
            // display) and drained in `observe`, so a lost ACK recovers.
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
                delivered: false,
                ack_id: None,  // inbound — no outbound id to confirm
                stored: false, // inbound — nothing of ours is queued
                abandoned: false,
            };
            self.conversation_mut(&peer).messages.push(msg);
        }
    }

    /// Hold the in-memory conversation threads to a constant size so the hot state —
    /// deep-cloned in full on every ~300 ms snapshot poll — can't grow without bound on
    /// an always-on station. Trims each thread to the most-recent [`MAX_THREAD_MSGS`]
    /// messages and evicts the least-recently-active directed threads past
    /// [`MAX_THREADS`]. The `*` band feed and the active peer's thread are always kept.
    /// Called each slot from [`AppState::observe`]; mirrors [`AppState::export_conversations`].
    fn bound_conversations(&mut self) {
        // Trim each thread to a rolling tail of its most-recent messages.
        for conv in self.conversations.values_mut() {
            let len = conv.messages.len();
            if len > MAX_THREAD_MSGS {
                conv.messages.drain(0..len - MAX_THREAD_MSGS);
            }
        }
        // Evict least-recently-active directed threads past the cap. The `*` band feed
        // and the active peer's thread are protected (never counted, never removed).
        let active = self.active_peer.clone();
        let mut directed: Vec<(String, u64)> = self
            .conversations
            .iter()
            .filter(|(peer, _)| peer.as_str() != "*" && active.as_deref() != Some(peer.as_str()))
            .map(|(peer, conv)| {
                (
                    peer.clone(),
                    conv.messages.last().map(|m| m.slot).unwrap_or(0),
                )
            })
            .collect();
        if directed.len() > MAX_THREADS {
            // Most-recently-active first; drop everything past the cap.
            directed.sort_by_key(|(_, slot)| std::cmp::Reverse(*slot));
            for (peer, _) in directed.into_iter().skip(MAX_THREADS) {
                self.conversations.remove(&peer);
            }
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
            lotw_user: false, // stamped by the engine snapshot loop (resolver lives there)
            presence,
            // Set by the engine from the logbook (worked-before); default false here.
            worked: false,
            // Resolved by the engine from the DXCC resolver; None at this layer.
            country: None,
            tier: h.mode.map(Tier::from_mode_kind),
            // Stamped by the engine from the grid-rarity resolver; None here.
            grid_rarity: None,
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
            ai_cw: crate::dto::AiCwStatus::default(),
            mycall: self.mycall.clone(),
            mygrid: self.mygrid.clone(),
            // Mode + per-mode status are owned by the Engine; default here and
            // let `engine::Engine::snapshot` fill them in.
            mode: OpMode::Chat,
            radio: self.radio.clone(),
            // Multi-radio summaries are owned by the Engine (fills them in snapshot()).
            radios: Vec::new(),
            active_radio_id: 0,
            radio_pegged: false,
            link: self.link.clone(),
            chat_cq: String::new(), // Engine fills the run state in snapshot()
            stations,
            conversations,
            active_peer: self.active_peer.clone(),
            qso: None,
            // Work-a-spot navigation hint — owned by the engine (fills in snapshot()).
            work_tick: 0,
            work_view: None,
            work_call: None,
            field_day: None,
            // Filled by the engine from its last decodes; empty at the AppState layer.
            recent_decodes: Vec::new(),
            highlights: Vec::new(),
            clear_tick: 0,
            hunt: None,
            // Filled by the engine while coordinated QSY is enabled; None here.
            qsy: None,
            // Filled by the engine from its session HARQ tally; 0 at this layer.
            harq_rescues: 0,
            // Filled by the engine when a QSO awaits confirm-before-log; None here.
            pending_log: None,
            // Filled by the engine from its last connector-upload note; empty here.
            upload_note: None,
            upload_ok: false,
            upload_tick: 0,
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
    fn set_identity_preserves_conversations_roster_and_band_feed() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.send_message("W9XYZ", "MEET AT THE REPEATER"); // directed thread + store queue
        app.note_broadcast("CQ FN31"); // the `*` band feed
        assert!(app.conversation("W9XYZ").is_some());
        assert!(app.conversation("*").is_some());
        let pending_before = app.store.pending();

        // Fix a typo'd callsign (and bump the grid) — an in-place rebind must keep
        // ALL history (the reported "Settings save blanks the band feed" defect).
        app.set_identity("K2DEFX", "FN42");
        assert_eq!(app.mycall, "K2DEFX");
        assert_eq!(app.mygrid, "FN42");
        assert_eq!(
            app.inbox.mycall, "K2DEFX",
            "inbox attribution rebound to the new call"
        );
        assert!(
            app.conversation("W9XYZ").is_some(),
            "directed thread preserved"
        );
        assert!(app.conversation("*").is_some(), "band feed preserved");
        assert_eq!(
            app.store.pending(),
            pending_before,
            "pending queue preserved"
        );
    }

    #[test]
    fn conversations_round_trip_through_export_load() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.send_message("W9XYZ", "MEET AT NOON");
        app.note_broadcast("CQ FN31");
        let saved = app.export_conversations();
        assert_eq!(saved.len(), 2, "directed thread + the * band feed");

        // A fresh session (e.g. app restart) restores them.
        let mut restored = AppState::new("K2DEF", "FN31");
        restored.load_conversations(saved);
        assert!(
            restored.conversation("W9XYZ").is_some(),
            "directed thread restored"
        );
        assert!(restored.conversation("*").is_some(), "band feed restored");
        assert_eq!(
            restored.conversation("W9XYZ").unwrap().messages[0].text,
            "MEET AT NOON"
        );
    }

    #[test]
    fn export_conversations_bounds_thread_count_and_keeps_band_feed() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.note_broadcast("CQ"); // the `*` band feed
        for i in 0..250 {
            app.send_message(&format!("W{i}AA"), "HI");
        }
        let saved = app.export_conversations();
        assert!(
            saved.len() <= 201,
            "thread count bounded (200 directed + band), got {}",
            saved.len()
        );
        assert!(
            saved.iter().any(|c| c.peer == "*"),
            "band feed is always kept"
        );
    }

    #[test]
    fn observe_bounds_in_memory_thread_message_count() {
        // Regression: the in-memory thread is deep-cloned in full on every snapshot
        // poll, so an always-on station's messages must stay bounded (keeping the
        // most-recent tail) — not just the on-disk copy. Overfill one thread, then let
        // a quiet slot tick trim it.
        let mut app = AppState::new("K2DEF", "FN31");
        for i in 0..(MAX_THREAD_MSGS + 50) {
            app.send_message("W9XYZ", &format!("MSG {i}"));
        }
        assert!(
            app.conversation("W9XYZ").unwrap().messages.len() > MAX_THREAD_MSGS,
            "precondition: thread overfilled before a slot tick trims it"
        );

        app.observe(&[], 1); // a quiet slot tick runs the in-memory bound

        let conv = app.conversation("W9XYZ").unwrap();
        assert_eq!(
            conv.messages.len(),
            MAX_THREAD_MSGS,
            "in-memory thread trimmed to the rolling cap"
        );
        assert_eq!(
            conv.messages.last().unwrap().text,
            format!("MSG {}", MAX_THREAD_MSGS + 50 - 1),
            "the most-recent message is retained (tail, not head)"
        );
    }

    #[test]
    fn observe_bounds_in_memory_thread_count_keeping_band_and_active_peer() {
        // Regression: the number of in-memory directed threads must also stay bounded,
        // while the `*` band feed and the operator's currently-selected peer are never
        // evicted.
        let mut app = AppState::new("K2DEF", "FN31");
        app.note_broadcast("CQ"); // the `*` band feed
        for i in 0..(MAX_THREADS + 50) {
            app.send_message(&format!("W{i}AA"), "HI");
        }
        app.select_peer("W5AA"); // protect one thread as the active peer

        app.observe(&[], 1); // a quiet slot tick runs the in-memory bound

        let directed = app
            .conversations
            .keys()
            .filter(|k| k.as_str() != "*")
            .count();
        assert!(
            directed <= MAX_THREADS + 1,
            "directed threads bounded (cap + protected active peer), got {directed}"
        );
        assert!(app.conversation("*").is_some(), "band feed always kept");
        assert!(
            app.conversation("W5AA").is_some(),
            "active peer's thread is never evicted"
        );
    }

    #[test]
    fn archive_conversation_removes_only_the_targeted_thread() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.send_message("W9XYZ", "HI");
        app.send_message("K7ABC", "HELLO");
        app.select_peer("W9XYZ");
        app.archive_conversation("W9XYZ");
        assert!(
            app.conversation("W9XYZ").is_none(),
            "archived thread removed"
        );
        assert!(
            app.conversation("K7ABC").is_some(),
            "other threads preserved"
        );
        assert_eq!(
            app.active_peer, None,
            "active peer cleared when its thread is archived"
        );
    }

    #[test]
    fn deleting_a_thread_cancels_its_queued_traffic_but_not_other_peers() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.send_message("W9XYZ", "HI");
        app.send_message("W9XYZ", "AGAIN");
        app.send_message("K7ABC", "HELLO");
        assert_eq!(app.store.pending(), 3, "all three queued");

        app.archive_conversation("W9XYZ");
        assert_eq!(
            app.store.pending(),
            1,
            "deleting the thread cancels ONLY that peer's queued messages — otherwise the \
             radio keeps transmitting to a conversation the operator deleted"
        );

        // The surviving message must still be releasable: prove it by hearing K7ABC and
        // draining the queue. If drop_for had over-matched, there'd be no frames here.
        app.observe(&[dec("K2DEF K7ABC EN61", -5)], 1);
        let (frames, _) = app.due_frames(1, 30, 4);
        assert!(
            frames.iter().any(|f| f.contains("K7ABC")),
            "K7ABC's message still releases after W9XYZ's thread was deleted: {frames:?}"
        );
    }

    #[test]
    fn a_held_message_is_marked_stored_until_it_first_goes_on_the_air() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.set_tier(Tier::Ft1);
        app.send_message("W9XYZ", "HI");

        let held = |a: &AppState| a.conversation("W9XYZ").unwrap().messages[0].stored;
        assert!(
            held(&app),
            "queued for a peer we have not heard — held, not on the air"
        );

        // Peer still unheard: the queue releases nothing, so it stays held.
        let (frames, _) = app.due_frames(1, 30, 4);
        assert!(frames.is_empty(), "nothing releases for an absent peer");
        assert!(held(&app), "still held while the peer is absent");

        // Hear the peer → the message releases → the bubble stops claiming "waiting".
        app.observe(&[dec("K2DEF W9XYZ EN61", -5)], 2);
        let (frames, bodies) = app.due_frames(2, 30, 4);
        assert!(!frames.is_empty(), "released once the peer is present");
        assert_eq!(bodies.len(), 1, "one first-release body");
        assert!(
            !held(&app),
            "cleared on FIRST release — it is now sent-awaiting-ACK, not held"
        );
        assert!(
            !app.conversation("W9XYZ").unwrap().messages[0].delivered,
            "on the air is NOT delivered — that still waits for the ACK"
        );
    }

    #[test]
    fn a_restored_held_message_is_marked_abandoned_not_sent() {
        // The queue journal is missing (pre-journal session, corrupt file): the bubbles come
        // back but the queue didn't. A message that was HELD can never transmit now. Clearing
        // `stored` alone made it render as a plain "Sent" — an INVISIBLE broken promise:
        // the operator believes it went out and never re-sends. It must say so instead.
        let mut app = AppState::new("K2DEF", "FN31");
        let mut sender = AppState::new("K2DEF", "FN31");
        sender.send_message("W9XYZ", "HI");
        let persisted = sender.export_conversations();
        assert!(
            persisted[0].messages[0].stored,
            "precondition: it was held when persisted"
        );

        app.load_conversations(persisted);
        assert_eq!(
            app.store.pending(),
            0,
            "precondition: the queue did NOT survive the restart"
        );
        let m = &app.conversation("W9XYZ").unwrap().messages[0];
        assert!(
            !m.stored,
            "no longer 'waiting to send' — there is no queue left to send it"
        );
        assert!(
            m.abandoned,
            "and it must SAY it was abandoned — rendering a never-transmitted message as \
             'Sent' is the app asserting something false"
        );
    }

    #[test]
    fn a_journaled_held_message_survives_the_restart_still_held() {
        // THE store-and-forward persistence fix (2026-07-21): when the queue journal
        // survives, a held message comes back still HELD — it will transmit when the
        // peer is next heard, which is the entire point of store-and-forward. Restore
        // order matters: queue BEFORE conversations, since the held-vs-abandoned
        // decision on each bubble reads the live queue.
        let mut sender = AppState::new("K2DEF", "FN31");
        sender.send_message("W9XYZ", "HI");
        let convs = sender.export_conversations();
        let journal = sender.export_pending();
        assert_eq!(journal.len(), 1, "precondition: one journaled entry");

        let mut app = AppState::new("K2DEF", "FN31");
        app.restore_pending(journal);
        app.load_conversations(convs);
        assert_eq!(app.store.pending(), 1, "the queue survived the restart");
        let m = &app.conversation("W9XYZ").unwrap().messages[0];
        assert!(
            m.stored,
            "still 'waiting to send' — the queue really does hold it"
        );
        assert!(
            !m.abandoned,
            "a live queued message must not read as abandoned"
        );

        // And a SECOND bubble whose message is NOT in the journal still goes abandoned —
        // the split is per-message, not per-session.
        let mut sender2 = AppState::new("K2DEF", "FN31");
        sender2.send_message("W9XYZ", "FIRST");
        sender2.send_message("W9XYZ", "SECOND");
        let convs2 = sender2.export_conversations();
        let mut journal2 = sender2.export_pending();
        journal2.retain(|p| p.text == "SECOND"); // FIRST's entry "lost"
        let mut app2 = AppState::new("K2DEF", "FN31");
        app2.restore_pending(journal2);
        app2.load_conversations(convs2);
        let msgs = &app2.conversation("W9XYZ").unwrap().messages;
        let first = msgs.iter().find(|m| m.text == "FIRST").unwrap();
        let second = msgs.iter().find(|m| m.text == "SECOND").unwrap();
        assert!(first.abandoned && !first.stored, "lost entry → abandoned");
        assert!(
            second.stored && !second.abandoned,
            "surviving entry → still held"
        );
    }

    #[test]
    fn observe_directed_decode_updates_roster_and_creates_attributed_inbound() {
        let mut app = AppState::new("K2DEF", "FN31");
        app.set_tier(Tier::Ft1); // conversation folding is a Tempo (FT1) feature

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
    fn ft8_decodes_do_not_create_tempo_conversations() {
        // The reported leak: operating FT8, its decodes must NOT become phantom chats.
        let mut app = AppState::new("K2DEF", "FN31");
        app.set_tier(Tier::Ft8);
        // A directed-context frame + free text that WOULD thread under FT1.
        app.observe(&[dec("K2DEF W9XYZ EN37", -8)], 0);
        for (i, f) in text::chunk("TNX 73 GL DX", 'A').iter().enumerate() {
            app.observe(&[dec(f, -8)], (i as u64) + 1);
        }
        assert!(
            app.conversation("W9XYZ").is_none(),
            "FT8 decodes must not create a Tempo conversation"
        );
        // The roster still learns the station (inbox.observe runs for Operate).
        assert!(
            app.inbox.roster.get("W9XYZ").is_some(),
            "roster still updates"
        );
        // Switching to FT1 resumes real chat folding (no replay of the FT8 frames).
        app.set_tier(Tier::Ft1);
        app.observe(&[dec("K2DEF N0ABC EM48", -8)], 10);
        for (i, f) in text::chunk("HI SETH", 'B').iter().enumerate() {
            app.observe(&[dec(f, -8)], 11 + i as u64);
        }
        assert!(
            app.conversation("N0ABC").is_some(),
            "FT1 chat still threads"
        );
        assert!(
            app.conversation("W9XYZ").is_none(),
            "no FT8 replay on FT1 switch"
        );
    }

    #[test]
    fn load_conversations_drops_phantom_ft8_threads() {
        let mut app = AppState::new("K2DEF", "FN31");
        let inbound = |tier: Tier| ChatMessage {
            from: Some("W9XYZ".into()),
            to: Some("K2DEF".into()),
            text: "RR73".into(),
            slot: 0,
            directed_to_me: true,
            outbound: false,
            snr: None,
            freq_hz: None,
            dt_sec: None,
            tier: Some(tier),
            delivered: false,
            ack_id: None,
            stored: false,
            abandoned: false,
        };
        let outbound = ChatMessage {
            outbound: true,
            ..inbound(Tier::Ft1)
        };
        app.load_conversations(vec![
            // Phantom: all inbound, FT8 tier → dropped.
            Conversation {
                peer: "PHANTOM".into(),
                messages: vec![inbound(Tier::Ft8)],
            },
            // Real FT1 inbound chat → kept.
            Conversation {
                peer: "REALRX".into(),
                messages: vec![inbound(Tier::Ft1)],
            },
            // Operator participated (outbound), even if FT8 tier → kept.
            Conversation {
                peer: "REALTX".into(),
                messages: vec![outbound],
            },
            // The band feed is always kept.
            Conversation {
                peer: "*".into(),
                messages: vec![inbound(Tier::Ft8)],
            },
            // Untagged legacy message (persisted before ChatMessage carried a
            // tier) → kept; we can't prove it was a decode leak.
            Conversation {
                peer: "LEGACY".into(),
                messages: vec![ChatMessage {
                    tier: None,
                    ..inbound(Tier::Ft1)
                }],
            },
        ]);
        assert!(app.conversation("PHANTOM").is_none(), "FT8 phantom dropped");
        assert!(
            app.conversation("REALRX").is_some(),
            "real FT1 inbound kept"
        );
        assert!(
            app.conversation("REALTX").is_some(),
            "operator-participated kept"
        );
        assert!(app.conversation("*").is_some(), "band feed kept");
        assert!(
            app.conversation("LEGACY").is_some(),
            "untagged legacy thread kept"
        );
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
        assert_eq!(link["tier"], "FT8"); // default mode is FT8

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
            "stored",
        ] {
            assert!(msg.get(key).is_some(), "missing message.{key}: {msg}");
        }
        assert_eq!(msg["outbound"], true);
        assert_eq!(msg["tier"], "FT8"); // default mode is FT8

        // Full round-trip back into the typed snapshot.
        let back: AppSnapshot = serde_json::from_value(v).expect("round-trips");
        assert_eq!(back.mycall, "K2DEF");
        assert_eq!(back.active_peer.as_deref(), Some("W9XYZ"));
    }
}
