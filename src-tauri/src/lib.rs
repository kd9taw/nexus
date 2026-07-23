//! Tempo desktop shell (Tauri v2).
//!
//! A thin host around the live [`tempo_app::engine::Engine`] (which wraps the
//! UI-facing `AppState` and the FT1 modem). The engine is shared as
//! `Arc<Mutex<Engine>>` between the Tauri command handlers and — when built with
//! the `radio` feature — the background radio loop ([`tempo_audio::service`])
//! that drives the sound card and PTT on the FT1 slot clock. Each command locks
//! the engine, calls it, and returns one of the shared camelCase DTOs.
//!
//! ## Commands (matched to `../ui/src/api.ts`)
//! - `get_snapshot` -> `AppSnapshot`
//! - `send_message { peer, text }` -> `AppSnapshot`
//! - `select_peer { peer }` -> `AppSnapshot`
//! - `set_tier { tier }` -> `AppSnapshot`  (tier is "TempoFast" | "FT8" | "FT4" | "TempoDeep")
//! - `get_spectrum_row` -> `Spectrum`      (one waterfall row)
//!
//! ## Live radio (`--features radio`, built on the station PC)
//! With the `radio` feature, `run()` spawns [`tempo_audio::service::run_radio`]
//! on a dedicated thread: it opens the default sound devices (cpal), keys PTT via
//! `rigctld` (run `rigctld -m <model> -r <port>`; or VOX), and on each slot
//! transmits the engine's `poll_tx` audio or decodes the captured frame into the
//! shared engine. Without the feature the shell still builds and serves state;
//! it just does not touch the radio (handy for UI development on a box with no
//! audio).
//!
//! Build on a machine with a WebView toolchain (Linux: webkit2gtk-4.1 + libsoup;
//! Windows: WebView2; macOS: WKWebView): `cargo tauri dev` /
//! `cargo tauri build --features radio`.

/// Window → radio-chain addressing: the `(panel, instance)` token grammar, the window-label
/// parser both the geometry store and the (future) chain resolver share, and the one-entry
/// chain registry. Inert at runtime — see the module docs.
mod chains;

use chains::{panel_key, panel_label, Instance};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;
use tauri::State;
use tempo_app::dto::{
    AppSnapshot, DiagnosticsReportDto, ImportStats, LoggedQso, LotwSyncResult, SourceKind,
    Spectrum, Tier, UploadReportDto,
};
use tempo_app::engine::Engine;
use tempo_app::settings::{Settings, VoiceMessage};

/// The engine, shared between UI commands and the radio loop.
type SharedEngine = Arc<Mutex<Engine>>;

/// Cached propagation nowcast: `(fetched_at, snapshot)`. Caching enforces PSK
/// Reporter's ≥5-minute-per-dataset query limit across UI polls.
type PropCache = Arc<Mutex<Option<(std::time::Instant, propagation::PropagationSnapshot)>>>;
/// TTL cache for the OVATION aurora oval (distinct payload type from PropCache, so
/// a distinct TypeId for `.manage()`).
type AuroraCache = Arc<
    Mutex<
        Option<(
            std::time::Instant,
            Vec<propagation::live::aurora::AuroraPoint>,
        )>,
    >,
>;
/// TTL cache for the KC2G ionosonde MUF map. Distinct payload type → distinct
/// TypeId for `.manage()`.
type Kc2gCache = Arc<Mutex<Option<(std::time::Instant, Vec<propagation::MufStation>)>>>;
type ProtonCache = Arc<Mutex<Option<(std::time::Instant, propagation::live::protons::ProtonFlux)>>>;
/// TTL cache for the NOAA R/S/G scales + recent SWPC alerts (one fetch pair).
/// Distinct payload type → distinct TypeId for `.manage()`.
type ScalesCache = Arc<
    Mutex<
        Option<(
            std::time::Instant,
            (propagation::NoaaScalesView, Vec<propagation::AlertView>),
        )>,
    >,
>;

/// Recent DX-cluster / RBN spots, fed by the background cluster thread and read
/// by `get_need_alerts`.
type SharedSpots = Arc<Mutex<tempo_net::cluster::SpotBuffer>>;

/// Rolling space-weather sample history (SFI/Kp/X-ray + representative MUF), fed once
/// per fresh SWPC fetch in `get_propagation`, read for the "MUF building / Kp rising"
/// trend. Distinct payload type → distinct TypeId for `.manage()`.
type SharedWxHistory = Arc<Mutex<propagation::SpaceWxHistory>>;

/// The cluster thread runs for the process lifetime (a desktop daemon thread);
/// this stop flag exists only so `cluster::run`'s signature is satisfied.
static CLUSTER_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Outbound command queue for the HUMAN DX-cluster nodes — the `post_spot`
/// command pushes a formatted `DX …` line here and the nodes' pump loops flush
/// it once logged in. Nodes DRAIN (remove) each line, so with several nodes up
/// exactly ONE posts each spot (whichever pump grabs it first). RBN skimmer
/// feeds get [`RBN_DEAD_OUTBOX`] instead — you never post spots to a skimmer.
static CLUSTER_OUTBOX: std::sync::Mutex<std::collections::VecDeque<String>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());
/// A never-fed outbox for the receive-only RBN feeds (satisfies `run`'s signature).
static RBN_DEAD_OUTBOX: std::sync::Mutex<std::collections::VecDeque<String>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// Per-host once-latch for the human DX-cluster nodes (the SSB/phone aggregator): the set of
/// node hosts already spawned this process. `set_settings` re-runs the spawn for every save,
/// so this lets a NEWLY-added node connect live (skipping ones already up) without a restart.
/// Also the source for the phone-source health label. Cleared on a callsign restart.
static HUMAN_NODES_STARTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// One connected-flag per spawned human node, so "phone source up" can mean ANY node is up
/// (a single shared bool would flicker false whenever one of several nodes reconnected).
/// `get_feed_health` ORs them; cleared alongside [`HUMAN_NODES_STARTED`] on a restart.
static PHONE_NODE_CONNS: Mutex<Vec<Arc<std::sync::atomic::AtomicBool>>> = Mutex::new(Vec::new());

/// The RBN skimmer firehoses (connected automatically when cluster spotting is on): CW/RTTY
/// on 7000, FT8/FT4 digital on 7001. Huge volume + exact frequencies → the CW + digital need
/// evidence. SSB/phone has no skimmer network, so it comes from the human nodes (cluster_hosts).
const RBN_CW_HOST: &str = "telnet.reversebeacon.net:7000";
const RBN_DIGITAL_HOST: &str = "telnet.reversebeacon.net:7001";
static RBN_CW_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RBN_DIGITAL_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Recent live PSK Reporter MQTT reception reports (the "getting out" firehose),
/// fed by the background MQTT thread and merged into the propagation nowcast.
type SharedLivePaths = Arc<Mutex<propagation::LiveSpots>>;
/// Last fetched POTA+SOTA activator spots (unix stamp + rows) — refreshed by the
/// hunter view's poll; read lock-only by the Needed scorer for POTA/SOTA tags.
type SharedOtaSpots =
    Arc<Mutex<std::collections::HashMap<String, (i64, Vec<propagation::OtaSpot>)>>>;

/// The locally-searchable POTA park directory (imported or downloaded once, searched offline).
/// Distinct payload type → distinct TypeId for `.manage()`.
type SharedParks = Arc<Mutex<tempo_core::pota::ParkIndex>>;

/// Persistent opening-detection tracker (anomaly/onset hysteresis + onset
/// stamping) across successive `get_propagation` polls. Stateful so it can flag a
/// genuine onset (`is_new`) exactly once and keep a sustained opening latched.
type SharedOpeningTracker = Arc<Mutex<propagation::OpeningTracker>>;

/// Near-region opening spots (Phase 2): spots geographically near the operator on
/// the VHF/10 m opening bands where NEITHER end is the operator. Kept SEPARATE
/// from `SharedLivePaths` so they enrich the opening detector without polluting the
/// advisor's own-call "who hears me / who I hear".
///
/// A NEWTYPE, not a `type` alias: Tauri keys managed state by `TypeId`, and an
/// alias to `Arc<Mutex<LiveSpots>>` would collide with `SharedLivePaths` (same
/// TypeId) → `.manage()` panics at startup and DI can't tell the buffers apart.
struct SharedRegionPaths(Arc<Mutex<propagation::LiveSpots>>);

/// Lifetime stop flag for the PSK Reporter MQTT daemon thread (see CLUSTER_STOP).
static PSKR_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One-shot latch for the PSK Reporter MQTT daemon thread (see RBN_CW_STARTED).
static PSKR_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// WSPR poller ownership: (generation, callsign being polled). A start for the
/// SAME call is a no-op (apply_settings calls this on every save); a start for
/// a DIFFERENT call — or an invalidated one — bumps the generation, which the
/// running thread observes and exits on (checked immediately before every
/// push, so even a fetch in flight across restart_live_feeds' drain can never
/// repollute the buffer with the old call's evidence — review catch).
static WSPR_FEED: Mutex<(u64, String)> = Mutex::new((0, String::new()));

/// One-shot latch for the near-region opening MQTT daemon thread (Phase 2).
static PSKR_REGION_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Keep a near-region spot only if an end is within this many km of the operator.
/// 800 km — a station within ~strong-tropo / short-Es-hop range, i.e. "activity
/// genuinely touching my region". The old 2000 km ("one full Es hop") admitted
/// far↔far paths a continent away that polluted the operator-anchored opening
/// detector + advisor with activity the operator could NOT hear (the phantom-6m
/// complaint). The per-band global stream is gated client-side (the broker can't
/// filter by region).
const REGION_RADIUS_KM: f64 = 800.0;

/// Liveness of the background live feeds, updated from their daemon threads and
/// read by `get_feed_health` for the Now-Bar connector pills. Timestamps are Unix
/// secs of the last *successfully parsed* event; `0` = none yet this session.
#[derive(Default)]
struct FeedHealthState {
    /// Last parsed DX-cluster / RBN spot.
    cluster_last: std::sync::atomic::AtomicI64,
    /// Last successfully parsed PSK Reporter MQTT report.
    pskr_last_event: std::sync::atomic::AtomicI64,
    /// Cluster telnet session currently up (set on TCP-established, cleared on drop).
    /// Lets the pill say "connected" (quiet but healthy) instead of an ambiguous
    /// "waiting" that reads as broken.
    cluster_connected: std::sync::atomic::AtomicBool,
    /// PSK Reporter MQTT session currently up (set on accepted CONNACK, cleared on drop).
    pskr_connected: std::sync::atomic::AtomicBool,
    /// Last parsed PHONE-CLASSED spot from any human DX-cluster node — the true "is SSB
    /// actually arriving?" signal. Stamped ONLY when a human-node spot classifies as Phone
    /// (a human node's feed is mostly CW; stamping on every spot made the pill read live off
    /// CW traffic and masked a phone drought). Per-node connected state lives in
    /// [`PHONE_NODE_CONNS`].
    phone_cluster_last: std::sync::atomic::AtomicI64,
    /// Running count of PHONE-classed spots seen from human nodes this session — the
    /// diagnostic number behind "N SSB spots" on the Needed board. Reset on a feed restart.
    phone_spots_seen: std::sync::atomic::AtomicI64,
}
type SharedHealth = Arc<FeedHealthState>;

/// Cached QRZ XML session key (in-memory only — it's IP-bound, short-lived, and
/// re-derivable from the keychain password, so it never persists). `None` = not
/// logged in yet / expired.
type SharedQrzSession = Arc<Mutex<Option<String>>>;

/// Cached HamQTH XML session id (in-memory only — short-lived, re-derivable from
/// the keychain password, never persisted). `None` = not logged in yet / expired.
///
/// A NEWTYPE, not a bare alias like `SharedQrzSession`: a `type` alias of the same
/// `Arc<Mutex<Option<String>>>` shares QRZ's `TypeId`, so Tauri's TypeId-keyed
/// managed-state DI could not tell the two session caches apart (and `.manage()`
/// would collide). This is the same reason `SharedRegionPaths` is a newtype beside
/// `SharedLivePaths`.
#[derive(Default)]
struct SharedHamQthSession(Arc<Mutex<Option<String>>>);

/// A feed counts as "live" if it parsed an event within this window; older ⇒
/// "idle" (a lull on a quiet band, or a feed problem — the UI tooltip says so).
/// Generous so a normal band lull doesn't flip the pill.
const FEED_FRESH_SECS: i64 = 900;

/// PSK Reporter MQTT broker (plain MQTT over TCP).
const PSKR_MQTT_ADDR: &str = "mqtt.pskreporter.info:1883";

/// Current Unix time in seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A plausibly-real operator callsign — checked by SHAPE, not a denylist. Network
/// features that log in to public services (DX cluster / RBN, PSK Reporter MQTT)
/// gate on this so they never key an unset/garbage call on a public service.
///
/// The old check denylisted exactly `"KD9TAW"` (the shipped placeholder) — but
/// that is also a real operator's call, so it left every feed permanently OFF for
/// that operator. The default is now empty (`""`), and "configured" = a real call
/// has been entered (3–10 chars, has a letter AND a digit, alnum/`/` only).
fn is_real_call(call: &str) -> bool {
    let c = call.trim();
    let len = c.chars().count();
    (3..=10).contains(&len)
        && c.chars().any(|ch| ch.is_ascii_digit())
        && c.chars().any(|ch| ch.is_ascii_alphabetic())
        && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '/')
}

/// Connect ALL enabled spot sources (SpotCollector-style aggregation): the RBN CW + RBN
/// digital skimmer firehoses, plus EVERY human DX-cluster node in `cluster_hosts` — the
/// SSB/phone aggregator. RBN endpoints are wired once via fixed latches; the human nodes use
/// a per-host latch ([`HUMAN_NODES_STARTED`]) so a node added in Settings connects on the next
/// save with no restart, and an RBN endpoint that sneaks into the list is skipped (no
/// double-connect). No-op per feed unless `mycall` is a [`is_real_call`]; the caller owns the
/// `cluster_enabled` gate. All push into the one shared `spots` buffer the need-matcher reads.
fn start_cluster_feeds(
    spots: &SharedSpots,
    cluster_hosts: &[String],
    mycall: &str,
    health: &SharedHealth,
) {
    start_cluster_feed(spots, RBN_CW_HOST, mycall, health, &RBN_CW_STARTED);
    start_cluster_feed(
        spots,
        RBN_DIGITAL_HOST,
        mycall,
        health,
        &RBN_DIGITAL_STARTED,
    );
    for host in cluster_hosts {
        let h = host.trim();
        if h.is_empty() || h.contains("reversebeacon.net") {
            continue; // blank, or an RBN endpoint already wired above
        }
        start_human_cluster_feed(spots, h, mycall, health);
    }
}

/// Spawn one RBN skimmer telnet feed (CW or digital). Once-latched via `started`; each parsed
/// spot stamps the aggregate `cluster_last` and the session toggles `cluster_connected`.
fn start_cluster_feed(
    spots: &SharedSpots,
    cluster_host: &str,
    mycall: &str,
    health: &SharedHealth,
    started: &std::sync::atomic::AtomicBool,
) {
    if !is_real_call(mycall) || started.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    conn_log(
        "RBN",
        "info",
        format!(
            "connecting to {} as {}",
            cluster_host,
            mycall.trim().to_uppercase()
        ),
    );
    let buf = spots.clone();
    let hp = health.clone();
    let hp_conn = health.clone();
    let host = cluster_host.to_string();
    let call = mycall.trim().to_string();
    std::thread::spawn(move || {
        tempo_net::cluster::run(
            &host,
            &call,
            |sp| {
                hp.cluster_last
                    .store(now_unix(), std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut b) = buf.lock() {
                    // Mark the skimmer origin so a leading "RTTY" comment token can
                    // UPGRADE the display label (is_rbn_rtty) — trusted only here, on
                    // the machine-generated RBN wire, never on the human-node path.
                    let mut s = sp.clone();
                    s.rbn = true;
                    b.push(s);
                }
            },
            &CLUSTER_STOP,
            &hp_conn.cluster_connected,
            &RBN_DEAD_OUTBOX, // RBN is receive-only — never post to a skimmer
        );
    });
}

/// Spawn ONE human DX-cluster node feed (an SSB/phone source). Per-host once-latch via
/// [`HUMAN_NODES_STARTED`] (so re-running the aggregator only spawns nodes not already up).
/// Each parsed spot stamps BOTH the aggregate `cluster_last` AND `phone_cluster_last`, and the
/// session toggles this node's own connected flag (registered in [`PHONE_NODE_CONNS`]) so the
/// phone-source pill reflects "any node up" — readable independently of the busy RBN feeds.
fn start_human_cluster_feed(spots: &SharedSpots, host: &str, mycall: &str, health: &SharedHealth) {
    if !is_real_call(mycall) {
        return;
    }
    {
        let mut started = match HUMAN_NODES_STARTED.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if started.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            return; // this node is already connected this session (case-insensitive)
        }
        started.push(host.to_string());
    }
    conn_log(
        "DX Cluster",
        "info",
        format!("connecting to {} as {}", host, mycall.trim().to_uppercase()),
    );
    let conn = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut v) = PHONE_NODE_CONNS.lock() {
        v.push(conn.clone());
    }
    let buf = spots.clone();
    let hp = health.clone();
    let host = host.to_string();
    let call = mycall.trim().to_string();
    std::thread::spawn(move || {
        tempo_net::cluster::run(
            &host,
            &call,
            |sp| {
                let ts = now_unix();
                hp.cluster_last
                    .store(ts, std::sync::atomic::Ordering::Relaxed);
                // Only a PHONE-classed spot proves SSB is actually reaching us — a human
                // node's feed is mostly CW (RBN relay + human CW), which would otherwise keep
                // the Phone pill green and hide a phone drought. Classify the same way the
                // need-matcher does.
                if matches!(
                    propagation::classify_spot_mode(sp.freq_mhz()),
                    propagation::ModeClass::Phone
                ) {
                    hp.phone_cluster_last
                        .store(ts, std::sync::atomic::Ordering::Relaxed);
                    hp.phone_spots_seen
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                if let Ok(mut b) = buf.lock() {
                    b.push(sp.clone());
                }
            },
            &CLUSTER_STOP,
            &conn,
            &CLUSTER_OUTBOX, // the post target — `post_spot` pushes DX lines here
        );
    });
}

/// Spawn the PSK Reporter MQTT firehose thread once per process (the [`PSKR_STARTED`]
/// latch makes repeat calls a no-op). No-op unless `mycall` is a [`is_real_call`].
/// Parsed live `PathSpot`s flow into `live_paths`, which `get_propagation` merges
/// into the nowcast; each parsed report stamps `health.pskr_last_event` (so a
/// connected-but-all-drops feed shows "waiting", not "live"). The CAS latch
/// guarantees no double-spawn across concurrent `set_settings` calls and startup.
fn start_pskr_feed(live_paths: &SharedLivePaths, mycall: &str, health: &SharedHealth) {
    if !is_real_call(mycall) || PSKR_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    conn_log(
        "PSKR MQTT",
        "info",
        format!("subscribing for {}", mycall.trim().to_uppercase()),
    );
    let buf = live_paths.clone();
    let hp = health.clone();
    let hp_conn = health.clone();
    let call = mycall.trim().to_string();
    std::thread::spawn(move || {
        let topics = propagation::pskr_mqtt_topics(&call);
        let topic_refs: Vec<&str> = topics.iter().map(|s| s.as_str()).collect();
        tempo_net::mqtt::subscribe(
            PSKR_MQTT_ADDR,
            &format!("nexus-{call}"),
            &topic_refs,
            |topic, payload| {
                if let Some(spot) = propagation::parse_pskr_mqtt_payload(topic, payload, now_unix())
                {
                    hp.pskr_last_event
                        .store(now_unix(), std::sync::atomic::Ordering::Relaxed);
                    // Rarity census: every heard TX grid is activity evidence.
                    if let Some(g) = spot.tx_grid.as_deref() {
                        if let Ok(mut c) = propagation::gridrarity::census().write() {
                            c.observe(g);
                        }
                    }
                    if let Ok(mut b) = buf.lock() {
                        b.push(spot);
                    }
                }
            },
            &PSKR_STOP,
            &hp_conn.pskr_connected,
        );
    });
}

/// Poll wspr.live every 5 min for who's hearing MYCALL's WSPR beacons — the
/// beacon-grade "getting out" evidence lane (bounded 1-hour query per the
/// wspr.live fair-use policy; data courtesy of wspr.live / wsprnet.org). Rows
/// at-or-before the newest already-pushed timestamp are skipped, so re-fetching
/// the sliding window never duplicates bus entries. Best-effort: a WSPR-less
/// station simply gets empty answers. Each call OWNS a new WSPR_GEN generation:
/// the previous poller (if any) exits at its next check, so a callsign change
/// tears the old-call query down instead of leaving it repolluting the drained
/// buffer (review catch).
fn start_wspr_feed(live_paths: &SharedLivePaths, mycall: &str) {
    let call_ok = is_real_call(mycall);
    let call = mycall.trim().to_uppercase();
    let my_gen = {
        let Ok(mut g) = WSPR_FEED.lock() else { return };
        if call_ok && g.1 == call {
            return; // already polling this exact call — idempotent
        }
        // Supersede whatever ran before (a rename kills the old-call poller
        // even when the new call is invalid — feeds stay down, honestly).
        g.0 += 1;
        g.1 = if call_ok { call.clone() } else { String::new() };
        if !call_ok {
            return;
        }
        g.0
    };
    let alive = move || WSPR_FEED.lock().map(|g| g.0 == my_gen).unwrap_or(false);
    let buf = live_paths.clone();
    std::thread::spawn(move || {
        let mut newest = 0i64;
        loop {
            if !alive() {
                return; // superseded (rename/restart) — die quietly
            }
            if let Ok(spots) = propagation::live::wspr::fetch_wspr(&call) {
                // Re-check AFTER the (up to 20 s) fetch so an in-flight answer
                // for the OLD call can never land after the rename drain.
                if !alive() {
                    return;
                }
                let fresh: Vec<_> = spots.into_iter().filter(|s| s.time > newest).collect();
                if let Some(m) = fresh.iter().map(|s| s.time).max() {
                    newest = m;
                }
                if !fresh.is_empty() {
                    if let Ok(mut b) = buf.lock() {
                        for sp in fresh {
                            b.push(sp);
                        }
                    }
                }
            }
            // 5-min cadence, sliced so a superseded poller exits within ~2 s.
            for _ in 0..150 {
                if !alive() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    });
}

/// Spawn the near-region opening MQTT thread once per process (Phase 2). No-op
/// unless `mycall` is real, `mygrid` resolves (nothing can be "near" otherwise),
/// and the operator hasn't opted out. Subscribes to the per-band VHF/10 m global
/// streams, then keeps only spots that are (a) NOT the operator's own paths (those
/// live in `live_paths`) and (b) within `REGION_RADIUS_KM` of the operator — so the
/// opening detector can flag "a band is open around you" while the buffers stay
/// disjoint and field-rig-bounded. Time-evicts on push so a wide opening can't
/// truncate the baseline.
fn start_pskr_region_feed(region_paths: &SharedRegionPaths, mycall: &str, mygrid: &str) {
    if !is_real_call(mycall)
        || propagation::geo::maidenhead_to_latlon(mygrid).is_none()
        || PSKR_REGION_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let buf = region_paths.0.clone();
    let call = mycall.trim().to_string();
    let grid = mygrid.trim().to_string();
    std::thread::spawn(move || {
        // VHF/10m: global per-band streams (self-throttling). HF F2: the
        // grid-targeted census — server-side filtered to the operator's region
        // so the broker never sends the global 20m firehose.
        let mut topics = propagation::pskr_region_topics();
        topics.extend(propagation::hf_region_topics(&grid, REGION_RADIUS_KM));
        let topic_refs: Vec<&str> = topics.iter().map(|s| s.as_str()).collect();
        let base_w = propagation::OpeningConfig::default().base_w;
        // No Now-Bar pill for the region feed — session state isn't surfaced.
        let connected = std::sync::atomic::AtomicBool::new(false);
        tempo_net::mqtt::subscribe(
            PSKR_MQTT_ADDR,
            &format!("nexus-rgn-{call}"),
            &topic_refs,
            |topic, payload| {
                let now = now_unix();
                let Some(spot) = propagation::parse_pskr_mqtt_payload(topic, payload, now) else {
                    return;
                };
                // Own-call paths belong to live_paths; keep only far↔far so the
                // two buffers are disjoint (no double-count in the anomaly rate).
                if spot.side(&call) != propagation::Side::Neither {
                    return;
                }
                // Near-region gate, failing CLOSED: at least one end within radius.
                let near = [spot.tx_grid.as_deref(), spot.rx_grid.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|g| {
                        propagation::geo::grid_distance_km(&grid, g)
                            .is_some_and(|d| d <= REGION_RADIUS_KM)
                    });
                if !near {
                    return;
                }
                if let Some(g) = spot.tx_grid.as_deref() {
                    if let Ok(mut c) = propagation::gridrarity::census().write() {
                        c.observe(g);
                    }
                }
                if let Ok(mut b) = buf.lock() {
                    b.push(spot);
                    b.trim_older_than(now - (base_w + 600));
                }
            },
            &PSKR_STOP,
            &connected,
        );
    });
}

/// Per-feed liveness for the Now-Bar connector pills.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedStatus {
    /// The feed's daemon thread is running. It is started once a real callsign
    /// (and, for the cluster, its toggle) is set, and then runs until app exit —
    /// so this can stay true after the cluster toggle is later turned off (the
    /// connection genuinely persists until restart). When false the UI hides the pill.
    enabled: bool,
    /// Seconds since the last parsed spot/report; `null` if none yet this session.
    last_event_secs: Option<i64>,
    /// "off" | "connecting" | "connected" | "live" | "idle" | "reconnecting"
    /// (only meaningful when `enabled`). "connected" = session up but no event parsed
    /// yet (normal on a quiet band — NOT broken); "connecting" = thread running, no
    /// session yet; "reconnecting" = had events, session currently down.
    state: String,
}

/// Liveness of the background live feeds.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedHealth {
    cluster: FeedStatus,
    pskr: FeedStatus,
    /// The HUMAN DX-cluster nodes as a group — the SSB/phone source, reported separately from
    /// the aggregate `cluster` pill (which the RBN CW/digital firehose keeps green on its own).
    /// `connected` = ANY node up; `enabled: false` when no human node is configured (RBN-only).
    phone_cluster: FeedStatus,
    /// Compact phone-source label: the host for one node, "host +N" for several, `None` for
    /// none (RBN-only operator).
    phone_cluster_host: Option<String>,
    /// Count of PHONE-classed spots received from human nodes this session — lets the Needed
    /// board show "N SSB spots", splitting "SSB isn't arriving" (0) from "arriving but not a
    /// need" (>0 with no phone rows).
    phone_spots_seen: i64,
}

fn feed_status(started: bool, connected: bool, last: i64, now: i64) -> FeedStatus {
    if !started {
        return FeedStatus {
            enabled: false,
            last_event_secs: None,
            state: "off".into(),
        };
    }
    if last == 0 {
        // No event parsed yet — the connected flag is what separates "healthy but
        // quiet" (normal: nobody has spotted the operator yet) from "can't reach the
        // server". Without it this read as a permanent, broken-looking "waiting".
        return FeedStatus {
            enabled: true,
            last_event_secs: None,
            state: if connected { "connected" } else { "connecting" }.into(),
        };
    }
    let age = (now - last).max(0);
    FeedStatus {
        enabled: true,
        last_event_secs: Some(age),
        state: if !connected {
            "reconnecting"
        } else if age <= FEED_FRESH_SECS {
            "live"
        } else {
            "idle"
        }
        .into(),
    }
}

/// Compact phone-source label for N connected human nodes: `None` for none, the bare host
/// for one, and "host +N" for several — enough for the Now-Bar pill / Needed-board line.
fn summarize_hosts(hosts: &[String]) -> Option<String> {
    match hosts.len() {
        0 => None,
        1 => Some(hosts[0].clone()),
        n => Some(format!("{} +{}", hosts[0], n - 1)),
    }
}

/// Liveness of the background live feeds, for the Now-Bar connector pills. A feed
/// is "live" if it parsed an event within FEED_FRESH_SECS, "waiting" if started
/// but silent so far, "idle" if it has gone quiet (a lull on a quiet band, or a
/// feed problem — the UI tooltip says so). Disabled feeds are hidden by the UI.
#[tauri::command]
fn get_feed_health(health: State<'_, SharedHealth>) -> FeedHealth {
    use std::sync::atomic::Ordering::Relaxed;
    let now = now_unix();
    // The human DX-cluster nodes (the SSB/phone aggregator): the spawned host list drives the
    // started flag + label, and "connected" = ANY node's session up.
    let human_hosts: Vec<String> = HUMAN_NODES_STARTED
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let phone_connected = PHONE_NODE_CONNS
        .lock()
        .map(|v| v.iter().any(|b| b.load(Relaxed)))
        .unwrap_or(false);
    FeedHealth {
        cluster: feed_status(
            // Any of the cluster sources spawned (RBN CW/digital firehoses + any human node).
            RBN_CW_STARTED.load(Relaxed)
                || RBN_DIGITAL_STARTED.load(Relaxed)
                || !human_hosts.is_empty(),
            health.cluster_connected.load(Relaxed),
            health.cluster_last.load(Relaxed),
            now,
        ),
        pskr: feed_status(
            PSKR_STARTED.load(Relaxed),
            health.pskr_connected.load(Relaxed),
            health.pskr_last_event.load(Relaxed),
            now,
        ),
        // The human nodes as a group — their own started/connected/last, so a down SSB
        // source is visible even while RBN keeps the aggregate `cluster` pill green.
        phone_cluster: feed_status(
            !human_hosts.is_empty(),
            phone_connected,
            health.phone_cluster_last.load(Relaxed),
            now,
        ),
        phone_cluster_host: summarize_hosts(&human_hosts),
        phone_spots_seen: health.phone_spots_seen.load(Relaxed),
    }
}

/// How long a live propagation nowcast is reused before refetching (seconds).
const PROP_TTL_SECS: u64 = 300;

/// When the last live-propagation refetch was ATTEMPTED (success or failure).
/// Rate-limits refetches to one per [`PROP_TTL_SECS`] independent of the UI poll
/// cadence, so a failing fetch on a cold cache can't storm PSK Reporter into 429s.
static PROP_FETCH_BACKOFF: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Last-good real-time solar wind (Bz/speed/…) — refreshed on each fresh SWPC-cadence poll
/// and reused on cache hits, so the leading-indicator insight survives between refetches.
/// `None` until the first successful fetch (SolarWind is Copy, so reads are cheap clones).
static LAST_SOLAR_WIND: Mutex<Option<propagation::SolarWind>> = Mutex::new(None);

/// Last-good 12-month smoothed sunspot number (R12) from SWPC's predicted
/// solar cycle — the p533 engine's proper solar input (daily SFI is the wrong
/// quantity for monthly-median CCIR maps; without this the engine falls back
/// to a Covington inversion of SFI). Refreshed on the space-wx fetch cadence.
static LAST_SSN: Mutex<Option<f32>> = Mutex::new(None);

/// The X-ray "fast lane" cache: (fetched-at, flux W/m², reading unix time). A
/// flare's rise takes minutes, so `get_xray_now` refetches GOES every 60 s —
/// much fresher than the 5-min snapshot TTL — and serves last-good on failure.
static LAST_XRAY: Mutex<Option<(std::time::Instant, f32, i64)>> = Mutex::new(None);

/// The base config root: `%APPDATA%` on Windows, `$XDG_CONFIG_HOME`/`~/.config` on Unix,
/// else the CWD. The `tempo[-<profile>]` app directory hangs off this.
fn config_base() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
}

/// Accept only a filesystem-safe profile name — `[A-Za-z0-9_-]`, 1..=32 chars. Anything
/// else (empty, path separators, junk) → `None` = the default profile, so a bad value can
/// never escape the config root or silently split the config into a garbage directory.
fn sanitize_profile(s: &str) -> Option<String> {
    let t = s.trim();
    let ok = !t.is_empty()
        && t.len() <= 32
        && t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| t.to_string())
}

/// The active instance profile, resolved once: `--profile <name>` (or `--profile=<name>`) on
/// the command line wins, else the `NEXUS_PROFILE` env var. `None` = the DEFAULT profile,
/// whose config dir is plain `tempo` — byte-identical to the pre-profile layout, so a single
/// instance is unaffected. Two instances get separate config by launching with distinct
/// profiles; that is what keeps them from clobbering each other's settings/journals.
fn active_profile() -> Option<&'static str> {
    static PROFILE: OnceLock<Option<String>> = OnceLock::new();
    PROFILE
        .get_or_init(|| {
            let from_arg = {
                let mut found = None;
                let mut args = std::env::args().skip(1);
                while let Some(a) = args.next() {
                    if a == "--profile" {
                        found = args.next();
                        break;
                    } else if let Some(v) = a.strip_prefix("--profile=") {
                        found = Some(v.to_string());
                        break;
                    }
                }
                found
            };
            from_arg
                .or_else(|| std::env::var("NEXUS_PROFILE").ok())
                .and_then(|s| sanitize_profile(&s))
        })
        .as_deref()
}

/// The app-directory name for a profile: `tempo` (default) or `tempo-<profile>`. Pure, so the
/// naming is testable without touching env/args.
fn profile_dir_name(profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("tempo-{p}"),
        None => "tempo".to_string(),
    }
}

/// The per-instance config directory: `<base>/tempo` for the default profile, or
/// `<base>/tempo-<profile>` for a named one. Holds settings.json + the journals beside it
/// (window geometry, Field Day, pending-QSO/msgs queues) — all correctly per-instance, since
/// each of those derives from [`settings_path`]'s parent.
fn config_dir() -> PathBuf {
    config_base().join(profile_dir_name(active_profile()))
}

/// The SHARED data directory that holds the ONE unified logbook across all instances.
/// Default = `<base>/tempo` (the default profile's dir — i.e. the pre-profile location, so an
/// existing `log.adi` stays put and simply becomes the shared one, no migration). Override
/// with `NEXUS_DATA_DIR` to place the shared log on a NAS/Drive-synced folder (a multi-PC shack).
fn shared_data_dir() -> PathBuf {
    match std::env::var_os("NEXUS_DATA_DIR").filter(|s| !s.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => config_base().join("tempo"),
    }
}

/// Where settings are persisted: `<config_dir>/settings.json` — per-instance (profile-suffixed).
fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// Persisted geometry (logical px) + side-dock of the torn-off band-map window, so the
/// vertical band map reopens where the operator left it instead of re-arranging it every
/// launch. A tiny sibling of settings.json (not part of the big Settings blob — it's pure
/// window state, written from the window-event handler and read at window-open time).
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct BandmapWindow {
    w: f64,
    h: f64,
    x: f64,
    y: f64,
    /// "left" | "right" | "" (free-floating). When set, the window re-snaps to that edge on open.
    #[serde(default)]
    dock: String,
}

/// Per-window geometry file — keyed by `(slug, instance)` so the two band maps DON'T clobber
/// each other's saved size/position/dock when both are torn off, and so a band map bound to a
/// second radio gets its own rect rather than fighting the first one's.
///
/// `main` keeps the historic `bandmap-window-<slug>.json` name byte-for-byte (zero migration:
/// every rect an operator has already saved keeps loading). `None` means this surface's
/// geometry must NOT be persisted at all — see [`Instance::persists_geometry`].
fn bandmap_window_path(slug: &str, inst: Instance) -> Option<PathBuf> {
    if !inst.persists_geometry() {
        return None;
    }
    let name = match inst {
        Instance::Main => format!("bandmap-window-{slug}.json"),
        other => format!("bandmap-window-{slug}-{other}.json"),
    };
    Some(settings_path().with_file_name(name))
}

/// The band-map surface ("bandmapCw"/"bandmapPhone" + its instance) for a window, or None for
/// any other window.
fn bandmap_key(window: &tauri::WebviewWindow) -> Option<(&str, Instance)> {
    let (slug, inst) = panel_key(window.label())?;
    slug.starts_with("bandmap").then_some((slug, inst))
}

fn load_bandmap_window(slug: &str, inst: Instance) -> Option<BandmapWindow> {
    let g: BandmapWindow =
        serde_json::from_str(&std::fs::read_to_string(bandmap_window_path(slug, inst)?).ok()?)
            .ok()?;
    // Reject a degenerate/blank record so a corrupt file falls back to the default size.
    (g.w >= 200.0 && g.h >= 200.0).then_some(g)
}

fn save_bandmap_window(slug: &str, inst: Instance, g: &BandmapWindow) {
    let Some(p) = bandmap_window_path(slug, inst) else {
        return;
    };
    if let Ok(txt) = serde_json::to_string(g) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, txt);
    }
}

/// Snapshot a band-map window's current size+position to its own per-surface file (logical px),
/// preserving its dock choice. Called on close so the next open restores it. No-op otherwise.
fn capture_bandmap_window(window: &tauri::WebviewWindow) {
    let Some((slug, inst)) = bandmap_key(window) else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let (Ok(size), Ok(pos)) = (window.inner_size(), window.outer_position()) else {
        return;
    };
    let dock = load_bandmap_window(slug, inst)
        .map(|g| g.dock)
        .unwrap_or_default();
    save_bandmap_window(
        slug,
        inst,
        &BandmapWindow {
            w: size.width as f64 / scale,
            h: size.height as f64 / scale,
            x: pos.x as f64 / scale,
            y: pos.y as f64 / scale,
            dock,
        },
    );
}

/// Snap a band-map window to the left/right edge of its monitor's WORK AREA (excludes the
/// taskbar — full monitor size overlapped it) as a full-height strip, keeping its width.
/// Applies the move AND persists the resulting geometry + dock. Best-effort: a window with no
/// resolvable monitor (e.g. not yet mapped) is left as-is.
fn snap_bandmap_to_edge(
    window: &tauri::WebviewWindow,
    side: &str,
) -> Result<BandmapWindow, String> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let cur_w = window
        .inner_size()
        .map(|s| (s.width as f64 / scale).clamp(300.0, 640.0))
        .unwrap_or(420.0);
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no monitor for this window".to_string())?;
    // work_area() is the usable region (taskbar/dock excluded) — unlike size()/position().
    let area = monitor.work_area();
    let mw = area.size.width as f64 / scale;
    let mh = area.size.height as f64 / scale;
    let mx = area.position.x as f64 / scale;
    let my = area.position.y as f64 / scale;
    let w = cur_w;
    let h = mh;
    let x = if side == "left" { mx } else { mx + mw - w };
    let g = BandmapWindow {
        w,
        h,
        x,
        y: my,
        dock: side.to_string(),
    };
    window
        .set_size(tauri::LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(x, my))
        .map_err(|e| e.to_string())?;
    if let Some((slug, inst)) = bandmap_key(window) {
        save_bandmap_window(slug, inst, &g);
    }
    Ok(g)
}

/// Where the ADIF logbook is persisted: the SHARED data dir (NOT the per-profile config
/// dir), so every instance reads and writes the ONE unified log. Default
/// `%APPDATA%\tempo\log.adi` / `~/.config/tempo/log.adi` (unchanged for a single instance);
/// `NEXUS_DATA_DIR` relocates it (multi-PC shack). See [`shared_data_dir`].
fn logbook_path() -> PathBuf {
    shared_data_dir().join("log.adi")
}

/// Where the Field Day contest log's durable ADIF journal lives (beside
/// settings.json). The engine rewrites it on every FD contact and restores it
/// when Field Day mode starts; the exit flush writes the SAME file.
fn fd_backup_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fieldday_backup.adi")
}

/// Durable journal for the ONE QSO held by the prompt-to-log popup (beside settings.json).
/// The engine rewrites it the moment the hold changes and deletes it once the operator
/// confirms or discards, so a crash or power cut while the popup waits can't destroy a real
/// contact the other station has already logged.
fn pending_qso_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pending_qso.json")
}

/// `<config dir>/pending_msgs.json` — the store-and-forward outbound-queue journal,
/// beside the pending-QSO journal. Held Tempo messages survive a restart.
fn pending_msgs_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pending_msgs.json")
}

/// Where the grid-activity census is persisted (beside settings.json): a small
/// bounded JSON of decayed per-grid heard counts — the demote-only refinement
/// evidence for the rarity gems. Losing it is harmless (it re-accumulates).
fn census_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("grid_census.json")
}

/// Persistent openings log — completed opening episodes (6m/2m tropo/Es/aurora …),
/// oldest first on disk, capped so it can't grow unbounded. Sibling of settings.json.
const OPENINGS_LOG_CAP: usize = 500;
static OPENINGS_LOG: Mutex<Vec<propagation::OpeningEpisode>> = Mutex::new(Vec::new());

fn openings_log_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("openings_log.json")
}

/// Append newly-closed episodes to the openings log and persist it (atomic
/// tmp+rename, best-effort — a failed write never disturbs the running app).
fn record_opening_episodes(mut eps: Vec<propagation::OpeningEpisode>) {
    if eps.is_empty() {
        return;
    }
    let mut log = OPENINGS_LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.append(&mut eps);
    if log.len() > OPENINGS_LOG_CAP {
        let excess = log.len() - OPENINGS_LOG_CAP;
        log.drain(..excess);
    }
    if let Ok(text) = serde_json::to_string(&*log) {
        let path = openings_log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Historical opening episodes for the Connect openings-log pane, newest first.
#[tauri::command]
fn get_openings_log() -> Vec<propagation::OpeningEpisode> {
    let log = OPENINGS_LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.iter().rev().cloned().collect()
}

/// Where the downloaded/imported POTA park directory CSV is cached (beside settings.json), so the
/// list survives restarts and is searched offline. Losing it is harmless (re-download / re-import).
fn parks_cache_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("parks.csv")
}

/// The WSJT-X-format decode log (`ALL.TXT`), in the app's LOCAL data dir
/// (`%LOCALAPPDATA%\Nexus` on Windows — the same class of location WSJT-X uses for
/// its own ALL.TXT; `$XDG_DATA_HOME`/`~/.local/share/Nexus` on Unix). Deliberately a
/// findable, app-named folder rather than the Roaming `tempo` config dir, and NOT the
/// install dir (Program Files isn't writable without elevation — a write there would
/// fail silently). Written only when `settings.write_all_txt` is on; the Settings
/// panel surfaces this path + a "Reveal" button (`all_txt_location`/`reveal_all_txt`).
fn all_txt_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("Nexus")
        .join("ALL.TXT")
}

/// Append the engine's buffered WSJT-X-format decode lines to `ALL.TXT` (best-effort:
/// a write hiccup must never disturb the snapshot the UI is waiting on). Creates the
/// parent dir first — `OpenOptions::create` makes the file, not the directory.
fn flush_all_txt(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    use std::io::Write;
    let path = all_txt_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{}", lines.join("\n"));
    }
}

/// The resolved absolute `ALL.TXT` path, for the Settings panel to show the operator
/// exactly where the decode log lives.
#[tauri::command]
fn all_txt_location() -> String {
    all_txt_path().to_string_lossy().into_owned()
}

/// Where received SSTV images are saved — the same operator-findable local data
/// dir as ALL.TXT (`%LOCALAPPDATA%\Nexus\sstv-gallery` on Windows,
/// `~/.local/share/Nexus/sstv-gallery` on Unix). Each finished image lands here
/// as `<UTC stamp>_<mode>.bmp` with a `gallery.json` metadata sidecar beside
/// the images (written by the `sstvrx` decode thread).
fn sstv_gallery_dir() -> PathBuf {
    all_txt_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sstv-gallery")
}

/// Open the ALL.TXT location so the operator can find the decode log: reveal the file
/// when it exists, else open the (freshly-created) folder. Called from Rust via the
/// opener plugin, so no JS package or ACL capability entry is required.
#[tauri::command]
fn reveal_all_txt(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = all_txt_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if path.exists() {
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| e.to_string())
    } else if let Some(dir) = path.parent() {
        app.opener()
            .open_path(dir.to_string_lossy().to_string(), None::<&str>)
            .map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

/// Where Tempo conversation threads persist (so chat history survives a restart):
/// `<config dir>/conversations.json`, beside settings.json + the logbook.
fn conversations_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("conversations.json")
}

/// Atomically write the conversation JSON: create the dir if missing (fresh
/// profile), write a temp file, then rename — so a crash mid-write can't truncate
/// the history (mirrors `Logbook::save`). Returns whether it succeeded.
fn write_conversations_atomic(text: &str) -> bool {
    let path = conversations_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_ok()
}

/// Export + atomically persist the engine's conversation threads. Used for the
/// flush-on-exit (so quitting within the periodic-save window doesn't lose recent
/// chat or resurrect an archived thread). Recovers a poisoned lock.
fn persist_conversations(engine: &SharedEngine) {
    let convs = engine
        .lock()
        .map(|e| e.export_conversations())
        .unwrap_or_else(|e| e.into_inner().export_conversations());
    if let Ok(text) = serde_json::to_string(&convs) {
        write_conversations_atomic(&text);
    }
}

/// Flush the in-memory Field Day contest log to its ADIF journal
/// ([`fd_backup_path`]) via the engine's own per-contact write path. Kept as an
/// exit-time backstop; no-op when not in Field Day or the log is empty.
/// Recovers a poisoned lock (mirrors `persist_conversations`).
fn persist_field_day_log(engine: &SharedEngine) {
    engine
        .lock()
        .map(|e| e.persist_fd_log())
        .unwrap_or_else(|e| e.into_inner().persist_fd_log());
}

/// Directory for phone voice-keyer recordings: `<settings dir>/voice` (12 kHz mono WAVs).
fn voice_dir() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voice")
}

/// Directory for QSO recordings (audio bridge): `<settings dir>/recordings` (12 kHz mono WAVs).
fn recordings_dir() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("recordings")
}

/// Full UI snapshot (`AppSnapshot`) — the UI renders all three zones from this.
#[tauri::command]
async fn get_snapshot(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    // Drain any buffered ALL.TXT decode lines (the engine is I/O-free) and snapshot,
    // then release the lock before the file append so the UI poll never waits on disk.
    let all_txt = eng.take_all_txt_pending();
    let snap = eng.snapshot();
    drop(eng);
    flush_all_txt(&all_txt);
    Ok(snap)
}

/// Queue an outbound free-text message to `peer` (auto-chunked + presence-gated
/// store-and-forward by the engine). Returns the refreshed snapshot.
#[tauri::command]
fn send_message(
    state: State<'_, SharedEngine>,
    peer: String,
    text: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.send_message(&peer, &text);
    Ok(eng.snapshot())
}

/// Select `peer` as the active conversation. Returns the refreshed snapshot.
#[tauri::command]
fn select_peer(
    state: State<'_, SharedEngine>,
    peer: Option<String>,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    match peer.as_deref() {
        Some(p) => eng.select_peer(p),
        // Deselect must reach the engine too — a lingering active peer kept stale
        // roster/QSY context alive backend-side.
        None => eng.clear_peer(),
    }
    Ok(eng.snapshot())
}

/// Delete a conversation thread (the recents-list ✕). Also cancels any still-queued
/// outbound messages for that peer, so nothing further for it goes on the air.
/// Returns the refreshed snapshot.
///
/// Persists IMMEDIATELY rather than waiting for the 15 s periodic save: without this, a
/// crash or power loss inside that window resurrects the deleted thread from
/// conversations.json on next launch. A failed write is logged, not returned — the
/// in-memory delete already succeeded and the periodic save will retry.
#[tauri::command]
fn archive_conversation(
    state: State<'_, SharedEngine>,
    peer: String,
) -> Result<AppSnapshot, String> {
    // Scope the lock: `persist_conversations` locks internally, and `SharedEngine` is a
    // non-reentrant std Mutex — calling it inside this scope would self-deadlock.
    let (snap, text) = {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.archive_conversation(&peer);
        (
            eng.snapshot(),
            serde_json::to_string(&eng.export_conversations()).ok(),
        )
    };
    match text {
        Some(t) if write_conversations_atomic(&t) => {}
        _ => eprintln!("archive_conversation: failed to persist conversations for {peer}"),
    }
    Ok(snap)
}

/// Switch the waveform mode/tier ("TempoFast" | "FT8" | "FT4" | "TempoDeep"). Operator-
/// visible via `LinkState.tier`. FT1 = fast 4 s coherent; FT8 = 15 s; FT4 =
/// 7.5 s; DX1 = robust non-coherent 15 s. All decode/encode natively through the
/// engine's signal source.
#[tauri::command]
fn set_tier(state: State<'_, SharedEngine>, tier: String) -> Result<AppSnapshot, String> {
    let tier: Tier =
        serde_json::from_value(serde_json::Value::String(tier.clone())).map_err(|_| {
            format!("invalid tier {tier:?}: expected \"FT1\", \"FT8\", \"FT4\", or \"DX1\"")
        })?;
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tier(tier);
    Ok(eng.snapshot())
}

/// Switch the RX signal source: `"native"` (decode local audio) or `"companion"`
/// (ride an upstream WSJT-X/JTDX/MSHV decode stream over UDP). Companion binds the
/// configured listen address; returns an error (source unchanged) if it can't.
#[tauri::command]
fn set_source(state: State<'_, SharedEngine>, kind: String) -> Result<AppSnapshot, String> {
    let kind: SourceKind = serde_json::from_value(serde_json::Value::String(kind.clone()))
        .map_err(|_| format!("invalid source {kind:?}: expected \"native\" or \"companion\""))?;
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_source(kind)?;
    // Persist the choice so it survives restart (set_source recorded it in settings).
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist signal source: {e}");
    }
    Ok(eng.snapshot())
}

/// The propagation & opening-intelligence nowcast, built from LIVE NOAA SWPC +
/// PSK Reporter data for the operator's call/grid. Cached for [`PROP_TTL_SECS`]
/// to respect PSK Reporter's query cadence; falls back to the last good snapshot
/// (then an honest, empty `offline` snapshot — never fabricated data) if a fetch
/// fails or the operator hasn't set a real callsign.
#[tauri::command]
async fn get_propagation(
    state: State<'_, SharedEngine>,
    cache: State<'_, PropCache>,
    live_paths: State<'_, SharedLivePaths>,
    region_paths: State<'_, SharedRegionPaths>,
    opening_tracker: State<'_, SharedOpeningTracker>,
    spots: State<'_, SharedSpots>,
    wx_history: State<'_, SharedWxHistory>,
) -> Result<propagation::PropagationSnapshot, String> {
    let (mycall, mygrid, needs, local_spots) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        let (mycall, mygrid) = (s.mycall.clone(), s.mygrid.clone());
        // Derive the operator's needs from the ADIF logbook (cty.dat-resolved).
        // Empty log → every active DXpedition shows as an ATNO candidate.
        let mut needs = propagation::LogNeeds::new();
        for q in eng.get_log() {
            // A "needs confirmation" must be award-grade (LoTW/paper), not eQSL.
            needs.add(
                &q.call,
                &q.band,
                &q.mode,
                q.grid.as_deref(),
                q.state.as_deref(),
                q.award_confirmed,
            );
        }
        // The operator's OWN decoded roster on the current band → "I heard X"
        // PathSpots. This feeds the opening detector + advisor from MONITORING
        // alone — a band the operator can SEE open in their decode window now lights
        // up even with zero PSKR/cluster coverage (the highest-leverage fix for
        // "I can see 6m is open but get no alert").
        let mut local_spots: Vec<propagation::PathSpot> = Vec::new();
        if !mycall.trim().is_empty() {
            let snap = eng.snapshot();
            if let Some(band) = propagation::model::Band::from_label(&snap.radio.band) {
                let t = now_unix();
                let me_grid = (!mygrid.trim().is_empty()).then(|| mygrid.clone());
                for st in &snap.stations {
                    local_spots.push(propagation::PathSpot {
                        time: t,
                        tx_call: st.call.to_uppercase(),
                        tx_grid: st.grid.clone(),
                        rx_call: mycall.to_uppercase(),
                        rx_grid: me_grid.clone(),
                        band,
                        mode: None,
                        snr: Some(st.snr as f32),
                        freq_mhz: None, // own decodes are band-level here
                    });
                }
            }
        }
        (mycall, mygrid, needs, local_spots)
    };

    let now = now_unix();

    // No real callsign → can't query "who hears me"; return an honest, EMPTY offline
    // snapshot (never fabricated data). Gated on `is_real_call` (not just non-empty)
    // so a garbage call also yields offline instead of a guaranteed-fail live query.
    // Checked BEFORE the cache so a cleared/changed callsign can't keep serving the
    // previous identity's live-labeled openings from a warm cache.
    if !is_real_call(&mycall) {
        return Ok(propagation::offline(now, &mycall, &mygrid));
    }

    // --- base snapshot: fresh cache, else a live refetch, else last-good/offline ---
    let cached = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .filter(|(when, _)| when.elapsed().as_secs() < PROP_TTL_SECS)
            .map(|(_, snap)| snap.clone())
    };

    // Track whether THIS poll fetched fresh space weather — only then do we push a
    // trend sample (the SWPC values change at most every PROP_TTL_SECS, and pushing on
    // every 30 s UI poll would just dedup-collapse and never accumulate a real trend).
    let mut fetched_fresh = false;
    let mut snap = if let Some(s) = cached {
        s
    } else {
        // Rate-limit live refetches to PROP_TTL_SECS even on FAILURE. The snapshot
        // cache is written only on success, so without this a cold cache + a failing
        // fetch would let the 30 s UI poll hammer PSK Reporter (1 query / 5 min) into
        // a perpetual 429 storm — the root cause of "propagation stays stuck". The
        // live MQTT firehose still feeds spots between XML refreshes, so backing off
        // the XML query loses nothing.
        let backoff_active = PROP_FETCH_BACKOFF
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|t| t.elapsed().as_secs() < PROP_TTL_SECS);

        let live = if backoff_active {
            Err("propagation refetch backing off".to_string())
        } else {
            // Stamp the attempt BEFORE fetching so a failure still arms the back-off.
            if let Ok(mut g) = PROP_FETCH_BACKOFF.lock() {
                *g = Some(std::time::Instant::now());
            }
            // Live PSK Reporter MQTT spots since the last rebuild, merged with the
            // rate-limited XML query. Refetch (blocking HTTP) — run on the blocking
            // pool so its stacked seconds-long reqwest timeouts never tie up a tokio
            // runtime worker (get_snapshot, the decode/waterfall heartbeat, shares this
            // multi-threaded runtime). Same pattern as get_path_outlook/get_band_outlook.
            let extra = live_paths
                .lock()
                .map(|b| b.recent(now, 1800))
                .unwrap_or_default();
            let (mc, mg) = (mycall.clone(), mygrid.clone());
            tauri::async_runtime::spawn_blocking(move || {
                propagation::live::snapshot_with_spots(&mc, &mg, 1800, &needs, &extra)
            })
            .await
            .map_err(|e| e.to_string())?
        };

        match live {
            Ok(snap) => {
                if let Ok(mut guard) = cache.lock() {
                    *guard = Some((std::time::Instant::now(), snap.clone()));
                }
                fetched_fresh = true;
                snap
            }
            // Fetch failed (or backing off): serve the last good snapshot marked
            // stale, else an honest empty offline snapshot — NEVER fabricated data.
            Err(_) => {
                let guard = cache.lock().map_err(|e| e.to_string())?;
                guard
                    .as_ref()
                    .map(|(_, s)| {
                        let mut s = s.clone();
                        s.source = "cached".to_string();
                        s
                    })
                    .unwrap_or_else(|| propagation::offline(now, &mycall, &mygrid))
            }
        }
    };

    // --- opening detection + tracker: run on EVERY poll (incl. cache hits) ---
    // Decoupled from the snapshot's 300 s HTTP cache so onset/`is_new` advance at
    // the UI poll cadence ("alert the moment a band comes alive"). Reads a WIDE
    // live-spot window (the detector's full baseline) and replaces the snapshot's
    // openings with the tracker-stamped set (hysteresis-entered, onset-timed).
    let cfg = propagation::OpeningConfig::default();
    let wx = propagation::SpaceWx {
        sfi: snap.space_wx.sfi,
        ssn: None, // opening detector/heuristic don't read R12
        kp: snap.space_wx.kp,
        a_index: snap.space_wx.a_index,
        // Use the lossless flux (not the flare bool), so the flare insight reports the
        // true R-scale — an X-class blackout no longer collapses to "R1 / M-class".
        xray_long: snap.space_wx.xray_long,
    };
    // ANCHORED window = operator-REACHABLE evidence only: own-call PSK Reporter MQTT
    // + the near-region (≤REGION_RADIUS_KM) census (disjoint — the region feed drops
    // own-call spots) + the operator's own decodes. This is what drives the BAND
    // ADVISOR ("best band FOR YOU"). The continent-wide DX-cluster / RBN firehose is
    // deliberately EXCLUDED here: 10 m is the busiest band worldwide at high solar,
    // and its raw global spot volume would otherwise always win the "best band"
    // headline regardless of what the operator can actually work.
    let mut anchored = live_paths
        .lock()
        .map(|b| b.recent(now, cfg.base_w))
        .unwrap_or_default();
    let mut regional_scope = false;
    if let Ok(r) = region_paths.0.lock() {
        let regional = r.recent(now, cfg.base_w);
        if !regional.is_empty() {
            regional_scope = true;
            anchored.extend(regional);
        }
    }
    // The operator's own decodes (IHeard spots) are operator-anchored, so they join
    // the anchored window (driving the advisor + the operator-anchored opening gate).
    anchored.extend(local_spots);

    // WIDE window = anchored + the continent-wide DX-cluster / RBN firehose. This
    // drives the MAP (which SHOULD show worldwide activity) and the opening detector
    // (whose operator/regional gates already stop a one-directional skimmer from
    // opening a band on its own — cluster only adds legitimate HF liveness/anomaly
    // signal). CW/RTTY (RBN) skimmers are geolocated via the real skimmer→grid table
    // so reception carries true near/far geometry; skimmers not in the table still
    // light band LIVENESS (no grid → activity census only).
    let mut wide = anchored.clone();
    let me_ll_for_gate = propagation::geo::maidenhead_to_latlon(mygrid.trim());
    if let Ok(buf) = spots.lock() {
        let cluster = buf.recent_within(
            std::time::Instant::now(),
            std::time::Duration::from_secs(cfg.base_w as u64),
        );
        for cs in cluster {
            if let Some(band) = propagation::model::Band::from_mhz(cs.freq_mhz()) {
                let rx_grid = propagation::skimmer_grid(&cs.spotter).map(str::to_string);
                // VHF locality gate (weak-signal-sleuth principle): on 6m/4m/2m a
                // continent-wide RBN spot says NOTHING about the operator's band —
                // Es is patchy; a Florida skimmer hearing 6 m must not light the
                // band ladder / opening detector for Wisconsin. Admit a VHF cluster
                // spot only when the SKIMMER itself is within the region radius of
                // the operator (skimmers without a known grid can't prove locality
                // → dropped on VHF). HF keeps the continent-wide census: F2
                // footprints genuinely span it.
                if band.is_vhf() {
                    let near = match (&rx_grid, me_ll_for_gate) {
                        (Some(g), Some(me)) => propagation::geo::maidenhead_to_latlon(g)
                            .is_some_and(|rx| {
                                propagation::geo::haversine_km(me, rx) <= REGION_RADIUS_KM
                            }),
                        _ => false,
                    };
                    if !near {
                        continue;
                    }
                }
                wide.push(propagation::PathSpot {
                    time: now,
                    tx_call: cs.dx_call.to_uppercase(),
                    tx_grid: None,
                    rx_call: cs.spotter.to_uppercase(),
                    rx_grid,
                    band,
                    // Cluster/RBN carry the goods the band-level feeds lack: the EXACT
                    // spot frequency. The mode is the band plan's call, NEVER the free-text
                    // comment (which holds QSY requests / chit-chat / times).
                    mode: Some(
                        propagation::classify_spot_mode(cs.freq_mhz())
                            .label()
                            .to_string(),
                    ),
                    snr: None,
                    freq_mhz: Some(cs.freq_mhz()),
                });
                // NOTE: cluster data deliberately does NOT set `regional_scope`.
                // RBN is a one-directional skimmer network — it can't satisfy the
                // Phase-2 regional gate's reciprocity premise; only the PSK Reporter
                // near-region feed (a true two-way census near the operator) does.
            }
        }
    }
    if let Ok(mut tr) = opening_tracker.lock() {
        snap.openings = propagation::detect_openings_tracked(
            &mycall,
            &mygrid,
            now,
            &wide,
            &wx,
            &mut tr,
            regional_scope,
        );
        // Journal any episodes the tracker just closed into the persistent
        // openings log (the Connect historical-review pane reads it).
        record_opening_episodes(tr.drain_closed());
    }

    // Rebuild the band advisor on the ANCHORED (operator-reachable) window — "best
    // band FOR YOU", reflecting activity AROUND the operator, NOT the busiest band
    // worldwide. Rebuild whenever we have any anchored evidence (own-call MQTT,
    // near-region, or own decodes); when empty we keep the cached snapshot's
    // own-call advisory (never the global firehose, which `anchored` excludes).
    if !anchored.is_empty() {
        let advisor = propagation::PropAdvisor::new(&mycall, &mygrid);
        snap.advisory = advisor.advise(now, &anchored, &wx);
        // Best-band-per-region recommender + the (region, band) activity matrix, from the
        // SAME anchored window (operator-reachable only — never the firehose). One pass.
        let rb = advisor.region_band(now, &anchored, &snap.advisory.bands);
        snap.best_to_region = rb.best_to_region;
        snap.region_band = rb.cells;
    }
    // "Worldwide activity" = the SAME advisor over the GLOBAL firehose window, so the
    // UI can show busy-worldwide beside best-FOR-YOU (workable vs merely-loud). Only
    // when the firehose actually adds something beyond the anchored set.
    if wide.len() > anchored.len() {
        snap.worldwide =
            Some(propagation::PropAdvisor::new(&mycall, &mygrid).advise(now, &wide, &wx));
    }

    // Locate the merged window for the map (grid or DXCC centroid), so the map
    // fills with the cluster/RBN/PSKR firehose + own decodes, not just the native
    // roster. Capped so a busy RBN window can't flood the canvas.
    snap.spots = propagation::build_map_spots(now, &mycall, &wide, 400);

    // --- space-weather trend + trend-aware predictive insights (A2 + A3) ---
    // Push ONE sample per fresh SWPC fetch (rate-limited), then compute the rolling
    // trend and regenerate the insight feed with it (overwriting the engine layer's
    // threshold-only insights). me_ll drives the representative MUF + greyline test.
    let me_ll = propagation::geo::maidenhead_to_latlon(&mygrid);
    if fetched_fresh {
        if let Ok(mut h) = wx_history.lock() {
            let muf = me_ll
                .map(|me| propagation::representative_muf(me, now, &wx))
                .unwrap_or(0.0);
            h.push(propagation::SpaceWxSample {
                t: now,
                sfi: wx.sfi,
                kp: wx.kp,
                xray_long: wx.xray_long,
                muf,
            });
        }
        // Refresh the real-time solar wind on the same (rate-limited) cadence — the LEADING
        // geomagnetic indicator (Bz/speed lead Kp by hours). Best-effort: a failed fetch
        // keeps the last-good value for the cache-hit polls in between.
        // Blocking HTTP → blocking pool (never a runtime worker), same as above.
        if let Ok(Ok(sw)) =
            tauri::async_runtime::spawn_blocking(propagation::live::solar_wind::fetch_solar_wind)
                .await
        {
            if let Ok(mut g) = LAST_SOLAR_WIND.lock() {
                *g = Some(sw);
            }
        }
        // The month's predicted smoothed SSN (R12) for the p533 engine — same
        // best-effort contract: keep the last-good value between successes.
        let (yy, mm) = propagation::solar_cycle::year_month(now);
        if let Ok(Ok(ssn)) = tauri::async_runtime::spawn_blocking(move || {
            propagation::live::solar_cycle::fetch_predicted_ssn(yy, mm)
        })
        .await
        {
            if let Ok(mut g) = LAST_SSN.lock() {
                *g = Some(ssn);
            }
        }
    }
    // Attach the last-good solar wind so the UI space-wx pane + the leading-indicator
    // insight can read it (reused between fresh fetches; None until the first success).
    snap.space_wx.solar_wind = LAST_SOLAR_WIND.lock().ok().and_then(|g| *g);
    snap.wx_trend = wx_history
        .lock()
        .map(|h| h.trend(now, 3 * 3600))
        .unwrap_or_default();
    snap.insights = propagation::generate_insights(
        now,
        &wx,
        Some(&snap.wx_trend),
        &snap.advisory.bands,
        &snap.openings,
        me_ll,
        snap.space_wx.solar_wind.as_ref(),
    );

    Ok(snap)
}

/// Per-path HF outlook to a selected station's `grid` — the heuristic
/// PathPredictor (the VOACAP-ready seam) over the operator↔DX great circle, under
/// the current space weather. Answers "is THIS path workable, which band, when"
/// for a station you may have no live spots on. Empty bands if either grid is
/// unknown (operator hasn't set a grid, or the station has none).
#[tauri::command]
async fn get_path_outlook(
    grid: String,
    state: State<'_, SharedEngine>,
    cache: State<'_, PropCache>,
) -> Result<propagation::PathPrediction, String> {
    let (mygrid, prop_engine, station_power_w, ant_gain_dbi) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (
            st.mygrid.clone(),
            st.prop_engine.clone(),
            st.station_power_w,
            st.ant_tx_gain_dbi + st.ant_rx_gain_dbi,
        )
    };
    let me = propagation::geo::maidenhead_to_latlon(&mygrid);
    let Some(dx) = propagation::geo::maidenhead_to_latlon(grid.trim()) else {
        return Ok(propagation::PathPrediction {
            engine: "heuristic".to_string(),
            bands: Vec::new(),
            muf_now: 0.0,
            muf_hourly: Vec::new(),
        });
    };
    // Current space weather from the propagation cache's last snapshot (the same
    // SWPC-fed values get_propagation uses); benign defaults if the cache is cold.
    let wx = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .map(|(_, s)| propagation::SpaceWx {
                sfi: s.space_wx.sfi,
                ssn: LAST_SSN.lock().ok().and_then(|g| *g),
                kp: s.space_wx.kp,
                a_index: s.space_wx.a_index,
                xray_long: if s.space_wx.flare { 1e-5 } else { 1e-7 },
            })
            .unwrap_or_default()
    };
    // The configured engine: "p533" (native ITU-R P.533, ~100 ms/prediction) or
    // the heuristic fallback. p533 is compute-heavy → keep it off the async core.
    let eng = propagation::make_predictor(&prop_engine, me, station_power_w, ant_gain_dbi);
    let t = now_unix();
    tauri::async_runtime::spawn_blocking(move || eng.predict(dx, t, &wx))
        .await
        .map_err(|e| e.to_string())
}

/// Ring-outlook cache (p533 only): (computed-at, params-key, value). The 8-azimuth
/// p533 sweep is ~1 s of pure compute — day-scale climatology, so serve it cached
/// until the params (UTC day, grid, power/gain, SSN) change or 6 h pass. The
/// heuristic path never touches this (it's microseconds and Kp-sensitive).
static RING_OUTLOOK: Mutex<Option<(std::time::Instant, String, propagation::PathPrediction)>> =
    Mutex::new(None);

/// The no-selection "Band outlook (modelled)": modeled per-band workability + MUF to
/// a ring of representative long-haul DX directions (best per band over the ring), so
/// the Connect view can answer "which bands are modeled-open for DX right now" without
/// a selected station. Needs only the operator's grid; empty if it's unset. Honors the
/// configured prediction engine (Settings ▸ "Prediction engine"): p533 runs off the
/// async core behind a windows-style params cache; the heuristic stays sync + uncached.
#[tauri::command]
async fn get_band_outlook(
    state: State<'_, SharedEngine>,
    cache: State<'_, PropCache>,
) -> Result<propagation::PathPrediction, String> {
    const RING_TTL_SECS: u64 = 6 * 3600;
    let (mygrid, prop_engine, station_power_w, ant_gain_dbi) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (
            st.mygrid.clone(),
            st.prop_engine.clone(),
            st.station_power_w,
            st.ant_tx_gain_dbi + st.ant_rx_gain_dbi,
        )
    };
    let Some(me) = propagation::geo::maidenhead_to_latlon(mygrid.trim()) else {
        return Ok(propagation::PathPrediction {
            engine: "heuristic".to_string(),
            bands: Vec::new(),
            muf_now: 0.0,
            muf_hourly: Vec::new(),
        });
    };
    let p533 = prop_engine == "p533";
    let wx = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .map(|(_, s)| propagation::SpaceWx {
                // R12 only matters to p533; withholding it from the heuristic keeps
                // that path byte-identical to its pre-engine-seam behavior.
                ssn: if p533 {
                    LAST_SSN.lock().ok().and_then(|g| *g)
                } else {
                    None
                },
                sfi: s.space_wx.sfi,
                kp: s.space_wx.kp,
                a_index: s.space_wx.a_index,
                xray_long: if s.space_wx.flare { 1e-5 } else { 1e-7 },
            })
            .unwrap_or_default()
    };
    let t = now_unix();
    // 8 azimuths at ~9000 km — direction-agnostic "best band to ANY far DX now".
    if !p533 {
        let eng = propagation::HeuristicEngine::new(Some(me));
        return Ok(propagation::band_outlook_ring(&eng, me, 9000.0, 8, t, &wx));
    }
    let key = format!(
        "{}|{mygrid}|{ant_gain_dbi}|{:?}|{:?}",
        t / 86_400,
        station_power_w,
        wx.ssn.map(|v| v.round() as i32),
    );
    if let Ok(g) = RING_OUTLOOK.lock() {
        if let Some((when, k, v)) = g.as_ref() {
            if *k == key && when.elapsed().as_secs() < RING_TTL_SECS {
                // The cached hourly grids are day-anchored; the two "now"
                // scalars (muf_now + each band's mode_now chips) drift across
                // hours — re-derive BOTH for the serving hour (review catch:
                // mode_now froze at compute time and served 6 h stale).
                let mut out = v.clone();
                let h = (t.rem_euclid(86_400) / 3600) as usize;
                if let Some(&m) = out.muf_hourly.get(h) {
                    out.muf_now = m;
                }
                for b in &mut out.bands {
                    if !b.mode_hourly.is_empty() {
                        b.mode_now = propagation::mode_now_at(&b.mode_hourly, h);
                    }
                }
                return Ok(out);
            }
        }
    }
    let eng = propagation::make_predictor(&prop_engine, Some(me), station_power_w, ant_gain_dbi);
    let out = tauri::async_runtime::spawn_blocking(move || {
        propagation::band_outlook_ring(eng.as_ref(), me, 9000.0, 8, t, &wx)
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Ok(mut g) = RING_OUTLOOK.lock() {
        *g = Some((std::time::Instant::now(), key, out.clone()));
    }
    Ok(out)
}

/// One DXpedition's modelled contact windows from the operator's QTH — the
/// "Your Window" data for the cards + calendar (best-shot line + 24h×band grid).
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DxpedWindow {
    call: String,
    /// Which model produced it ("p533" → the UI badge shows P.533).
    engine: String,
    /// One-line headline, e.g. "17m Good 0230–0430Z" (CalendarEntry.best format).
    best: String,
    /// Top bands' 24 h outlooks, best first — feeds LikelihoodHeatmap directly.
    outlook: Vec<propagation::BandOutlook>,
    /// Week planner: per-day best shot for the next `days` days (index 0 = today,
    /// anchored at now + n·24 h so the engine sees each day's own date). Empty
    /// when the caller asked for a single day.
    days: Vec<DxpedDayBest>,
    /// Announced on-air dates (from the forward calendar). None for expeditions
    /// active NOW (the dashboard cards carry no end date) — consumers treat None
    /// as "on the air, no date gate". The wake-me alarm needs these so it never
    /// fires for a station that is not transmitting yet.
    start_unix: Option<i64>,
    end_unix: Option<i64>,
}

/// One day of the week planner: the day's best-band headline + its 0..1 score
/// (the calendar strip colors by score; the headline is the tooltip).
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DxpedDayBest {
    day_unix: i64,
    best: String,
    score: f32,
}

/// Windows cache, keyed by the full params string (UTC day, grid, engine, power,
/// SSN, target set, day count). The windows are month-scale climatology — entries
/// live 6 h. Keyed (not single-slot) because Connect polls 1-day windows while
/// the DXpeditions board polls the 7-day planner; a single slot would thrash and
/// re-run the p533 sweep on every alternating call. Expired entries are pruned on
/// insert, so the map stays at the handful of live param shapes.
static DXPED_WINDOWS: Mutex<Vec<(std::time::Instant, String, Vec<DxpedWindow>)>> =
    Mutex::new(Vec::new());

/// Modelled best-contact windows for every active + upcoming DXpedition, from
/// the operator's grid, using the CONFIGURED prediction engine (Settings ▸
/// "Prediction engine" — p533 or heuristic; the `engine` field badges which).
/// Deliberately a separate command from `get_propagation`: the dashboard builds
/// synchronously inside the live snapshot fetch, and 10–30 p533 predictions
/// would stall that path. Targets come from the cached snapshot's dashboard
/// (positions recovered via bearing+distance — same entity-centroid fidelity
/// the cards were built from). Empty until the first snapshot exists.
#[tauri::command]
async fn get_dxped_windows(
    state: State<'_, SharedEngine>,
    cache: State<'_, PropCache>,
    days: Option<u32>,
) -> Result<Vec<DxpedWindow>, String> {
    const WINDOWS_TTL_SECS: u64 = 6 * 3600;
    // 1 = today only (Connect's default); the DXpeditions board asks for 7 (the
    // week planner). Clamped so a bad caller can't request an unbounded sweep.
    let days = days.unwrap_or(1).clamp(1, 10);
    let (mygrid, prop_engine, station_power_w, ant_gain_dbi) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (
            st.mygrid.clone(),
            st.prop_engine.clone(),
            st.station_power_w,
            st.ant_tx_gain_dbi + st.ant_rx_gain_dbi,
        )
    };
    let Some(me) = propagation::geo::maidenhead_to_latlon(mygrid.trim()) else {
        return Ok(Vec::new());
    };
    // Targets (call → latlon via bearing+distance) + space weather, both from the
    // cached snapshot (the same values the dashboard itself was built from).
    let (targets, wx) = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        let Some((_, s)) = guard.as_ref() else {
            return Ok(Vec::new()); // no snapshot yet — the board is empty too
        };
        let mut seen = std::collections::HashSet::new();
        let mut targets: Vec<(String, (f64, f64), Option<i64>, Option<i64>)> = Vec::new();
        // Active cards carry no dates (they're on the air NOW); calendar entries
        // carry the announced start/end so the alarm can gate on them.
        let cards = s
            .dxpeditions
            .workable_now
            .iter()
            .map(|c| (&c.call, c.bearing_deg, c.distance_km, None, None));
        let cal = s.dxpeditions.upcoming.iter().map(|e| {
            (
                &e.call,
                e.bearing_deg,
                e.distance_km,
                Some(e.start_unix),
                Some(e.end_unix),
            )
        });
        for (call, brg, km, start, end) in cards.chain(cal) {
            if seen.insert(call.clone()) {
                targets.push((
                    call.clone(),
                    propagation::geo::destination_point(me, brg as f64, km as f64),
                    start,
                    end,
                ));
            }
        }
        let wx = propagation::SpaceWx {
            sfi: s.space_wx.sfi,
            ssn: LAST_SSN.lock().ok().and_then(|g| *g),
            kp: s.space_wx.kp,
            a_index: s.space_wx.a_index,
            xray_long: s.space_wx.xray_long,
        };
        (targets, wx)
    };
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let day = now_unix() / 86_400;
    let mut calls: Vec<&str> = targets.iter().map(|(c, ..)| c.as_str()).collect();
    calls.sort_unstable();
    let key = format!(
        "{day}|{days}|{mygrid}|{prop_engine}|{ant_gain_dbi}|{:?}|{:?}|{}",
        station_power_w,
        wx.ssn.map(|v| v.round() as i32),
        calls.join(",")
    );
    if let Ok(g) = DXPED_WINDOWS.lock() {
        if let Some((_, _, v)) = g
            .iter()
            .find(|(when, k, _)| *k == key && when.elapsed().as_secs() < WINDOWS_TTL_SECS)
        {
            return Ok(v.clone());
        }
    }
    // Build the engine ONCE and sweep every target inside one spawn_blocking —
    // the p533 CCIR-cell memo makes same-month targets amortize each other.
    let eng = propagation::make_predictor(&prop_engine, Some(me), station_power_w, ant_gain_dbi);
    let t = now_unix();
    let out = tauri::async_runtime::spawn_blocking(move || {
        targets
            .into_iter()
            .map(|(call, dx, start_unix, end_unix)| {
                let mut p = eng.predict(dx, t, &wx);
                p.bands.truncate(4);
                let best_line = |p: &propagation::PathPrediction| {
                    p.bands
                        .first()
                        .map(|b| format!("{} {} {}", b.band, b.workability, b.window))
                        .unwrap_or_default()
                };
                let best = best_line(&p);
                // Week planner: day 0 reuses today's prediction; each further day
                // re-anchors at now + n·24 h so the engine derives that day's own
                // date (month boundaries included). Same-month days amortize via
                // the CCIR-cell memo, so the 7-day sweep is far below 7× cost.
                let days_out = if days > 1 {
                    (0..days as i64)
                        .map(|n| {
                            let dt = t + n * 86_400;
                            let dp = if n == 0 {
                                p.clone()
                            } else {
                                eng.predict(dx, dt, &wx)
                            };
                            DxpedDayBest {
                                day_unix: dt,
                                best: best_line(&dp),
                                score: dp.bands.first().map(|b| b.score).unwrap_or(0.0),
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                DxpedWindow {
                    call,
                    engine: p.engine,
                    best,
                    outlook: p.bands,
                    days: days_out,
                    start_unix,
                    end_unix,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Ok(mut g) = DXPED_WINDOWS.lock() {
        g.retain(|(when, _, _)| when.elapsed().as_secs() < WINDOWS_TTL_SECS);
        g.retain(|(_, k, _)| *k != key);
        g.push((std::time::Instant::now(), key, out.clone()));
    }
    Ok(out)
}

/// "Am I getting out?" — who is hearing the operator right now, from the live PSK
/// Reporter / RBN firehose (spots where the operator is the TX side). Pure
/// observed data — the most reassuring live answer a station can get.
#[tauri::command]
async fn get_getting_out(
    state: State<'_, SharedEngine>,
    live_paths: State<'_, SharedLivePaths>,
) -> Result<propagation::GettingOut, String> {
    let (mycall, mygrid) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (s.mycall.clone(), s.mygrid.clone())
    };
    let now = now_unix();
    let spots = live_paths
        .lock()
        .map(|b| b.recent(now, 1800))
        .unwrap_or_default();
    Ok(propagation::getting_out(&mycall, &mygrid, &spots, now))
}

/// The current OVATION aurora oval (downsampled prob ≥ 8 %), for the map overlay.
/// Cached `AURORA_TTL_SECS`; serves the last-good set on a fetch failure.
#[tauri::command]
async fn get_aurora(
    cache: State<'_, AuroraCache>,
) -> Result<Vec<propagation::live::aurora::AuroraPoint>, String> {
    const AURORA_TTL_SECS: u64 = 600;
    {
        let g = cache.lock().map_err(|e| e.to_string())?;
        if let Some((when, pts)) = g.as_ref() {
            if when.elapsed().as_secs() < AURORA_TTL_SECS {
                return Ok(pts.clone());
            }
        }
    }
    match propagation::live::aurora::fetch_aurora() {
        Ok(pts) => {
            if let Ok(mut g) = cache.lock() {
                *g = Some((std::time::Instant::now(), pts.clone()));
            }
            Ok(pts)
        }
        Err(_) => {
            // Serve a stale oval rather than nothing; empty if we never had one.
            let g = cache.lock().map_err(|e| e.to_string())?;
            Ok(g.as_ref().map(|(_, p)| p.clone()).unwrap_or_default())
        }
    }
}

/// The current polar-cap absorption (PCA) view for the map overlay + space-wx
/// insight: GOES integral-proton flux (SWPC, cached 5 min, stale-served on
/// fetch failure) composed through the Sauer & Wilkinson D-RAP2 model
/// (propagation::pca). `None` when no proton data has EVER been fetched —
/// the honest offline state; a quiet sky returns Some with empty `points`
/// (the map draws nothing).
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PcaView {
    /// J(≥10 MeV) pfu — the NOAA S-scale driver (S1=10, S2=100, …).
    j10: f64,
    /// Day/night 30 MHz cap absorption (dB) — the headline numbers.
    a30_day: f64,
    a30_night: f64,
    /// Polar-cap cutoff (geomagnetic latitude, °) at the current Kp.
    cutoff_deg: f64,
    /// Polar shading samples (empty when quiet — draw nothing).
    points: Vec<propagation::pca::PcaPoint>,
}

#[tauri::command]
async fn get_pca(
    cache: State<'_, PropCache>,
    protons: State<'_, ProtonCache>,
) -> Result<Option<PcaView>, String> {
    const PROTON_TTL_SECS: u64 = 300;
    /// Below this 30 MHz cap absorption the event isn't operationally visible.
    const MIN_DB: f64 = 0.5;
    let cached = {
        let g = protons.lock().map_err(|e| e.to_string())?;
        g.as_ref()
            .and_then(|(when, p)| (when.elapsed().as_secs() < PROTON_TTL_SECS).then_some(*p))
    };
    let flux = match cached {
        Some(p) => Some(p),
        None => {
            match tauri::async_runtime::spawn_blocking(propagation::live::protons::fetch_protons)
                .await
                .map_err(|e| e.to_string())?
            {
                Ok(p) => {
                    if let Ok(mut g) = protons.lock() {
                        *g = Some((std::time::Instant::now(), p));
                    }
                    Some(p)
                }
                // Serve the stale reading rather than nothing; None if never had one.
                Err(_) => protons
                    .lock()
                    .map_err(|e| e.to_string())?
                    .as_ref()
                    .map(|(_, p)| *p),
            }
        }
    };
    let Some(flux) = flux else {
        return Ok(None); // never fetched — honest offline, not a fabricated quiet
    };
    let kp = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .map(|(_, s)| s.space_wx.kp as f64)
            .unwrap_or(0.0)
    };
    let now = now_unix();
    Ok(Some(PcaView {
        j10: flux.j10,
        a30_day: propagation::pca::a30_day(flux.j5),
        a30_night: propagation::pca::a30_night(flux.j1),
        cutoff_deg: propagation::pca::cutoff_lat_deg(kp),
        points: propagation::pca::pca_layer(flux.j5, flux.j1, kp, now, MIN_DB),
    }))
}

/// Magnetic declination (degrees, east-positive) at the operator's QTH right
/// now — WMM2025 from the vendored NOAA coefficients. The UI subtracts it from
/// true bearings to show the magnetic heading a compass-zeroed rotator needs.
/// `None` when the grid is unset/invalid.
#[tauri::command]
fn get_declination(state: State<'_, SharedEngine>) -> Result<Option<f64>, String> {
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
    };
    Ok(propagation::wmm::declination_for_grid(&mygrid, now_unix()))
}

/// ARRL LoTW user-activity data: call → last-upload unix. Feeds the injected
/// engine resolver (decode/roster LoTW marks). Empty until the operator fetches
/// (or a persisted copy loads at startup) — the honest default is no highlight.
static LOTW_ACTIVITY: std::sync::LazyLock<
    std::sync::RwLock<std::collections::HashMap<String, i64>>,
> = std::sync::LazyLock::new(Default::default);
/// The operator's recency window (days), synced from settings; the resolver reads it.
static LOTW_MAX_AGE_DAYS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(365);
/// Unix time of the last successful fetch/refresh check (0 = never).
static LOTW_FETCHED_AT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn lotw_users_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lotw_users.csv")
}

fn lotw_users_meta_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lotw_users.meta.json")
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct LotwUsersMeta {
    etag: Option<String>,
    last_modified: Option<String>,
    fetched_at: i64,
}

/// The Settings row's status: how many calls are loaded + when last fetched.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LotwUsersStatus {
    count: usize,
    fetched_at: i64,
}

fn lotw_status() -> LotwUsersStatus {
    LotwUsersStatus {
        count: LOTW_ACTIVITY.read().map(|m| m.len()).unwrap_or(0),
        fetched_at: LOTW_FETCHED_AT.load(std::sync::atomic::Ordering::Relaxed),
    }
}

#[tauri::command]
fn get_lotw_users_status() -> LotwUsersStatus {
    lotw_status()
}

/// Fetch/refresh ARRL's LoTW user-activity list (the Settings "Fetch now"
/// button — manual by design, mirroring WSJT-X; the file changes weekly).
/// Conditional GET: an unchanged file costs a 304, not 6 MB. On success the
/// CSV + validators persist beside settings so restarts load instantly.
#[tauri::command]
async fn fetch_lotw_users() -> Result<LotwUsersStatus, String> {
    use propagation::live::lotw_users::{fetch_user_activity, parse_user_activity, LotwUsersFetch};
    let meta: LotwUsersMeta = std::fs::read_to_string(lotw_users_meta_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // Only send validators when we actually HOLD the data — otherwise a
    // surviving meta.json with a deleted/corrupt CSV earns a 304 that can
    // never repopulate the empty list (review catch).
    let have_data = LOTW_ACTIVITY.read().map(|m| !m.is_empty()).unwrap_or(false);
    let (etag, last_modified) = if have_data {
        (meta.etag.clone(), meta.last_modified.clone())
    } else {
        (None, None)
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        fetch_user_activity(etag.as_deref(), last_modified.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;
    let now = now_unix();
    match result {
        LotwUsersFetch::NotModified => {
            LOTW_FETCHED_AT.store(now, std::sync::atomic::Ordering::Relaxed);
            let _ = std::fs::write(
                lotw_users_meta_path(),
                serde_json::to_string(&LotwUsersMeta {
                    fetched_at: now,
                    ..meta
                })
                .unwrap_or_default(),
            );
            Ok(lotw_status())
        }
        LotwUsersFetch::Fresh {
            csv,
            etag,
            last_modified,
        } => {
            let map = parse_user_activity(&csv);
            if map.is_empty() {
                return Err("LoTW list downloaded but parsed to zero calls".to_string());
            }
            let _ = std::fs::write(lotw_users_path(), &csv);
            let _ = std::fs::write(
                lotw_users_meta_path(),
                serde_json::to_string(&LotwUsersMeta {
                    etag,
                    last_modified,
                    fetched_at: now,
                })
                .unwrap_or_default(),
            );
            if let Ok(mut g) = LOTW_ACTIVITY.write() {
                *g = map;
            }
            LOTW_FETCHED_AT.store(now, std::sync::atomic::Ordering::Relaxed);
            Ok(lotw_status())
        }
    }
}

/// Where fetched TLEs persist (beside settings.json): day-scale orbital elements,
/// so surviving a restart matters more than freshness-to-the-minute.
fn tles_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tles.json")
}

/// Celestrak amateur TLE cache: (fetched-at, elements). 12 h TTL per the spec —
/// Celestrak asks consumers to cache; TLEs are day-scale data.
static TLE_CACHE: Mutex<Option<(std::time::Instant, Vec<propagation::sat::Tle>)>> =
    Mutex::new(None);
/// Computed PASS-LIST cache (the expensive 24 h scan). Subpoints are NEVER
/// cached — a LEO ground track moves ~4°/min, so positions are recomputed on
/// every call (one cheap sgp4 eval per bird) while the pass scan reuses this.
static SAT_PASSES: Mutex<Option<(std::time::Instant, String, Vec<SatPassDto>)>> = Mutex::new(None);

/// One bird's sub-satellite point right now.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SatBird {
    name: String,
    lat: f64,
    lon: f64,
    alt_km: f64,
    /// Horizon-circle radius (km) — the footprint ring the map draws for chased birds.
    footprint_km: f64,
    /// Ground track ±(trail/projection) around now, one point per minute:
    /// [unix, lat, lon]. Past points draw the fading trail; future points the
    /// dashed projection — and the UI interpolates along them so the icon
    /// MOVES in real time between polls.
    track: Vec<(i64, f64, f64)>,
}

/// The satellites view: positions NOW + upcoming passes over the operator's QTH.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SatView {
    /// Age of the OLDEST element set in days — the UI badges > 14 d as stale.
    tle_age_days: f64,
    birds: Vec<SatBird>,
    /// Next-24 h passes over the QTH, all birds (empty when the grid is unset).
    /// Sorted by AOS. Geometry only — no transponder/workability claim.
    passes: Vec<SatPassDto>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SatPassDto {
    name: String,
    aos_unix: i64,
    los_unix: i64,
    max_el_deg: f64,
    aos_az_deg: f64,
    los_az_deg: f64,
    /// SatNOGS operational status, stamped only on Satellites-section schedule
    /// rows (from the weekly cache); absent on the map view + when offline.
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

/// Current amateur-satellite picture: sub-satellite points for the Celestrak
/// amateur group + next-24 h passes over the operator's grid. TLEs cached 12 h
/// Post the operator's own DX spot to the human DX cluster. Formats a canonical
/// `DX <freq_khz> <call> <comment>` line and queues it for the connected human
/// node(s) to send. Gated on a node being connected NOW — a spot must not buffer
/// and post stale hours later — and on a real callsign + a sane frequency.
#[tauri::command]
fn post_spot(freq_mhz: f64, call: String, comment: String) -> Result<(), String> {
    if !freq_mhz.is_finite() || freq_mhz <= 0.0 {
        return Err("invalid frequency".into());
    }
    let call = call.trim().to_ascii_uppercase();
    if !is_real_call(&call) {
        return Err("enter a valid callsign to spot".into());
    }
    let connected = PHONE_NODE_CONNS
        .lock()
        .map(|v| {
            v.iter()
                .any(|b| b.load(std::sync::atomic::Ordering::Relaxed))
        })
        .unwrap_or(false);
    if !connected {
        return Err("no DX cluster connected — set a cluster host in Settings".into());
    }
    let line = tempo_net::cluster::format_dx_spot(freq_mhz * 1000.0, &call, &comment);
    CLUSTER_OUTBOX
        .lock()
        .map_err(|_| "spot queue unavailable".to_string())?
        .push_back(line);
    Ok(())
}

/// Upcoming amateur-radio contests from the WA7BNM calendar RSS feed. Off the
/// async runtime (blocking HTTP + parse). Rejects if the feed is unreachable.
const CONTEST_RSS_URL: &str = "https://www.contestcalendar.com/calendar.rss";

#[tauri::command]
async fn get_contests() -> Result<Vec<propagation::live::contests::ContestEvent>, String> {
    tauri::async_runtime::spawn_blocking(
        || -> Result<Vec<propagation::live::contests::ContestEvent>, String> {
            let xml = propagation::live::contests::fetch(CONTEST_RSS_URL)?;
            Ok(propagation::live::contests::parse_contest_rss(&xml))
        },
    )
    .await
    .map_err(|e| e.to_string())?
}

/// (+ persisted beside settings, so a restart serves instantly and offline
/// starts stay honest). Subpoints are recomputed EVERY call (LEO tracks move
/// ~4°/min); only the 24 h pass scan caches (10 min). Staleness is per bird:
/// elements >30 days drop that bird alone (SGP4 accuracy is gone); `None` only
/// when no usable elements exist at all — the UI draws nothing.
#[tauri::command]
async fn get_satellites(state: State<'_, SharedEngine>) -> Result<Option<SatView>, String> {
    const VIEW_TTL_SECS: u64 = 600;
    const STALE_DAYS: f64 = 30.0;
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
    };
    let now = now_unix();
    let key = format!("{}|{}", now / (VIEW_TTL_SECS as i64), mygrid);
    let cached_passes = {
        let g = SAT_PASSES.lock().map_err(|e| e.to_string())?;
        g.as_ref().and_then(|(when, k, v)| {
            (*k == key && when.elapsed().as_secs() < VIEW_TTL_SECS).then(|| v.clone())
        })
    };
    let tles = load_tles().await?;
    if tles.is_empty() {
        return Ok(None); // never had elements — honest no-data
    }
    let observer = propagation::geo::maidenhead_to_latlon(mygrid.trim());
    let need_passes = cached_passes.is_none();
    let out = tauri::async_runtime::spawn_blocking(move || {
        use propagation::sat;
        const RE_KM: f64 = 6371.0;
        // Staleness is PER BIRD (the spec's rule): a decaying cubesat with a
        // month-old epoch drops out alone — it must never blank the fresh
        // majority (review catch: the old max-age gate killed the whole view).
        let fresh: Vec<&propagation::sat::Tle> = tles
            .iter()
            .filter(|t| sat::tle_age_days(&t.line1, now).is_some_and(|a| a <= STALE_DAYS))
            .collect();
        if fresh.is_empty() {
            return None; // every element set is decayed — honest no-data
        }
        // Badge scalar = oldest INCLUDED bird (an excluded outlier must not
        // make the whole pane read stale).
        let tle_age_days = fresh
            .iter()
            .filter_map(|t| sat::tle_age_days(&t.line1, now))
            .fold(0.0f64, f64::max);
        let mut birds = Vec::new();
        let mut computed_passes = Vec::new();
        for t in &fresh {
            if let Some((lat, lon, alt_km)) = sat::subpoint(t, now) {
                let footprint_km = RE_KM * (RE_KM / (RE_KM + alt_km)).acos();
                // 10 min of trail + 25 min of projection at 1-min steps — one
                // TLE parse per bird (the batch fn), ~ms for the whole flock.
                let track = sat::track(t, now, 600, 1_500, 60);
                birds.push(SatBird {
                    name: t.name.clone(),
                    lat,
                    lon,
                    alt_km,
                    footprint_km,
                    track,
                });
                if need_passes {
                    if let Some(obs) = observer {
                        for p in sat::passes(t, obs, now, 24) {
                            computed_passes.push(SatPassDto {
                                name: t.name.clone(),
                                aos_unix: p.aos_unix,
                                los_unix: p.los_unix,
                                max_el_deg: p.max_el_deg,
                                aos_az_deg: p.aos_az_deg,
                                los_az_deg: p.los_az_deg,
                                status: None,
                            });
                        }
                    }
                }
            }
        }
        let passes = match cached_passes {
            Some(p) => p,
            None => {
                computed_passes.sort_by_key(|p| p.aos_unix);
                computed_passes
            }
        };
        Some((
            SatView {
                tle_age_days,
                birds,
                passes: passes.clone(),
            },
            passes,
            need_passes,
        ))
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(match out {
        Some((view, passes, computed)) => {
            if computed {
                if let Ok(mut g) = SAT_PASSES.lock() {
                    *g = Some((std::time::Instant::now(), key, passes));
                }
            }
            Some(view)
        }
        None => None,
    })
}

/// Elements for every sat command: fresh cache → network (persisting on
/// success) → stale cache → disk. Empty = we truly never had elements — the
/// callers render honest no-data, never a guess.
async fn load_tles() -> Result<Vec<propagation::sat::Tle>, String> {
    const TLE_TTL_SECS: u64 = 12 * 3600;
    let cached = {
        let g = TLE_CACHE.lock().map_err(|e| e.to_string())?;
        g.as_ref()
            .and_then(|(when, t)| (when.elapsed().as_secs() < TLE_TTL_SECS).then(|| t.clone()))
    };
    Ok(match cached {
        Some(t) => t,
        None => {
            match tauri::async_runtime::spawn_blocking(propagation::live::tle::fetch_tles)
                .await
                .map_err(|e| e.to_string())?
            {
                Ok(t) if !t.is_empty() => {
                    if let Ok(mut g) = TLE_CACHE.lock() {
                        *g = Some((std::time::Instant::now(), t.clone()));
                    }
                    if let Ok(json) = serde_json::to_string(&t) {
                        let _ = std::fs::write(tles_path(), json);
                    }
                    t
                }
                _ => {
                    // Fetch failed — serve the stale cache, else the persisted set.
                    let stale = TLE_CACHE
                        .lock()
                        .ok()
                        .and_then(|g| g.as_ref().map(|(_, t)| t.clone()));
                    match stale {
                        Some(t) => t,
                        None => {
                            let disk: Option<Vec<propagation::sat::Tle>> =
                                std::fs::read_to_string(tles_path())
                                    .ok()
                                    .and_then(|s| serde_json::from_str(&s).ok());
                            match disk {
                                Some(t) if !t.is_empty() => {
                                    if let Ok(mut g) = TLE_CACHE.lock() {
                                        *g = Some((std::time::Instant::now(), t.clone()));
                                    }
                                    t
                                }
                                _ => Vec::new(),
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Where the SatNOGS snapshot persists (beside settings.json). Week-scale data:
/// statuses and transponders change on human timescales, and SatNOGS asks bulk
/// consumers to be gentle — one filtered fetch a week is plenty.
fn satnogs_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("satnogs.json")
}

/// The persisted SatNOGS snapshot: fetch stamp + statuses + transmitters for
/// the birds we track. Data CC-BY-SA 4.0 — the UI credits it where shown.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SatnogsSnapshot {
    fetched_at: i64,
    /// The NORAD set this snapshot was fetched FOR. Coverage is part of
    /// freshness: a time-fresh snapshot that was never asked about a bird is a
    /// MISS for that bird (review catch — a single-bird detail fetch must
    /// never freeze a 1-bird snapshot for a week).
    #[serde(default)]
    norads: Vec<u32>,
    statuses: Vec<propagation::live::satnogs::SatStatus>,
    transmitters: Vec<propagation::live::satnogs::Transmitter>,
}

static SATNOGS: Mutex<Option<SatnogsSnapshot>> = Mutex::new(None);
static SATNOGS_FETCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Last refresh ATTEMPT (unix) — failed fetches back off 30 min instead of
/// being re-tripped every 30 s by the alarm tick (SatNOGS asks bulk consumers
/// to be gentle; a dead network must not turn into a full-catalog hammer).
static SATNOGS_LAST_TRY: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Best-available SatNOGS data NOW (memory → disk), kicking a background
/// refresh when the snapshot is older than a week. Returns `None` when we have
/// never fetched — callers show "no data yet", never invented statuses. A stale
/// snapshot is still served (with its honest fetch stamp) while the refresh runs.
fn satnogs_snapshot(norads: Vec<u32>) -> Option<SatnogsSnapshot> {
    use std::sync::atomic::Ordering;
    const TTL_SECS: i64 = 7 * 24 * 3600;
    const RETRY_BACKOFF_SECS: i64 = 1800;
    let mem = SATNOGS.lock().ok().and_then(|g| g.clone());
    let snap = mem.or_else(|| {
        let disk: Option<SatnogsSnapshot> = std::fs::read_to_string(satnogs_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        if let Some(d) = &disk {
            if let Ok(mut g) = SATNOGS.lock() {
                *g = Some(d.clone());
            }
        }
        disk
    });
    // Fresh = recent AND covering every requested bird. The refresh fetches
    // the UNION of requested + already-covered, so a narrow caller can only
    // ever GROW the snapshot, never shrink it for everyone else.
    let now = now_unix();
    let covered: std::collections::HashSet<u32> = snap
        .as_ref()
        .map(|sn| sn.norads.iter().copied().collect())
        .unwrap_or_default();
    let fresh = snap
        .as_ref()
        .is_some_and(|sn| now - sn.fetched_at < TTL_SECS)
        && norads.iter().all(|n| covered.contains(n));
    let backoff_ok = now - SATNOGS_LAST_TRY.load(Ordering::SeqCst) >= RETRY_BACKOFF_SECS;
    if !fresh && !norads.is_empty() && backoff_ok && !SATNOGS_FETCHING.swap(true, Ordering::SeqCst)
    {
        SATNOGS_LAST_TRY.store(now, Ordering::SeqCst);
        let mut want: Vec<u32> = covered
            .union(&norads.iter().copied().collect())
            .copied()
            .collect();
        want.sort_unstable();
        tauri::async_runtime::spawn_blocking(move || {
            let statuses = propagation::live::satnogs::fetch_satellites(&want);
            let transmitters = propagation::live::satnogs::fetch_transmitters(&want);
            if let (Ok(statuses), Ok(transmitters)) = (statuses, transmitters) {
                let sn = SatnogsSnapshot {
                    fetched_at: now_unix(),
                    norads: want,
                    statuses,
                    transmitters,
                };
                if let Ok(json) = serde_json::to_string(&sn) {
                    let _ = std::fs::write(satnogs_path(), json);
                }
                if let Ok(mut g) = SATNOGS.lock() {
                    *g = Some(sn);
                }
            } // a failed fetch keeps whatever we had — retried after the backoff
            SATNOGS_FETCHING.store(false, Ordering::SeqCst);
        });
    }
    snap
}

/// Passes for NAMED birds (the ★ favorites) over the next `hours` (1–72),
/// SatNOGS status stamped when the weekly cache knows the bird. Empty when the
/// grid is unset or no named bird has usable elements. Geometry is modelled
/// (SGP4); status is community-measured — the two are labeled apart in the UI.
#[tauri::command]
async fn get_sat_schedule(
    state: State<'_, SharedEngine>,
    names: Vec<String>,
    hours: u32,
) -> Result<Vec<SatPassDto>, String> {
    const STALE_DAYS: f64 = 30.0;
    let hours = hours.clamp(1, 72);
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
    };
    let Some(obs) = propagation::geo::maidenhead_to_latlon(mygrid.trim()) else {
        return Ok(Vec::new());
    };
    let tles = load_tles().await?;
    if tles.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: std::collections::HashSet<String> =
        names.iter().map(|n| n.trim().to_uppercase()).collect();
    let now = now_unix();
    let out = tauri::async_runtime::spawn_blocking(move || {
        use propagation::sat;
        let mine: Vec<&sat::Tle> = tles
            .iter()
            .filter(|t| wanted.contains(&t.name.to_uppercase()))
            .filter(|t| sat::tle_age_days(&t.line1, now).is_some_and(|a| a <= STALE_DAYS))
            .collect();
        // Status lookup by NORAD id (name-mapping-proof), from the weekly cache.
        let norads: Vec<u32> = mine
            .iter()
            .filter_map(|t| sat::norad_id(&t.line1))
            .collect();
        let status_by_norad: std::collections::HashMap<u32, String> = satnogs_snapshot(norads)
            .map(|sn| {
                sn.statuses
                    .into_iter()
                    .map(|st| (st.norad, st.status))
                    .collect()
            })
            .unwrap_or_default();
        let mut passes = Vec::new();
        for t in mine {
            let status = sat::norad_id(&t.line1).and_then(|n| status_by_norad.get(&n).cloned());
            // Scan from 6 h back so a pass ALREADY in progress keeps its real
            // AOS — MEO birds (IO-117-style) fly multi-hour passes, and a short
            // backscan fabricated a window-edge AOS + understated max el. The
            // horizon is widened to compensate so `hours` stays FORWARD-looking.
            for p in sat::passes(t, obs, now - 21_600, hours + 6) {
                if p.los_unix <= now {
                    continue;
                }
                passes.push(SatPassDto {
                    name: t.name.clone(),
                    aos_unix: p.aos_unix,
                    los_unix: p.los_unix,
                    max_el_deg: p.max_el_deg,
                    aos_az_deg: p.aos_az_deg,
                    los_az_deg: p.los_az_deg,
                    status: status.clone(),
                });
            }
        }
        passes.sort_by_key(|p| p.aos_unix);
        passes
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(out)
}

/// The ISS's current-or-next pass over the operator's QTH, or `None`. Keyed on
/// NORAD **25544** (name-proof — the "ISS (ZARYA)" element name is not), so the
/// SSTV auto-arm opt-in can never tune the rig off a mis-mapped bird. Requires a
/// resolvable grid; scans from 30 min back (a pass already in progress keeps its
/// real AOS) over a 3 h horizon and returns the first pass still above the
/// horizon. Geometry only — no transponder claim (the ISS SSTV downlink is
/// event-scheduled; the operator opts in per the auto-arm setting).
#[tauri::command]
async fn get_iss_pass(state: State<'_, SharedEngine>) -> Result<Option<SatPassDto>, String> {
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
    };
    let Some(obs) = propagation::geo::maidenhead_to_latlon(mygrid.trim()) else {
        return Ok(None);
    };
    let tles = load_tles().await?;
    if tles.is_empty() {
        return Ok(None);
    }
    let now = now_unix();
    tauri::async_runtime::spawn_blocking(move || iss_pass_from_tles(&tles, obs, now))
        .await
        .map_err(|e| e.to_string())
}

/// The ISS (NORAD 25544) pass that's up now or rises next, from `tles` over
/// `obs`, at `now` (unix s). Filters by catalog number so a renamed element set
/// still matches; 30 min backscan + 3 h horizon; first pass whose LOS is in the
/// future. `None` when the ISS isn't in the set / has no usable elements / has
/// no pass in the window. Pure — [`get_iss_pass`] wraps it; the tests drive it.
fn iss_pass_from_tles(
    tles: &[propagation::sat::Tle],
    obs: (f64, f64),
    now: i64,
) -> Option<SatPassDto> {
    use propagation::sat;
    let tle = tles.iter().find(|t| sat::norad_id(&t.line1) == Some(25544))?;
    sat::passes(tle, obs, now - 1800, 3)
        .into_iter()
        .find(|p| p.los_unix > now)
        .map(|p| SatPassDto {
            name: tle.name.clone(),
            aos_unix: p.aos_unix,
            los_unix: p.los_unix,
            max_el_deg: p.max_el_deg,
            aos_az_deg: p.aos_az_deg,
            los_az_deg: p.los_az_deg,
            status: None,
        })
}

/// Per-bird detail for the Satellites section: SatNOGS status + transponders
/// (absent fields when we've never fetched — offline honesty) and the
/// current/next pass with its az/el sky track for the polar plot.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SatDetailDto {
    name: String,
    norad: Option<u32>,
    status: Option<String>,
    transmitters: Vec<propagation::live::satnogs::Transmitter>,
    data_fetched_at: Option<i64>,
    pass: Option<SatPassDto>,
    pass_track: Vec<(i64, f64, f64)>,
}

#[tauri::command]
async fn get_sat_detail(
    state: State<'_, SharedEngine>,
    name: String,
) -> Result<SatDetailDto, String> {
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
    };
    let obs = propagation::geo::maidenhead_to_latlon(mygrid.trim());
    let tles = load_tles().await?;
    let now = now_unix();
    let out = tauri::async_runtime::spawn_blocking(move || {
        use propagation::sat;
        let key = name.trim().to_uppercase();
        let tle = tles.iter().find(|t| t.name.to_uppercase() == key);
        let norad = tle.and_then(|t| sat::norad_id(&t.line1));
        let snap = satnogs_snapshot(norad.into_iter().collect());
        let status = norad.and_then(|n| {
            snap.as_ref()
                .and_then(|sn| sn.statuses.iter().find(|st| st.norad == n))
                .map(|st| st.status.clone())
        });
        let transmitters = norad
            .and_then(|n| {
                snap.as_ref().map(|sn| {
                    sn.transmitters
                        .iter()
                        .filter(|tr| tr.norad == n)
                        .cloned()
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        let (pass, pass_track) = match (tle, obs) {
            (Some(t), Some(o)) => {
                // 6 h backscan: a mid-pass MEO bird keeps its true AOS (see
                // get_sat_schedule); +6 h keeps the horizon 24 h forward.
                let next = sat::passes(t, o, now - 21_600, 30)
                    .into_iter()
                    .find(|p| p.los_unix > now);
                match next {
                    Some(p) => {
                        let track = sat::pass_track(t, o, p.aos_unix, p.los_unix, 30);
                        (
                            Some(SatPassDto {
                                name: t.name.clone(),
                                aos_unix: p.aos_unix,
                                los_unix: p.los_unix,
                                max_el_deg: p.max_el_deg,
                                aos_az_deg: p.aos_az_deg,
                                los_az_deg: p.los_az_deg,
                                status: status.clone(),
                            }),
                            track,
                        )
                    }
                    None => (None, Vec::new()),
                }
            }
            _ => (None, Vec::new()),
        };
        SatDetailDto {
            name,
            norad,
            status,
            transmitters,
            data_fetched_at: snap.map(|sn| sn.fetched_at),
            pass,
            pass_track,
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Live rotor auto-track state. Generation-owned like the WSPR feed: starting a
/// new track (or stopping) bumps the generation and the old loop exits on its
/// next tick — one loop owns the rotor at a time.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SatTrackDto {
    name: String,
    /// "armed" (waiting, no rotor commands until 5 min before AOS),
    /// "prepositioning" (parked on the AOS azimuth) or "tracking".
    state: String,
    az_deg: f64,
    el_deg: f64,
    aos_unix: i64,
    los_unix: i64,
}

static SAT_TRACK_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SAT_TRACK: Mutex<Option<SatTrackDto>> = Mutex::new(None);

/// Arm rotor auto-track for a bird's pass. Far from AOS the loop is merely
/// "armed" (no rotor commands); from 5 min out it slews to the AOS azimuth
/// (the S.A.T. behavior — on target before the bird rises), then follows
/// az/el every 3 s until LOS, then stops the rotor. `aos_unix` picks WHICH
/// pass (±3 min tolerance — the schedule row the operator clicked); omitted =
/// the current/next one. `None` = no rotor configured, no grid, or no matching
/// pass in the next 48 h — the UI says so plainly.
#[tauri::command]
async fn start_sat_track(
    state: State<'_, SharedEngine>,
    name: String,
    aos_unix: Option<i64>,
) -> Result<Option<SatTrackDto>, String> {
    let (mygrid, addr) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (st.mygrid.clone(), effective_rotator_addr(st))
    };
    let Some(addr) = addr else {
        return Ok(None);
    };
    let Some(obs) = propagation::geo::maidenhead_to_latlon(mygrid.trim()) else {
        return Ok(None);
    };
    let tles = load_tles().await?;
    let key = name.trim().to_uppercase();
    let now = now_unix();
    let Some(tle) = tles.iter().find(|t| t.name.to_uppercase() == key).cloned() else {
        return Ok(None);
    };
    // 6 h backscan (true AOS for mid-pass MEO birds) + 48 h forward horizon
    // (any schedule row is armable).
    let Some(pass) = propagation::sat::passes(&tle, obs, now - 21_600, 54)
        .into_iter()
        .filter(|p| p.los_unix > now)
        .find(|p| aos_unix.is_none_or(|h| (p.aos_unix - h).abs() <= 180))
    else {
        return Ok(None);
    };
    let gen = SAT_TRACK_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let initial = SatTrackDto {
        name: tle.name.clone(),
        state: if now < pass.aos_unix - 300 {
            "armed"
        } else if now < pass.aos_unix {
            "prepositioning"
        } else {
            "tracking"
        }
        .to_string(),
        az_deg: pass.aos_az_deg,
        el_deg: 0.0,
        aos_unix: pass.aos_unix,
        los_unix: pass.los_unix,
    };
    {
        // Guarded like every other badge write: a racing newer start must not
        // have its badge clobbered by this older one.
        let mut g = SAT_TRACK.lock().map_err(|e| e.to_string())?;
        if SAT_TRACK_GEN.load(std::sync::atomic::Ordering::SeqCst) == gen {
            *g = Some(initial.clone());
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        use propagation::sat;
        use std::sync::atomic::Ordering;
        let mut azel_ok = true; // az-only rotors: fall back, probe to recover
        let mut az_only_ticks = 0u32;
        let mut misses = 0u32;
        let update_badge = |dto: SatTrackDto| {
            if let Ok(mut g) = SAT_TRACK.lock() {
                if SAT_TRACK_GEN.load(Ordering::SeqCst) == gen {
                    *g = Some(dto);
                }
            }
        };
        loop {
            if SAT_TRACK_GEN.load(Ordering::SeqCst) != gen {
                return; // replaced or stopped — the newer owner drives the rotor
            }
            let t = now_unix();
            if t > pass.los_unix {
                break;
            }
            // Far from AOS: ARMED — hold fire entirely (the operator keeps the
            // rotor for HF until 5 min before the bird rises).
            if t < pass.aos_unix - 300 {
                update_badge(SatTrackDto {
                    name: tle.name.clone(),
                    state: "armed".to_string(),
                    az_deg: pass.aos_az_deg,
                    el_deg: 0.0,
                    aos_unix: pass.aos_unix,
                    los_unix: pass.los_unix,
                });
                std::thread::sleep(std::time::Duration::from_secs(3));
                continue;
            }
            let (az, el, phase) = if t < pass.aos_unix {
                (pass.aos_az_deg, 0.0, "prepositioning")
            } else {
                match sat::look_at(&tle, obs, t) {
                    Some((az, el)) => (az, el.max(0.0), "tracking"),
                    None => break, // propagation diverged — stop honestly
                }
            };
            // Stop pressed while we computed? Re-check right before the wire
            // write — narrows the one-command-after-halt window to microseconds.
            if SAT_TRACK_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let sent = if azel_ok {
                match tempo_audio::rotator::point_azel(&addr, az, el) {
                    Ok(()) => true,
                    Err(_) => {
                        // Az-only rotor (rotctld rejects el)? Try plain azimuth;
                        // if that works, go az-only — but PROBE below, so a
                        // transient error doesn't downgrade the whole pass.
                        if tempo_audio::rotator::point(&addr, az).is_ok() {
                            azel_ok = false;
                            az_only_ticks = 0;
                            true
                        } else {
                            false
                        }
                    }
                }
            } else {
                az_only_ticks += 1;
                if az_only_ticks >= 20 {
                    // ~60 s recovery probe: if az/el works again (the earlier
                    // failure was transient comms, not an az-only rotor), resume.
                    az_only_ticks = 0;
                    match tempo_audio::rotator::point_azel(&addr, az, el) {
                        Ok(()) => {
                            azel_ok = true;
                            true
                        }
                        Err(_) => tempo_audio::rotator::point(&addr, az).is_ok(),
                    }
                } else {
                    tempo_audio::rotator::point(&addr, az).is_ok()
                }
            };
            if sent {
                misses = 0;
                update_badge(SatTrackDto {
                    name: tle.name.clone(),
                    state: phase.to_string(),
                    az_deg: az,
                    // Honesty: report what was COMMANDED. An az-only fallback
                    // never commands elevation, so it must not claim one.
                    el_deg: if azel_ok { el } else { 0.0 },
                    aos_unix: pass.aos_unix,
                    los_unix: pass.los_unix,
                });
            } else {
                misses += 1;
                if misses >= 5 {
                    break; // rotor stopped answering — clear the badge, don't lie
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        // LOS / rotor lost: halt the rotor and clear the badge if still ours
        // (gen check INSIDE the lock — a newer track's badge must survive).
        let _ = tempo_audio::rotator::stop(&addr);
        if let Ok(mut g) = SAT_TRACK.lock() {
            if SAT_TRACK_GEN.load(Ordering::SeqCst) == gen {
                *g = None;
            }
        }
    });
    Ok(Some(initial))
}

/// Disarm auto-track: the loop exits on its next tick; halt the rotor now.
#[tauri::command]
async fn stop_sat_track(state: State<'_, SharedEngine>) -> Result<(), String> {
    SAT_TRACK_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = SAT_TRACK.lock() {
        *g = None;
    }
    let addr = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        effective_rotator_addr(eng.settings())
    };
    if let Some(addr) = addr {
        let _ =
            tauri::async_runtime::spawn_blocking(move || tempo_audio::rotator::stop(&addr)).await;
    }
    Ok(())
}

/// The live auto-track state; `None` = idle.
#[tauri::command]
fn sat_track_status() -> Result<Option<SatTrackDto>, String> {
    Ok(SAT_TRACK.lock().map_err(|e| e.to_string())?.clone())
}

/// Real-time KC2G ionosonde MUF/foF2 station fixes for the Connect map's MUF
/// overlay. Cached `KC2G_TTL_SECS`; serves the last-good set on a fetch failure,
/// empty if we never had one (never fabricated).
#[tauri::command]
async fn get_kc2g_muf(cache: State<'_, Kc2gCache>) -> Result<Vec<propagation::MufStation>, String> {
    const KC2G_TTL_SECS: u64 = 300;
    {
        let g = cache.lock().map_err(|e| e.to_string())?;
        if let Some((when, v)) = g.as_ref() {
            if when.elapsed().as_secs() < KC2G_TTL_SECS {
                return Ok(v.clone());
            }
        }
    }
    match propagation::live::kc2g::fetch_kc2g_muf() {
        Ok(v) => {
            if let Ok(mut g) = cache.lock() {
                *g = Some((std::time::Instant::now(), v.clone()));
            }
            Ok(v)
        }
        Err(_) => {
            // Serve a stale set rather than nothing; empty if we never had one.
            let g = cache.lock().map_err(|e| e.to_string())?;
            Ok(g.as_ref().map(|(_, v)| v.clone()).unwrap_or_default())
        }
    }
}

/// The freshest GOES long-band X-ray flux for the D-RAP flare layer + heads-up.
#[derive(serde::Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct XrayNow {
    /// GOES 0.1–0.8 nm flux, W/m².
    flux: f32,
    /// When the reading was fetched (Unix seconds, UTC).
    as_of: i64,
}

/// The 60 s X-ray fast lane: refetches GOES far more often than the 5-min prop
/// snapshot so a flare's ONSET reaches the map + alert in ~1 min. Serves the
/// last-good reading on a fetch failure; errors only when we never had one.
#[tauri::command]
async fn get_xray_now() -> Result<XrayNow, String> {
    const XRAY_TTL_SECS: u64 = 60;
    if let Some((when, flux, as_of)) = LAST_XRAY.lock().ok().and_then(|g| *g) {
        if when.elapsed().as_secs() < XRAY_TTL_SECS {
            return Ok(XrayNow { flux, as_of });
        }
    }
    let fetched = tauri::async_runtime::spawn_blocking(propagation::live::swpc::fetch_xray_now)
        .await
        .map_err(|e| e.to_string())?;
    match fetched {
        Ok(flux) => {
            let as_of = now_unix();
            if let Ok(mut g) = LAST_XRAY.lock() {
                *g = Some((std::time::Instant::now(), flux, as_of));
            }
            Ok(XrayNow { flux, as_of })
        }
        Err(e) => {
            // Serve stale rather than nothing (and re-arm the TTL so a NOAA outage
            // is retried once a minute, not on every UI poll).
            if let Ok(mut g) = LAST_XRAY.lock() {
                if let Some(entry) = g.as_mut() {
                    entry.0 = std::time::Instant::now();
                }
            }
            LAST_XRAY
                .lock()
                .ok()
                .and_then(|g| *g)
                .map(|(_, flux, as_of)| XrayNow { flux, as_of })
                .ok_or(e)
        }
    }
}

/// Fetch the NOAA R/S/G space-weather scales (today + tomorrow's G) plus the most
/// recent SWPC alert/watch/warning bulletins, for the space-weather strip.
/// Returned as a `(scales, alerts)` tuple — the TS side destructures it as a
/// `[scales, alerts]` array.
///
/// Cached `SCALES_TTL_SECS`; the two products degrade together (a partial fetch
/// serves the last-good pair rather than a half-empty mix), and a cold failure
/// returns quiet-default scales + no alerts — honest neutral, never fabricated.
#[tauri::command]
async fn get_space_wx_scales(
    cache: State<'_, ScalesCache>,
) -> Result<(propagation::NoaaScalesView, Vec<propagation::AlertView>), String> {
    const SCALES_TTL_SECS: u64 = 900;
    {
        let g = cache.lock().map_err(|e| e.to_string())?;
        if let Some((when, pair)) = g.as_ref() {
            if when.elapsed().as_secs() < SCALES_TTL_SECS {
                return Ok(pair.clone());
            }
        }
    }
    match (
        propagation::live::swpc_scales::fetch_noaa_scales(),
        propagation::live::swpc_scales::fetch_alerts(),
    ) {
        (Ok(mut scales), Ok(alerts)) => {
            // Provenance stamp: only a REAL fetch carries as_of — the cold-cache
            // default below stays None so the UI can't render "offline" as calm.
            scales.as_of = Some(now_unix());
            let pair = (scales, alerts);
            if let Ok(mut g) = cache.lock() {
                *g = Some((std::time::Instant::now(), pair.clone()));
            }
            Ok(pair)
        }
        _ => {
            // Any feed down → serve last-good if we have it, else quiet defaults.
            let g = cache.lock().map_err(|e| e.to_string())?;
            Ok(g.as_ref().map(|(_, pair)| pair.clone()).unwrap_or_default())
        }
    }
}

/// One waterfall row (Goertzel power spectrum of the last received frame).
#[tauri::command]
fn get_spectrum_row(state: State<'_, SharedEngine>) -> Result<Spectrum, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.spectrum_row())
}

/// Set the operating mode: "chat" | "qso-run" | "qso-monitor" | "fieldday-run"
/// | "fieldday-sp". Returns the refreshed snapshot.
#[tauri::command]
fn set_mode(state: State<'_, SharedEngine>, mode: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    // Refuse to enter a keying FT8/FT4 mode without the identity its messages need, so
    // the operator gets a clear reason instead of a silently-suppressed over. Calling
    // CQ sends a grid (CQ/Tx1); a Field Day run sends an exchange with no grid (callsign
    // only). qso-monitor / fieldday-sp are passive on entry (the backstop covers TX).
    match mode.as_str() {
        "qso-run" => eng.structured_tx_ready(true)?,
        "fieldday-run" => eng.structured_tx_ready(false)?,
        _ => {}
    }
    eng.set_mode(&mode)?;
    Ok(eng.snapshot())
}

/// Current operator/station settings.
#[tauri::command]
fn get_settings(state: State<'_, SharedEngine>) -> Result<Settings, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    // Defensive: re-mirror the ACTIVE radio's profile into the flat fields the UI reads (idempotent —
    // a no-op when already in sync). Guarantees the Settings Rig/Audio form always shows the active
    // radio's own CAT + audio device, independent of which code path last flipped the active radio.
    let mut s = eng.settings().clone();
    s.sync_flat_from_active();
    Ok(s)
}

/// Apply + persist new settings. Returns the refreshed snapshot.
///
/// Also lazily starts the live network feeds: if this change supplies a real
/// callsign (or enables the cluster) for the first time this session, the
/// cluster/RBN and PSK Reporter MQTT daemons start immediately — no app restart.
/// The `start_*_feed` latches make this a no-op once a feed is running (so an
/// already-running feed keeps its original callsign until the next restart).
#[tauri::command]
fn set_settings(
    state: State<'_, SharedEngine>,
    spots: State<'_, SharedSpots>,
    live_paths: State<'_, SharedLivePaths>,
    region_paths: State<'_, SharedRegionPaths>,
    health: State<'_, SharedHealth>,
    cache: State<'_, PropCache>,
    mut settings: Settings,
) -> Result<AppSnapshot, String> {
    // Mirror the legacy single `cluster_host` to the list head (empty when the list is
    // empty), so clearing the node list to go RBN-only actually sticks — otherwise `load`'s
    // upgrade seed would re-inject the stale legacy host on the next launch.
    settings.cluster_host = settings.cluster_hosts.first().cloned().unwrap_or_default();
    // The LoTW-mark resolver reads its recency window from this atomic.
    LOTW_MAX_AGE_DAYS.store(
        settings.lotw_max_age_days,
        std::sync::atomic::Ordering::Relaxed,
    );
    // Integrated rotator daemon follows the settings (spawn/respawn/kill).
    sync_rotctld(&settings);
    // Capture the feed config before `settings` moves into the engine.
    let cluster_enabled = settings.cluster_enabled;
    let cluster_hosts = settings.cluster_hosts.clone();
    let mycall = settings.mycall.clone();
    let mygrid = settings.mygrid.clone();
    let opening_regional = settings.opening_regional;

    let snap = {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        // The LoTW sync cursor is bound to the exact query (notably the username);
        // if the username changed, reset it to a full pull so a config edit can't
        // silently skip confirmations.
        if eng.settings().lotw_username.trim() != settings.lotw_username.trim() {
            settings.lotw_last_qsl.clear();
        }
        // Same for eQSL — its cursor is account-bound (see download_eqsl_report).
        if eng.settings().eqsl_username.trim() != settings.eqsl_username.trim() {
            settings.eqsl_last_sync.clear();
        }
        // A ClubLog credential change re-arms auto-push (clears the 403 suspend).
        let cur = eng.settings();
        if cur.clublog_email != settings.clublog_email
            || cur.clublog_callsign != settings.clublog_callsign
            || cur.clublog_api_key != settings.clublog_api_key
        {
            CLUBLOG_SUSPENDED.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        // Keep the live DXpedition layer's most-wanted key current (Settings
        // override, else the build's baked application key).
        propagation::live::dxped::set_clublog_key(&effective_clublog_key(
            &settings.clublog_api_key,
        ));
        // Apply FIRST, then persist the engine's AUTHORITATIVE merged state — apply_settings keeps the
        // LIVE dual-radio roster / active radio / peg / tune (discarding the form's possibly-stale
        // copies), so saving the raw form here would write a roster that diverges from the engine and
        // revert the active radio on the next launch. Persist eng.settings() post-merge, like every
        // light verb does.
        eng.apply_settings(settings);
        if let Err(e) = eng.settings().save(&settings_path()) {
            eprintln!("tempo: failed to persist settings: {e}");
        }
        eng.snapshot()
    }; // release the engine lock before spawning feed threads

    // The live feeds (cluster telnet login, PSKR MQTT topic filters) are BOUND to
    // the callsign — a changed call tears them down, clears old-call buffers, and
    // restarts them under the new call (background drain; ~3 s blackout). The
    // decision is made under ONE lock (no TOCTOU between rapid saves), the drain
    // is single-flight (a second change during a drain doesn't spawn a second
    // drain — the in-flight one re-reads the LATEST settings at its end), and an
    // emptied callsign also tears down (the restart then no-ops via is_real_call).
    let call_changed = {
        let mut prev = PREV_FEED_CALL.lock().unwrap_or_else(|e| e.into_inner());
        let changed = !prev.is_empty() && prev.to_uppercase() != mycall.trim().to_uppercase();
        *prev = mycall.trim().to_string();
        changed
    };
    if call_changed {
        // The propagation cache + refetch back-off are keyed to the OLD identity.
        // Drop both so the new callsign refetches live immediately — no previous
        // identity's openings served from a warm cache, and no back-off delay
        // carried over (otherwise a new call could wait up to PROP_TTL_SECS).
        if let Ok(mut g) = cache.lock() {
            *g = None;
        }
        if let Ok(mut g) = PROP_FETCH_BACKOFF.lock() {
            *g = None;
        }
        if !FEED_RESTART_IN_FLIGHT.swap(true, std::sync::atomic::Ordering::SeqCst) {
            restart_live_feeds(
                state.inner().clone(),
                spots.inner().clone(),
                live_paths.inner().clone(),
                region_paths.0.clone(),
                health.inner().clone(),
            );
        }
        // In-flight drain re-reads current settings at its end — nothing to do.
        return Ok(snap);
    }

    if cluster_enabled {
        start_cluster_feeds(spots.inner(), &cluster_hosts, &mycall, health.inner());
    }
    start_pskr_feed(live_paths.inner(), &mycall, health.inner());
    start_wspr_feed(live_paths.inner(), &mycall);
    if opening_regional {
        start_pskr_region_feed(region_paths.inner(), &mycall, &mygrid);
    }
    Ok(snap)
}

/// Tear down the callsign-bound live feeds (cluster telnet + PSKR MQTT + region)
/// and clear their buffers, so `start_*` calls (which the caller issues next, via
/// the normal set_settings tail or app flow) reconnect under the NEW callsign.
/// Runs the slow drain on a background thread: the stop flags are polled by the
/// feed loops within ≤2 s; the start latches reset after the drain so the
/// freshly-started threads can't race the dying ones for the latch.
/// The callsign the live feeds were last started under (detects renames).
static PREV_FEED_CALL: Mutex<String> = Mutex::new(String::new());
/// Single-flight latch for the feed drain/restart — rapid successive callsign
/// changes must not spawn competing drain threads (they'd race the latches).
static FEED_RESTART_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn restart_live_feeds(
    engine: SharedEngine,
    spots: SharedSpots,
    live_paths: SharedLivePaths,
    region_paths: Arc<Mutex<propagation::LiveSpots>>,
    health: SharedHealth,
) {
    use std::sync::atomic::Ordering::SeqCst;
    conn_log(
        "Feeds",
        "info",
        "callsign changed — restarting cluster + PSK Reporter + WSPR feeds under the new call",
    );
    CLUSTER_STOP.store(true, SeqCst);
    PSKR_STOP.store(true, SeqCst);
    std::thread::spawn(move || {
        // Both feed loops observe their stop flags within the socket read timeout;
        // the +1 covers scheduling. The constant makes the coupling explicit.
        std::thread::sleep(std::time::Duration::from_secs(
            tempo_net::FEED_STOP_OBSERVE_SECS + 1,
        ));
        // Old-call data must not linger on the boards/map.
        if let Ok(mut b) = spots.lock() {
            *b = tempo_net::cluster::SpotBuffer::default();
        }
        if let Ok(mut b) = live_paths.lock() {
            *b = propagation::LiveSpots::default();
        }
        if let Ok(mut b) = region_paths.lock() {
            *b = propagation::LiveSpots::new(propagation::REGION_SPOT_CAP);
        }
        health.cluster_last.store(0, SeqCst);
        health.pskr_last_event.store(0, SeqCst);
        health.cluster_connected.store(false, SeqCst);
        health.pskr_connected.store(false, SeqCst);
        health.phone_cluster_last.store(0, SeqCst);
        health.phone_spots_seen.store(0, SeqCst);
        // Re-arm: clear stops + release the once-latches, then start fresh threads
        // from the LATEST persisted settings (NOT spawn-time captures — a second
        // save during the drain must win). An emptied/invalid callsign simply
        // no-ops inside start_* (is_real_call gate) → feeds stay down, correctly.
        CLUSTER_STOP.store(false, SeqCst);
        PSKR_STOP.store(false, SeqCst);
        // ALL cluster latches must re-arm — CLUSTER_STOP halts the RBN threads too, so
        // leaving their latches set would strand RBN (CW/digital) down after a rename.
        RBN_CW_STARTED.store(false, SeqCst);
        RBN_DIGITAL_STARTED.store(false, SeqCst);
        if let Ok(mut v) = HUMAN_NODES_STARTED.lock() {
            v.clear();
        }
        if let Ok(mut v) = PHONE_NODE_CONNS.lock() {
            v.clear();
        }
        PSKR_STARTED.store(false, SeqCst);
        PSKR_REGION_STARTED.store(false, SeqCst);
        let (cluster_enabled, cluster_hosts, mycall, mygrid, opening_regional) = match engine.lock()
        {
            Ok(eng) => {
                let st = eng.settings();
                (
                    st.cluster_enabled,
                    st.cluster_hosts.clone(),
                    st.mycall.clone(),
                    st.mygrid.clone(),
                    st.opening_regional,
                )
            }
            Err(_) => return,
        };
        if cluster_enabled {
            start_cluster_feeds(&spots, &cluster_hosts, &mycall, &health);
        }
        start_pskr_feed(&live_paths, &mycall, &health);
        start_wspr_feed(&live_paths, &mycall);
        if opening_regional {
            start_pskr_region_feed(&SharedRegionPaths(region_paths), &mycall, &mygrid);
        }
        FEED_RESTART_IN_FLIGHT.store(false, SeqCst);
    });
}

/// Export the Field Day log as text in `format` ("cabrillo" | "adif"). Errors if
/// not in Field Day mode.
#[tauri::command]
fn export_log(state: State<'_, SharedEngine>, format: String) -> Result<String, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    eng.export_log(&format)
        .ok_or_else(|| "nothing to export (enter Field Day mode first)".to_string())
}

/// Export the **general** logbook (all Chat/QSO contacts, any mode) as
/// `format` ("adif" | "csv"). Independent of the Field Day contest log.
#[tauri::command]
fn export_general_log(state: State<'_, SharedEngine>, format: String) -> Result<String, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.export_logbook(&format))
}

/// Write export text to a file in the operator's Downloads folder and return the FULL saved path.
/// The Logbook "Export ADIF/CSV" buttons use this instead of a webview `<a download>` blob — that
/// browser trick is unreliable in a WebView2 window (a synchronous URL revoke can abort the write,
/// and a silent save to an unknown folder gives the operator no confidence it worked). A real Rust
/// write to a known path, echoed back as a toast, is dependable and visible. `filename` is reduced
/// to its bare name so it can never escape Downloads.
#[tauri::command]
fn save_text_to_downloads(
    app: tauri::AppHandle,
    filename: String,
    text: String,
) -> Result<String, String> {
    use tauri::Manager;
    let name = std::path::Path::new(&filename)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .ok_or("Invalid file name")?;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| format!("Could not locate your Downloads folder: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// Arm or disarm the native CI-V bus diagnostic log. When enabled, every byte the CI-V
/// engine reads/writes is recorded (timestamped + decoded) to a file in the operator's
/// Downloads folder; returns that path so the UI can show it. Off by default and not
/// persisted — a support tool to capture the real bus traffic during a hardware-only fault.
#[tauri::command]
fn civ_diagnostic_log(app: tauri::AppHandle, enable: bool) -> Result<String, String> {
    use tauri::Manager;
    if !enable {
        tempo_audio::civ::diag::stop();
        return Ok(String::new());
    }
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| format!("Could not locate your Downloads folder: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("nexus-civ-diagnostic.log");
    tempo_audio::civ::diag::start(&path).map_err(|e| format!("Could not start the log: {e}"))?;
    Ok(path.display().to_string())
}

/// The active CI-V diagnostic-log path, or an empty string when logging is off. The UI
/// queries this on load so the toggle reflects the real backend state — logging keeps
/// running while you leave Settings to transmit, so a local-only toggle would wrongly read
/// "off" (and re-arming would truncate the capture).
#[tauri::command]
fn civ_diagnostic_status() -> String {
    tempo_audio::civ::diag::status().unwrap_or_default()
}

/// Transmit an open broadcast (FT8-style "to all") free-text message.
#[tauri::command]
fn broadcast(state: State<'_, SharedEngine>, text: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.broadcast(&text);
    Ok(eng.snapshot())
}

/// Serial ports available for CAT / serial PTT (for the Settings dropdown).
#[tauri::command]
async fn get_serial_ports() -> Vec<String> {
    #[cfg(feature = "radio")]
    {
        tempo_audio::ports::available_ports()
    }
    #[cfg(not(feature = "radio"))]
    {
        Vec::new()
    }
}

/// A serial port plus a human label (its USB product string). Lets the picker tell
/// otherwise-identical ports apart — e.g. a Xiegu X6100 exposes TWO CH342 interfaces
/// ("USB-Enhanced-SERIAL-A" and "-B"; CAT answers on B only), which look like bare
/// "COMx" without the product. Non-USB ports get an empty label.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SerialPortInfo {
    name: String,
    label: String,
}

/// Serial ports WITH a descriptive USB-product label, for the Settings picker.
#[tauri::command]
async fn get_serial_ports_detailed() -> Vec<SerialPortInfo> {
    #[cfg(feature = "radio")]
    {
        let usb = tempo_audio::ports::available_usb_ports();
        tempo_audio::ports::available_ports()
            .into_iter()
            .map(|name| {
                let label = usb
                    .iter()
                    .find(|u| u.port_name == name)
                    .map(|u| u.product.clone())
                    .unwrap_or_default();
                SerialPortInfo { name, label }
            })
            .collect()
    }
    #[cfg(not(feature = "radio"))]
    {
        Vec::new()
    }
}

/// Available sound-card input/output device names (for the Settings dropdowns).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioDevices {
    input: Vec<String>,
    output: Vec<String>,
}

/// Enumerate sound-card devices for the Settings audio-device pickers. Empty
/// lists when built without the `radio` feature (mirrors `get_serial_ports`).
#[tauri::command]
async fn get_audio_devices() -> AudioDevices {
    #[cfg(feature = "radio")]
    {
        let (input, output) = tempo_audio::device::available_devices();
        AudioDevices { input, output }
    }
    #[cfg(not(feature = "radio"))]
    {
        AudioDevices {
            input: Vec::new(),
            output: Vec::new(),
        }
    }
}

/// One auto-detected USB radio, for the zero-config setup picker.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DetectedRigDto {
    port_name: String,
    vid: u16,
    pid: u16,
    product: String,
    manufacturer: String,
    /// Hamlib model + name guessed from the USB product string (null = bridge chip
    /// identified but not the specific rig — the operator picks the model).
    suggested_model: Option<u32>,
    suggested_model_name: Option<String>,
    /// Human bridge-chip name (e.g. "Silicon Labs CP210x"), or "USB (native)".
    chip: String,
    /// Driver guidance when one is needed on this OS (null = native/bundled).
    driver_note: Option<String>,
    driver_url: Option<String>,
    driver_bundled: bool,
    /// Best-guess paired sound device (the rig's USB-Audio CODEC).
    suggested_audio: Option<String>,
}

/// Zero-config station setup: enumerate connected USB radios and resolve each to a
/// suggested Hamlib model (from the USB product string), bridge-chip + OS-aware
/// driver guidance, and a paired sound device. Empty without the `radio` feature.
/// The operator one-click-applies a result (fills rig model + port + audio).
#[tauri::command]
async fn detect_rigs() -> Vec<DetectedRigDto> {
    #[cfg(feature = "radio")]
    {
        use tempo_audio::usbrig::UsbSerialChip;
        let ports = tempo_audio::ports::available_usb_ports();
        let (audio_in, _out) = tempo_audio::device::available_devices();
        let os = tempo_audio::usbrig::current_os();
        tempo_audio::usbrig::detect_rigs(&ports, &audio_in, os)
            .into_iter()
            .map(|r| {
                let chip = match (&r.driver, r.chip) {
                    (Some(d), _) => d.chip.to_string(),
                    (None, UsbSerialChip::Other) => "USB (native)".to_string(),
                    (None, c) => format!("{c:?}"),
                };
                DetectedRigDto {
                    port_name: r.port_name,
                    vid: r.vid,
                    pid: r.pid,
                    product: r.product,
                    manufacturer: r.manufacturer,
                    suggested_model: r.suggested_model,
                    suggested_model_name: r.suggested_model_name.map(|s| s.to_string()),
                    chip,
                    driver_note: r.driver.as_ref().map(|d| d.note.to_string()),
                    driver_url: r
                        .driver
                        .as_ref()
                        .filter(|d| !d.url.is_empty())
                        .map(|d| d.url.to_string()),
                    driver_bundled: r.driver.as_ref().is_some_and(|d| d.bundled),
                    suggested_audio: r.suggested_audio,
                }
            })
            .collect()
    }
    #[cfg(not(feature = "radio"))]
    {
        Vec::new()
    }
}

/// Result of "Auto-test ports": the working (port, baud, Hamlib model) the prober
/// auto-selected, plus a human-readable detail line. The UI applies the fields to the
/// CAT settings and saves (the normal apply path), so this command stays side-effect-free.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CatProbeResult {
    found: bool,
    port_name: String,
    baud: u32,
    model: u32,
    model_name: String,
    freq_mhz: f64,
    detail: String,
    /// The model was a GUESS (a common-rig seed, tried because none was configured). The UI should
    /// apply port + baud but keep the operator picking the exact Rig Model.
    model_seeded: bool,
}

/// Auto-test which serial port actually drives the rig: probe each USB port (read-only,
/// never TX) and return the first that reads back a plausible dial frequency. The
/// fallback Hamlib model is the operator's configured rig (for ports whose USB descriptor
/// doesn't name a model). Run it when CAT isn't already connected (the setup wizard, or a
/// not-working CAT) — a live daemon holding the real port blocks that one port's probe.
#[tauri::command]
async fn probe_cat_ports(state: State<'_, SharedEngine>) -> Result<CatProbeResult, String> {
    #[cfg(feature = "radio")]
    {
        // Read the configured model, then release the lock for the seconds-long probe so
        // the UI's snapshot polling never blocks on it.
        let model = {
            let eng = state.lock().map_err(|e| e.to_string())?;
            eng.settings().rig_model
        };
        // 4599: a private TCP port for the throwaway rigctld, distinct from the live one.
        let hit = tauri::async_runtime::spawn_blocking(move || {
            tempo_audio::port_prober::probe_cat_ports(model, 4599)
        })
        .await
        .map_err(|e| e.to_string())?;
        Ok(match hit {
            Some(h) => {
                let mhz = h.freq_hz as f64 / 1.0e6;
                let detail = if h.model_seeded {
                    format!(
                        "Found the port: {} @ {} baud — reads {:.3} MHz. Now pick your exact Rig \
                         Model below ({} answered, but the model is a guess).",
                        h.port_name, h.baud, mhz, h.model_name
                    )
                } else {
                    format!(
                        "{} on {} @ {} baud — reads {:.3} MHz",
                        h.model_name, h.port_name, h.baud, mhz
                    )
                };
                CatProbeResult {
                    found: true,
                    detail,
                    port_name: h.port_name,
                    baud: h.baud,
                    model: h.model,
                    model_name: h.model_name,
                    freq_mhz: mhz,
                    model_seeded: h.model_seeded,
                }
            }
            None => CatProbeResult {
                found: false,
                port_name: String::new(),
                baud: 0,
                model: 0,
                model_name: String::new(),
                freq_mhz: 0.0,
                detail: "No rig answered on any USB port. Check the cable and that the rig is on \
                         (and not already connected elsewhere), then retry."
                    .to_string(),
                model_seeded: false,
            },
        })
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = state;
        Err("radio support is not built into this binary".to_string())
    }
}

/// The spawned rotctld daemon (integrated rotator) + the params it was
/// spawned with, so a settings change respawns only when something changed.
static ROTCTLD: Mutex<Option<(tempo_audio::rigctld_proc::RigctldProc, (u32, String, u32))>> =
    Mutex::new(None);
/// The integrated daemon's local port (rotctld's upstream default; the rig's
/// rigctld owns 4532, so there is no collision).
const ROTCTLD_PORT: u16 = 4533;

/// The address every rotator command talks to: the ADVANCED external override
/// when set, else the integrated daemon (when a model is configured), else
/// None — no rotator.
fn effective_rotator_addr(st: &tempo_app::settings::Settings) -> Option<String> {
    let host = st.rotator_host.trim();
    if !host.is_empty() {
        return Some(host.to_string());
    }
    (st.rotator_model > 0).then(|| format!("127.0.0.1:{ROTCTLD_PORT}"))
}

/// Reconcile the integrated rotctld daemon with settings: spawn it when a
/// model is configured (and no external override), respawn on param changes,
/// kill it when unconfigured. Errors surface on the connection log — a dead
/// rotctld must be visible, not silent.
fn sync_rotctld(st: &tempo_app::settings::Settings) {
    let want = st.rotator_host.trim().is_empty() && st.rotator_model > 0;
    let params = (
        st.rotator_model,
        st.rotator_port.trim().to_string(),
        st.rotator_baud,
    );
    let Ok(mut g) = ROTCTLD.lock() else { return };
    match (&mut *g, want) {
        (Some((_, p)), true) if *p == params => {} // running with the right params
        (slot, true) => {
            *slot = None; // kill-on-drop reaps a stale daemon first
            match tempo_audio::rigctld_proc::spawn_rotctld(
                params.0,
                &params.1,
                params.2,
                ROTCTLD_PORT,
            ) {
                Ok(proc) => {
                    conn_log(
                        "Rotator",
                        "info",
                        format!(
                            "rotctld launched (model {} on {} @ {}, :{ROTCTLD_PORT})",
                            params.0,
                            if params.1.is_empty() { "-" } else { &params.1 },
                            params.2
                        ),
                    );
                    *slot = Some((proc, params));
                }
                Err(e) => {
                    conn_log("Rotator", "error", format!("rotctld launch failed: {e}"));
                }
            }
        }
        (slot @ Some(_), false) => {
            conn_log("Rotator", "info", "rotctld stopped (rotator unconfigured)");
            *slot = None;
        }
        (None, false) => {}
    }
}

/// Point the antenna rotator at an absolute azimuth (degrees) via rotctld.
#[tauri::command]
async fn point_rotator(state: State<'_, SharedEngine>, az_deg: f64) -> Result<(), String> {
    #[cfg(feature = "radio")]
    {
        let host = {
            let eng = state.lock().map_err(|e| e.to_string())?;
            effective_rotator_addr(eng.settings())
        };
        let Some(host) = host else {
            return Err(
                "Set up your rotator in Settings (pick a model + port; Nexus runs rotctld for you)."
                    .to_string(),
            );
        };
        tauri::async_runtime::spawn_blocking(move || tempo_audio::rotator::point(&host, az_deg))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = (state, az_deg);
        Err("radio support is not built into this binary".to_string())
    }
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FlexRadioDto {
    model: String,
    nickname: String,
    ip: String,
}

/// Listen ~3 s for FlexRadio LAN discovery broadcasts (UDP 4992) — the
/// Settings "Find my Flex" button. Empty = nothing announced. Read-only.
#[tauri::command]
async fn discover_flex() -> Result<Vec<FlexRadioDto>, String> {
    tauri::async_runtime::spawn_blocking(|| tempo_net::flexdisc::discover(3))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
        .map(|v| {
            v.into_iter()
                .map(|r| FlexRadioDto {
                    model: r.model,
                    nickname: r.nickname,
                    ip: r.ip,
                })
                .collect()
        })
}

/// Stop the rotator immediately (rotctld `S`) — the control panel's STOP.
#[tauri::command]
async fn stop_rotator(state: State<'_, SharedEngine>) -> Result<(), String> {
    #[cfg(feature = "radio")]
    {
        let host = {
            let eng = state.lock().map_err(|e| e.to_string())?;
            effective_rotator_addr(eng.settings())
        };
        let Some(host) = host else {
            return Err(
                "Set up your rotator in Settings (pick a model + port; Nexus runs rotctld for you)."
                    .to_string(),
            );
        };
        tauri::async_runtime::spawn_blocking(move || tempo_audio::rotator::stop(&host))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = state;
        Err("radio support is not built into this binary".to_string())
    }
}

/// Point the rotator at a callsign's DXCC entity — the great-circle bearing from your
/// grid. Returns the bearing pointed to (degrees) for UI feedback.
#[tauri::command]
async fn point_rotator_at_call(
    state: State<'_, SharedEngine>,
    call: String,
) -> Result<f64, String> {
    #[cfg(feature = "radio")]
    {
        let (host, mygrid) = {
            let eng = state.lock().map_err(|e| e.to_string())?;
            (
                effective_rotator_addr(eng.settings()),
                eng.settings().mygrid.clone(),
            )
        };
        let Some(host) = host else {
            return Err(
                "Set up your rotator in Settings (pick a model + port; Nexus runs rotctld for you)."
                    .to_string(),
            );
        };
        let me = propagation::geo::maidenhead_to_latlon(mygrid.trim())
            .ok_or("Set your grid square in Settings so a bearing can be computed.")?;
        let info = propagation::dxcc::resolve(&call)
            .ok_or_else(|| format!("Couldn't locate {call} (unknown callsign)."))?;
        let bearing = propagation::geo::bearing_deg(me, (info.lat, info.lon));
        tauri::async_runtime::spawn_blocking(move || tempo_audio::rotator::point(&host, bearing))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok(bearing)
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = (state, call);
        Err("radio support is not built into this binary".to_string())
    }
}

/// Current rotator azimuth (degrees), or `None` if rotctld is unset / unreachable.
#[tauri::command]
async fn read_rotator(state: State<'_, SharedEngine>) -> Result<Option<f64>, String> {
    #[cfg(feature = "radio")]
    {
        let host = {
            let eng = state.lock().map_err(|e| e.to_string())?;
            effective_rotator_addr(eng.settings())
        };
        let Some(host) = host else {
            return Ok(None); // no rotator configured — the pane shows its hint
        };
        Ok(
            tauri::async_runtime::spawn_blocking(move || tempo_audio::rotator::read_azimuth(&host))
                .await
                .map_err(|e| e.to_string())?,
        )
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = state;
        Ok(None)
    }
}

/// A worked-station callsign candidate parsed from the CW decode (the "copilot" chips).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CwCandidateDto {
    call: String,
    best: bool,
}

/// A single-signal CW decode of the recent receive audio (text + estimated WPM) plus the
/// "CW copilot" analysis: worked-call candidates, the read exchange, and guided next-step.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CwDecodeResult {
    text: String,
    wpm: u32,
    /// TX echo: recent EXPANDED CW transmissions (oldest→newest) so the cockpit shows
    /// exactly what was sent (macro tokens resolved).
    sent: Vec<String>,
    /// A CW-keyer failure to surface (e.g. the rig rejected CAT `send_morse`), else null.
    keyer_error: Option<String>,
    /// Ranked worked-station callsign candidates from the decode (click to confirm).
    candidates: Vec<CwCandidateDto>,
    /// The RST they sent us, read from the decode (e.g. "599"), else null.
    rst: Option<String>,
    /// The other station's name, read from the decode (e.g. "BOB"), else null.
    name: Option<String>,
    /// Guided QSO-state tag: "listening" | "cq" | "answered" | "report" | "73".
    state: String,
    /// Plain-English state, e.g. "W1ABC is calling CQ".
    headline: String,
    /// The guided instruction, e.g. "Press Answer (F2) to call them".
    prompt: String,
    /// Recommended action id to highlight: "F2" | "F3" | "log", or null.
    recommended: Option<String>,
    /// The operator-confirmed worked callsign (the active peer), if any.
    worked_call: Option<String>,
}

/// Decode CW from the recent RX audio at the operator's pitch — a live readout for the
/// CW cockpit. Empty text unless there's a clear keyed signal under the marker.
#[tauri::command]
fn cw_decode(state: State<'_, SharedEngine>, sensitivity: f32) -> Result<CwDecodeResult, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_cw_sensitivity(sensitivity); // operator slider; scales the decode gates
    let d = eng.cw_decode();
    let sent = eng.cw_sent();
    let worked = eng.active_peer();
    let mycall = eng.settings().mycall.clone();
    // Parse the decode into copilot context. The DXCC resolver (src-tauri has it; tempo-core
    // doesn't) supplies the real-prefix check that filters CW misdecodes out of the chips.
    let assist = tempo_core::cw_parse::analyze(&d.text, &sent, &mycall, worked.as_deref(), |b| {
        propagation::dxcc::resolve(b).is_some()
    });
    Ok(CwDecodeResult {
        text: d.text,
        wpm: d.wpm,
        sent,
        keyer_error: eng.cw_keyer_error(),
        candidates: assist
            .candidates
            .into_iter()
            .map(|c| CwCandidateDto {
                call: c.call,
                best: c.best,
            })
            .collect(),
        rst: assist.exchange.rst,
        name: assist.exchange.name,
        state: assist.guidance.state,
        headline: assist.guidance.headline,
        prompt: assist.guidance.prompt,
        recommended: assist.guidance.recommended,
        worked_call: worked,
    })
}

/// Toggle the AI CW decoder (beta) — persisted; the decode thread + audio ring follow it.
#[tauri::command]
fn set_ai_cw(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_ai_cw_enabled(on);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist ai-cw toggle: {e}");
    }
    Ok(eng.snapshot())
}

/// Clear the streaming CW decoder's accumulated transcript (the cockpit's Clear button).
#[tauri::command]
fn cw_clear(state: State<'_, SharedEngine>) -> Result<(), String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.cw_clear();
    Ok(())
}

/// Expand a CW macro to the exact text it will send, WITHOUT sending — the reply preview.
#[tauri::command]
fn preview_cw(state: State<'_, SharedEngine>, text: String) -> Result<String, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.preview_cw(&text))
}

/// One signal found by the wideband CW skimmer (audio pitch + decoded text + WPM).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SkimHitDto {
    pitch_hz: u32,
    text: String,
    wpm: u32,
}

/// Wideband CW skim of the recent RX audio — every distinct keyed signal across the CW
/// passband (the multi-signal sibling of `cw_decode`).
#[tauri::command]
fn cw_skim(state: State<'_, SharedEngine>) -> Result<Vec<SkimHitDto>, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng
        .cw_skim()
        .into_iter()
        .map(|h| SkimHitDto {
            pitch_hz: h.pitch_hz,
            text: h.text,
            wpm: h.wpm,
        })
        .collect())
}

/// Live RTTY state for the cockpit poll: RX (armed + AFC + the decoded-character
/// ring tail with per-char confidence) and TX (configured baud/shift/backend, the
/// live sending flag, and any keyer failure to surface).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RttyStateDto {
    armed: bool,
    /// AFC offset from the nominal mark/space pair (Hz).
    afc_hz: f32,
    /// AFC has acquired-then-frozen on a signal.
    afc_locked: bool,
    /// Current mark/space audio tones (Hz) the demod is netted on — the waterfall
    /// mark/space cursor positions. Un-netted = the nominal 2125/2295 pair.
    mark_hz: f32,
    space_hz: f32,
    /// The decoded-text ring tail (caps at ~4000 chars; oldest drop off).
    text: String,
    /// Per-character confidence 0–100, parallel to `text`'s chars — render low
    /// values faint (the ATC soft metric).
    char_conf: Vec<u8>,
    /// Configured baud rate (true 45.45 by default — never 45).
    baud: f64,
    /// Configured mark/space shift (Hz).
    shift_hz: u32,
    /// Keying backend: "afsk" (soundcard tones, rig in LSB) | "fsk" (serial
    /// keyline, rig in RTTY mode).
    backend: String,
    /// An RTTY over is on the air or queued behind one (the TX indicator).
    sending: bool,
    /// A keyer failure to surface (FSK port wouldn't open / rig refused PTT), else null.
    keyer_error: Option<String>,
    /// The RTTY auto-sequencer is active (the operator's Auto toggle is on).
    auto: bool,
    /// Live sequencer state: idle | calling_cq | answering | exchange_sent |
    /// confirmed | done.
    seq_state: String,
    /// The station currently being worked, once one is locked, else null.
    peer: Option<String>,
    /// The peer's copied exchange in schema order, as `[key, value]` pairs.
    peer_exchange: Vec<(String, String)>,
    /// A CQ surfaced from the transcript for the operator to click-to-answer (only
    /// while Auto is on), else null. Surfacing only — clicking it is the human gate.
    heard_cq: Option<String>,
}

fn rtty_state_dto(eng: &Engine) -> RttyStateDto {
    let s = eng.rtty_state();
    RttyStateDto {
        armed: s.armed,
        afc_hz: s.afc_hz,
        afc_locked: s.afc_locked,
        mark_hz: s.mark_hz,
        space_hz: s.space_hz,
        text: s.text,
        char_conf: s.conf,
        baud: s.baud,
        shift_hz: s.shift_hz,
        backend: s.backend,
        sending: s.sending,
        keyer_error: s.keyer_error,
        auto: s.auto,
        seq_state: s.seq_state,
        peer: s.peer,
        peer_exchange: s.peer_exchange,
        heard_cq: s.heard_cq,
    }
}

/// Arm/disarm the RTTY RX decoder (session-only runtime state — never persisted,
/// so the app never launches armed). Returns the fresh state.
#[tauri::command]
fn rtty_arm(state: State<'_, SharedEngine>, on: bool) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_rtty_armed(on);
    Ok(rtty_state_dto(&eng))
}

/// The live RTTY state (poll while the RTTY cockpit is visible).
#[tauri::command]
fn get_rtty_state(state: State<'_, SharedEngine>) -> Result<RttyStateDto, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(rtty_state_dto(&eng))
}

/// Queue RTTY text to transmit — an explicit operator send, the ONLY way RTTY TX
/// starts. The engine validates every gate up front (TX armed, license privileges
/// at the current dial, the RTTY section owning the rig, no tune carrier, no other
/// transmission) and returns WHY a send was refused; the radio loop keys the
/// queued text via the configured backend (soundcard AFSK / true-FSK keyline).
#[tauri::command]
fn rtty_send(state: State<'_, SharedEngine>, text: String) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_send_text(&text)?;
    Ok(rtty_state_dto(&eng))
}

/// Stop RTTY now: abort the over in progress, drop everything queued, and unkey.
#[tauri::command]
fn rtty_stop(state: State<'_, SharedEngine>) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_stop();
    Ok(rtty_state_dto(&eng))
}

/// Clear the decoded-RTTY transcript (the cockpit's Clear button). RX display only.
#[tauri::command]
fn rtty_clear(state: State<'_, SharedEngine>) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_clear();
    Ok(rtty_state_dto(&eng))
}

/// Drop + rebuild the RTTY RX demodulator (fresh AFC acquire) — the recovery for
/// an AFC frozen on the wrong neighbor. RX only; never touches TX.
#[tauri::command]
fn rtty_afc_reset(state: State<'_, SharedEngine>) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_rtty_afc_reset();
    Ok(rtty_state_dto(&eng))
}

/// Net the RTTY decoder onto a new audio center (Hz) from a waterfall click —
/// rebuilds the demod around the new mark/space pair. RX-only decoder state, so
/// it needs no TX/privilege gate and is safe during a transmission.
#[tauri::command]
fn rtty_net(state: State<'_, SharedEngine>, hz: f32) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_net(hz);
    Ok(rtty_state_dto(&eng))
}

/// Turn the RTTY auto-sequencer on/off. On builds the sequencer from the operator's
/// identity + active exchange (Field Day class/section vs casual RST/name/QTH); off
/// aborts any live session and stops TX. NEVER transmits — a session only ever
/// starts from an explicit CQ/Answer (the human-initiate gate).
#[tauri::command]
fn rtty_set_auto(state: State<'_, SharedEngine>, on: bool) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_rtty_auto(on);
    Ok(rtty_state_dto(&eng))
}

/// Operator starts an auto CQ run (a human-initiate gate). The engine re-checks every
/// TX gate and returns WHY a start was refused (Auto off, TX locked, …).
#[tauri::command]
fn rtty_auto_cq(state: State<'_, SharedEngine>) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_auto_cq()?;
    Ok(rtty_state_dto(&eng))
}

/// Operator answers a surfaced CQ — search & pounce (a human-initiate gate). Same
/// gate re-check + reason on refusal.
#[tauri::command]
fn rtty_auto_answer(state: State<'_, SharedEngine>, call: String) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_auto_answer(&call)?;
    Ok(rtty_state_dto(&eng))
}

/// Operator kills the live auto session: abort the sequencer, drop the queue, unkey.
#[tauri::command]
fn rtty_auto_abort(state: State<'_, SharedEngine>) -> Result<RttyStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rtty_auto_abort();
    Ok(rtty_state_dto(&eng))
}

/// Live SSTV RX state: armed flag, in-flight decode progress (+ a downscaled
/// raw-RGB preview), and the saved-image gallery. RX decode only — no TX path.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SstvStateDto {
    armed: bool,
    /// Mode label while an image is in flight (e.g. "Scottie 1"), else null.
    mode: Option<String>,
    lines_done: u32,
    lines_total: u32,
    /// Base64 of `preview_width × preview_height × 3` raw RGB bytes (the
    /// in-progress thumbnail, ≤160 px wide), else null.
    preview_rgb_base64: Option<String>,
    preview_width: u32,
    preview_height: u32,
    /// Saved images, oldest first (persisted in the sstv-gallery folder).
    gallery: Vec<tempo_app::dto::SstvGalleryEntry>,
    // ----- TX side (an operator-initiated image transmission) -----
    /// An image is queued or streaming to the rig (the cockpit's TX indicator).
    sending: bool,
    /// Mode label of the image being (or last) transmitted, e.g. "Scottie 1", else null.
    tx_mode: Option<String>,
    /// TX progress 0.0–1.0 (elapsed / total key-down), 0 when idle.
    tx_progress: f32,
    /// Seconds of key-down elapsed / total for the in-flight image.
    tx_elapsed_secs: f32,
    tx_total_secs: f32,
}

fn sstv_state_dto(eng: &Engine) -> SstvStateDto {
    let p = eng.sstv_progress();
    let (tx_elapsed_secs, tx_total_secs, tx_progress) = match eng.sstv_tx_progress() {
        Some((played_ms, total_ms)) if total_ms > 0.0 => (
            (played_ms / 1000.0) as f32,
            (total_ms / 1000.0) as f32,
            (played_ms / total_ms).clamp(0.0, 1.0) as f32,
        ),
        _ => (0.0, 0.0, 0.0),
    };
    SstvStateDto {
        armed: eng.sstv_armed(),
        mode: p.map(|p| p.mode.clone()),
        lines_done: p.map_or(0, |p| p.lines_done),
        lines_total: p.map_or(0, |p| p.lines_total),
        preview_rgb_base64: p
            .filter(|p| !p.preview_rgb.is_empty())
            .map(|p| b64_encode(&p.preview_rgb)),
        preview_width: p.map_or(0, |p| p.preview_w),
        preview_height: p.map_or(0, |p| p.preview_h),
        gallery: eng.sstv_gallery().to_vec(),
        sending: eng.sstv_sending(),
        tx_mode: eng.sstv_tx_mode().map(str::to_string),
        tx_progress,
        tx_elapsed_secs,
        tx_total_secs,
    }
}

/// Arm/disarm the SSTV RX decoder (session-only, like `rtty_arm`). Returns the
/// fresh state.
#[tauri::command]
fn sstv_arm(state: State<'_, SharedEngine>, on: bool) -> Result<SstvStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_sstv_armed(on);
    Ok(sstv_state_dto(&eng))
}

/// The live SSTV RX state (poll while the SSTV view is visible).
#[tauri::command]
fn get_sstv_state(state: State<'_, SharedEngine>) -> Result<SstvStateDto, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(sstv_state_dto(&eng))
}

/// Resolve an SSTV mode slug (the stable `short_name`: "pd120", "scottie1",
/// "scottiedx", "martin1", …) to its [`tempo_sstv::SstvMode`]. Case-insensitive.
fn parse_sstv_mode(slug: &str) -> Option<tempo_sstv::SstvMode> {
    use tempo_sstv::SstvMode::{
        Martin1, Martin2, Pd120, Pd160, Pd180, Pd240, Pd290, Pd50, Pd90, Robot24, Robot36,
        Robot72, Scottie1, Scottie2, ScottieDx,
    };
    Some(match slug.trim().to_ascii_lowercase().as_str() {
        "pd50" => Pd50,
        "pd90" => Pd90,
        "pd120" => Pd120,
        "pd160" => Pd160,
        "pd180" => Pd180,
        "pd240" => Pd240,
        "pd290" => Pd290,
        "robot24" => Robot24,
        "robot36" => Robot36,
        "robot72" => Robot72,
        "scottie1" => Scottie1,
        "scottie2" => Scottie2,
        "scottiedx" => ScottieDx,
        "martin1" => Martin1,
        "martin2" => Martin2,
        _ => return None,
    })
}

/// Transmit an SSTV image. The webview decodes/resizes the operator's picture to the
/// mode's EXACT dimensions (cover-crop) and sends raw row-major RGB as base64 — so no
/// image-decode dependency enters the Rust tree. The command resolves the mode,
/// validates the dimensions against the mode's `ModeSpec`, encodes the full over-the-air
/// waveform OFF the engine lock, then hands it to the engine's gated TX path (which keys
/// nothing until the radio loop takes it behind every safety gate). Returns the fresh
/// state (with `sending` true). Refuses — with the reason — on an unknown mode, a
/// dimension mismatch, or any TX gate being down.
#[tauri::command]
fn sstv_send(
    state: State<'_, SharedEngine>,
    mode: String,
    width: u32,
    height: u32,
    rgb_base64: String,
) -> Result<SstvStateDto, String> {
    let sstv_mode = parse_sstv_mode(&mode).ok_or_else(|| format!("Unknown SSTV mode: {mode}"))?;
    let spec = tempo_sstv::for_mode(sstv_mode);
    // Dimensions must match the mode EXACTLY — the webview cover-crops to these; a
    // mismatch means the wrong canvas size, so refuse rather than transmit a garbled frame.
    if width != spec.line_pixels || height != spec.image_lines {
        return Err(format!(
            "Image is {width}×{height} but {} needs {}×{} — resize to the mode's exact size",
            spec.name, spec.line_pixels, spec.image_lines
        ));
    }
    let bytes = b64_decode(&rgb_base64)?;
    let want = spec.line_pixels as usize * spec.image_lines as usize * 3;
    if bytes.len() != want {
        return Err(format!(
            "Image data is {} bytes but {} needs {want} (width × height × 3)",
            bytes.len(),
            spec.name
        ));
    }
    let rgb: Vec<[u8; 3]> = bytes.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    let img = tempo_sstv::SourceImage { width, height, rgb };
    // Pre-flight the TX gate under a SHORT lock so a refused send (wrong frequency, TX
    // off, another over in flight) fails fast BEFORE we spend CPU on the encode.
    {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.sstv_tx_gate()?;
    }
    // Encode the whole 12 kHz waveform OFF the engine lock (tens of ms even for PD290).
    let samples = tempo_sstv::encode_image(sstv_mode, &img, 12_000).map_err(|e| e.to_string())?;
    // Re-take the lock and hand it to the gated engine path (re-runs the full gate + the
    // duration-budget check; nothing keys until the radio loop takes it).
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.sstv_send(samples, spec.name.to_string())?;
    Ok(sstv_state_dto(&eng))
}

/// Stop the SSTV transmission now: abort the image in progress, drop the queued job, and
/// unkey (the radio loop flushes the output ring on its next tick). Mirrors `rtty_stop`.
#[tauri::command]
fn sstv_stop(state: State<'_, SharedEngine>) -> Result<SstvStateDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.sstv_stop();
    Ok(sstv_state_dto(&eng))
}

/// Standard-alphabet base64 DECODE (RFC 4648, with or without padding) — the inverse of
/// [`b64_encode`], for the raw RGB the webview sends on an SSTV transmit. Rejects any
/// non-alphabet byte. One decode-only call site, so no new dependency.
fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        if chunk.len() == 1 {
            return Err("Invalid base64 length".to_string());
        }
        let mut acc = 0u32;
        for &c in chunk {
            let v = val(c).ok_or_else(|| "Invalid base64 character".to_string())?;
            acc = (acc << 6) | v;
        }
        // A 4/3/2-char group carries 3/2/1 output bytes; left-align the accumulator.
        acc <<= 6 * (4 - chunk.len());
        let n = chunk.len() - 1; // 4→3, 3→2, 2→1 output bytes
        for i in 0..n {
            out.push((acc >> (16 - 8 * i)) as u8);
        }
    }
    Ok(out)
}

/// Minimal standard-alphabet base64 (with padding) for the SSTV preview bytes —
/// one encode-only call site doesn't justify a new dependency.
fn b64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Enable/disable normal slot transmit ("Monitor"). `false` mutes transmit and
/// clears anything queued; `true` re-enables it and clears a tripped watchdog.
#[tauri::command]
fn set_tx_enabled(state: State<'_, SharedEngine>, enabled: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tx_enabled(enabled);
    Ok(eng.snapshot())
}

/// Set the TX audio drive level (0.0–1.0) live — the "Pwr" slider. The radio loop
/// applies it to the audio backend on the next slot; persisted so it survives
/// restart. Returns the refreshed snapshot.
#[tauri::command]
fn set_tx_level(state: State<'_, SharedEngine>, level: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tx_level(level);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: set_tx_level save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Set the RX capture gain (≥1.0 multiplier on received audio before decode) — headroom for a
/// quiet interface. Applied live by the audio service; persisted so it survives restart. Returns
/// the refreshed snapshot.
#[tauri::command]
fn set_rx_gain(state: State<'_, SharedEngine>, gain: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_rx_gain(gain);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: set_rx_gain save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Switch the active radio (dual-radio). The light path — mirrors the chosen profile's
/// CAT/audio into the flat fields so the radio loop swaps the rig on the next tick (carrier
/// dropped first), restores that radio's last tune, and never touches Mode/TX-queues (unlike
/// `apply_settings`). Persisted so the active radio survives a restart. Returns the snapshot.
#[tauri::command]
fn set_active_radio(state: State<'_, SharedEngine>, id: u32) -> Result<AppSnapshot, String> {
    let (snap, settings) = {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.set_active_radio(id);
        if let Err(e) = eng.settings().save(&settings_path()) {
            eprintln!("tempo: set_active_radio save failed: {e}");
        }
        (eng.snapshot(), eng.settings().clone())
    }; // drop the engine lock before touching the rotator daemon
       // Each radio carries its own rotator — re-sync the rotctld daemon to the newly-active radio's
       // rotator config (mirrors set_settings). The rig loop swaps CAT/audio on its own via the flat
       // mirror, but the rotator daemon only follows an explicit sync.
    sync_rotctld(&settings);
    Ok(snap)
}

/// Peg-lock the active radio (dual-radio): when on, selecting a band never auto-switches the
/// active radio (P4 routing respects it). Persisted. Returns the refreshed snapshot.
#[tauri::command]
fn set_peg_lock(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_radio_pegged(on);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: set_peg_lock save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Add a radio to the roster (dual-radio). Appends a new profile with distinct daemon ports; does
/// not change the active radio (the operator switches to it to configure its CAT). Returns the
/// snapshot — the switcher then shows the new radio.
#[tauri::command]
fn add_radio(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.add_radio();
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: add_radio save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Remove a radio from the roster (no-op on the active or last radio). Returns the snapshot.
#[tauri::command]
fn remove_radio(state: State<'_, SharedEngine>, id: u32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.remove_radio(id);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: remove_radio save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Rename a radio profile (its switcher label). Returns the snapshot.
#[tauri::command]
fn rename_radio(
    state: State<'_, SharedEngine>,
    id: u32,
    name: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.rename_radio(id, &name);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: rename_radio save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Set a radio's band-coverage set (empty = covers everything) for auto band-routing. Returns the
/// snapshot.
#[tauri::command]
fn set_radio_bands(
    state: State<'_, SharedEngine>,
    id: u32,
    bands: Vec<String>,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_radio_bands(id, bands);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: set_radio_bands save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Hold (`true`) or release (`false`) a steady tune carrier for ATU/amp tuning.
/// While tuning, normal slot TX is suppressed and the radio loop plays a steady
/// f0 sine. Returns the refreshed snapshot.
#[tauri::command]
fn set_tune(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tune(on);
    Ok(eng.snapshot())
}

/// Stop transmitting now: drop any queued frames and clear the TX indicator.
#[tauri::command]
fn halt_tx(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.halt_tx();
    Ok(eng.snapshot())
}

/// Result of a "Test CAT" probe (WSJT-X-style): whether the rig is reachable and
/// a human-readable detail line (the read frequency, or a specific error).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CatTestResult {
    ok: bool,
    detail: String,
}

/// Test the rig/CAT connection now. Asks the radio loop to (re)open + probe the
/// rig using the **current** settings, then reports the result. The UI saves
/// settings first, so the loop reconfigures (launching rigctld for CAT) before
/// this returns. Mirrors WSJT-X's "Test CAT": green + frequency, or a red error.
#[tauri::command]
async fn test_cat(state: State<'_, SharedEngine>) -> Result<CatTestResult, String> {
    #[cfg(feature = "radio")]
    {
        {
            let mut eng = state.lock().map_err(|e| e.to_string())?;
            eng.request_cat_reprobe();
        }
        // The radio loop polls at ~50 Hz; allow time for a rigctld spawn + probe.
        std::thread::sleep(std::time::Duration::from_millis(1300));
        let eng = state.lock().map_err(|e| e.to_string())?;
        let r = eng.snapshot().radio;
        Ok(CatTestResult {
            ok: r.cat_ok.unwrap_or(false),
            detail: if r.cat_detail.is_empty() {
                "No CAT status yet — set your rig + PTT method, Save, then test.".to_string()
            } else {
                r.cat_detail
            },
        })
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = state;
        Ok(CatTestResult {
            ok: false,
            detail: "This build has no radio support (built without the `radio` feature)."
                .to_string(),
        })
    }
}

/// Curated Hamlib rig models `(model_number, name)` for the Settings dropdown.
#[tauri::command]
fn get_rig_models() -> Vec<(u32, String)> {
    #[cfg(feature = "radio")]
    {
        tempo_audio::rigmodels::rig_models()
            .into_iter()
            .map(|(n, s)| (n, s.to_string()))
            .collect()
    }
    #[cfg(not(feature = "radio"))]
    {
        Vec::new()
    }
}

/// Full Hamlib rig catalog (verified + extended) for the Settings "show all
/// models" toggle — so an owner of a supported-but-uncurated rig can still find
/// and select it (or type its model number).
#[tauri::command]
fn get_all_rig_models() -> Vec<(u32, String)> {
    #[cfg(feature = "radio")]
    {
        tempo_audio::rigmodels::all_rig_models()
            .into_iter()
            .map(|(n, s)| (n, s.to_string()))
            .collect()
    }
    #[cfg(not(feature = "radio"))]
    {
        Vec::new()
    }
}

/// Tempo's proposed calling-frequency band plan (HF + VHF/UHF), for the band
/// selector. Each entry is General-legal + clear of the existing watering holes.
#[tauri::command]
fn get_band_plan(
    state: State<'_, SharedEngine>,
) -> Result<Vec<tempo_app::bandplan::BandChannel>, String> {
    // Tier-aware (FT8/FT4 → the standard WSJT-X watering holes; FT1/DX1 →
    // native plan) WITH the operator's Settings ▸ Frequencies overrides applied
    // — the band picker must show the dials the engine will actually QSY to.
    Ok(state.lock().map_err(|e| e.to_string())?.band_plan())
}

/// Set the operator's amateur license class (Technician/General/Extra/Open) — drives the
/// transmit-privilege lockout + the licensed-segment band dropdown. Used by the first-run
/// wizard and Settings.
#[tauri::command]
fn set_license_class(state: State<'_, SharedEngine>, class: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_license_class(&class);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist license class: {e}");
    }
    Ok(eng.snapshot())
}

/// The bands the operator may use in the CURRENT operating mode, each parked at the START of
/// their licensed segment (CW-segment start in CW, phone-segment start in Phone) — the
/// per-cockpit band dropdown. Bands with no privilege for this class+mode are omitted; Open
/// shows the conventional starts. (60 m is omitted — it's channelized; tune it manually.)
#[tauri::command]
fn get_licensed_band_plan(
    state: State<'_, SharedEngine>,
    mode: String,
) -> Result<Vec<tempo_app::bandplan::BandChannel>, String> {
    use tempo_app::bandplan::BandChannel;
    use tempo_app::settings::OperatingMode;
    const BANDS: &[(&str, &str)] = &[
        ("160m", "HF"),
        ("80m", "HF"),
        ("40m", "HF"),
        ("30m", "HF"),
        ("20m", "HF"),
        ("17m", "HF"),
        ("15m", "HF"),
        ("12m", "HF"),
        ("10m", "HF"),
        ("6m", "VHF"),
        ("2m", "VHF"),
        ("1.25m", "VHF"),
        ("70cm", "UHF"),
    ];
    let eng = state.lock().map_err(|e| e.to_string())?;
    let class = eng.settings().license_class;
    // RTTY / SSTV: fixed standard watering-hole channels (like WSJT-X's per-mode
    // dials), license-filtered per band — a Technician sees only the bands their
    // class can key there (RTTY rides digital privileges, SSTV rides phone).
    let lower = mode.to_ascii_lowercase();
    if lower == "rtty" || lower == "sstv" {
        let (plan, priv_mode) = if lower == "rtty" {
            (tempo_app::bandplan::rtty_band_plan(), OperatingMode::Digital)
        } else {
            (tempo_app::bandplan::sstv_band_plan(), OperatingMode::Phone)
        };
        return Ok(plan
            .into_iter()
            .filter(|c| {
                // Channel band ids may carry a suffix ("2m-call") — privilege-check
                // the base band.
                let base = c.band.split('-').next().unwrap_or(&c.band);
                tempo_app::privileges::segment_start(class, base, priv_mode).is_some()
            })
            .collect());
    }
    // The caller (the cockpit) passes its mode explicitly — the engine's operating_mode is
    // set asynchronously on section entry, so reading it here would race the first mount.
    let mode = match lower.as_str() {
        "phone" => OperatingMode::Phone,
        "cw" => OperatingMode::Cw,
        _ => OperatingMode::Digital,
    };
    let mut out = Vec::new();
    for (band, group) in BANDS {
        if let Some(seg) = tempo_app::privileges::segment_start(class, band, mode) {
            // In CW, park in the CW ACTIVITY window (14.030, not the dead 14.000 edge) —
            // clamped to the licensed segment start so it never drops below privileges.
            let dial = if matches!(mode, OperatingMode::Cw) {
                tempo_app::bandplan::cw_activity_mhz(band).map_or(seg, |a| a.max(seg))
            } else {
                seg
            };
            // Sideband stored: USB/LSB by band for phone; digital-safe USB otherwise (the
            // rig-mode policy forces CW in the CW section regardless of this field).
            let sideband = if matches!(mode, OperatingMode::Phone) && dial < 10.0 {
                "LSB"
            } else {
                "USB"
            };
            out.push(BandChannel {
                band: band.to_string(),
                group: group.to_string(),
                dial_mhz: dial,
                mode: sideband.to_string(),
                label: format!("{band} · {dial:.3} MHz"),
                note: String::new(),
            });
        }
    }
    Ok(out)
}

/// Change band / dial frequency / mode live (does not reset the operating mode).
/// `mode` is "USB" or "FM". Persists, retunes the rig, returns the snapshot.
#[tauri::command]
fn set_frequency(
    state: State<'_, SharedEngine>,
    dial_mhz: f64,
    band: String,
    mode: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_frequency(dial_mhz, &band, &mode);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist frequency: {e}");
    }
    Ok(eng.snapshot())
}

/// Set the per-section operating mode — the rig-mode policy. "digital" obeys the rig
/// (FT8/FT4 default); "phone" forces USB/LSB by band; "cw" forces CW. The phone/CW
/// operating sections call this so the rig follows; the radio loop applies it on the
/// next tune. `follow_freq` = true when the operator clicks an actual operating-section tab
/// (QSY to that mode's home freq); false for incidental nav and the Needed click. Persists,
/// returns the snapshot.
#[tauri::command]
fn set_operating_mode(
    state: State<'_, SharedEngine>,
    mode: String,
    follow_freq: bool,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_operating_mode(&mode, follow_freq);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist operating mode: {e}");
    }
    Ok(eng.snapshot())
}

/// Work a spotted station (the Needed click): set the operating mode AND QSY to the spot's
/// exact frequency atomically (one engine lock, one round-trip) — so the rig can't end up in
/// the new mode at the old dial, and the UI never sees a half-applied state. Persists,
/// returns the snapshot.
#[tauri::command]
fn work_spot(
    state: State<'_, SharedEngine>,
    spots: State<'_, SharedSpots>,
    mode: String,
    freq_mhz: f64,
    band: String,
    call: Option<String>,
) -> Result<AppSnapshot, String> {
    // Pile-up SPLIT: if the freshest cluster spot for this call names a listening
    // offset ("UP 2" / "QSX …"), configure rig split so TX lands where the DX is
    // listening — the N1MM behavior, using the spot we already hold. Tolerant
    // lookup (3Y0J/MM matches 3Y0J); no spot or no offset → simplex.
    let split_up_khz = call.as_deref().and_then(|c| {
        let c = c.to_uppercase();
        // Slash-boundary tolerant identity ONLY ("3Y0J" ⇔ "3Y0J/MM") — bare prefix
        // matching would let "K9A" pick up "K9AB"'s spot (a different station).
        let same_station = |dx: &str| {
            dx == c || dx.starts_with(&format!("{c}/")) || c.starts_with(&format!("{dx}/"))
        };
        spots.lock().ok().and_then(|buf| {
            buf.recent_within(
                std::time::Instant::now(),
                std::time::Duration::from_secs(1800),
            )
            .into_iter()
            .filter(|cs| {
                same_station(&cs.dx_call.to_uppercase())
                    // The spot must be for THIS frequency neighborhood — a 20 m CW
                    // spot's split must not apply to the same call worked on 40 m.
                    && (cs.freq_mhz() - freq_mhz).abs() < 0.05
            })
            .find_map(|cs| cs.split_offset_khz())
        })
    });
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.work_spot_split(&mode, freq_mhz, &band, split_up_khz);
    eng.note_work_call(call); // cross-window prefill hint (pop-out band map → main window log)
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist worked spot: {e}");
    }
    Ok(eng.snapshot())
}

/// Queue CW to transmit (CAT keyer path). `text` is an F-key macro template or literal
/// type-ahead; the engine expands it (mycall/name/grid + the worked call + a 599 report)
/// and the radio loop keys it via the rig. Operator-initiated; respects Monitor.
#[tauri::command]
fn send_cw(state: State<'_, SharedEngine>, text: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.send_cw(&text);
    Ok(eng.snapshot())
}

/// Record the worked station's QRZ name + US state for the `{HISNAME}`/`{HISSTATE}` CW-macro
/// tokens. Pushed by the log form when a callbook lookup resolves; `call` keys it to the contact
/// so a stale lookup never keys the wrong name. Empty `call` clears it. Fire-and-forget (no
/// snapshot — the peer info only surfaces at macro-expand time).
#[tauri::command]
fn set_cw_peer_info(
    state: State<'_, SharedEngine>,
    call: String,
    name: String,
    peer_state: String,
) -> Result<(), String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_cw_peer_info(call, name, peer_state);
    Ok(())
}

/// Set the CW keyer speed in WPM (5–50).
#[tauri::command]
fn set_cw_wpm(state: State<'_, SharedEngine>, wpm: u32, commit: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_cw_wpm(wpm);
    // `commit` gates the DISK write, not the live change. Two reasons it isn't unconditional:
    // (1) this is driven by a range slider — one drag is ~45 calls, and each save is a
    //     create-tmp + write + fsync + rename of the whole settings file;
    // (2) the CW decoder's automatic speed-match calls the same path, and matching the
    //     station you're working must not overwrite the operator's own stored speed.
    // The UI commits once after the operator settles. SF ticket #2.
    if commit {
        if let Err(e) = eng.settings().save(&settings_path()) {
            eprintln!("tempo: set_cw_wpm save failed: {e}");
        }
    }
    Ok(eng.snapshot())
}

/// Abort CW in progress (Esc) — stops the rig keyer and clears the queue.
#[tauri::command]
fn stop_cw(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.stop_cw();
    Ok(eng.snapshot())
}

/// Manual PTT for live phone — key (true) / unkey (false) the rig. Operator push-to-
/// talk; respects Monitor (a key request is ignored while TX is disabled).
#[tauri::command]
fn set_ptt(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_ptt(on);
    Ok(eng.snapshot())
}

/// Set RF output power as a 0.0–1.0 fraction; the radio loop applies it to the rig.
#[tauri::command]
fn set_rf_power(state: State<'_, SharedEngine>, power: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_rf_power(power);
    Ok(eng.snapshot())
}

/// Set mic gain as a 0.0–1.0 fraction; the radio loop applies it to the rig.
#[tauri::command]
fn set_mic_gain(state: State<'_, SharedEngine>, gain: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_mic_gain(gain);
    Ok(eng.snapshot())
}

/// Set the noise-reduction level as a 0.0–1.0 fraction; the radio loop applies it.
#[tauri::command]
fn set_nr_level(state: State<'_, SharedEngine>, level: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_nr_level(level);
    Ok(eng.snapshot())
}

/// Set the AGC speed ("fast"|"mid"|"slow"); the radio loop applies it to the rig.
#[tauri::command]
fn set_agc(state: State<'_, SharedEngine>, speed: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_agc(&speed);
    Ok(eng.snapshot())
}

/// Set (`Some`) or clear (`None`) the desired split TX dial (MHz); the radio loop applies it.
/// `Some(tx)` = TX split to that dial (e.g. "up 5"), `None` = back to simplex.
#[tauri::command]
fn set_split(state: State<'_, SharedEngine>, tx_mhz: Option<f64>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_split(tx_mhz);
    Ok(eng.snapshot())
}

/// Enable/disable a rig DSP function ("nb"|"nr"|"notch"|"comp"|"vox"); the radio loop applies it
/// next cycle. The snapshot reflects the requested state optimistically (the loop's GET reconciles).
#[tauri::command]
fn set_rig_func(
    state: State<'_, SharedEngine>,
    func: String,
    on: bool,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_rig_func(&func, on);
    Ok(eng.snapshot())
}

/// Set (`Some("USB"|"LSB"|"FM")`) or clear (`None` = AUTO) the transient Phone mode override; the
/// radio loop applies it next cycle, and a band change reverts to the band-auto sideband.
#[tauri::command]
fn set_sideband_override(
    state: State<'_, SharedEngine>,
    mode: Option<String>,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_sideband_override(mode.as_deref());
    Ok(eng.snapshot())
}

/// Set the rig RX filter/passband width in Hz; the radio loop applies it via set_mode next cycle.
#[tauri::command]
fn set_filter_width(state: State<'_, SharedEngine>, hz: u32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_filter_width(hz);
    Ok(eng.snapshot())
}

/// Set the native Icom scope SPAN (± half-width, Hz); applied to the rig by the radio loop.
#[tauri::command]
fn set_scope_span(state: State<'_, SharedEngine>, hz: u32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_scope_span(hz);
    Ok(eng.snapshot())
}

/// Set the native Icom scope REFERENCE level (tenths of a dB, −200..+200).
#[tauri::command]
fn set_scope_ref(state: State<'_, SharedEngine>, tenths_db: i32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_scope_ref(tenths_db);
    Ok(eng.snapshot())
}

/// Set the FlexRadio native-panadapter BANDWIDTH (Hz); the FlexSpectrum worker applies it live.
#[tauri::command]
fn set_flex_pan_span(state: State<'_, SharedEngine>, hz: f64) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_flex_pan_span(hz);
    Ok(eng.snapshot())
}

/// Set the FlexRadio panadapter REFERENCE level (dBm); `None`/absent = auto.
#[tauri::command]
fn set_flex_pan_ref(
    state: State<'_, SharedEngine>,
    ref_dbm: Option<i32>,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_flex_pan_ref(ref_dbm);
    Ok(eng.snapshot())
}

/// Set the native Icom scope center/fixed mode (`true` = fixed band-edge view).
#[tauri::command]
fn set_scope_fixed(state: State<'_, SharedEngine>, fixed: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_scope_fixed(fixed);
    Ok(eng.snapshot())
}

/// Set the RIT (receive incremental tuning) offset in Hz — 0 turns RIT off. Applied next loop.
#[tauri::command]
fn set_rit(state: State<'_, SharedEngine>, hz: i32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_rit(hz);
    Ok(eng.snapshot())
}

/// Set the XIT (transmit incremental tuning) offset in Hz — 0 turns XIT off.
#[tauri::command]
fn set_xit(state: State<'_, SharedEngine>, hz: i32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_xit(hz);
    Ok(eng.snapshot())
}

/// Select the active VFO ("A" / "B").
#[tauri::command]
fn set_vfo(state: State<'_, SharedEngine>, vfo: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_vfo(vfo.trim().eq_ignore_ascii_case("B"));
    Ok(eng.snapshot())
}

/// Swap the active VFO (A↔B).
#[tauri::command]
fn swap_vfo(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.request_swap_vfo();
    Ok(eng.snapshot())
}

/// Choose the CW keyer back-end ("cat" = rig send_morse / "soundcard" = keyed tone)
/// and tone pitch (Hz; <=0 keeps the current pitch). Soundcard moves the rig to USB.
#[tauri::command]
fn set_cw_keyer(
    state: State<'_, SharedEngine>,
    backend: String,
    pitch: f32,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_cw_keyer(&backend, pitch);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist CW keyer: {e}");
    }
    Ok(eng.snapshot())
}

// ----- Phone voice keyer: play / record / import recorded WAV messages -----

/// Play a voice-keyer message: read the slot's WAV and queue it for the radio loop to
/// transmit (PTT + audio). Errors if the slot has no recording.
#[tauri::command]
fn play_voice_message(state: State<'_, SharedEngine>, slot: u8) -> Result<AppSnapshot, String> {
    let file = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.voice_messages()
            .iter()
            .find(|m| m.slot == slot)
            .map(|m| m.file.clone())
            .unwrap_or_default()
    };
    if file.trim().is_empty() {
        return Err(format!(
            "No recording in F{slot} yet — record or import one first"
        ));
    }
    #[cfg(feature = "radio")]
    {
        let samples = tempo_audio::voice::read_wav_12k(&file)
            .map_err(|e| format!("Could not read voice message: {e}"))?;
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.send_voice(samples);
        Ok(eng.snapshot())
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = file;
        Err("Voice keyer needs the radio build".to_string())
    }
}

/// Stop voice playback in progress (Esc) — flush queued audio + unkey.
#[tauri::command]
fn stop_voice(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.stop_voice();
    Ok(eng.snapshot())
}

/// Begin recording a voice message — the radio loop captures from the input device into
/// the engine until `stop_voice_recording`.
#[tauri::command]
fn start_voice_recording(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.start_recording();
    Ok(eng.snapshot())
}

/// Cancel an in-progress recording, DISCARDING the captured audio (no WAV written). Used
/// to tear down cleanly when the operator leaves the Phone section mid-record.
#[tauri::command]
fn cancel_voice_recording(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let _ = eng.stop_recording(); // take + drop the buffer
    Ok(eng.snapshot())
}

/// Stop recording, write the captured audio to the slot's WAV, bind it (+ label), and
/// return the updated message list.
#[tauri::command]
fn stop_voice_recording(
    state: State<'_, SharedEngine>,
    slot: u8,
    label: String,
) -> Result<Vec<VoiceMessage>, String> {
    let samples = {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.stop_recording()
    };
    if samples.is_empty() {
        return Err("Nothing was recorded — check your input device".to_string());
    }
    #[cfg(feature = "radio")]
    {
        let path = voice_dir().join(format!("slot{slot}.wav"));
        tempo_audio::voice::write_wav_12k_atomic(&path, &samples)
            .map_err(|e| format!("Could not save recording: {e}"))?;
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        let lbl = (!label.trim().is_empty()).then_some(label.as_str());
        eng.set_voice_message(slot, lbl, Some(&path.to_string_lossy()));
        if let Err(e) = eng.settings().save(&settings_path()) {
            eprintln!("tempo: failed to persist voice message: {e}");
        }
        Ok(eng.voice_messages().to_vec())
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = (state, slot, label, samples);
        Err("Voice keyer needs the radio build".to_string())
    }
}

/// Import a `.wav` file (raw bytes from the UI) into a slot — normalized to 12 kHz mono.
#[tauri::command]
fn import_voice_message(
    state: State<'_, SharedEngine>,
    slot: u8,
    label: String,
    bytes: Vec<u8>,
) -> Result<Vec<VoiceMessage>, String> {
    #[cfg(feature = "radio")]
    {
        let dir = voice_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("slot{slot}.import.tmp.wav"));
        std::fs::write(&tmp, &bytes).map_err(|e| format!("Could not stage import: {e}"))?;
        // Normalize to 12 kHz mono (this also validates it's a readable WAV).
        let samples = match tempo_audio::voice::read_wav_12k(&tmp) {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("Not a readable WAV file: {e}"));
            }
        };
        let _ = std::fs::remove_file(&tmp);
        if samples.is_empty() {
            return Err("The imported file had no audio".to_string());
        }
        let path = dir.join(format!("slot{slot}.wav"));
        tempo_audio::voice::write_wav_12k_atomic(&path, &samples)
            .map_err(|e| format!("Could not save the import: {e}"))?;
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        let lbl = (!label.trim().is_empty()).then_some(label.as_str());
        eng.set_voice_message(slot, lbl, Some(&path.to_string_lossy()));
        if let Err(e) = eng.settings().save(&settings_path()) {
            eprintln!("tempo: failed to persist voice message: {e}");
        }
        Ok(eng.voice_messages().to_vec())
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = (state, slot, label, bytes);
        Err("Voice keyer needs the radio build".to_string())
    }
}

/// Rename a voice-keyer slot's label (no audio change).
#[tauri::command]
fn set_voice_label(
    state: State<'_, SharedEngine>,
    slot: u8,
    label: String,
) -> Result<Vec<VoiceMessage>, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_voice_message(slot, Some(&label), None);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist voice label: {e}");
    }
    Ok(eng.voice_messages().to_vec())
}

/// Clear the recording bound to a slot (keeps the label).
#[tauri::command]
fn clear_voice_message(
    state: State<'_, SharedEngine>,
    slot: u8,
) -> Result<Vec<VoiceMessage>, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.clear_voice_message(slot);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist voice clear: {e}");
    }
    // Clear means gone: delete the orphaned recording too (best-effort).
    let _ = std::fs::remove_file(voice_dir().join(format!("slot{slot}.wav")));
    Ok(eng.voice_messages().to_vec())
}

/// The configured voice-keyer message slots (for the Phone cockpit's keyer strip).
#[tauri::command]
fn get_voice_messages(state: State<'_, SharedEngine>) -> Result<Vec<VoiceMessage>, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.voice_messages().to_vec())
}

// ----- QSO recording (audio bridge): stream live RX capture to a WAV on disk -----

/// Start recording the live RX audio to a timestamped WAV under `recordings/`. The radio
/// loop streams capture to the file (no RAM buffer); recording persists across UI nav until
/// `stop_qso_recording`. Only flips engine state — the loop (radio build) owns the file I/O.
#[tauri::command]
fn start_qso_recording(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    #[cfg(feature = "radio")]
    {
        let dir = recordings_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Err(format!("Could not create the recordings folder: {e}"));
        }
        // Millisecond stamp so a quick stop→start in the same second can't clobber the file.
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("qso-{ms}.wav"));
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.start_qso_recording(&path.to_string_lossy());
        Ok(eng.snapshot())
    }
    // No radio loop = nothing to stream the file; don't light a REC badge that can't record.
    #[cfg(not(feature = "radio"))]
    {
        let _ = state;
        Err("Recording needs the radio build".to_string())
    }
}

/// Stop the in-progress QSO recording — the radio loop finalizes the WAV on its next pass.
#[tauri::command]
fn stop_qso_recording(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    #[cfg(feature = "radio")]
    {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.stop_qso_recording();
        Ok(eng.snapshot())
    }
    #[cfg(not(feature = "radio"))]
    {
        let _ = state;
        Err("Recording needs the radio build".to_string())
    }
}

/// Set the TX-slot period: `true` = transmit on even/"1st" slots, `false` =
/// odd/"2nd". Two stations must use OPPOSITE periods to complete a QSO. Persists.
#[tauri::command]
fn set_tx_even(state: State<'_, SharedEngine>, even: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tx_even(even);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist tx period: {e}");
    }
    Ok(eng.snapshot())
}

/// Toggle smart auto-cycle — when on, answering a heard station auto-picks the opposite
/// T/R cycle (FT8-style). Turning it on re-enables the auto pick; a manual Tx 1st/2nd
/// selection (set_tx_even) turns it off.
#[tauri::command]
fn set_tx_cycle_auto(state: State<'_, SharedEngine>, auto: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tx_cycle_auto(auto);
    Ok(eng.snapshot())
}

/// Toggle the presence heartbeat — a periodic low-cadence beacon so listening Tempo
/// stations enter each other's rosters and store-and-forward can deliver. Persisted.
#[tauri::command]
fn set_beacon(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_beacon(on);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist heartbeat setting: {e}");
    }
    Ok(eng.snapshot())
}

/// The operator erased a decode pane — queue the WSJT-X UDP Clear so
/// cooperating apps (JTAlert/GridTracker) mirror it. 0 = Band, 1 = Rx, 2 = both.
#[tauri::command]
fn notify_erase(state: State<'_, SharedEngine>, window: u8) -> Result<(), String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.notify_erase(window);
    Ok(())
}

/// WSJT-X "Decode" button / F6: re-run the decoder over the last period's
/// audio with the current settings; only newly-found lines are ingested.
#[tauri::command]
fn redecode(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let _ = eng.redecode();
    Ok(eng.snapshot())
}

/// Start a CQ run; `dir` = a directed-CQ token ("DX"/"NA"/"POTA"/…) or None
/// for a plain CQ (also clears a sticky directed token).
#[tauri::command]
fn start_cq(state: State<'_, SharedEngine>, dir: Option<String>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.start_cq(dir.as_deref())?;
    Ok(eng.snapshot())
}

/// Call CQ from Tempo (chat-first): emit ONE structured `CQ <mycall> <mygrid>` frame
/// and arm TX, staying in Chat mode. Errors if the callsign/grid aren't set (so a CQ
/// never goes out malformed). `dir` = an optional directed-CQ token (None = plain CQ).
#[tauri::command]
fn call_cq(state: State<'_, SharedEngine>, dir: Option<String>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.call_cq(dir.as_deref())?;
    Ok(eng.snapshot())
}

/// Toggle the chat CQ RUN (Tempo keep-calling loop): on = immediate CQ + re-send
/// every idle TX slot until answered (auto-pauses) or stopped.
#[tauri::command]
fn set_chat_cq(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_chat_cq(on)?;
    Ok(eng.snapshot())
}

/// Resume a paused chat CQ run now (skip the idle auto-resume wait).
#[tauri::command]
fn resume_chat_cq(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.resume_chat_cq()?;
    Ok(eng.snapshot())
}

/// WSJT-X Tx-slot click: force `text` as the next transmission to `call`
/// (starts/retargets the QSO, arms per the double-click behavior option).
#[tauri::command]
fn override_next_tx(
    state: State<'_, SharedEngine>,
    call: String,
    grid: Option<String>,
    text: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.override_next_tx(&call, grid.as_deref(), &text);
    Ok(eng.snapshot())
}

/// Set the receive audio offset (Hz) — the green waterfall marker. The TX offset
/// follows unless "Hold Tx Freq" is on. Persists.
#[tauri::command]
fn set_rx_offset(state: State<'_, SharedEngine>, hz: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_rx_offset(hz);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist rx offset: {e}");
    }
    Ok(eng.snapshot())
}

/// Set the transmit audio offset (Hz) — the red waterfall marker. Persists.
#[tauri::command]
fn set_tx_offset(state: State<'_, SharedEngine>, hz: f32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tx_offset(hz);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist tx offset: {e}");
    }
    Ok(eng.snapshot())
}

/// Hold the TX offset fixed when the RX offset changes (WSJT-X "Hold Tx Freq").
/// Persists.
#[tauri::command]
fn set_hold_tx_freq(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_hold_tx_freq(on);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist hold-tx: {e}");
    }
    Ok(eng.snapshot())
}

/// Open (or focus) a standalone OS window showing one panel — multi-monitor
/// tear-off. The detached window loads the app at `?panel=<panel>` and renders just
/// that panel against the same shared engine the main window uses.
///
/// `instance` addresses WHICH surface of that panel (see [`chains`]): absent/`main` is the one
/// window that has always existed, `w<n>` is an extra unbound board. Omitting it is the entire
/// existing call surface, and produces a byte-identical label and URL — zero migration.
#[tauri::command]
async fn open_panel_window(
    app: tauri::AppHandle,
    panel: String,
    instance: Option<String>,
) -> Result<(), String> {
    let slug: String = panel
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if slug.is_empty() {
        return Err("invalid panel".into());
    }
    // The instance is REJECTED when malformed, never stripped. The slug filter above is the
    // cautionary tale: it silently drops non-alphanumerics, which is why `operate-2`,
    // `operate:2` and `operate2` all collapse onto one window. That is now harmless (the slug
    // can never contain the `-` that separates an instance) but must not be repeated here — a
    // typo'd token that got coerced would address a different radio.
    let inst = match instance.as_deref() {
        None => Instance::Main,
        Some(token) => Instance::parse(token)?,
    };
    // HARD GATE — see `chains::openable`.
    chains::openable(inst)?;
    let label = panel_label(&slug, inst);
    // Already open → just focus it (one window per SURFACE — distinct instances of the same
    // panel are distinct windows).
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        return Ok(());
    }
    // Friendly window title so multi-monitor users can tell torn-off windows apart.
    let title = match slug.as_str() {
        "connect" => "Nexus — Connect".to_string(),
        "dxped" => "Nexus — DXpeditions".to_string(),
        "needed" => "Nexus — Needed".to_string(),
        "operate" => "Nexus — Operate".to_string(),
        "fieldday" => "Nexus — Field Day".to_string(),
        "waterfall" => "Nexus — Waterfall".to_string(),
        "bandmapPhone" => "Nexus — Band map (Phone)".to_string(),
        "bandmapCw" => "Nexus — Band map (CW)".to_string(),
        other => format!("Nexus — {other}"),
    };
    // The Operate cockpit (waterfall + Band Activity + roster) needs more room than the
    // narrower insight panels; the band map is tall + narrow (a vertical frequency axis).
    let is_bandmap = slug == "bandmapPhone" || slug == "bandmapCw";
    // The band map reopens where the operator left it (size + position), so a Windows-snapped
    // vertical strip on the side survives restarts. Other pop-outs keep their fixed defaults.
    let saved = if is_bandmap {
        load_bandmap_window(&slug, inst)
    } else {
        None
    };
    // A docked window re-snaps to the CURRENT monitor work area AFTER build (so a resolution
    // change since last session can't strand it off-screen); a free window replays its saved
    // absolute position.
    let docked_side = saved
        .as_ref()
        .map(|g| g.dock.clone())
        .filter(|d| d == "left" || d == "right");
    let (w, h) = if let Some(g) = &saved {
        (g.w, g.h)
    } else if slug == "operate" {
        (1140.0, 760.0)
    } else if is_bandmap {
        (420.0, 780.0)
    } else if slug == "fieldday" {
        (560.0, 760.0) // the scoreboard: operator + tiles + sections board
    } else if slug == "waterfall" {
        (900.0, 300.0) // a wide, short monitoring strip
    } else {
        (760.0, 660.0)
    };
    // `instance` is a SEPARATE query parameter, never baked into the slug — the slug filter
    // would strip the separator and alias it. Appended only above `main`, so every window that
    // is openable today keeps the exact URL it has always had.
    let url = match inst {
        Instance::Main => format!("index.html?panel={slug}"),
        other => format!("index.html?panel={slug}&instance={other}"),
    };
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url.into()),
    )
    .title(title)
    .inner_size(w, h)
    .min_inner_size(
        if slug == "waterfall" { 380.0 } else { 420.0 },
        if slug == "waterfall" { 180.0 } else { 360.0 },
    );
    if let Some(g) = &saved {
        // Open at the saved position. For a FREE window that's the exact restore; for a DOCKED
        // window it lands it on the RIGHT monitor (multi-monitor) before the re-snap below
        // refines it to THAT monitor's work-area edge — and it's the fallback if the re-snap
        // can't resolve a monitor.
        builder = builder.position(g.x, g.y);
    }
    // Pop-outs are ordinary windows — the operator must be able to send them behind the
    // main UI (a tester couldn't hide the waterfall while it was pinned always-on-top). A
    // future "pin" toggle can call `window.set_always_on_top(true)` on demand.
    let win = builder.build().map_err(|e| e.to_string())?;
    if let Some(side) = docked_side {
        // Re-pin to the edge of the current work area (best-effort; ignore if unmapped) so a
        // resolution change since last session can't strand it off-screen.
        let _ = snap_bandmap_to_edge(&win, &side);
    }
    Ok(())
}

/// Snap the calling band-map window to the left/right edge of its monitor as a full-height
/// vertical strip (the built-in version of the Windows-snap the operator was doing by hand),
/// keeping its current width. `side` = "left" | "right"; anything else un-docks (free-float,
/// just persists the current geometry). The dock + geometry persist so it restores on relaunch.
#[tauri::command]
fn dock_bandmap_window(window: tauri::WebviewWindow, side: String) -> Result<(), String> {
    if side == "left" || side == "right" {
        snap_bandmap_to_edge(&window, &side)?;
        return Ok(());
    }
    // Un-dock: leave the window where it is, just clear the dock flag (per-surface) so a later
    // open doesn't re-snap it. Persist the current geometry.
    //
    // NOTE for a non-persisting surface (`w<n>`): dock rides in the same file as the rect, so
    // both load and save no-op and the un-dock is in-memory only — the surface re-docks on every
    // open. That is the accepted price of "never persist a recycled instance" (the alternative,
    // GC-on-boot, risks a later `w3` inheriting a stranger's rect). Safe either way: the path is
    // `None` on BOTH sides, so a `w<n>` can never read or clobber `main`'s file. Unreachable
    // today — `w<n>` windows are refused in `open_panel_window`.
    capture_bandmap_window(&window);
    if let Some((slug, inst)) = bandmap_key(&window) {
        let mut g = load_bandmap_window(slug, inst).unwrap_or_default();
        g.dock = String::new();
        save_bandmap_window(slug, inst, &g);
    }
    Ok(())
}

/// Initiate a directed QSO with a specific station (the UI "work this station"
/// action). Enters QSO mode answering `call`. `message`/`snr` are the exact
/// decoded line the operator double-clicked (when available) so the auto-sequencer
/// jumps to the correct next Tx — WSJT-X double-click semantics — instead of
/// restarting at the grid. Returns the refreshed snapshot.
#[tauri::command]
fn call_station(
    state: State<'_, SharedEngine>,
    call: String,
    grid: Option<String>,
    message: Option<String>,
    snr: Option<i32>,
    freq: Option<f32>,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    // Working a station keys a standard FT8/FT4 message (your grid in Tx1) — refuse
    // without a valid callsign + grid so we never emit a grid-less directed call.
    eng.structured_tx_ready(true)?;
    let g = grid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let msg = message.as_deref().map(str::trim).filter(|s| !s.is_empty());
    // `freq` = the decoded station's audio offset (Hz); move our RX/TX onto it (WSJT-X
    // double-click). Ignore non-positive values (no usable frequency).
    let dx_freq = freq.filter(|f| *f > 0.0);
    eng.call_station_ctx(&call, g, msg, snr, dx_freq);
    Ok(eng.snapshot())
}

/// Toggle Skip Tx1 (WSJT-X parity). Session-only — the engine flag resets to off each
/// launch (not persisted), matching WSJT-X; the UI holds its own session state to match.
#[tauri::command]
fn set_skip_tx1(state: State<'_, SharedEngine>, enabled: bool) -> Result<(), String> {
    state
        .lock()
        .map_err(|e| e.to_string())?
        .set_skip_tx1(enabled);
    Ok(())
}

/// Switch the top-level operating area: "dx" (FT8/FT4 structured) or "msg"
/// (FT1/DX1 free-text chat). Atomically sets the area-appropriate tier + mode.
#[tauri::command]
fn set_area(state: State<'_, SharedEngine>, area: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_area(&area);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: set_area save failed: {e}");
    }
    Ok(eng.snapshot())
}

/// Operator "Resend": re-arm the current QSO message so a stalled/uncopied step
/// transmits again on the next TX slot. No-op outside a QSO.
#[tauri::command]
fn qso_resend(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qso_resend();
    Ok(eng.snapshot())
}

/// Operator in-QSO free text (WSJT-X Tx5): override the next transmission with
/// `text`, directed to the current DX station when known. No-op outside a QSO.
#[tauri::command]
fn qso_freetext(state: State<'_, SharedEngine>, text: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qso_freetext(&text);
    Ok(eng.snapshot())
}

/// Operator "Log QSO": log the active QSO's contact now (inline cockpit button).
#[tauri::command]
fn log_current_qso(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.log_current_qso();
    Ok(eng.snapshot())
}

/// Confirm-and-log a QSO held by the prompt-to-log popup. `record` is the
/// (possibly edited) contact. Returns the refreshed snapshot.
#[tauri::command]
fn confirm_pending_log(
    state: State<'_, SharedEngine>,
    record: LoggedQso,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.confirm_pending_log(record.into());
    Ok(eng.snapshot())
}

/// Discard a QSO held by the prompt-to-log popup without logging it.
#[tauri::command]
fn discard_pending_log(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.discard_pending_log();
    Ok(eng.snapshot())
}

/// Manually log a contact to the ADIF logbook (the UI "Log QSO" button). Adds in
/// memory and persists to the log file. Returns the refreshed snapshot.
#[tauri::command]
fn log_qso(state: State<'_, SharedEngine>, record: LoggedQso) -> Result<AppSnapshot, String> {
    let call = record.call.clone();
    let (snap, wav) = {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.log_qso(record.into());
        // Per-QSO WAV (off by default): grab the recent RX audio under the lock; write it
        // to disk below, after releasing the lock, so the snapshot poll never waits on I/O.
        let wav = eng.settings().save_qso_wav.then(|| eng.recent_rx_pcm());
        (eng.snapshot(), wav)
    };
    if let Some(pcm) = wav {
        if !pcm.is_empty() {
            let dir = recordings_dir();
            let _ = std::fs::create_dir_all(&dir);
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let safe: String = call.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            let path = dir.join(format!("qso-{safe}-{ms}.wav"));
            // 12 kHz: the engine's RX-audio rate (tempo_fast::SAMPLE_RATE).
            let _ = tempo_core::wavfile::write_wav_i16(&path, &pcm, 12_000);
        }
    }
    Ok(snap)
}

/// The full logbook as serializable contacts (for the UI log view).
#[tauri::command]
fn get_log(state: State<'_, SharedEngine>) -> Result<Vec<LoggedQso>, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.get_log().into_iter().map(LoggedQso::from).collect())
}

/// Edit logbook entry `index` (oldest-first, as returned by `get_log`) — a
/// correction. Confirmation/credit/upload state is preserved by the engine.
/// Returns the refreshed snapshot.
#[tauri::command]
fn edit_qso(
    state: State<'_, SharedEngine>,
    index: usize,
    record: LoggedQso,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    if !eng.update_qso(index, record.into()) {
        return Err("That contact no longer exists — reload the log and try again.".into());
    }
    Ok(eng.snapshot())
}

/// Mark logbook entry `index` (oldest-first, as returned by `get_log`) as
/// QSL-sent — operator-declared truth that a card/request was sent `via`
/// "B"(ureau) / "D"(irect) / "E"(lectronic), dated now. A request is NOT a
/// confirmation: this never flips `confirmed`/`awardConfirmed`. Returns the
/// refreshed snapshot.
#[tauri::command]
fn mark_qsl_sent(
    state: State<'_, SharedEngine>,
    index: usize,
    via: String,
) -> Result<AppSnapshot, String> {
    let via = tempo_core::logbook::QslVia::from_code(&via)
        .ok_or_else(|| format!("Unknown QSL-sent method '{via}' — use B, D, or E."))?;
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    if !eng.mark_qsl_sent(index, via) {
        return Err("That contact no longer exists — reload the log and try again.".into());
    }
    Ok(eng.snapshot())
}

/// Delete logbook entry `index` (oldest-first, as returned by `get_log`). Returns
/// the refreshed snapshot. Indices shift after a delete — the UI reloads the log.
#[tauri::command]
fn delete_qso(state: State<'_, SharedEngine>, index: usize) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    if !eng.delete_qso(index) {
        return Err("That contact no longer exists — reload the log and try again.".into());
    }
    Ok(eng.snapshot())
}

/// Purge the ENTIRE logbook — delete every contact and truncate the ADIF file to
/// an empty log. Destructive and irreversible; the UI gates this behind an explicit
/// confirmation dialog. Returns the number of contacts removed (for the toast).
#[tauri::command]
fn purge_log(state: State<'_, SharedEngine>) -> Result<usize, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.clear_logbook())
}

/// DXCC-first award progress from the logbook (cty.dat-resolved): entities +
/// entity×band "Challenge" slots worked/confirmed, the per-band breakdown, and
/// the worked-but-unconfirmed "new one" chase. Pure/offline — online LoTW/eQSL/
/// QRZ/ClubLog sync (which would flip `confirmed`) is a later increment.
#[tauri::command]
fn get_awards(state: State<'_, SharedEngine>) -> Result<propagation::AwardSummary, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    let mut awards = propagation::Awards::new();
    // Tell the accumulator our own entity so "First DX" counts only foreign ones.
    awards.set_home_call(&eng.settings().mycall);
    for q in eng.get_log() {
        // Award-eligible confirmation only (LoTW/paper) — eQSL doesn't count; plus
        // whether ARRL has granted DXCC-family credit (DXCC / DXCC_BAND /
        // DXCC_MODE / … — real LoTW exports use the granular codes).
        let credited = q.credit_granted.iter().any(|c| c.starts_with("DXCC"));
        awards.add_with_credit(
            &q.call,
            &q.band,
            &q.mode,
            q.award_confirmed,
            credited,
            q.state.as_deref(),
            q.grid.as_deref(),
            q.ota.iota.as_deref(),
        );
    }
    Ok(awards.summary())
}

/// The geographic slice of the logbook — QSOs by WAC continent, by CQ zone, and a DX-vs-domestic
/// split — the dimensions the frontend `StatsView` can't derive on its own (the stored record has
/// no continent/zone; both re-resolve per callsign here via cty.dat, anchored on the operator's
/// own call for the DX split). The rest of the Statistics dashboard (band/mode/year/hour/state/
/// confirmations) is computed frontend-side from `get_log`. Pure/offline.
#[tauri::command]
fn get_log_stats(state: State<'_, SharedEngine>) -> Result<propagation::LogStats, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    let my_call = eng.settings().mycall.clone();
    let calls: Vec<String> = eng.get_log().into_iter().map(|q| q.call).collect();
    Ok(propagation::compute_log_stats(&calls, &my_call))
}

/// The Journey snapshot — the in-app, beginner-first achievement layer (auto-detected
/// firsts, tiered sub-award ladders toward the big awards, fill-the-map collections,
/// novel ham feats, personal bests, an XP/level spine, and an opt-in weekly streak),
/// computed locally from the log + the operator's grid/power. Async (off the main
/// thread; the log scan can be large). Local-only — no network.
#[tauri::command]
async fn get_journey(
    state: State<'_, SharedEngine>,
) -> Result<propagation::JourneySummary, String> {
    use propagation::model::{Band, ModeClass};
    let eng = state.lock().map_err(|e| e.to_string())?;
    let s = eng.settings();
    let qsos: Vec<propagation::JourneyQso> = eng
        .get_log()
        .into_iter()
        .map(|r| propagation::JourneyQso {
            call: r.call,
            grid: r.grid,
            state: r.state,
            band: Band::from_label(&r.band),
            mode: ModeClass::from_adif(&r.mode),
            when_unix: r.when_unix as i64,
            // Award-eligible confirmation (LoTW/paper — not eQSL), matching the
            // awards + "first confirmation" semantics.
            confirmed: r.award_confirmed,
            // The Journey "strongest signal" stat is a digital dB SNR concept; parse
            // the numeric report only for DIGITAL QSOs (a phone "59"/CW "599" isn't dB).
            rst_rcvd: if ModeClass::from_adif(&r.mode) == ModeClass::Digital {
                r.rst_rcvd
                    .as_deref()
                    .and_then(|s| s.trim().parse::<i32>().ok())
            } else {
                None
            },
            pota: r
                .ota
                .their_program
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("POTA")),
            sota: r
                .ota
                .their_program
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("SOTA")),
            // Hunter ladders count DISTINCT park/summit references.
            pota_ref: if r
                .ota
                .their_program
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("POTA"))
            {
                r.ota.their_ref.clone()
            } else {
                None
            },
            sota_ref: if r
                .ota
                .their_program
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("SOTA"))
            {
                r.ota.their_ref.clone()
            } else {
                None
            },
        })
        .collect();
    let grid = (!s.mygrid.is_empty()).then_some(s.mygrid.as_str());
    Ok(propagation::compute_journey(
        &qsos,
        &s.mycall,
        grid,
        s.station_power_w,
        s.journey_streak_enabled,
        now_unix(),
    ))
}

/// Silent match-failure diagnostics: per-QSO "why isn't this confirmed, and what's
/// the one fix?" + a leverage-ranked rollup, from the log + the last LoTW/eQSL
/// reconcile orphans (this session). Pure/offline; cty.dat resolves each call's
/// DXCC entity for the WAS (R4d) US-family gate.
#[tauri::command]
fn get_confirmation_diagnostics(
    state: State<'_, SharedEngine>,
) -> Result<DiagnosticsReportDto, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    let report = eng.confirmation_diagnostics(now_unix(), |call| {
        propagation::dxcc::resolve(call).map(|i| i.entity.to_string())
    });
    Ok(report.into())
}

/// One raw cluster/RBN spot for the Spots panel (the SpotCollector-style firehose view).
/// UNLIKE the Needed board, this is NOT needs-gated — every recent spot is returned and
/// the UI filters client-side. `mode` is the frequency-derived class (CW/Phone/Digital) that
/// drives the filter chips + privilege gate; `submode` carries the RBN skimmer's specific mode
/// (FT8/FT4/RTTY/PSK/CW) when present, so the panel can show it instead of a bare "Digital".
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SpotRow {
    call: String,
    /// DXCC entity, "" if the call doesn't resolve.
    entity: String,
    /// CQ zone, 0 if unknown.
    zone: u8,
    /// US state (WAS code), best-effort from the roster's cached grid for a station heard before
    /// (own decodes / PSK Reporter). `None` for a cluster/RBN spot of a station not yet heard with
    /// a grid, or a non-US station — cluster spots carry no grid of their own.
    state: Option<String>,
    /// Band label ("20m"), "" if off the band plan.
    band: String,
    freq_mhz: f64,
    /// Frequency-derived mode CLASS: "CW" | "Phone" | "Digital". Drives the 3-class filter
    /// chips and the privilege gate.
    mode: String,
    /// The SPECIFIC mode a skimmer reported on the RBN wire ("FT8" / "FT4" / "RTTY" / "PSK" /
    /// "CW"), when this spot carries one — so the firehose can distinguish FT8 from FT4 from
    /// RTTY instead of lumping them all as "Digital". `None` for human-node spots (whose
    /// free-text mode is untrusted, per the 21.074-CW doctrine). The UI shows `submode ?? mode`.
    submode: Option<String>,
    spotter: String,
    /// Other spotters of the same DX (multi-endpoint evidence the buffer carries forward).
    corroborators: Vec<String>,
    /// Seconds since the spot was received; -1 if unknown (no receive stamp).
    age_secs: i64,
    comment: String,
    /// True when the operator's license class may TRANSMIT at this spot's frequency in
    /// its mode (privileges::tx_allowed; `Open` class ⇒ always true). Drives the Spots
    /// panel's "my privileges" filter — computed here so the legal band data has ONE
    /// source of truth (the same tables as the TX lockout).
    licensed: bool,
}

/// Raw spot firehose for the Spots panel — every recent spot (CW/Phone/Digital, all
/// sources), newest first, NOT filtered by operator needs. The buffer's age-based
/// retention (≈20 min) bounds the set; the UI applies band/mode/age filters client-side.
#[tauri::command]
fn get_all_spots(spots: State<'_, SharedSpots>, state: State<'_, SharedEngine>) -> Vec<SpotRow> {
    use tempo_app::settings::{LicenseClass, OperatingMode};
    let now = now_unix();
    let recent = match spots.lock() {
        Ok(buf) => buf.recent_within(
            std::time::Instant::now(),
            std::time::Duration::from_secs(1200),
        ),
        Err(_) => return Vec::new(),
    };
    // Hold the engine lock across row-building: read the license class once, and resolve each
    // spot's US state from the roster's cached grid (a station heard before → its grid → state).
    // A poisoned lock degrades to Open (no gate) + no state — never a wrongly-hidden spot.
    let eng = state.lock().ok();
    let class = eng
        .as_ref()
        .map(|e| e.settings().license_class)
        .unwrap_or(LicenseClass::Open);
    let mut rows: Vec<SpotRow> = recent
        .into_iter()
        .map(|cs| {
            let freq = cs.freq_mhz();
            let band = propagation::Band::from_mhz(freq)
                .map(|b| b.label().to_string())
                .unwrap_or_default();
            let (entity, zone) = propagation::dxcc::resolve(&cs.dx_call)
                .map(|i| (i.entity.to_string(), i.cq_zone))
                .unwrap_or_default();
            // US state from the roster's cached grid, when this station was heard before with one.
            let state = eng
                .as_ref()
                .and_then(|e| e.roster_grid(&cs.dx_call))
                .and_then(propagation::state_for_grid)
                .map(str::to_string);
            let age_secs = if cs.received_unix > 0 {
                (now - cs.received_unix as i64).max(0)
            } else {
                -1
            };
            let mode_label = propagation::classify_spot_mode(freq).label();
            let licensed = tempo_app::privileges::tx_allowed(
                class,
                freq,
                match mode_label {
                    "Phone" => OperatingMode::Phone,
                    "CW" => OperatingMode::Cw,
                    _ => OperatingMode::Digital,
                },
            );
            SpotRow {
                call: cs.dx_call.clone(),
                entity,
                zone,
                state,
                band,
                freq_mhz: freq,
                mode: mode_label.to_string(),
                // Trusted specific mode from the RBN skimmer wire, when present (never human
                // free-text). The class-level `mode` above still drives the filter + privilege.
                submode: cs.skimmer_mode().map(str::to_string),
                spotter: cs.spotter.clone(),
                corroborators: cs.corroborators.clone(),
                age_secs,
                comment: cs.comment.clone(),
                licensed,
            }
        })
        .collect();
    // Newest first; unknown-age spots sort last.
    rows.sort_by_key(|r| if r.age_secs < 0 { i64::MAX } else { r.age_secs });
    rows
}

/// Need-aware spotting: rank the stations WORKABLE FROM HERE RIGHT NOW by award
/// value (new DXCC / CQ zone / band-slot / mode). Crucially, the value is the bands
/// you're NOT tuned to — so the evidence is EMPIRICAL near-me reception, not a
/// model: a station counts if a receiver NEAR YOU (PSK Reporter / near-region feed,
/// band-aware radius) is hearing it, plus whatever your own radio is decoding on the
/// current band. (Weak-signal-sleuth: someone local to you actually copied the DX,
/// so the path is open from your QTH too — not "someone in Spain heard the Spanish
/// station.")
#[tauri::command]
async fn get_need_alerts(
    state: State<'_, SharedEngine>,
    live_paths: State<'_, SharedLivePaths>,
    region_paths: State<'_, SharedRegionPaths>,
    spots: State<'_, SharedSpots>,
    ota_cache: State<'_, SharedOtaSpots>,
) -> Result<Vec<propagation::NeedAlert>, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    // Two-instance freshness: if the OTHER radio just logged/confirmed something in the shared
    // log, fold it in BEFORE computing needs — otherwise this (possibly monitoring) radio would
    // flag a DXCC/state/grid as needed that the other one already worked. Mtime-gated, so this is
    // a cheap `stat` whenever the file is unchanged.
    eng.sync_shared_log_if_changed();
    let mut needs = propagation::LogNeeds::new();
    for q in eng.get_log() {
        needs.add(
            &q.call,
            &q.band,
            &q.mode,
            q.grid.as_deref(),
            q.state.as_deref(),
            q.award_confirmed,
        );
    }
    let snap = eng.snapshot();
    // Operator "wanted" watch list (W1.5) — captured before the lock drops.
    let wanted_calls = eng.settings().wanted_calls.clone();
    // License class for the privilege gate below — a station on a frequency the operator may not
    // transmit to is not a "need". Open (non-US) short-circuits tx_allowed to true, so no gate.
    let license_class = eng.settings().license_class;
    drop(eng); // nothing below needs the engine — don't hold the hot lock
    let band = snap.radio.band.clone();
    // Your own radio's decodes on the CURRENT band (you are the receiver). These come
    // from the digital modem, so the truthful mode label is the active TIER (FT8/FT4/
    // FT1/DX1) — all of which map to the Digital class for click-to-work routing.
    let tier_mode = format!("{:?}", snap.link.tier).to_uppercase();
    let mut heard: Vec<propagation::Heard> = snap
        .stations
        .iter()
        .map(|s| propagation::Heard {
            call: s.call.clone(),
            band: band.clone(),
            mode: tier_mode.clone(),
            freq_mhz: None, // own decodes are band-level here
            admitted_at: None,
            evidence: Some("decoded by YOUR radio on this band".to_string()),
            grid: s.grid.clone(), // own decode's Maidenhead grid → drives NewGrid
            // Best-guess US state from the grid (drives the WAS NewState hint).
            us_state: s
                .grid
                .as_deref()
                .and_then(propagation::state_for_grid)
                .map(|st| st.to_string()),
        })
        .collect();
    // The real value (empirical evidence, not a model): two complementary signals
    // over the PSK Reporter firehose + near-region feed —
    //   1. heard_near_me: a receiver NEAR YOU is hearing the DX (their signal
    //      reaches you), gated by a band-aware "local to me" radius (Es on VHF is
    //      tighter than F2 on HF);
    //   2. workable_by_getting_out: a third party is hearing a DX in a region your
    //      OWN signal is reaching (who-heard-me reports) on that band — you can
    //      likely work it even if you aren't hearing it yet.
    let now = now_unix() as i64;
    let me_ll = propagation::geo::maidenhead_to_latlon(snap.mygrid.trim());
    if let Ok(buf) = live_paths.lock() {
        let recent = buf.recent(now, 900);
        if let Some(me) = me_ll {
            heard.extend(propagation::heard_near_me(&recent, me));
        }
        heard.extend(propagation::workable_by_getting_out(&recent, &snap.mycall));
    }
    if let Some(me) = me_ll {
        if let Ok(buf) = region_paths.0.lock() {
            heard.extend(propagation::heard_near_me(&buf.recent(now, 900), me));
        }
    }
    // Privilege-gate the DIGITAL evidence paths (own decodes + PSKR near-region / getting-out)
    // the SAME way the cluster/RBN loop below gates each spot: a band the operator may not
    // TRANSMIT digital on is not a "work these" need. Since the PSKR near-region census now
    // covers HF F2 bands (40/20/17/15/12m), a Technician (no HF digital except 10m) would
    // otherwise see 20/40m FT8 DX as workable needs. Everything in `heard` so far is
    // digital-tier, so gate as Digital at the band's FT8 DIAL (band_digital_mhz — NOT band
    // centre: 80m centre 3.600 sits in General's no-privilege gap). `Open` ⇒ tx_allowed true;
    // General/Extra pass all HF digital, so only restricted classes change. Unparseable band ⇒
    // keep (fail-open, matching get_all_spots' "never wrongly hide" posture).
    heard.retain(|h| {
        propagation::Band::from_label(&h.band).is_none_or(|b| {
            tempo_app::privileges::tx_allowed(
                license_class,
                propagation::band_digital_mhz(b),
                tempo_app::settings::OperatingMode::Digital,
            )
        })
    });
    // CW / Phone (all bands) AND digital on HF come from the DX-cluster + RBN spot
    // firehose, which carries an EXACT frequency (→ click-to-work can QSY to the spot).
    // For a digital HF need the near-region PSKR census now also covers 40/20/17/15/12m
    // (hf_region_topics), so the cluster path here is partly redundant with it — but the
    // cluster still adds the EXACT freq (PSKR is band-level) and every human-node CW/Phone
    // spot. Digital is admitted on HF ONLY here; VHF digital stays out (VHF locality is gated
    // above and Es/MS digital adds noise). The frontend's mode gating still hides CW/Phone
    // rows unless those features are on, so a digital board gains only HF digital. Both the
    // PSKR paths (gated just above) and these cluster spots (gated per-spot below) honor
    // tx_allowed, so a restricted class never sees an un-workable row.
    if let Ok(buf) = spots.lock() {
        let recent = buf.recent_within(
            std::time::Instant::now(),
            std::time::Duration::from_secs(900),
        );
        for cs in recent {
            let freq = cs.freq_mhz();
            let Some(band) = propagation::Band::from_mhz(freq) else {
                continue; // off the band plan → skip
            };
            // VHF locality gate (weak-signal-sleuth principle): a 6m/4m/2m cluster
            // spot is only WORKABLE-FROM-HERE evidence when the SPOTTER is inside
            // the operator's Es-patch radius — a Florida skimmer hearing a 6 m CW
            // beacon must never become a Wisconsin "contact to work". Applied
            // BEFORE the mode-class filter so CW and SSB spots gate identically;
            // spotters with no known grid can't prove locality → dropped on VHF.
            // HF keeps the continent-wide cluster (F2 footprints span it).
            if band.is_vhf() {
                // ALL independent voices for this DX (current spotter + the
                // spotters whose earlier reports the buffer's dedup replaced):
                // count how many are inside the Es-patch radius. VHF needs >= 2
                // — the PSKR path has required two near receivers all along,
                // but a single near RBN skimmer could still sneak a 6 m CW spot
                // through here (the last uncorroborated hole, the 4U1UN case).
                let near_spotters: Vec<&str> = std::iter::once(cs.spotter.as_str())
                    .chain(cs.corroborators.iter().map(|c| c.as_str()))
                    .filter(|sp| match (propagation::skimmer_grid(sp), me_ll) {
                        (Some(g), Some(me)) => propagation::geo::maidenhead_to_latlon(g)
                            .is_some_and(|rx| {
                                propagation::geo::haversine_km(me, rx)
                                    <= propagation::near_me_radius_km(band)
                            }),
                        _ => false,
                    })
                    .collect();
                if near_spotters.len() < 2 {
                    continue;
                }
                // …and the DX must be propagation-FAR, not a groundwave local (the
                // 6 m CQ machine 50 km away is spotted by every nearby skimmer,
                // opening or not). Cluster spots carry no grid, so judge by DXCC:
                // a different entity is DX by definition; same-entity falls back to
                // the centroid distance (coarse, but US locals resolve to the same
                // ~country centroid as the operator and correctly read "near").
                let dx_far = match (propagation::dxcc::resolve(&cs.dx_call), me_ll) {
                    (Some(info), Some(me)) => {
                        let d = propagation::geo::haversine_km(me, (info.lat, info.lon));
                        let my_entity = propagation::dxcc::resolve(&snap.mycall).map(|i| i.entity);
                        let far_enough =
                            my_entity != Some(info.entity) || d >= propagation::VHF_MIN_DX_KM;
                        // …and NOT past the terrestrial ceiling: on 2 m/4 m anything beyond
                        // single-hop Es (~2400 km) reaches only via EME, which a nearby
                        // big-gun's spot doesn't make workable from an ordinary station.
                        let within_terrestrial = propagation::vhf_max_terrestrial_km(band)
                            .is_none_or(|max| d <= max);
                        far_enough && within_terrestrial
                    }
                    _ => false,
                };
                if !dx_far {
                    continue;
                }
            }
            let class = propagation::classify_spot_mode(freq);
            // PRIVILEGE GATE (operator 2026-07-22, "LU6HL on 7.140 shouldn't show"): a station
            // on a frequency the operator may not TRANSMIT to is not a need — the Needed board is
            // a "work these" list, and 7.140 SSB sits in the Extra-only 40 m phone segment a
            // General may not use. SpotsPanel already gates its firehose on the same tables; the
            // Needed board never did, which is the leak. Open (non-US) short-circuits to allowed.
            let op_mode = match class {
                propagation::ModeClass::Cw => tempo_app::settings::OperatingMode::Cw,
                propagation::ModeClass::Phone => tempo_app::settings::OperatingMode::Phone,
                _ => tempo_app::settings::OperatingMode::Digital,
            };
            if !tempo_app::privileges::tx_allowed(license_class, freq, op_mode) {
                continue;
            }
            // CW/Phone on any band; digital on HF only (the missing HF evidence path
            // for a digital op). The need-matcher is demand-driven, so a busy HF FT8
            // firehose only surfaces the stations the operator actually NEEDS.
            let surface = matches!(
                class,
                propagation::ModeClass::Cw | propagation::ModeClass::Phone
            ) || (!band.is_vhf() && matches!(class, propagation::ModeClass::Digital));
            if surface {
                // Evidence line: who spotted it (RBN/cluster path). Cluster
                // lines age out of the buffer at 15 min, so "recently" is the
                // honest stamp without per-spot wall-clock plumbing.
                // push_at's filter guarantees spotter ∉ corroborators (and no
                // dupes within) — no re-dedup needed here.
                let spotters: Vec<&str> = std::iter::once(cs.spotter.as_str())
                    .chain(cs.corroborators.iter().map(|c| c.as_str()))
                    .take(3)
                    .collect();
                heard.push(propagation::Heard {
                    call: cs.dx_call.to_ascii_uppercase(),
                    band: band.label().to_string(),
                    // The surface gate above used `class` (the frequency-derived class,
                    // still Digital for RTTY). Here we only UPGRADE the display/routing
                    // label of an RBN RTTY skimmer spot — the machine-generated wire's
                    // leading mode token, trusted like the OTA feed. Everything else keeps
                    // its frequency-derived class label.
                    mode: if cs.is_rbn_rtty() {
                        "RTTY".to_string()
                    } else {
                        class.label().to_string()
                    },
                    freq_mhz: Some(freq),
                    // The spot's REAL receive time — stamping poll-time made
                    // every cluster row read "just now" forever.
                    admitted_at: (cs.received_unix > 0).then_some(cs.received_unix as i64),
                    evidence: Some(format!(
                        "spotted by {} via cluster/RBN",
                        spotters.join(" + ")
                    )),
                    grid: None,     // cluster/RBN spots carry no grid
                    us_state: None, // no grid → no state hint from a cluster spot
                });
            }
        }
    }
    let mut alerts = propagation::rank_needs(&heard, &needs, &needs.slots());
    // Never alert on the operator's own call (their PSKR "heard me" echoes can
    // otherwise surface it as a phantom row).
    let me_up = snap.mycall.to_uppercase();
    alerts.retain(|a| a.call != me_up);
    // DXpedition tagging: a heard call that belongs to an ACTIVE announced
    // expedition gets the Dxped chip + a priority nudge — limited-time windows
    // must be findable at a glance on the board. APPENDED (never tags[0]) so the
    // award tier keeps driving the row color. Reads the lock-only cached plan list
    // (warmed by a startup primer + every prop refresh) — NOT the PropCache, which
    // is only populated once the operator visits Connect/DXpeditions. The match is
    // suffix/prefix-tolerant ("3Y0J/MM" still tags as 3Y0J).
    let active = propagation::live::dxped::cached_active_calls(now_unix() as i64);
    if !active.is_empty() {
        for a in &mut alerts {
            let call = a.call.to_uppercase();
            if active
                .iter()
                .any(|act| propagation::live::dxped::call_matches(act, &call))
            {
                a.tags.push(propagation::NeedTag::Dxped);
                // +15 floats expedition rows up WITHIN their award tier without ever
                // crossing into the next one (tier floors are 10/30/50/70/100 — the
                // smallest gap is 20).
                a.priority += 15;
                a.headline = format!("{} · active DXpedition", a.headline);
            }
        }
        alerts.sort_by(|x, y| y.priority.cmp(&x.priority));
    }
    // POTA/SOTA tagging: a heard call that is a LIVE activator (per the hunter
    // feed's cache, <= 10 min fresh) gets the program chip — park chasers spot
    // them on the board at a glance. Appended like Dxped; no priority change
    // (a park is a park — the award tier still drives the row).
    if let Ok(cache) = ota_cache.lock() {
        // All fresh programs' activators (POTA + SOTA when both are polled).
        let spots: Vec<&propagation::OtaSpot> = cache
            .values()
            .filter(|(stamp, _)| now_unix().saturating_sub(*stamp) <= 600)
            .flat_map(|(_, v)| v.iter())
            .collect();
        if !spots.is_empty() {
            for a in &mut alerts {
                if let Some(sp) = spots
                    .iter()
                    // Base-call match: the spot says K1ABC, the decode may say
                    // K1ABC/P — the suffix is exactly the portable case a park
                    // chaser cares about.
                    .find(|sp| tempo_core::message::same_call(&sp.activator, &a.call))
                {
                    let tag = if sp.program.eq_ignore_ascii_case("SOTA") {
                        propagation::NeedTag::Sota
                    } else {
                        propagation::NeedTag::Pota
                    };
                    if !a.tags.contains(&tag) {
                        a.tags.push(tag);
                        a.headline = format!("{} · {} {}", a.headline, sp.program, sp.reference);
                    }
                }
            }
        }
    }
    // Feed LIVE POTA/SOTA activators onto the board as chase opportunities in their OWN
    // right — not merely as chips on a coincidentally-cluster-spotted station. Most park
    // activators live on the POTA/SOTA networks and never hit the DX cluster, so without
    // this they'd only ever appear in the dedicated hunter panel. Each currently-active
    // activator (fresh poller + recent spot time) that isn't already a row for the same
    // (call, band, mode) is scored via `activation_alert` (any DX award it also satisfies
    // is merged, so a new-entity park outranks a domestic one) and appended.
    const OTA_ACTIVE_SECS: i64 = 3600; // an activation counts as "current" for ~1 h
    if let Ok(cache) = ota_cache.lock() {
        let now = now_unix();
        let fresh: Vec<propagation::OtaSpot> = cache
            .values()
            // Cache STAMP proves the poller is alive; per-spot TIME proves the activation
            // itself is current (SOTA's count-based feed can carry stale rows).
            .filter(|(stamp, _)| now.saturating_sub(*stamp) <= 900)
            .flat_map(|(_, v)| v.iter().cloned())
            .filter(|sp| {
                sp.spot_time_unix
                    .is_none_or(|t| now.saturating_sub(t) <= OTA_ACTIVE_SECS)
            })
            .collect();
        drop(cache);
        for sp in &fresh {
            let Some(alert) = propagation::activation_alert(sp, &needs, &needs.slots()) else {
                continue;
            };
            if alert.call == me_up {
                continue; // never chase yourself
            }
            // Skip if the cluster path already produced this row — the tag loop above has
            // decorated it with the P/S chip + reference.
            if alerts
                .iter()
                .any(|a| a.call == alert.call && a.band == alert.band && a.mode == alert.mode)
            {
                continue;
            }
            alerts.push(alert);
        }
        // Re-sort: an activation that is ALSO a new one must land among the new ones.
        alerts.sort_by(|x, y| y.priority.cmp(&x.priority));
    }
    // Wanted watch list (W1.5): a station on the operator's list must top the
    // board even if it advances no award. This aggregated needs path carries no
    // per-spot CQ status or SNR, so the cq_only/min_snr gates can't be honored
    // here — the operator-facing controls for them are intentionally not shipped
    // (only wanted_calls). We pass is_cq=true / snr=None so every watch-list hit
    // surfaces; `wanted_match`/`wanted_alert` treat unknown SNR as passing.
    if !wanted_calls.is_empty() {
        let wcfg = propagation::WantedConfig {
            calls: &wanted_calls,
            cq_only: false,
            min_snr: None,
        };
        // (a) Decorate existing rows that are on the watch list — loud, on top.
        for a in &mut alerts {
            if !a.tags.contains(&propagation::NeedTag::Wanted)
                && propagation::wanted_match(&a.call, true, None, &wcfg)
            {
                a.tags.insert(0, propagation::NeedTag::Wanted);
                a.priority = a.priority.max(120);
                a.headline = format!("Wanted · {}", a.headline);
            }
        }
        // (b) Surface a loud row for a wanted heard station that produced no
        //     alert (an already-worked entity you still want to catch).
        for h in &heard {
            let up = h.call.to_ascii_uppercase();
            if up == me_up || alerts.iter().any(|a| a.call == up && a.band == h.band) {
                continue;
            }
            if let Some(mut a) = propagation::wanted_alert(
                &h.call,
                &h.band,
                &h.mode,
                h.grid.as_deref(),
                true,
                None,
                &wcfg,
                &needs,
                &needs.slots(),
            ) {
                // wanted_alert doesn't know the spot metadata — carry it over.
                a.freq_mhz = h.freq_mhz;
                a.admitted_at = h.admitted_at;
                a.evidence = h.evidence.clone();
                alerts.push(a);
            }
        }
        alerts.sort_by(|x, y| y.priority.cmp(&x.priority));
    }
    Ok(alerts)
}

/// Import an external ADIF logbook (deduped merge → real "needs"). Takes the
/// file's text; the UI reads the file so no fs/dialog plugin is needed.
#[tauri::command]
fn import_adif(state: State<'_, SharedEngine>, text: String) -> Result<ImportStats, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let (added, skipped, total) = eng.import_adif(&text);
    Ok(ImportStats {
        added,
        skipped,
        total,
    })
}

/// Reconcile a confirmation/credit report (LoTW ADIF export) INTO the existing
/// log — upgrades confirmation + credit on already-logged QSOs (which a plain
/// import would skip), rewrites the log file, and returns the diff + any
/// confirmations that matched no logged QSO. Offline; the live LoTW download is a
/// later increment that feeds this the same ADIF.
#[tauri::command]
async fn sync_lotw_report(
    state: State<'_, SharedEngine>,
    text: String,
) -> Result<LotwSyncResult, String> {
    conn_logged(
        "LoTW",
        |r: &LotwSyncResult| {
            format!(
                "file sync OK — {} newly confirmed, {} credited",
                r.newly_confirmed, r.newly_credited
            )
        },
        (|| {
            let mut eng = state.lock().map_err(|e| e.to_string())?;
            Ok(eng.merge_lotw_report(&text).into())
        })(),
    )
}

// ----- LoTW credential vault + authenticated download -----------------------
// The LoTW *website* password lives in the OS keychain (Windows Credential
// Manager / macOS Keychain / Linux Secret Service), never in settings.json or a
// log. The username + the incremental high-water cursor are non-secret and live
// in Settings.

const LOTW_KEYCHAIN_SERVICE: &str = "tempo";
const LOTW_KEYCHAIN_USER: &str = "lotw-password";
const EQSL_KEYCHAIN_USER: &str = "eqsl-password";
const QRZ_KEYCHAIN_USER: &str = "qrz-password";
const QRZ_LOGBOOK_KEYCHAIN_USER: &str = "qrz-logbook-key";
const HAMQTH_KEYCHAIN_USER: &str = "hamqth-password";
const CLUBLOG_KEYCHAIN_USER: &str = "clublog-password";
const HRDLOG_KEYCHAIN_USER: &str = "hrdlog-code";
const CLOUDLOG_KEYCHAIN_USER: &str = "cloudlog-key";

/// Client name Nexus sends to HRDLog.net's `NewEntry.aspx` as `App` (aids their
/// support / usage stats). Non-secret.
const HRDLOG_APP_NAME: &str = "Nexus";

/// Session-level kill-switch for ClubLog auto-push: set on a 403 (bad creds) so we
/// stop re-POSTing every QSO (ClubLog IP-blocks repeated auth failures); reset when
/// the operator changes a ClubLog credential.
static CLUBLOG_SUSPENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One connectivity event for the Settings ▸ Connections log — the answer to
/// "I hit save / it synced / it failed and I couldn't tell". Every connector
/// action (credential save, login, download, push, feed start/stop, rejection)
/// records one of these; the UI shows the rolling tail.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnEvent {
    ts_unix: i64,
    connector: String,
    /// "ok" | "info" | "error"
    level: String,
    message: String,
}

static CONN_LOG: Mutex<std::collections::VecDeque<ConnEvent>> =
    Mutex::new(std::collections::VecDeque::new());
const CONN_LOG_CAP: usize = 200;

/// Record a connectivity event (and mirror it to stderr for dev logs).
fn conn_log(connector: &str, level: &str, message: impl Into<String>) {
    let message = message.into();
    eprintln!("conn[{connector}/{level}]: {message}");
    let mut log = CONN_LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.push_back(ConnEvent {
        ts_unix: now_unix() as i64,
        connector: connector.to_string(),
        level: level.to_string(),
        message,
    });
    while log.len() > CONN_LOG_CAP {
        log.pop_front();
    }
}

/// Wrap a connector operation result into the connection log: success and
/// failure BOTH become visible events (the operator could previously tell
/// neither). Returns the result unchanged.
fn conn_logged<T>(
    connector: &str,
    ok_msg: impl FnOnce(&T) -> String,
    r: Result<T, String>,
) -> Result<T, String> {
    match &r {
        Ok(v) => conn_log(connector, "ok", ok_msg(v)),
        Err(e) => conn_log(connector, "error", e.clone()),
    }
    r
}

/// The rolling connectivity log, newest first.
#[tauri::command]
fn get_connection_log() -> Vec<ConnEvent> {
    let log = CONN_LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.iter().rev().cloned().collect()
}

/// Which credentials are PRESENT (stored) per connector — so the operator can
/// finally SEE that a save took. Never returns the secrets themselves.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CredStatus {
    connector: String,
    /// A secret exists in the OS keychain (or key field) for this connector.
    stored: bool,
    /// The associated non-secret identity (username/email), for display.
    identity: String,
}

#[tauri::command]
fn get_credentials_status(state: State<'_, SharedEngine>) -> Result<Vec<CredStatus>, String> {
    let (lotw_user, eqsl_user, qrz_user, clublog_email, mycall, clublog_key, cloudlog_url) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (
            st.lotw_username.clone(),
            st.eqsl_username.clone(),
            st.qrz_username.clone(),
            st.clublog_email.clone(),
            st.mycall.clone(),
            // The API key is an APPLICATION (developer) credential, not a user
            // one: a Settings override OR the key baked into official installer
            // builds (CLUBLOG_API_KEY at build time) both satisfy it. The user's
            // own credentials are only email + app-password.
            !effective_clublog_key(&st.clublog_api_key).is_empty(),
            st.cloudlog_url.clone(),
        )
    };
    let has = |entry: Result<keyring::Entry, String>| {
        entry
            .and_then(|e| e.get_password().map_err(|er| er.to_string()))
            .is_ok()
    };
    Ok(vec![
        CredStatus {
            connector: "LoTW".into(),
            stored: has(lotw_keychain()),
            identity: lotw_user,
        },
        CredStatus {
            connector: "QRZ callbook (name/QTH)".into(),
            stored: has(qrz_keychain()),
            identity: qrz_user.clone(),
        },
        CredStatus {
            connector: "QRZ Logbook".into(),
            stored: has(qrz_logbook_keychain()),
            identity: qrz_user,
        },
        CredStatus {
            connector: "eQSL".into(),
            stored: has(eqsl_keychain()),
            identity: eqsl_user,
        },
        CredStatus {
            connector: "ClubLog".into(),
            stored: has(clublog_keychain()) && clublog_key,
            identity: clublog_email,
        },
        CredStatus {
            // The station callsign IS the HRDLog identity (the upload code is the
            // only secret); show mycall so a save is visibly attributed.
            connector: "HRDLog.net".into(),
            stored: has(hrdlog_keychain()),
            identity: mycall,
        },
        CredStatus {
            connector: "Cloudlog".into(),
            stored: has(cloudlog_keychain()),
            identity: cloudlog_url,
        },
        CredStatus {
            // The token is app-scoped (an rbuapp_… token the user generates on
            // RepeaterBook); there's no separate username identity to show.
            connector: "RepeaterBook".into(),
            stored: has(repeaterbook_keychain()),
            identity: String::new(),
        },
    ])
}

/// The effective ClubLog **application** key: the Settings override when set,
/// else the key baked into official installer builds at compile time
/// (`CLUBLOG_API_KEY`, see build.rs). Empty = this build has no key (source
/// build without the env var) and ClubLog features that need one are off.
fn effective_clublog_key(settings_key: &str) -> String {
    let k = settings_key.trim();
    if !k.is_empty() {
        return k.to_string();
    }
    option_env!("CLUBLOG_API_KEY")
        .unwrap_or("")
        .trim()
        .to_string()
}

fn lotw_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, LOTW_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn eqsl_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, EQSL_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn qrz_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, QRZ_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn qrz_logbook_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, QRZ_LOGBOOK_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn hamqth_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, HAMQTH_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn clublog_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, CLUBLOG_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn cloudlog_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, CLOUDLOG_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

fn hrdlog_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, HRDLOG_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

/// Delete a keychain entry idempotently — a missing entry counts as success
/// (nothing to forget). Shared by every credentialed connector's clear command.
fn clear_keychain_entry(entry: &keyring::Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("couldn't clear the system keychain: {e}")),
    }
}

/// Store (or, if `password` is empty, clear) the LoTW website password in the OS
/// keychain. Write-only: the password is never read back to the UI.
#[tauri::command]
fn set_lotw_password(password: String) -> Result<(), String> {
    let entry = lotw_keychain()?;
    if password.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log("LoTW", "info", "password cleared from the OS keychain");
        return Ok(());
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("LoTW", "ok", "password saved to the OS keychain");
    Ok(())
}

/// Remove the stored LoTW password from the OS keychain (idempotent).
#[tauri::command]
fn clear_lotw_password() -> Result<(), String> {
    let r = clear_keychain_entry(&lotw_keychain()?);
    if r.is_ok() {
        conn_log("LoTW", "info", "password cleared from the OS keychain");
    }
    r
}

/// Store (or, if empty, clear) the eQSL website password in the OS keychain.
/// Write-only, like the LoTW counterpart. Saving also switches eQSL auto-upload
/// ON (entering the credential is the intent).
#[tauri::command]
fn set_eqsl_password(password: String, state: State<'_, SharedEngine>) -> Result<(), String> {
    let entry = eqsl_keychain()?;
    if password.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log("eQSL", "info", "password cleared from the OS keychain");
        set_upload_toggle(&state, UploadToggle::Eqsl, false);
        return Ok(());
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("eQSL", "ok", "password saved to the OS keychain");
    set_upload_toggle(&state, UploadToggle::Eqsl, true);
    Ok(())
}

/// Remove the stored eQSL password from the OS keychain (idempotent).
#[tauri::command]
fn clear_eqsl_password(state: State<'_, SharedEngine>) -> Result<(), String> {
    let r = clear_keychain_entry(&eqsl_keychain()?);
    if r.is_ok() {
        conn_log("eQSL", "info", "password cleared from the OS keychain");
        set_upload_toggle(&state, UploadToggle::Eqsl, false);
    }
    r
}

/// Store (or, if empty, clear) the QRZ.com account password in the OS keychain.
/// Write-only, like the LoTW/eQSL counterparts.
#[tauri::command]
fn set_qrz_password(
    password: String,
    qrz_session: State<'_, SharedQrzSession>,
) -> Result<(), String> {
    // A credential change invalidates the cached XML session key — a stale key
    // kept working under the OLD identity until it expired server-side.
    if let Ok(mut g) = qrz_session.lock() {
        *g = None;
    }
    let entry = qrz_keychain()?;
    if password.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log("QRZ XML", "info", "password cleared from the OS keychain");
        return Ok(());
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("QRZ XML", "ok", "password saved to the OS keychain");
    Ok(())
}

/// Remove the stored QRZ password from the OS keychain (idempotent).
#[tauri::command]
fn clear_qrz_password() -> Result<(), String> {
    let r = clear_keychain_entry(&qrz_keychain()?);
    if r.is_ok() {
        conn_log("QRZ XML", "info", "password cleared from the OS keychain");
    }
    r
}

/// Store (or, if empty, clear) the HamQTH.com account password in the OS keychain.
/// Write-only, like the QRZ counterpart — the free-fallback callbook password.
#[tauri::command]
fn set_hamqth_password(
    password: String,
    hamqth_session: State<'_, SharedHamQthSession>,
) -> Result<(), String> {
    // A credential change invalidates the cached XML session id — a stale id kept
    // working under the OLD identity until it expired server-side.
    if let Ok(mut g) = hamqth_session.0.lock() {
        *g = None;
    }
    let entry = hamqth_keychain()?;
    if password.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log("HamQTH", "info", "password cleared from the OS keychain");
        return Ok(());
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("HamQTH", "ok", "password saved to the OS keychain");
    Ok(())
}

/// Remove the stored HamQTH password from the OS keychain (idempotent).
#[tauri::command]
fn clear_hamqth_password() -> Result<(), String> {
    let r = clear_keychain_entry(&hamqth_keychain()?);
    if r.is_ok() {
        conn_log("HamQTH", "info", "password cleared from the OS keychain");
    }
    r
}

/// Store (or, if empty, clear) the QRZ **Logbook API key** (distinct from the XML
/// password) in the OS keychain. Write-only. Saving a key also switches QRZ
/// auto-upload ON: entering the key IS the intent ("upload my QSOs to QRZ") —
/// previously the separate toggle silently stayed off and nothing uploaded.
#[tauri::command]
fn set_qrz_logbook_key(key: String, state: State<'_, SharedEngine>) -> Result<(), String> {
    let entry = qrz_logbook_keychain()?;
    if key.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log(
            "QRZ Logbook",
            "info",
            "API key cleared from the OS keychain",
        );
        set_upload_toggle(&state, UploadToggle::Qrz, false);
        return Ok(());
    }
    entry
        .set_password(&key)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("QRZ Logbook", "ok", "API key saved to the OS keychain");
    set_upload_toggle(&state, UploadToggle::Qrz, true);
    Ok(())
}

/// Which connector's auto-upload toggle to flip alongside its credential.
#[derive(Clone, Copy)]
enum UploadToggle {
    Qrz,
    Clublog,
    Eqsl,
    Hrdlog,
}

/// Flip a connector's auto-upload toggle (persisted) when its credential
/// changes: saving turns it ON (entering the credential IS the intent — a dead
/// toggle was exactly how "creds in place but nothing uploads" happened);
/// clearing turns it OFF (so the upload worker doesn't error-toast every QSO
/// against a credential that's gone). Uses the lightweight settings mutation,
/// NEVER `apply_settings` — that resets the operating mode and drops queued TX,
/// and a credential save mid-QSO must not kill the QSO. No-op when already in
/// the requested state.
fn set_upload_toggle(state: &State<'_, SharedEngine>, which: UploadToggle, on: bool) {
    if let Ok(mut eng) = state.lock() {
        let (connector, already) = {
            let s = eng.settings();
            match which {
                UploadToggle::Qrz => ("QRZ Logbook", s.qrz_logbook_upload),
                UploadToggle::Clublog => ("ClubLog", s.clublog_upload),
                UploadToggle::Eqsl => ("eQSL", s.eqsl_upload),
                UploadToggle::Hrdlog => ("HRDLog.net", s.hrdlog_upload),
            }
        };
        if already == on {
            return;
        }
        let updated = match which {
            UploadToggle::Qrz => eng.set_upload_toggles(Some(on), None, None),
            UploadToggle::Clublog => eng.set_upload_toggles(None, Some(on), None),
            UploadToggle::Eqsl => eng.set_upload_toggles(None, None, Some(on)),
            UploadToggle::Hrdlog => eng.set_hrdlog_upload(on),
        };
        if let Err(e) = updated.save(&settings_path()) {
            eprintln!("tempo: couldn't persist settings: {e}");
        }
        conn_log(
            connector,
            "info",
            if on {
                "auto-upload on log ENABLED (credential saved — turn off in Settings if unwanted)"
            } else {
                "auto-upload on log disabled (credential cleared)"
            },
        );
    }
}

/// Remove the stored QRZ Logbook API key from the OS keychain (idempotent).
#[tauri::command]
fn clear_qrz_logbook_key(state: State<'_, SharedEngine>) -> Result<(), String> {
    let r = clear_keychain_entry(&qrz_logbook_keychain()?);
    if r.is_ok() {
        conn_log(
            "QRZ Logbook",
            "info",
            "API key cleared from the OS keychain",
        );
        set_upload_toggle(&state, UploadToggle::Qrz, false);
    }
    r
}

/// Store (or, if empty, clear) the Cloudlog/Wavelog instance API key in the OS
/// keychain — write-only, kept out of settings.json at rest like every other
/// credential. Unlike QRZ, saving does NOT flip the upload toggle: Cloudlog also
/// needs a URL + station-profile id, so the operator enables it explicitly.
#[tauri::command]
fn set_cloudlog_key(key: String) -> Result<(), String> {
    let entry = cloudlog_keychain()?;
    if key.trim().is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log("Cloudlog", "info", "API key cleared from the OS keychain");
        return Ok(());
    }
    entry
        .set_password(key.trim())
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("Cloudlog", "ok", "API key saved to the OS keychain");
    Ok(())
}

/// Remove the stored Cloudlog/Wavelog API key from the OS keychain (idempotent).
#[tauri::command]
fn clear_cloudlog_key() -> Result<(), String> {
    let r = clear_keychain_entry(&cloudlog_keychain()?);
    if r.is_ok() {
        conn_log("Cloudlog", "info", "API key cleared from the OS keychain");
    }
    r
}

/// `RcvdSince` safety margin: eQSL does not document the timezone of this filter,
/// so we roll the cursor back ≥24 h to guarantee the request window overlaps the
/// server's true boundary regardless of its zone. The idempotent reconcile absorbs
/// the resulting re-pull overlap.
const EQSL_RCVD_MARGIN_SECS: i64 = 86_400;

/// Download new LoTW confirmations and reconcile them into the log.
///
/// Reads the LoTW username + incremental cursor from settings and the password
/// from the keychain, fetches the report since the stored high-water, validates
/// it, merges it via the same reconcile path as a pasted report, and advances the
/// cursor — but only if the response carried a new high-water (an empty
/// incremental response has none, so the cursor is preserved, not wiped). The
/// password-bearing request URL is never logged or surfaced.
/// True iff a LoTW report body is structurally **complete** — it carries the
/// documented `<APP_LoTW_EOF>` end-of-file trailer (case-insensitive). LoTW appends
/// this marker to every report (the same one Cloudlog validates); a body that
/// HTTP-200s but was cut off mid-stream (partial server-side generation, an
/// EOF-delimited/proxied response that "completes" cleanly at the transport layer)
/// lacks it. Mirrors `tempo_core::eqsl::is_complete_eqsl_body`: a truncated download
/// must not let the sync cursor advance over records it never received.
fn is_complete_lotw_body(body: &str) -> bool {
    body.to_ascii_lowercase().contains("<app_lotw_eof>")
}

#[tauri::command]
fn download_lotw_report(state: State<'_, SharedEngine>) -> Result<LotwSyncResult, String> {
    conn_logged(
        "LoTW",
        |r| {
            format!(
                "sync OK — {} newly confirmed, {} credited, {} promoted",
                r.newly_confirmed, r.newly_credited, r.promoted
            )
        },
        download_lotw_report_impl(state),
    )
}

fn download_lotw_report_impl(state: State<'_, SharedEngine>) -> Result<LotwSyncResult, String> {
    // Read username + cursor (non-secret) under a brief lock; the network fetch
    // below must NOT hold the engine lock (it can block for up to 60 s).
    let (username, owncall, since) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.lotw_username.trim().to_string(),
            s.mycall.trim().to_string(),
            s.lotw_last_qsl.trim().to_string(),
        )
    };
    if username.is_empty() {
        return Err("Set your LoTW username in Settings first.".to_string());
    }
    let password = lotw_keychain()?
        .get_password()
        .map_err(|_| "No LoTW password stored — set it in Settings.".to_string())?;
    let used_username = username.clone(); // for the post-fetch cursor-binding guard
    let owncall = Some(owncall).filter(|c| !c.is_empty());

    // --- Pull 1: confirmations (qso_qsl=yes, incremental via the cursor). ---
    // The password stays in scope for the second (own-echo) pull below; never log it.
    let body = {
        let query = tempo_core::lotw::LotwQuery {
            username: username.clone(),
            password: password.clone(),
            owncall: owncall.clone(),
            qsl_since: Some(since).filter(|c| !c.is_empty()),
        };
        let url = tempo_core::lotw::build_report_url(&query);
        propagation::live::lotw::fetch_report(&url)?
    }; // `query` + `url` (both hold the password) dropped here

    if !tempo_core::lotw::is_lotw_adif(&body) {
        return Err(
            "LoTW returned an unexpected response — check your username/password.".to_string(),
        );
    }

    // Merge via the shared reconcile path, then advance the cursor only on a real
    // high-water (re-lock: the fetch ran without the engine lock held). Capture the
    // own-echo lower bound (oldest in-flight upload) in the same lock, then release.
    let (mut result, own_start): (LotwSyncResult, Option<String>) = {
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        let summary: LotwSyncResult = eng.merge_lotw_report(&body).into();
        if let Some(high_water) = tempo_core::lotw::extract_last_qsl(&body) {
            // Advance the cursor ONLY if (a) the download is structurally complete —
            // a truncated-but-HTTP-200 body lacks the `<APP_LoTW_EOF>` trailer, and
            // every confirmation cut off in its tail carries qsl-date <= LASTQSL, so
            // advancing would make the next `qso_qslsince` pull skip them forever (the
            // merge above already ran, so keeping the old cursor just re-fetches the
            // tail — reconcile is idempotent) — AND (b) the username is still the one
            // this download used. If `set_settings` changed it during the (lock-free)
            // fetch, it already reset the cursor to a full pull for the new identity —
            // this high-water belongs to the old query, so binding it would risk
            // skipping records on the next incremental pull. Persist via a narrow
            // setter so the sync never disturbs live operation (no mode reset /
            // TX-queue clear).
            if is_complete_lotw_body(&body)
                && eng.settings().lotw_username.trim() == used_username.trim()
            {
                let updated = eng.set_lotw_cursor(high_water);
                if let Err(e) = updated.save(&settings_path()) {
                    conn_log(
                        "LoTW",
                        "error",
                        format!("failed to persist the sync cursor: {e}"),
                    );
                }
            }
        }
        let own_start = eng.oldest_pending_lotw_date();
        (summary, own_start)
    }; // engine lock released before the second network fetch

    // --- Pull 2: own-echo (qso_qsl=no) — promote in-flight uploads to Accepted. ---
    // Best-effort: only run when something is actually in flight, and never fail the
    // whole sync (the confirmations above already merged) on an own-echo hiccup.
    if let Some(start) = own_start {
        let own_body = {
            let query = tempo_core::lotw::LotwQuery {
                username,
                password,
                owncall,
                qsl_since: None,
            };
            let url = tempo_core::lotw::build_own_report_url(&query, Some(&start));
            propagation::live::lotw::fetch_report(&url)
        };
        match own_body {
            Ok(b) if tempo_core::lotw::is_lotw_adif(&b) => {
                let mut eng = state.lock().map_err(|e| e.to_string())?;
                result.promoted = eng.merge_lotw_own_echo(&b, now_unix());
            }
            Ok(_) => conn_log(
                "LoTW",
                "error",
                "own-echo pull returned a non-ADIF body; skipped",
            ),
            Err(e) => conn_log(
                "LoTW",
                "error",
                format!("own-echo pull failed (confirmations still synced): {e}"),
            ),
        }
    }

    Ok(result)
}

/// Locate the `tqsl` binary: a non-empty Settings override that exists, else the
/// first existing OS-default candidate, else the bare name on PATH (Command
/// resolves it; an ENOENT then drives the friendly "install TQSL" error).
fn resolve_tqsl(override_path: &str) -> std::path::PathBuf {
    let op = override_path.trim();
    if !op.is_empty() {
        let p = std::path::PathBuf::from(op);
        if p.exists() {
            return p;
        }
    }
    for cand in tempo_core::lotw_upload::tqsl_candidate_paths() {
        if cand.exists() {
            return cand;
        }
    }
    std::path::PathBuf::from(if cfg!(windows) { "tqsl.exe" } else { "tqsl" })
}

/// Sign + upload QSOs to LoTW via the operator's installed TQSL. `indices` selects
/// specific log rows; `None` = the default unsent-unconfirmed batch. No secret is
/// handled here — TQSL owns the Callsign Certificate; we pass only the non-secret
/// Station Location. Pre-flight (station location set, batch non-empty, TQSL
/// resolvable) BEFORE any state is stamped, so a missing tool never corrupts
/// upload state. The QSOs are stamped per the TQSL exit code (Pending/Duplicate/
/// Rejected/AuthFail; a network error leaves state untouched for a clean retry).
/// Mark every currently-unsent QSO as already on LoTW — the operator's declaration that an
/// imported legacy log was uploaded through another tool. Zeroes the "Upload to LoTW (N)" count
/// so a big redundant re-upload isn't offered. Returns how many were marked.
#[tauri::command]
fn mark_lotw_uploaded(state: State<'_, SharedEngine>) -> Result<usize, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.mark_lotw_uploaded_all())
}

#[tauri::command]
async fn upload_lotw_report(
    state: State<'_, SharedEngine>,
    indices: Option<Vec<usize>>,
) -> Result<UploadReportDto, String> {
    conn_logged(
        "LoTW",
        |r| format!("upload — {} QSO(s), outcome: {}", r.dispatched, r.outcome),
        upload_lotw_report_impl(state, indices).await,
    )
}

async fn upload_lotw_report_impl(
    state: State<'_, SharedEngine>,
    indices: Option<Vec<usize>>,
) -> Result<UploadReportDto, String> {
    // Brief lock: read config + build the batch + ADIF, then release before spawn.
    let (batch, adif, location, tqsl_path) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let use_adif_location = eng.settings().lotw_use_adif_location;
        let location = eng.settings().lotw_station_location.trim().to_string();
        // A named Station Location is required UNLESS the operator signs from the ADIF
        // (travelers who never create TQSL station locations).
        if location.is_empty() && !use_adif_location {
            return Err(
                "Set your LoTW Station Location in Settings before uploading (or enable \
                        \"sign from ADIF location\" if you don't create TQSL station locations)."
                    .into(),
            );
        }
        let batch = indices.unwrap_or_else(|| eng.lotw_unsent_indices());
        if batch.is_empty() {
            return Ok(UploadReportDto {
                dispatched: 0,
                outcome: "none".into(),
                detail: None,
            });
        }
        let adif = eng.lotw_upload_adif(&batch);
        let tqsl_path = eng.settings().tqsl_path.clone();
        // None in ADIF-location mode → tqsl_args omits `-l`.
        let location = (!use_adif_location).then_some(location);
        (batch, adif, location, tqsl_path)
    };

    // Write the batch ADIF to a temp file for TQSL to sign. Use a UNIQUE,
    // unpredictable name (not the old fixed `nexus_lotw_upload.adi` a co-tenant
    // could pre-symlink to hijack the write), create it O_EXCL so an existing path
    // fails rather than being followed, and remove it on every exit path — the file
    // holds the operator's log (PII). RAII guard = cleanup even on the `?` returns.
    struct TmpFile(std::path::PathBuf);
    impl Drop for TmpFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("nexus_lotw_{}_{nonce}.adi", std::process::id()));
    {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("Couldn't create the upload file: {e}"))?;
        f.write_all(adif.as_bytes())
            .map_err(|e| format!("Couldn't write the upload file: {e}"))?;
    }
    let _tmp_guard = TmpFile(path.clone());
    let path_str = path.to_string_lossy().to_string();

    // Resolve + run TQSL one-shot, capturing its result.
    let tqsl = resolve_tqsl(&tqsl_path);
    let args = tempo_core::lotw_upload::tqsl_args(location.as_deref(), &path_str);
    let mut cmd = std::process::Command::new(&tqsl);
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (TQSL is GUI-linked)
    }
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "TQSL isn't installed (or its path is wrong). LoTW uploads are signed locally by TQSL — install it from lotw.arrl.org, or set the TQSL path in Settings.".to_string()
        } else {
            format!("Couldn't run TQSL: {e}")
        }
    })?;
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = tempo_core::lotw_upload::sanitize_detail(&stderr);

    match tempo_core::lotw_upload::classify_tqsl_exit(code, &stderr) {
        // Network error → leave state untouched so the next attempt retries cleanly.
        None => Ok(UploadReportDto {
            dispatched: batch.len(),
            outcome: "retry".into(),
            detail: detail.or_else(|| Some("LoTW unreachable — try again shortly.".into())),
        }),
        Some(outcome) => {
            {
                let mut eng = state.lock().map_err(|e| e.to_string())?;
                eng.stamp_lotw_upload(&batch, outcome, now_unix(), detail.clone());
            }
            Ok(UploadReportDto {
                dispatched: batch.len(),
                outcome: outcome.code().to_string(),
                detail,
            })
        }
    }
}

/// Download new eQSL confirmations and reconcile them into the log.
///
/// Mirrors `download_lotw_report` but for eQSL's two-step InBox flow: reads the
/// eQSL username + cursor from settings + the password from the keychain, fetches
/// the InBox (HTML built-page → ephemeral `.adi`), validates it, and merges via the
/// same reconcile path. eQSL confirmations land `confirmed` but NOT `award_confirmed`
/// (they carry `EQSL_QSL_RCVD`), so they never credit ARRL DXCC/WAS. The
/// password-bearing request URL is never logged or surfaced.
#[tauri::command]
fn download_eqsl_report(state: State<'_, SharedEngine>) -> Result<LotwSyncResult, String> {
    conn_logged(
        "eQSL",
        |r| format!("inbox sync OK — {} newly confirmed", r.newly_confirmed),
        download_eqsl_report_impl(state),
    )
}

fn download_eqsl_report_impl(state: State<'_, SharedEngine>) -> Result<LotwSyncResult, String> {
    let (username, since) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.eqsl_username.trim().to_string(),
            s.eqsl_last_sync.trim().to_string(),
        )
    };
    if username.is_empty() {
        return Err("Set your eQSL username in Settings first.".to_string());
    }
    let password = eqsl_keychain()?
        .get_password()
        .map_err(|_| "No eQSL password stored — set it in Settings.".to_string())?;
    let used_username = username.clone();
    // Candidate next cursor: this sync's start, floored to the minute and rolled
    // back by the margin (eQSL's RcvdSince timezone is unstated — over-fetch so we
    // never skip records). Captured BEFORE the fetch so nothing arriving during it
    // is missed next time.
    let next_cursor = tempo_core::eqsl::format_rcvd_since(now_unix() - EQSL_RCVD_MARGIN_SECS);

    // Build the URL, fetch (two GETs), and drop the secret-bearing values after.
    let body = {
        let query = tempo_core::eqsl::EqslQuery {
            username,
            password,
            rcvd_since: Some(since).filter(|s| !s.is_empty()),
        };
        let url = tempo_core::eqsl::build_inbox_url(&query);
        propagation::live::eqsl::fetch_inbox(&url)?
    }; // `query` + `url` (both hold the password) dropped here

    if !tempo_core::eqsl::is_eqsl_adif(&body) {
        return Err(
            "eQSL returned an unexpected response — check your username/password.".to_string(),
        );
    }

    // Merge via the shared reconcile path (eQSL lands confirmed-not-award by
    // construction). Advance the cursor ONLY if (a) the body is structurally
    // complete — a truncated download must not skip unreceived records — AND (b) the
    // username is unchanged since this sync started (an in-flight change already
    // reset the cursor for the new account).
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let summary: LotwSyncResult = eng.merge_eqsl_report(&body).into();
    if tempo_core::eqsl::is_complete_eqsl_body(&body)
        && eng.settings().eqsl_username.trim() == used_username.trim()
    {
        let updated = eng.set_eqsl_cursor(next_cursor);
        if let Err(e) = updated.save(&settings_path()) {
            eprintln!("tempo: failed to persist eQSL cursor: {e}");
        }
    }
    Ok(summary)
}

/// Two-way QRZ Logbook sync: FETCH the operator's online QRZ logbook and merge it
/// INTO the local log. Pulls down QSOs logged elsewhere (e.g. a phone app in the
/// field) AND confirmation status. Uses the per-logbook API key (NOT the QRZ
/// password); the key-bearing request body is never logged. QRZ-native
/// confirmations land `confirmed` but NOT `award_confirmed`, so they can't inflate
/// DXCC/WAS counts.
#[tauri::command]
fn sync_qrz(state: State<'_, SharedEngine>) -> Result<LotwSyncResult, String> {
    conn_logged(
        "QRZ Logbook",
        |r: &LotwSyncResult| {
            format!(
                "sync OK — {} new QSOs, {} newly confirmed",
                r.added, r.newly_confirmed_any
            )
        },
        sync_qrz_impl(state),
    )
}

fn sync_qrz_impl(state: State<'_, SharedEngine>) -> Result<LotwSyncResult, String> {
    let key = qrz_logbook_keychain()?.get_password().map_err(|_| {
        "No QRZ Logbook API key stored — this is the per-logbook key from logbook.qrz.com \
         (Settings ▸ Logbook & QSL ▸ QRZ), NOT your QRZ password."
            .to_string()
    })?;
    // Build + send the FETCH; the body carries the key, so it's dropped right after.
    let resp = {
        let body = tempo_core::qrz::build_fetch_body(&key);
        propagation::live::qrz::post_form(tempo_core::qrz::QRZ_LOGBOOK_URL, body)?
    };
    let fetched = tempo_core::qrz::parse_fetch(&resp);
    if !fetched.ok {
        return Err(fetched
            .reason
            .unwrap_or_else(|| "QRZ rejected the FETCH — check your Logbook API key.".into()));
    }
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let (added, summary) = eng.merge_qrz_report(&fetched.adif);
    let mut result: LotwSyncResult = summary.into();
    result.added = added;
    Ok(result)
}

// ----- QRZ.com / HamQTH.com callsign lookup (session-key XML APIs) -----------

/// Outcome of one lookup attempt with a given session key/id. Shared by QRZ and its
/// HamQTH fallback — both flow into the same [`QrzLookupDto`](tempo_app::dto::QrzLookupDto).
enum QrzOutcome {
    Found(tempo_app::dto::QrzLookupDto),
    NotFound,
    NeedLogin, // the session key/id is expired/invalid → (re)login
}

/// One QRZ lookup with an existing session key (no login). Network only; holds no
/// lock. Errors are already redacted by the transport.
fn qrz_try_lookup(session_key: &str, callsign: &str) -> Result<QrzOutcome, String> {
    let url = tempo_core::qrz::build_lookup_url(session_key, callsign);
    let body = propagation::live::qrz::fetch(&url)?;
    if !tempo_core::qrz::is_qrz_xml(&body) {
        return Err("QRZ returned an unexpected response.".to_string());
    }
    if tempo_core::qrz::parse_session(&body).needs_login() {
        return Ok(QrzOutcome::NeedLogin);
    }
    Ok(match tempo_core::qrz::parse_callsign(&body) {
        Some(rec) => QrzOutcome::Found(rec.into()),
        None => QrzOutcome::NotFound,
    })
}

/// Log in to QRZ and return a fresh session key. The URL carries the password but
/// is local (dropped here); errors are redacted by the transport.
fn qrz_login(username: &str, password: &str) -> Result<String, String> {
    let url = tempo_core::qrz::build_login_url(&tempo_core::qrz::QrzLogin {
        username: username.to_string(),
        password: password.to_string(),
        agent: "nexus/0.1".to_string(),
    });
    let body = propagation::live::qrz::fetch(&url)?;
    if !tempo_core::qrz::is_qrz_xml(&body) {
        return Err("QRZ returned an unexpected response — check your credentials.".to_string());
    }
    let session = tempo_core::qrz::parse_session(&body);
    session.key.ok_or_else(|| {
        // QRZ's <Error> on a bad login (e.g. "Username/password incorrect") carries
        // no secret — surface it; else a generic message.
        session
            .error
            .map(|e| format!("QRZ login failed: {e}"))
            .unwrap_or_else(|| "QRZ login failed — check your username/password.".to_string())
    })
}

/// One HamQTH lookup with an existing session id (no login). The HamQTH mirror of
/// [`qrz_try_lookup`]. Network only; holds no lock; errors redacted by the transport.
fn hamqth_try_lookup(session_id: &str, callsign: &str) -> Result<QrzOutcome, String> {
    let url =
        tempo_core::hamqth::build_lookup_url(session_id, callsign, tempo_core::hamqth::HAMQTH_PRG);
    let body = propagation::live::hamqth::fetch(&url)?;
    if !tempo_core::hamqth::is_hamqth_xml(&body) {
        return Err("HamQTH returned an unexpected response.".to_string());
    }
    if tempo_core::hamqth::parse_session(&body).needs_login() {
        return Ok(QrzOutcome::NeedLogin);
    }
    Ok(match tempo_core::hamqth::parse_callsign(&body) {
        Some(rec) => QrzOutcome::Found(rec.into()),
        None => QrzOutcome::NotFound,
    })
}

/// Log in to HamQTH and return a fresh session id. The HamQTH mirror of [`qrz_login`];
/// the URL carries the password but is local (dropped here); errors redacted by the
/// transport. On a login response with no id, HamQTH's `<error>` says why.
fn hamqth_login(username: &str, password: &str) -> Result<String, String> {
    let url = tempo_core::hamqth::build_login_url(&tempo_core::hamqth::HamQthLogin {
        username: username.to_string(),
        password: password.to_string(),
    });
    let body = propagation::live::hamqth::fetch(&url)?;
    if !tempo_core::hamqth::is_hamqth_xml(&body) {
        return Err("HamQTH returned an unexpected response — check your credentials.".to_string());
    }
    let session = tempo_core::hamqth::parse_session(&body);
    session.session_id.ok_or_else(|| {
        // HamQTH's <error> on a bad login (e.g. "Wrong user name or password")
        // carries no secret — surface it; else a generic message.
        session
            .error
            .map(|e| format!("HamQTH login failed: {e}"))
            .unwrap_or_else(|| "HamQTH login failed — check your username/password.".to_string())
    })
}

/// One complete QRZ lookup pass: try the cached session key, and on expiry log in
/// **once** and retry (bounded — never loops). `Ok(Some(dto))` = a hit; `Ok(None)` =
/// QRZ has no record (so the caller can fall through to the HamQTH fallback); `Err` =
/// a transport/login error. Network runs without any lock held.
fn qrz_lookup_attempt(
    call: &str,
    username: &str,
    password: &str,
    qrz_session: &SharedQrzSession,
) -> Result<Option<tempo_app::dto::QrzLookupDto>, String> {
    // 1) Try the cached key, if any.
    let cached = qrz_session.lock().ok().and_then(|g| g.clone());
    if let Some(key) = cached {
        match qrz_try_lookup(&key, call)? {
            QrzOutcome::Found(dto) => return Ok(Some(dto)),
            QrzOutcome::NotFound => return Ok(None), // authoritative miss — don't re-login
            QrzOutcome::NeedLogin => {}              // fall through to a single re-login
        }
    }
    // 2) Log in once, cache the new key, retry the lookup once (bounded).
    let key = qrz_login(username, password)?;
    if let Ok(mut g) = qrz_session.lock() {
        *g = Some(key.clone());
    }
    match qrz_try_lookup(&key, call)? {
        QrzOutcome::Found(dto) => Ok(Some(dto)),
        QrzOutcome::NotFound => Ok(None),
        // A fresh key still reporting expiry is anomalous — give up (→ HamQTH fallback).
        QrzOutcome::NeedLogin => Ok(None),
    }
}

/// One complete HamQTH lookup pass — the free fallback, structurally identical to
/// [`qrz_lookup_attempt`]. `Ok(Some(dto))` = a hit; `Ok(None)` = no record; `Err` =
/// a transport/login error. Bounded (one login, no loop); no lock held over network.
fn hamqth_lookup_attempt(
    call: &str,
    username: &str,
    password: &str,
    hamqth_session: &SharedHamQthSession,
) -> Result<Option<tempo_app::dto::QrzLookupDto>, String> {
    // 1) Try the cached session id, if any.
    let cached = hamqth_session.0.lock().ok().and_then(|g| g.clone());
    if let Some(id) = cached {
        match hamqth_try_lookup(&id, call)? {
            QrzOutcome::Found(dto) => return Ok(Some(dto)),
            QrzOutcome::NotFound => return Ok(None), // authoritative miss — don't re-login
            QrzOutcome::NeedLogin => {}              // fall through to a single re-login
        }
    }
    // 2) Log in once, cache the new session id, retry the lookup once (bounded).
    let id = hamqth_login(username, password)?;
    if let Ok(mut g) = hamqth_session.0.lock() {
        *g = Some(id.clone());
    }
    match hamqth_try_lookup(&id, call)? {
        QrzOutcome::Found(dto) => Ok(Some(dto)),
        QrzOutcome::NotFound => Ok(None),
        // A fresh id still reporting expiry is anomalous — give up.
        QrzOutcome::NeedLogin => Ok(None),
    }
}

/// Look up a callsign, enriching with name / grid / QTH / state. QRZ is tried first
/// (its paid tier carries grid/state); when QRZ is **unconfigured** (no username or
/// no stored password) or has **no match**, the lookup falls through to the FREE
/// HamQTH fallback so it works without a QRZ subscription. Each path uses the same
/// bounded cached-session → login-once → retry pattern; both produce the same DTO, so
/// the command's return type and the whole UI are unchanged.
#[tauri::command]
async fn qrz_lookup(
    callsign: String,
    state: State<'_, SharedEngine>,
    qrz_session: State<'_, SharedQrzSession>,
    hamqth_session: State<'_, SharedHamQthSession>,
) -> Result<tempo_app::dto::QrzLookupDto, String> {
    let call = callsign.trim().to_string();
    if call.is_empty() {
        return Err("Enter a callsign to look up.".to_string());
    }
    let (qrz_username, hamqth_username) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.qrz_username.trim().to_string(),
            s.hamqth_username.trim().to_string(),
        )
    };

    let not_found = || format!("{} is not in the callbook.", call.to_uppercase());
    // Did any callbook actually RUN? A username set with no stored password means
    // the query is silently skipped — without this flag that lands on `not_found()`
    // below and misreports a *missing credential* as "not in the callbook" (the
    // "N6HU is not in the callbook, but he IS on QRZ" bug: QRZ was never queried
    // because the QRZ *password* wasn't set — people commonly set only the separate
    // Logbook API key, which the callbook lookup never uses).
    let mut queried_any = false;

    // 1) QRZ first, when configured (username + stored password). A missing username,
    //    a missing password, or a QRZ "not found" all fall through to HamQTH below.
    if !qrz_username.is_empty() {
        if let Ok(password) = qrz_keychain()?.get_password() {
            queried_any = true;
            if let Some(dto) =
                qrz_lookup_attempt(&call, &qrz_username, &password, qrz_session.inner())?
            {
                return Ok(dto);
            }
        }
    }

    // 2) HamQTH fallback, when configured. A free HamQTH account returns
    //    name/grid/us_state, so callsign lookup works without a QRZ subscription.
    if !hamqth_username.is_empty() {
        if let Ok(password) = hamqth_keychain()?.get_password() {
            queried_any = true;
            if let Some(dto) =
                hamqth_lookup_attempt(&call, &hamqth_username, &password, hamqth_session.inner())?
            {
                return Ok(dto);
            }
            // HamQTH was queried and answered — a genuine miss.
            return Err(not_found());
        }
    }

    // Neither callbook produced a record.
    if qrz_username.is_empty() && hamqth_username.is_empty() {
        Err("Set your QRZ or HamQTH username in Settings first.".to_string())
    } else if !queried_any {
        // A username IS set but no password is stored, so no lookup actually ran.
        // Name the exact fix instead of pretending the call wasn't found.
        let mut which = Vec::new();
        if !qrz_username.is_empty() {
            which.push("QRZ");
        }
        if !hamqth_username.is_empty() {
            which.push("HamQTH");
        }
        Err(format!(
            "Callbook lookup needs your {} password — add it in Settings ▸ Logbook & QSL. \
             For QRZ this is your QRZ.com login password, NOT the Logbook API key (that key \
             only uploads QSOs).",
            which.join(" or ")
        ))
    } else {
        Err(not_found())
    }
}

/// Push one logged QSO to the operator's QRZ.com logbook (the Logbook API, a
/// separate per-logbook API key). Builds the one-record ADIF, POSTs an INSERT, and
/// returns the outcome (a duplicate is the benign "already logged"). The UI fires
/// this after a successful `log_qso` when auto-upload is on. No lock held over the
/// network call.
#[tauri::command]
async fn qrz_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
) -> Result<tempo_app::dto::QrzPushResultDto, String> {
    let who = record.call.clone();
    // The impl does blocking HTTP (20 s timeout) — keep it off the async
    // executor so a slow QRZ can't stall every other Tauri command.
    let engine = state.inner().clone();
    let res = tauri::async_runtime::spawn_blocking(move || qrz_push_qso_impl(record, &engine))
        .await
        .map_err(|e| format!("upload task failed: {e}"))?;
    conn_logged(
        "QRZ Logbook",
        |r| format!("pushed {} — {}", who, r.result),
        res,
    )
}

/// Test the N3FJP connection: handshake `<CMD><PROGRAM></CMD>` and report
/// what's listening ("N3FJP's Field Day Contest Log v6.6") — run this at the
/// club site BEFORE the event starts.
#[tauri::command]
async fn n3fjp_test_connection(state: State<'_, SharedEngine>) -> Result<String, String> {
    let (host, port) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (st.n3fjp_host.trim().to_string(), st.n3fjp_port)
    };
    if host.is_empty() {
        return Err("No N3FJP host configured — set it in Settings ▸ Field Day.".into());
    }
    conn_logged(
        "N3FJP",
        |s: &String| format!("connection test OK — {s}"),
        tempo_net::n3fjp::test_connection(&host, port),
    )
}

/// Test the QRZ Logbook connection: a real STATUS round-trip that validates
/// the API key (and shows which logbook it unlocks) WITHOUT inserting anything.
/// This is the verification the operator runs after entering credentials.
#[tauri::command]
async fn qrz_test_connection() -> Result<String, String> {
    conn_logged(
        "QRZ Logbook",
        |s: &String| format!("connection test OK — {s}"),
        qrz_test_connection_impl().await,
    )
}

async fn qrz_test_connection_impl() -> Result<String, String> {
    let key = qrz_logbook_keychain()?
        .get_password()
        .map_err(|_| {
            "No QRZ Logbook API key stored — note this is the per-logbook key from              logbook.qrz.com (Settings ▸ Logbook ▸ API), NOT your QRZ password."
                .to_string()
        })?;
    let body = tempo_core::qrz::build_status_body(&key);
    let resp = propagation::live::qrz::post_form(tempo_core::qrz::QRZ_LOGBOOK_URL, body)?;
    let st = tempo_core::qrz::parse_status_response(&resp);
    if st.ok {
        let owner = st.owner.unwrap_or_else(|| "your account".into());
        let book = st.book.map(|b| format!(" ({b})")).unwrap_or_default();
        Ok(format!(
            "{owner}{book} — {} QSOs in the online logbook",
            st.count
        ))
    } else {
        Err(st
            .reason
            .unwrap_or_else(|| "QRZ rejected the API key".into()))
    }
}

fn qrz_push_qso_impl(
    record: LoggedQso,
    engine: &SharedEngine,
) -> Result<tempo_app::dto::QrzPushResultDto, String> {
    let key = qrz_logbook_keychain()?
        .get_password()
        .map_err(|_| "No QRZ Logbook API key stored — set it in Settings.".to_string())?;
    let rec: tempo_core::logbook::QsoRecord = record.into();
    let adif = tempo_core::logbook::adif_record(&rec);
    let body = tempo_core::qrz::build_insert_body(&key, &adif, false);
    let resp = propagation::live::qrz::post_form(tempo_core::qrz::QRZ_LOGBOOK_URL, body)?;
    let push = tempo_core::qrz::parse_push_response(&resp);
    // Record the outcome on the just-pushed QSO so diagnostics can surface R1 (never
    // pushed to QRZ) / R9 (QRZ upload bounced). QRZ outcomes are always definitive.
    {
        let outcome = push.result.to_upload_outcome();
        let detail = push
            .reason
            .as_deref()
            .and_then(tempo_core::lotw_upload::sanitize_detail);
        let mut eng = engine.lock().map_err(|e| e.to_string())?;
        eng.stamp_qrz_upload(&rec, outcome, now_unix(), detail);
    }
    Ok(push.into())
}

// ----- ClubLog realtime QSO push --------------------------------------------

/// Store (or, if empty, clear) the ClubLog **Application Password** in the OS
/// keychain. Also re-arms ClubLog auto-push (a credential change clears the 403
/// suspend latch). Write-only. Saving also switches ClubLog auto-upload ON
/// (entering the credential is the intent).
#[tauri::command]
fn set_clublog_password(password: String, state: State<'_, SharedEngine>) -> Result<(), String> {
    let entry = clublog_keychain()?;
    if password.is_empty() {
        // Clearing = "turn ClubLog off": drop the credential AND the toggle. The
        // suspend latch is deliberately NOT touched here — re-arming with no
        // password would only make the worker error on every QSO.
        clear_keychain_entry(&entry)?;
        conn_log(
            "ClubLog",
            "info",
            "app-password cleared from the OS keychain",
        );
        set_upload_toggle(&state, UploadToggle::Clublog, false);
        return Ok(());
    }
    // A real credential change re-arms auto-push (clears the 403 suspend latch).
    CLUBLOG_SUSPENDED.store(false, std::sync::atomic::Ordering::Relaxed);
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("ClubLog", "ok", "app-password saved to the OS keychain");
    set_upload_toggle(&state, UploadToggle::Clublog, true);
    Ok(())
}

/// Remove the stored ClubLog app-password from the OS keychain (idempotent);
/// also turns ClubLog auto-upload off (no credential to push with).
#[tauri::command]
fn clear_clublog_password(state: State<'_, SharedEngine>) -> Result<(), String> {
    clear_keychain_entry(&clublog_keychain()?)?;
    conn_log(
        "ClubLog",
        "info",
        "app-password cleared from the OS keychain",
    );
    set_upload_toggle(&state, UploadToggle::Clublog, false);
    Ok(())
}

// ----- HRDLog.net upload code + realtime QSO push ----------------------------

/// Store (or, if empty, clear) the HRDLog.net **upload code** in the OS keychain.
/// Write-only, like the QRZ/eQSL/ClubLog counterparts. Saving a code also switches
/// HRDLog.net auto-upload ON (entering the credential is the intent).
#[tauri::command]
fn set_hrdlog_code(code: String, state: State<'_, SharedEngine>) -> Result<(), String> {
    let entry = hrdlog_keychain()?;
    if code.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log(
            "HRDLog.net",
            "info",
            "upload code cleared from the OS keychain",
        );
        set_upload_toggle(&state, UploadToggle::Hrdlog, false);
        return Ok(());
    }
    entry
        .set_password(&code)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("HRDLog.net", "ok", "upload code saved to the OS keychain");
    set_upload_toggle(&state, UploadToggle::Hrdlog, true);
    Ok(())
}

/// Remove the stored HRDLog.net upload code from the OS keychain (idempotent);
/// also turns HRDLog.net auto-upload off (no credential to push with).
#[tauri::command]
fn clear_hrdlog_code(state: State<'_, SharedEngine>) -> Result<(), String> {
    let r = clear_keychain_entry(&hrdlog_keychain()?);
    if r.is_ok() {
        conn_log(
            "HRDLog.net",
            "info",
            "upload code cleared from the OS keychain",
        );
        set_upload_toggle(&state, UploadToggle::Hrdlog, false);
    }
    r
}

/// Push one logged QSO to HRDLog.net (`NewEntry.aspx`). Resolves the station
/// callsign (`mycall`) + the keychain upload code, uploads one ADIF record, and
/// classifies the XML response. HRDLog.net is a live-logging/awards site — NOT an
/// ARRL confirmation source, so this never touches confirmation/award state.
#[tauri::command]
async fn hrdlog_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
) -> Result<tempo_app::dto::HrdLogPushResultDto, String> {
    let who = record.call.clone();
    // Blocking HTTP off the async executor (see qrz_push_qso).
    let engine = state.inner().clone();
    let res = tauri::async_runtime::spawn_blocking(move || hrdlog_push_qso_impl(record, &engine))
        .await
        .map_err(|e| format!("upload task failed: {e}"))?;
    conn_logged(
        "HRDLog.net",
        |r| format!("pushed {} — {}", who, r.result),
        res,
    )
}

fn hrdlog_push_qso_impl(
    record: LoggedQso,
    engine: &SharedEngine,
) -> Result<tempo_app::dto::HrdLogPushResultDto, String> {
    let callsign = {
        let eng = engine.lock().map_err(|e| e.to_string())?;
        eng.settings().mycall.trim().to_string()
    };
    if callsign.is_empty() {
        return Err("Set your station callsign in Settings first.".to_string());
    }
    let code = hrdlog_keychain()?
        .get_password()
        .map_err(|_| "No HRDLog.net upload code stored — set it in Settings.".to_string())?;
    let rec: tempo_core::logbook::QsoRecord = record.into();
    let adif = tempo_core::logbook::adif_record(&rec);

    // Build + POST without the lock; the body carries the code — never logged.
    let resp = {
        let query = tempo_core::hrdlog::HrdLogQuery {
            callsign,
            code,
            app: HRDLOG_APP_NAME.to_string(),
            adif,
        };
        let body = tempo_core::hrdlog::build_upload_body(&query);
        propagation::live::hrdlog::post_form(tempo_core::hrdlog::HRDLOG_NEWENTRY_URL, body)?
    }; // `query` + `body` (both hold the code) dropped here

    Ok(tempo_core::hrdlog::classify_response(&resp).into())
}

/// Push one logged QSO to ClubLog (realtime). Resolves the 4 credentials (email +
/// callsign∥mycall + api-key from Settings or the build-time `option_env!` + the
/// keychain app-password), uploads, and classifies the HTTP-status response. A 403
/// **suspends** further auto-pushes this session (ClubLog IP-blocks hammering)
/// until a credential changes. No lock over the network.
#[tauri::command]
async fn clublog_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
) -> Result<tempo_app::dto::ClubLogPushResultDto, String> {
    let who = record.call.clone();
    // Blocking HTTP off the async executor (see qrz_push_qso).
    let engine = state.inner().clone();
    let res = tauri::async_runtime::spawn_blocking(move || clublog_push_qso_impl(record, &engine))
        .await
        .map_err(|e| format!("upload task failed: {e}"))?;
    conn_logged("ClubLog", |r| format!("pushed {} — {}", who, r.result), res)
}

fn clublog_push_qso_impl(
    record: LoggedQso,
    engine: &SharedEngine,
) -> Result<tempo_app::dto::ClubLogPushResultDto, String> {
    use std::sync::atomic::Ordering;
    if CLUBLOG_SUSPENDED.load(Ordering::Relaxed) {
        return Err(
            "ClubLog auto-upload paused after an auth failure — fix your credentials in Settings."
                .to_string(),
        );
    }
    let (email, callsign_setting, api_setting, mycall) = {
        let eng = engine.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.clublog_email.trim().to_string(),
            s.clublog_callsign.trim().to_string(),
            s.clublog_api_key.trim().to_string(),
            s.mycall.trim().to_string(),
        )
    };
    // API key: Settings first, else the build-time baked key (official installer).
    let api_key = effective_clublog_key(&api_setting);
    if api_key.is_empty() {
        return Err("This build has no ClubLog application key. Official installers bundle one; building from source, get a free key at clublog.org/requestapikey.php and add it in Settings ▸ Confirmations.".to_string());
    }
    if email.is_empty() {
        return Err("Set your ClubLog email in Settings first.".to_string());
    }
    let callsign = if callsign_setting.is_empty() {
        mycall
    } else {
        callsign_setting
    };
    let password = clublog_keychain()?
        .get_password()
        .map_err(|_| "No ClubLog app-password stored — set it in Settings.".to_string())?;
    let rec: tempo_core::logbook::QsoRecord = record.into();
    let adif = tempo_core::logbook::adif_record(&rec);

    let (status, resp) = {
        let query = tempo_core::clublog::ClubLogQuery {
            email,
            password,
            callsign,
            api_key,
            adif,
        };
        let body = tempo_core::clublog::build_realtime_body(&query);
        propagation::live::clublog::push_realtime(tempo_core::clublog::CLUBLOG_REALTIME_URL, body)?
    }; // `query` + `body` (both hold the secrets) dropped here

    let push = tempo_core::clublog::classify_response(status, &resp);
    if push.result == tempo_core::clublog::ClubLogResult::AuthFail {
        // Halt further auto-pushes until a credential changes (IP-block guard).
        {
            conn_log(
                "ClubLog",
                "error",
                "auth failed — auto-push SUSPENDED until credentials change in Settings",
            );
            CLUBLOG_SUSPENDED.store(true, Ordering::Relaxed);
        }
    }
    // Record the outcome on the just-pushed QSO so diagnostics can surface R1 (never
    // pushed to ClubLog) / R9 (bounced). Transient results (ServerError/Unknown) map
    // to None → leave it unstamped for a clean retry.
    if let Some(outcome) = push.result.to_upload_outcome() {
        let detail = push
            .message
            .as_deref()
            .and_then(tempo_core::lotw_upload::sanitize_detail);
        let mut eng = engine.lock().map_err(|e| e.to_string())?;
        eng.stamp_clublog_upload(&rec, outcome, now_unix(), detail);
    }
    Ok(push.into())
}

// ----- eQSL ADIF QSO upload --------------------------------------------------

/// Upload one logged QSO to eQSL.cc (ImportADIF.cfm, per-QSO `ADIFData`). Reads the
/// eQSL username from Settings + the password from the keychain, posts the record,
/// classifies the response, and stamps `upload.eqsl`. Returns the outcome string for
/// the UI ("accepted"|"duplicate"|"rejected"|"authfail"|"retry"). No lock held over
/// the network. eQSL is non-award (like QRZ) — it never credits ARRL DXCC/WAS.
#[tauri::command]
async fn eqsl_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
) -> Result<UploadReportDto, String> {
    let who = record.call.clone();
    // Blocking HTTP off the async executor (see qrz_push_qso).
    let engine = state.inner().clone();
    let res = tauri::async_runtime::spawn_blocking(move || eqsl_push_qso_impl(record, &engine))
        .await
        .map_err(|e| format!("upload task failed: {e}"))?;
    conn_logged(
        "eQSL",
        |r| format!("pushed {} — outcome: {}", who, r.outcome),
        res,
    )
}

fn eqsl_push_qso_impl(record: LoggedQso, engine: &SharedEngine) -> Result<UploadReportDto, String> {
    let user = {
        let eng = engine.lock().map_err(|e| e.to_string())?;
        eng.settings().eqsl_username.trim().to_string()
    };
    if user.is_empty() {
        return Err("Set your eQSL username in Settings first.".to_string());
    }
    let password = eqsl_keychain()?
        .get_password()
        .map_err(|_| "No eQSL password stored — set it in Settings.".to_string())?;
    let rec: tempo_core::logbook::QsoRecord = record.into();
    let adif = tempo_core::logbook::adif_record(&rec);

    // Build + POST without the lock; the body carries the password — never logged.
    let resp = {
        let body = tempo_core::eqsl::build_upload_body(&user, &password, &adif);
        propagation::live::eqsl::post_form(tempo_core::eqsl::EQSL_IMPORT_URL, body)?
    }; // `body` (holds the password) dropped here

    match tempo_core::eqsl::classify_upload(&resp) {
        // Transient (system down) → leave unstamped for a clean retry.
        None => Ok(UploadReportDto {
            dispatched: 1,
            outcome: "retry".into(),
            detail: Some("eQSL is temporarily unavailable — try again shortly.".into()),
        }),
        Some(outcome) => {
            let mut eng = engine.lock().map_err(|e| e.to_string())?;
            eng.stamp_eqsl_upload(&rec, outcome, now_unix(), None);
            Ok(UploadReportDto {
                dispatched: 1,
                outcome: outcome.code().to_string(),
                detail: None,
            })
        }
    }
}

// ----- Connector auto-upload (the log_qso funnel's worker side) --------------

/// Record an operator-facing upload note on the engine (UI toasts it on the
/// next snapshot poll — `upload_tick` bumps).
fn note_upload_shared(engine: &SharedEngine, msg: String, ok: bool) {
    if let Ok(mut eng) = engine.lock() {
        eng.note_upload(msg, ok);
    }
}

/// Push one freshly-logged QSO to every enabled connector. Each service gets a
/// connectivity-log line, and the QSO gets ONE combined operator-facing note
/// ("W9XYZ → QRZ ✓ · ClubLog dup · eQSL ✗ login invalid") — a single toast per
/// QSO, so a fast multi-connector push can't overwrite its own outcomes between
/// two snapshot polls. Outcomes are stamped on the QSO by the underlying
/// `*_push_qso_impl` (per-QSO upload state machine).
/// Band string ("20m") → N3FJP meters ("20"); leaves cm bands ("70cm") intact.
fn n3fjp_band_meters(band: &str) -> String {
    let b = band.trim();
    if b.to_ascii_lowercase().ends_with("cm") {
        b.to_string()
    } else {
        b.trim_end_matches(['m', 'M']).to_string()
    }
}

/// Nexus mode → N3FJP mode: fold USB/LSB to SSB; pass the rest (FT8/FT4/CW/FM/RTTY…) through.
fn n3fjp_mode(mode: &str) -> String {
    match mode.to_ascii_uppercase().as_str() {
        "USB" | "LSB" => "SSB".to_string(),
        other => other.to_string(),
    }
}

/// Forward ONE general (non-Field-Day) logged QSO to N3FJP ACLog over TCP ADDDIRECT. Uses the
/// same `n3fjp_host`/`n3fjp_port` as the Field-Day push; N3FJP's EXCLUDEDUPES dedupes any overlap.
fn n3fjp_push_qso_impl(dto: &LoggedQso, engine: &SharedEngine) -> Result<(), String> {
    let (host, port, mycall) = {
        let eng = engine.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.n3fjp_host.trim().to_string(),
            s.n3fjp_port,
            s.mycall.trim().to_string(),
        )
    };
    if host.is_empty() {
        return Err("no N3FJP host set".to_string());
    }
    let push = tempo_net::n3fjp::N3fjpQso {
        call: dto.call.clone(),
        class: String::new(),
        section: String::new(),
        band_meters: n3fjp_band_meters(&dto.band),
        mode: n3fjp_mode(&dto.mode),
        freq_mhz: dto.freq_mhz,
        when_unix: dto.when_unix,
        operator: mycall,
    };
    tempo_net::n3fjp::push_qso(&host, port, &push)
}

/// Forward ONE logged QSO to DXKeeper's TCP Network Service, off-thread.
///
/// Fire-and-forget by design. DXKeeper never acknowledges a directive, so there is nothing
/// to wait for — and the connect carries a 3 s timeout, which inline would stall EVERY log
/// by three seconds whenever DXKeeper simply is not running. That is the common case (the
/// operator has Nexus open and DXLab closed), so it must not be on the logging path.
///
/// Failures land in the connector log rather than a toast: a logger that is not running is
/// not an error worth interrupting an operator mid-QSO.
fn dxkeeper_push_async(host: String, base_port: u16, uploads: bool, adif: String) {
    std::thread::spawn(move || {
        match tempo_net::dxkeeper::push_qso(&host, base_port, &adif, uploads) {
            Ok(()) => conn_log(
                "DXKeeper",
                "ok",
                // Deliberately not "logged" — we cannot know that. DXKeeper replies to
                // nothing; its Server Log is the only place a rejection shows up.
                format!("sent to {host}:{}", tempo_net::dxkeeper::port_for_base(base_port)),
            ),
            Err(e) => conn_log("DXKeeper", "error", e),
        }
    });
}

/// Forward ONE logged QSO to a Cloudlog/Wavelog instance (HTTP JSON ADIF POST). URL +
/// station id come from Settings; the API key lives in the OS keychain (never
/// settings.json), read here at push time.
fn cloudlog_push_qso_impl(dto: &LoggedQso, engine: &SharedEngine) -> Result<String, String> {
    let (url, station_id) = {
        let eng = engine.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.cloudlog_url.trim().to_string(),
            s.cloudlog_station_id.trim().to_string(),
        )
    };
    let key = cloudlog_keychain()?
        .get_password()
        .unwrap_or_default()
        .trim()
        .to_string();
    if url.is_empty() {
        return Err("no Cloudlog URL set".to_string());
    }
    if key.is_empty() {
        return Err("no Cloudlog API key set".to_string());
    }
    let rec: tempo_core::logbook::QsoRecord = dto.clone().into();
    let adif = tempo_core::logbook::adif_record(&rec);
    propagation::live::cloudlog::upload(&url, &key, &station_id, &adif)
}

/// Push one logged QSO to each enabled+owed connector. Returns the bitmask of
/// legs that failed TRANSIENTLY (network down / service busy) and should be
/// retried — a permanent reject (bad auth, malformed) or a success is NOT in the
/// return, so the worker's re-queue never re-pushes a leg that already landed.
fn auto_push_one(
    engine: &SharedEngine,
    dto: LoggedQso,
    qrz_on: bool,
    clublog_on: bool,
    eqsl_on: bool,
    hrdlog_on: bool,
    n3fjp_on: bool,
    cloudlog_on: bool,
    owed: u8,
) -> u8 {
    use tempo_app::engine::upload_legs as legs;
    let call = dto.call.clone();
    let mut parts: Vec<String> = Vec::new();
    let mut all_ok = true;
    // Legs that failed transiently → the worker retries just these.
    let mut failed: u8 = 0;
    if qrz_on && owed & legs::QRZ != 0 {
        let (part, ok, transient) = match qrz_push_qso_impl(dto.clone(), engine) {
            Ok(r) => {
                let ok = matches!(r.result.as_str(), "ok" | "replace" | "duplicate");
                conn_log(
                    "QRZ Logbook",
                    if ok { "ok" } else { "error" },
                    format!("auto-push {call} — {}", r.result),
                );
                let part = match r.result.as_str() {
                    "ok" => "QRZ ✓".to_string(),
                    "replace" => "QRZ ✓ (updated)".to_string(),
                    "duplicate" => "QRZ dup".to_string(),
                    "authFail" => "QRZ ✗ key invalid — check Settings".to_string(),
                    _ => format!("QRZ ✗ {}", r.reason.as_deref().unwrap_or("failed")),
                };
                // A QRZ reply (auth-fail / reject) is definitive — don't retry.
                (part, ok, false)
            }
            Err(e) => {
                conn_log("QRZ Logbook", "error", format!("auto-push {call} — {e}"));
                (format!("QRZ ✗ {e}"), false, true) // transport error → retry
            }
        };
        parts.push(part);
        all_ok &= ok;
        if transient {
            failed |= legs::QRZ;
        }
    }
    if clublog_on && owed & legs::CLUBLOG != 0 {
        let (part, ok, transient) = match clublog_push_qso_impl(dto.clone(), engine) {
            Ok(r) => {
                let ok = matches!(r.result.as_str(), "ok" | "modified" | "duplicate");
                conn_log(
                    "ClubLog",
                    if ok { "ok" } else { "error" },
                    format!("auto-push {call} — {}", r.result),
                );
                let part = match r.result.as_str() {
                    "ok" | "modified" => "ClubLog ✓".to_string(),
                    "duplicate" => "ClubLog dup".to_string(),
                    "authFail" => "ClubLog ✗ auth — auto-upload paused".to_string(),
                    "serverError" => "ClubLog ✗ busy".to_string(),
                    _ => format!("ClubLog ✗ {}", r.message.as_deref().unwrap_or("rejected")),
                };
                // "serverError" = ClubLog temporarily busy → retry; auth/reject = don't.
                (part, ok, r.result.as_str() == "serverError")
            }
            Err(e) => {
                conn_log("ClubLog", "error", format!("auto-push {call} — {e}"));
                (format!("ClubLog ✗ {e}"), false, true)
            }
        };
        parts.push(part);
        all_ok &= ok;
        if transient {
            failed |= legs::CLUBLOG;
        }
    }
    if hrdlog_on && owed & legs::HRDLOG != 0 {
        let (part, ok, transient) = match hrdlog_push_qso_impl(dto.clone(), engine) {
            Ok(r) => {
                let ok = matches!(r.result.as_str(), "ok" | "duplicate");
                conn_log(
                    "HRDLog.net",
                    if ok { "ok" } else { "error" },
                    format!("auto-push {call} — {}", r.result),
                );
                // HRDLog.net is a live-logging/awards site — never DXCC/WAS credit.
                let part = match r.result.as_str() {
                    "ok" => "HRDLog ✓".to_string(),
                    "duplicate" => "HRDLog dup".to_string(),
                    "authFail" => "HRDLog ✗ code invalid — check Settings".to_string(),
                    "unknown" => "HRDLog ✗ unavailable".to_string(),
                    _ => format!("HRDLog ✗ {}", r.message.as_deref().unwrap_or("rejected")),
                };
                // "unknown" = HRDLog temporarily unavailable → retry; auth/reject = don't.
                (part, ok, r.result.as_str() == "unknown")
            }
            Err(e) => {
                conn_log("HRDLog.net", "error", format!("auto-push {call} — {e}"));
                (format!("HRDLog ✗ {e}"), false, true)
            }
        };
        parts.push(part);
        all_ok &= ok;
        if transient {
            failed |= legs::HRDLOG;
        }
    }
    if eqsl_on && owed & legs::EQSL != 0 {
        let (part, ok, transient) = match eqsl_push_qso_impl(dto.clone(), engine) {
            Ok(r) => {
                let ok = matches!(r.outcome.as_str(), "accepted" | "duplicate");
                conn_log(
                    "eQSL",
                    if ok { "ok" } else { "error" },
                    format!("auto-push {call} — {}", r.outcome),
                );
                let part = match r.outcome.as_str() {
                    "accepted" => "eQSL ✓".to_string(),
                    "duplicate" => "eQSL dup".to_string(),
                    "authfail" => "eQSL ✗ login invalid — check Settings".to_string(),
                    "retry" => "eQSL ✗ unavailable".to_string(),
                    _ => format!(
                        "eQSL ✗ rejected{}",
                        r.detail
                            .as_deref()
                            .map(|d| format!(": {d}"))
                            .unwrap_or_default()
                    ),
                };
                // "retry" = eQSL temporarily unavailable → retry; authfail/reject = don't.
                (part, ok, r.outcome.as_str() == "retry")
            }
            Err(e) => {
                conn_log("eQSL", "error", format!("auto-push {call} — {e}"));
                (format!("eQSL ✗ {e}"), false, true)
            }
        };
        parts.push(part);
        all_ok &= ok;
        if transient {
            failed |= legs::EQSL;
        }
    }
    if n3fjp_on && owed & legs::N3FJP != 0 {
        let (part, ok, transient) = match n3fjp_push_qso_impl(&dto, engine) {
            Ok(()) => {
                conn_log("N3FJP", "ok", format!("auto-forward {call}"));
                ("N3FJP ✓".to_string(), true, false)
            }
            Err(e) => {
                conn_log("N3FJP", "error", format!("auto-forward {call} — {e}"));
                (format!("N3FJP ✗ {e}"), false, true)
            }
        };
        parts.push(part);
        all_ok &= ok;
        if transient {
            failed |= legs::N3FJP;
        }
    }
    if cloudlog_on && owed & legs::CLOUDLOG != 0 {
        let (part, ok, transient) = match cloudlog_push_qso_impl(&dto, engine) {
            Ok(_) => {
                conn_log("Cloudlog", "ok", format!("auto-forward {call}"));
                ("Cloudlog ✓".to_string(), true, false)
            }
            Err(e) => {
                conn_log("Cloudlog", "error", format!("auto-forward {call} — {e}"));
                // Cloudlog's error covers both a down instance and a reject; retry
                // (bounded by MAX_UPLOAD_RETRIES) rather than silently drop.
                (format!("Cloudlog ✗ {e}"), false, true)
            }
        };
        parts.push(part);
        all_ok &= ok;
        if transient {
            failed |= legs::CLOUDLOG;
        }
    }
    if !parts.is_empty() {
        note_upload_shared(engine, format!("{call} → {}", parts.join(" · ")), all_ok);
    }
    failed
}

// ----- Parks / Summits On The Air -------------------------------------------

/// Current activation state for the POTA/SOTA panel.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivationDto {
    /// "POTA" | "SOTA", or null when not activating.
    program: Option<String>,
    /// Normalized park/summit reference, or null.
    reference: Option<String>,
    /// Logged QSOs carrying this activation's reference so far.
    qso_count: usize,
}

/// Activators currently on the air for `program` ("POTA" | "SOTA") — the hunter
/// feed. Network fetch (no auth); empty/Err only on a feed problem.
/// One hunter-feed row: the raw activator spot annotated with what THIS
/// operator cares about — is the park a new one, and is the band currently
/// carrying their signal out (live PSKR evidence, the "workable now" signal).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OtaSpotDto {
    #[serde(flatten)]
    spot: propagation::OtaSpot,
    /// This reference has never been logged (hunter side) — a NEW PARK.
    new_park: bool,
    /// The operator's own signal is being received on this band right now.
    band_open: bool,
}

#[tauri::command]
fn get_ota_spots(
    program: String,
    state: State<'_, SharedEngine>,
    live_paths: State<'_, SharedLivePaths>,
    ota_cache: State<'_, SharedOtaSpots>,
) -> Result<Vec<OtaSpotDto>, String> {
    let spots = match program.to_ascii_uppercase().as_str() {
        "POTA" => propagation::live::pota::fetch_pota_spots()?,
        "SOTA" => propagation::live::pota::fetch_sota_spots(30)?,
        other => return Err(format!("Unknown program '{other}' — use POTA or SOTA.")),
    };
    // Refresh the lock-only cache the Needed scorer reads for POTA/SOTA tags —
    // keyed PER PROGRAM ("Both" mode fetches POTA and SOTA concurrently; a
    // single slot let the last writer evict the other program's activators).
    if let Ok(mut c) = ota_cache.lock() {
        c.insert(program.to_ascii_uppercase(), (now_unix(), spots.clone()));
    }
    // Bands where MY signal is getting out right now (live PSKR receptions of
    // my call inside the last 15 min) — the "workable now" differentiator.
    let (mycall, park_worked): (String, Box<dyn Fn(&str) -> bool>) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let worked: std::collections::HashSet<String> = spots
            .iter()
            .filter(|sp| eng.park_worked(&sp.reference))
            .map(|sp| sp.reference.to_uppercase())
            .collect();
        (
            eng.settings().mycall.clone(),
            Box::new(move |r: &str| worked.contains(&r.to_uppercase())),
        )
    };
    let open_bands: std::collections::HashSet<String> = live_paths
        .lock()
        .map(|b| b.recent(now_unix(), 900))
        .unwrap_or_default()
        .iter()
        .filter(|p| tempo_core::message::same_call(&p.tx_call, &mycall))
        .map(|p| p.band.label().to_string())
        .collect();
    Ok(spots
        .into_iter()
        .map(|sp| {
            let band_open = propagation::Band::from_mhz(sp.freq_khz / 1000.0)
                .map(|b| open_bands.contains(b.label()))
                .unwrap_or(false);
            let new_park = !park_worked(&sp.reference);
            OtaSpotDto {
                spot: sp,
                new_park,
                band_open,
            }
        })
        .collect())
}

/// One-click hunt: remember the activator + park so the next QSO logged with
/// that call auto-tags SIG/SIG_INFO (the hunter-side ADIF credit).
#[tauri::command]
fn set_hunt_target(
    state: State<'_, SharedEngine>,
    call: String,
    program: String,
    reference: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_hunt_target(&call, &program, &reference)?;
    Ok(eng.snapshot())
}

/// Log a Field Day contact from the CW/Phone cockpits (all-mode FD). `mode` =
/// "CW" | "PH". Err when FD mode is off; Ok(false) = band+mode dupe.
#[tauri::command]
fn fd_log_manual(
    state: State<'_, SharedEngine>,
    call: String,
    class: String,
    section: String,
    mode: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let logged = eng.fd_log_manual(&call, &class, &section, &mode)?;
    if !logged {
        return Err(format!("{call} is a dupe on this band/mode"));
    }
    Ok(eng.snapshot())
}

#[tauri::command]
fn clear_hunt_target(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.clear_hunt_target();
    Ok(eng.snapshot())
}

/// Begin an activation — subsequent logged QSOs are tagged (your side). Validates +
/// normalizes the reference; returns the activation state.
#[tauri::command]
fn set_activation(
    state: State<'_, SharedEngine>,
    program: String,
    reference: String,
) -> Result<ActivationDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let (program, reference) = eng.set_activation(&program, &reference)?;
    let qso_count = eng.activation_qso_count();
    Ok(ActivationDto {
        program: Some(program),
        reference: Some(reference),
        qso_count,
    })
}

/// End the current activation (subsequent QSOs untagged).
#[tauri::command]
fn clear_activation(state: State<'_, SharedEngine>) -> Result<ActivationDto, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.clear_activation();
    Ok(ActivationDto {
        program: None,
        reference: None,
        qso_count: 0,
    })
}

/// The current activation state (for the panel on load / after logging).
#[tauri::command]
fn get_activation(state: State<'_, SharedEngine>) -> Result<ActivationDto, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    let (program, reference) = match eng.activation() {
        Some((p, r)) => (Some(p), Some(r)),
        None => (None, None),
    };
    let qso_count = eng.activation_qso_count();
    Ok(ActivationDto {
        program,
        reference,
        qso_count,
    })
}

/// POTA all-parks export (CSV). Public list of every park's reference/name/location/grid. NOTE:
/// endpoint + column layout should be verified against a live response — the parser is header-
/// aware + tolerant, and `import_parks_csv` is the fallback if this URL ever changes.
const PARKS_CSV_URL: &str = "https://pota.app/all_parks_ext.csv";

/// A park directory search result (serde mirror of `tempo_core::pota::Park`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParkDto {
    reference: String,
    name: String,
    grid: String,
    location: String,
    /// Coordinates — only the LIVE lookup carries these; the local CSV index doesn't.
    latitude: Option<f64>,
    longitude: Option<f64>,
}

impl From<tempo_core::pota::Park> for ParkDto {
    fn from(p: tempo_core::pota::Park) -> Self {
        ParkDto {
            reference: p.reference,
            name: p.name,
            grid: p.grid,
            location: p.location,
            latitude: None,
            longitude: None,
        }
    }
}

impl From<propagation::live::pota::LiveParkDetail> for ParkDto {
    fn from(p: propagation::live::pota::LiveParkDetail) -> Self {
        ParkDto {
            reference: p.reference,
            name: p.name,
            grid: p.grid,
            location: p.location,
            latitude: p.latitude,
            longitude: p.longitude,
        }
    }
}

/// Load the cached park CSV (if any) into the shared index at startup. Best-effort.
fn load_parks_cache(parks: &SharedParks) {
    if let Ok(csv) = std::fs::read_to_string(parks_cache_path()) {
        if let Ok(mut idx) = parks.lock() {
            *idx = tempo_core::pota::ParkIndex::parse_csv(&csv);
        }
    }
}

/// Search the local park directory (offline). Empty query or no list loaded → empty.
#[tauri::command]
fn search_parks(
    parks: State<'_, SharedParks>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<ParkDto>, String> {
    let idx = parks.lock().map_err(|e| e.to_string())?;
    Ok(idx
        .search(&query, limit.unwrap_or(12))
        .into_iter()
        .map(ParkDto::from)
        .collect())
}

/// How many parks are loaded in the local directory (0 = not downloaded/imported yet).
#[tauri::command]
fn parks_count(parks: State<'_, SharedParks>) -> Result<usize, String> {
    Ok(parks.lock().map_err(|e| e.to_string())?.len())
}

/// How many parks the operator has imported from their Hunted Parks.CSV (0 = none). Lets the UI
/// show the imported count after a restart (the set itself is reloaded from cache at startup).
#[tauri::command]
fn hunted_parks_count(state: State<'_, SharedEngine>) -> Result<usize, String> {
    Ok(state
        .lock()
        .map_err(|e| e.to_string())?
        .hunted_parks_import_count())
}

/// Import a park directory from CSV text the operator downloaded (the HRD workflow). Caches it and
/// swaps in the new index. Returns the park count. Always works regardless of the download URL.
#[tauri::command]
fn import_parks_csv(parks: State<'_, SharedParks>, csv: String) -> Result<usize, String> {
    let idx = tempo_core::pota::ParkIndex::parse_csv(&csv);
    if idx.is_empty() {
        return Err(
            "No parks parsed — is this a POTA parks CSV (needs a 'reference' column)?".into(),
        );
    }
    let n = idx.len();
    let _ = std::fs::write(parks_cache_path(), &csv); // cache; failure is non-fatal
    *parks.lock().map_err(|e| e.to_string())? = idx;
    Ok(n)
}

/// Where the imported POTA "Hunted Parks.CSV" is cached (beside settings.json), so the hunter's
/// worked-park set survives restarts. Losing it is harmless (re-import from the POTA stats page).
fn hunted_parks_cache_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hunted_parks.csv")
}

/// Load the cached Hunted Parks.CSV (if any) into the engine at startup. Best-effort.
fn load_hunted_parks_cache(engine: &SharedEngine) {
    if let Ok(csv) = std::fs::read_to_string(hunted_parks_cache_path()) {
        let refs = tempo_core::pota::ParkIndex::parse_csv(&csv).references();
        if let Ok(mut eng) = engine.lock() {
            eng.set_hunted_parks_import(refs);
        }
    }
}

/// Import the operator's POTA "Hunted Parks.CSV" (from the POTA stats page) to mark those parks as
/// worked. This is the honest source for the NEW PARK badge on CW hunts: the park reference is
/// never in a CW exchange, so it never reaches the log — the log alone can't know the park was
/// worked. Caches the file and replaces the imported set (a re-import is the full current picture).
#[tauri::command]
fn import_hunted_parks_csv(engine: State<'_, SharedEngine>, csv: String) -> Result<usize, String> {
    let refs = tempo_core::pota::ParkIndex::parse_csv(&csv).references();
    if refs.is_empty() {
        return Err(
            "No parks parsed — is this a POTA Hunted Parks CSV (needs a 'reference' column)?"
                .into(),
        );
    }
    let n = refs.len();
    let _ = std::fs::write(hunted_parks_cache_path(), &csv); // cache; failure is non-fatal
    engine
        .lock()
        .map_err(|e| e.to_string())?
        .set_hunted_parks_import(refs);
    Ok(n)
}

/// Download the current POTA all-parks list, cache it, and load it for offline search. Blocking
/// HTTP off the main thread (like the update check). Returns the park count.
#[tauri::command]
async fn download_parks(parks: State<'_, SharedParks>) -> Result<usize, String> {
    let csv = tauri::async_runtime::spawn_blocking(|| -> Result<String, String> {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(concat!("nexus/", env!("CARGO_PKG_VERSION"), " (+parks)"))
            .build()
            .map_err(|e| e.to_string())?
            .get(PARKS_CSV_URL)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let idx = tempo_core::pota::ParkIndex::parse_csv(&csv);
    if idx.is_empty() {
        return Err(
            "Downloaded list had no recognizable parks — the POTA export format may have changed."
                .into(),
        );
    }
    let n = idx.len();
    let _ = std::fs::write(parks_cache_path(), &csv);
    *parks.lock().map_err(|e| e.to_string())? = idx;
    Ok(n)
}

// ── Radio programming ("Program" section): repeater search, projects, exports ──────────────────
//
// Location → repeater directory → picked channels → CHIRP/generic CSV. Pure logic lives in
// propagation::{repeaters, memchan, chirp}; the live fetchers in propagation::live::{repeaterbook,
// hearham, geocode}. This shell owns: the RepeaterBook token (OS keychain), the per-user disk
// cache with TTL (compliance: per-user, on-demand, never bundled/redistributed), per-state fetch
// throttling, source fallback (no token / non-US / RB failure → hearham), and the saved
// programming projects in radioprog.json beside settings.json.

/// Saved programming projects (channel lists) — sidecar file so hours of curation survive webview
/// storage clears and stay reachable for future direct-programming paths.
fn radioprog_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("radioprog.json")
}

/// Per-source repeater-directory cache dir (rb_<state_id>.json / hearham.json), beside
/// settings.json. Losing it is harmless (re-fetch); it is never exported or shipped.
fn radioprog_cache_dir() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("radioprog_cache")
}

const REPEATERBOOK_KEYCHAIN_USER: &str = "repeaterbook-token";

fn repeaterbook_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, REPEATERBOOK_KEYCHAIN_USER)
        .map_err(|e| format!("couldn't open the system keychain: {e}"))
}

/// Store (or, if empty, clear) the operator's RepeaterBook API token (`rbuapp_…`, generated from
/// their RepeaterBook "API Apps" dashboard). Write-only, like the other connector secrets.
#[tauri::command]
fn set_repeaterbook_token(token: String) -> Result<(), String> {
    let entry = repeaterbook_keychain()?;
    if token.trim().is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log(
            "RepeaterBook",
            "info",
            "API token cleared from the OS keychain",
        );
        return Ok(());
    }
    entry
        .set_password(token.trim())
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))?;
    conn_log("RepeaterBook", "ok", "API token saved to the OS keychain");
    Ok(())
}

/// One cached directory payload: `{ fetchedUtc, body }`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepeaterCacheFile {
    fetched_utc: i64,
    body: String,
}

/// Directory cache TTL — weekly. Repeater listings move slowly; a 7-day per-user cache keeps us
/// well inside RepeaterBook's "minimize load, no offline database" posture (age is shown in the
/// UI and Refresh is explicit).
const RADIOPROG_TTL_SECS: i64 = 7 * 24 * 3600;
/// Manual-refresh throttle per RepeaterBook state export.
const RB_STATE_THROTTLE_SECS: i64 = 900;

fn read_repeater_cache(name: &str) -> Option<RepeaterCacheFile> {
    let p = radioprog_cache_dir().join(name);
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_repeater_cache(name: &str, body: &str) {
    let dir = radioprog_cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let f = RepeaterCacheFile {
        fetched_utc: now_unix(),
        body: body.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&f) {
        let _ = std::fs::write(dir.join(name), json);
    }
}

/// Last fetch attempt per RB state (unix secs) — the per-state refresh throttle.
static RB_LAST_TRY: std::sync::Mutex<Option<std::collections::HashMap<String, i64>>> =
    std::sync::Mutex::new(None);

fn rb_throttle_ok(state_id: &str, now: i64) -> bool {
    let mut g = RB_LAST_TRY.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(std::collections::HashMap::new);
    let ok = now - map.get(state_id).copied().unwrap_or(0) >= RB_STATE_THROTTLE_SECS;
    if ok {
        map.insert(state_id.to_string(), now);
    }
    ok
}

/// One search row: the directory record (display: distance/bearing/mode flags/status) plus the
/// ready-to-add memory channel derived from it (propagation::repeaters::to_channel — duplex,
/// offset and tone mapping stay in the tested Rust domain, never re-derived in TS).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RepeaterSearchRow {
    record: propagation::repeaters::RepeaterRecord,
    channel: propagation::memchan::Channel,
}

/// A repeater search result: which source answered, how old the data is, and the rows inside
/// the radius (distance/bearing filled, nearest first).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RepeaterSearchResult {
    /// "repeaterbook" | "hearham".
    source: String,
    /// Oldest payload timestamp behind these records (unix secs) — the UI's "as of" stamp.
    fetched_utc: i64,
    /// True when a fetch failed/was rate-limited and stale cache was served instead.
    stale: bool,
    rows: Vec<RepeaterSearchRow>,
}

fn search_rows(records: Vec<propagation::repeaters::RepeaterRecord>) -> Vec<RepeaterSearchRow> {
    records
        .into_iter()
        .map(|r| RepeaterSearchRow {
            channel: propagation::repeaters::to_channel(&r),
            record: r,
        })
        .collect()
}

/// Search repeaters within `radius_km` of a point. Source resolution for a US origin: RepeaterBook
/// state exports — through the operator's own token when one is stored, else through the Nexus
/// proxy (rb.hamradiotools.io, the centralized model: the app token lives server-side only).
/// Cached TTL 7d, per-state throttle, stale cache on failure/429. When RepeaterBook is
/// unreachable/dormant with no cache (or the origin is non-US) → hearham, same cache discipline.
/// All network + parsing off the main thread.
#[tauri::command]
async fn repeater_search(
    lat: f64,
    lon: f64,
    radius_km: f64,
) -> Result<RepeaterSearchResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use propagation::repeaters as rpt;
        let origin = (lat, lon);
        let radius = radius_km.clamp(1.0, 500.0);
        let token = repeaterbook_keychain()
            .ok()
            .and_then(|e| e.get_password().ok())
            .unwrap_or_default();
        let states = rpt::plan_states(origin, radius);
        let now = now_unix();

        if !states.is_empty() {
            let mut records = Vec::new();
            let mut oldest = i64::MAX;
            let mut stale = false;
            let mut any = false;
            let mut last_err = String::new();
            for st in &states {
                let cache_name = format!("rb_{st}.json");
                let cached = read_repeater_cache(&cache_name);
                let fresh = cached
                    .as_ref()
                    .is_some_and(|c| now - c.fetched_utc < RADIOPROG_TTL_SECS);
                let body = if fresh {
                    cached
                        .as_ref()
                        .map(|c| (c.body.clone(), c.fetched_utc, false))
                } else if rb_throttle_ok(st, now) {
                    // A stored personal token goes straight to RepeaterBook; otherwise the
                    // Nexus proxy (which holds the app token server-side + an edge cache).
                    let fetched = if token.is_empty() {
                        propagation::live::repeaterbook::fetch_state_proxy(st)
                    } else {
                        propagation::live::repeaterbook::fetch_state(&token, st)
                    };
                    match fetched {
                        Ok(b) => {
                            write_repeater_cache(&cache_name, &b);
                            Some((b, now, false))
                        }
                        Err(e) => {
                            last_err = e;
                            // Failure/429/dormant proxy → serve whatever cache exists, stale.
                            cached
                                .as_ref()
                                .map(|c| (c.body.clone(), c.fetched_utc, true))
                        }
                    }
                } else {
                    cached
                        .as_ref()
                        .map(|c| (c.body.clone(), c.fetched_utc, true))
                };
                if let Some((b, at, was_stale)) = body {
                    records.extend(rpt::parse_repeaterbook_json(&b));
                    oldest = oldest.min(at);
                    stale |= was_stale;
                    any = true;
                }
            }
            if any {
                return Ok(RepeaterSearchResult {
                    source: "repeaterbook".into(),
                    fetched_utc: if oldest == i64::MAX { now } else { oldest },
                    stale,
                    rows: search_rows(rpt::filter_sort(&records, origin, radius)),
                });
            }
            // Every state failed with no cache (e.g. the proxy is dormant pre-approval) — fall
            // through to hearham, but log the RB reason for the Connections panel.
            if !last_err.is_empty() {
                conn_log("RepeaterBook", "info", &last_err);
            }
        }

        // hearham: the no-token default, the non-US path, and the RB fallback.
        let cached = read_repeater_cache("hearham.json");
        let fresh = cached
            .as_ref()
            .is_some_and(|c| now - c.fetched_utc < RADIOPROG_TTL_SECS);
        let (body, at, stale) = if fresh {
            let c = cached.as_ref().unwrap();
            (c.body.clone(), c.fetched_utc, false)
        } else {
            match propagation::live::hearham::fetch_all() {
                Ok(b) => {
                    write_repeater_cache("hearham.json", &b);
                    (b, now, false)
                }
                Err(e) => match cached.as_ref() {
                    Some(c) => (c.body.clone(), c.fetched_utc, true),
                    None => return Err(e),
                },
            }
        };
        let records = rpt::parse_hearham_json(&body);
        Ok(RepeaterSearchResult {
            source: "hearham".into(),
            fetched_utc: at,
            stale,
            rows: search_rows(rpt::filter_sort(&records, origin, radius)),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// City-name search via OSM Nominatim (explicit Search click only — the UI never queries per
/// keystroke). Returns up to 5 candidates for the operator to pick from.
#[tauri::command]
async fn geocode_city(
    query: String,
) -> Result<Vec<propagation::live::geocode::GeoCandidate>, String> {
    tauri::async_runtime::spawn_blocking(move || propagation::live::geocode::search_city(&query))
        .await
        .map_err(|e| e.to_string())?
}

/// The saved-projects file: `{ version, projects: [...] }`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RadioProgFile {
    version: u32,
    projects: Vec<RadioProgProject>,
}

/// Where a project's repeaters were searched from (re-fetch seed + display label).
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ProgOrigin {
    /// "station" | "grid" | "city".
    kind: String,
    grid: String,
    /// Display label ("EN52", "Gatlinburg, TN…").
    label: String,
    lat: f64,
    lon: f64,
}

/// One named programming project — a curated channel list ("Home HT", "Denver trip").
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RadioProgProject {
    id: String,
    name: String,
    created_utc: i64,
    updated_utc: i64,
    origin: ProgOrigin,
    radius_km: f64,
    channels: Vec<propagation::memchan::Channel>,
}

fn load_radioprog() -> RadioProgFile {
    std::fs::read_to_string(radioprog_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn store_radioprog(f: &RadioProgFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    std::fs::write(radioprog_path(), json)
        .map_err(|e| format!("couldn't save programming projects: {e}"))
}

/// All saved programming projects.
#[tauri::command]
fn radioprog_list_projects() -> Result<Vec<RadioProgProject>, String> {
    Ok(load_radioprog().projects)
}

/// Create/update one project (upsert by id; stamps updatedUtc, and createdUtc on first save).
#[tauri::command]
fn radioprog_save_project(mut project: RadioProgProject) -> Result<(), String> {
    let mut f = load_radioprog();
    f.version = 1;
    project.updated_utc = now_unix();
    if let Some(existing) = f.projects.iter_mut().find(|p| p.id == project.id) {
        project.created_utc = existing.created_utc;
        *existing = project;
    } else {
        project.created_utc = now_unix();
        f.projects.push(project);
    }
    store_radioprog(&f)
}

/// Delete one project by id (missing id is a no-op success).
#[tauri::command]
fn radioprog_delete_project(id: String) -> Result<(), String> {
    let mut f = load_radioprog();
    f.projects.retain(|p| p.id != id);
    store_radioprog(&f)
}

/// Render a channel list to an export format: "chirp" (the CHIRP generic CSV, analog rows only)
/// or "csv" (the plain spreadsheet dump). The UI saves the returned text via its download path.
/// `attribution` names the data source shown in the file's trailing comment ("" = none).
#[tauri::command]
fn export_channels(
    channels: Vec<propagation::memchan::Channel>,
    format: String,
    name_cap: usize,
    attribution: String,
) -> Result<String, String> {
    let cap = name_cap.clamp(4, 16);
    match format.as_str() {
        "chirp" => Ok(propagation::chirp::to_chirp_csv(
            &channels,
            cap,
            &attribution,
        )),
        "csv" => Ok(propagation::memchan::to_generic_csv(
            &channels,
            &attribution,
        )),
        other => Err(format!("unknown export format: {other}")),
    }
}

/// Exact local lookup of one park by reference (offline, instant). `None` if the ref is malformed
/// or not in the loaded directory — the caller then falls back to the live lookup.
#[tauri::command]
fn lookup_park(
    parks: State<'_, SharedParks>,
    reference: String,
) -> Result<Option<ParkDto>, String> {
    let idx = parks.lock().map_err(|e| e.to_string())?;
    Ok(idx.lookup(&reference).map(ParkDto::from))
}

/// Live lookup of one park's details from the POTA directory (name/grid/location + coordinates).
/// Used when the local list is empty/stale or when coordinates are wanted. Blocking HTTP off the
/// main thread. `reference` should be a normalized ref.
#[tauri::command]
async fn lookup_park_live(reference: String) -> Result<ParkDto, String> {
    let detail = tauri::async_runtime::spawn_blocking(move || {
        propagation::live::pota::fetch_park(&reference)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(ParkDto::from(detail))
}

// ----- coordinated QSY ("move together") — a separate, opt-in feature ------
//
// All no-ops while disabled. Enabling/disabling + the channel set/cadence are
// persisted; the move-now/pause overrides are transient.

/// Enable or disable coordinated QSY. Enabling captures the current channel as
/// home and the selected peer as the roaming partner; disabling returns home.
#[tauri::command]
fn qsy_set_enabled(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qsy_set_enabled(on);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist qsy enable: {e}");
    }
    Ok(eng.snapshot())
}

/// Set the QSY channel set (band-plan tokens) + announce cadence (overs/hop).
#[tauri::command]
fn qsy_configure(
    state: State<'_, SharedEngine>,
    channels: Vec<String>,
    cadence: u64,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qsy_configure(channels, cadence);
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist qsy config: {e}");
    }
    Ok(eng.snapshot())
}

/// Manual override: force the initiator to announce a move on its next over.
#[tauri::command]
fn qsy_move_now(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qsy_move_now();
    Ok(eng.snapshot())
}

/// Manual override: hold on the current channel (`on=true`) or resume hopping.
#[tauri::command]
fn qsy_pause(state: State<'_, SharedEngine>, on: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qsy_pause(on);
    Ok(eng.snapshot())
}

/// Manual override: stop coordinated QSY and return to the home channel.
#[tauri::command]
fn qsy_stop(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.qsy_stop();
    if let Err(e) = eng.settings().save(&settings_path()) {
        eprintln!("tempo: failed to persist qsy stop: {e}");
    }
    Ok(eng.snapshot())
}

/// Bridges the CAT broker (rigctld server) to Nexus's live engine: other apps read
/// the dial/mode/PTT and can retune Nexus. CAT-sharing (freq/mode) is always on;
/// foreign PTT is ARBITRATED (Engine::broker_ptt): allowed only behind the
/// cat_broker_ptt opt-in, with TX enabled/legal and Nexus idle — Nexus's own key
/// always wins, and un-key is always honored.
#[cfg(feature = "radio")]
struct EngineRig(SharedEngine);

#[cfg(feature = "radio")]
impl tempo_audio::rigctld_server::RigBackend for EngineRig {
    fn freq_hz(&self) -> u64 {
        self.0
            .lock()
            .map(|e| (e.settings().dial_mhz * 1_000_000.0).round() as u64)
            .unwrap_or(0)
    }
    fn mode(&self) -> (String, u32) {
        // Report the CAT mode to a foreign app sharing the radio. When Nexus sets
        // the mode, report that; when it's obeying the radio (rig_mode empty),
        // best-effort report the sideband.
        let m = self
            .0
            .lock()
            .map(|e| {
                let s = e.settings();
                let rm = s.rig_mode();
                if !rm.is_empty() {
                    rm
                } else if s.sideband.trim().is_empty() {
                    "USB".into()
                } else {
                    s.sideband.clone()
                }
            })
            .unwrap_or_else(|_| "USB".into());
        (m, 2700)
    }
    fn ptt(&self) -> bool {
        self.0
            .lock()
            .map(|e| e.snapshot().radio.transmitting)
            .unwrap_or(false)
    }
    fn set_freq(&self, hz: u64) -> bool {
        let Ok(mut e) = self.0.lock() else {
            return false;
        };
        let mhz = hz as f64 / 1_000_000.0;
        // Derive the band label from the freq; keep the current band if off-plan.
        let band = propagation::model::Band::from_mhz(mhz)
            .map(|b| b.label().to_string())
            .unwrap_or_else(|| e.settings().band.clone());
        let mode = {
            let m = e.settings().sideband.clone();
            if m.is_empty() {
                "USB".to_string()
            } else {
                m
            }
        };
        e.set_frequency(mhz, &band, &mode);
        true
    }
    fn set_mode(&self, mode: &str, _passband_hz: u32) -> bool {
        let Ok(mut e) = self.0.lock() else {
            return false;
        };
        let (mhz, band) = {
            let s = e.settings();
            (s.dial_mhz, s.band.clone())
        };
        // Collapse data submodes (PKTUSB/DATA-U/FT8/…) to the underlying sideband.
        let up = mode.to_ascii_uppercase();
        let sb = if up.contains("LSB") {
            "LSB"
        } else if up == "FM" {
            "FM"
        } else {
            "USB"
        };
        e.set_frequency(mhz, &band, sb);
        true
    }
    fn set_ptt(&self, on: bool) -> bool {
        // v2 arbitration: a foreign app may key ONLY when the operator opted in
        // (Settings cat_broker_ptt), TX is enabled/legal, and Nexus is idle —
        // the engine owns the decision (Engine::broker_ptt). Un-key always lands.
        self.0.lock().map(|mut e| e.broker_ptt(on)).unwrap_or(false)
    }
}

// ---- Self-update check (Phase 1: notify + open the download page) ---------------------------
//
// Reads the latest released version from our own update endpoint (version.json), falling back to
// SourceForge's best_release.json, and compares it to this build. The frontend surfaces a single
// dismissible prompt (and remembers a dismissed version); nothing is ever downloaded or run —
// signed auto-update is a later phase.

/// Our own update endpoint (schema 1): a `version.json` with a direct `"latest"` field. Primary,
/// GitHub-first, and under our control — so update accuracy no longer depends on SourceForge's
/// per-release "Default Download" flip.
const VERSION_JSON_URL: &str = "https://hamradiotools.io/nexus/version.json";
/// SourceForge `best_release.json` — the FALLBACK release feed if the endpoint is unreachable.
const BEST_RELEASE_URL: &str = "https://sourceforge.net/projects/nexus-ham-radio/best_release.json";
/// The download page the "Download" button opens — GitHub Releases (primary distribution), which
/// lists every installer plus notes; SourceForge mirrors it.
const DOWNLOAD_PAGE_URL: &str = "https://github.com/kd9taw/nexus/releases/latest";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    /// The running build's version (this crate's CARGO_PKG_VERSION).
    current: String,
    /// Latest version parsed from best_release.json; null if it couldn't be read.
    latest: Option<String>,
    /// True only when `latest` is strictly newer than `current`.
    update_available: bool,
    /// The page the frontend opens for the operator to download the new build.
    download_url: String,
}

/// Blocking GET of a text/JSON URL — mirrors the propagation crate's reqwest usage (rustls, short
/// timeout, a UA). Returns the raw body; call it via `spawn_blocking` from the command.
fn fetch_text(url: &str) -> Result<String, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("nexus/", env!("CARGO_PKG_VERSION"), " (+update-check)"))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())
}

/// Resolve the latest released version: try our own endpoint first (authoritative `latest`), then
/// fall back to SourceForge's best_release.json. `Ok(None)` = a feed was reachable but carried no
/// usable version; `Err` = BOTH feeds failed to fetch (treated as offline → the frontend stays
/// silent). Runs blocking HTTP, so call via `spawn_blocking`.
fn fetch_latest_version() -> Result<Option<String>, String> {
    // Primary: our own endpoint.
    if let Ok(body) = fetch_text(VERSION_JSON_URL) {
        if let Some(v) = tempo_app::update::parse_latest_from_endpoint(&body) {
            return Ok(Some(v));
        }
        // Reachable but unparseable — fall through to the SF feed as a cross-check.
    }
    // Fallback: SourceForge. An error here (with the endpoint also unavailable) means offline.
    let body = fetch_text(BEST_RELEASE_URL)?;
    Ok(tempo_app::update::parse_latest_version(&body))
}

/// Check SourceForge for a newer release. Returns the current/latest versions and whether an
/// update exists; the frontend decides whether to show the dismissible prompt. Returns `Err`
/// offline or on a fetch error — the frontend treats that as a silent no-op (offline honesty).
#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let latest = tauri::async_runtime::spawn_blocking(fetch_latest_version)
        .await
        .map_err(|e| e.to_string())??;
    // Compare against the version Tauri actually shipped (tauri.conf.json) — the SAME source the
    // bundler derives the installer filename from — so the two can never drift into a false nag.
    let current = app.package_info().version.to_string();
    let update_available = latest
        .as_deref()
        .is_some_and(|l| tempo_app::update::version_is_newer(l, &current));
    Ok(UpdateInfo {
        current,
        latest,
        update_available,
        download_url: DOWNLOAD_PAGE_URL.to_string(),
    })
}

/// Open the download page (GitHub Releases) in the operator's default browser. Opened from Rust via
/// the opener plugin, so no JS package or ACL capability entry is required.
#[tauri::command]
fn open_download_page(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(DOWNLOAD_PAGE_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Stamp POTA/SOTA park refs from a pota.app hunter/activator ADIF export onto
/// matching existing QSOs. Stamp-only by design (operator anti-abuse rule): no
/// records are ever created and no existing ref is overwritten.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PotaStampResult {
    stamped: usize,
    already: usize,
    unmatched: usize,
}

#[tauri::command]
fn import_pota_log(state: State<'_, SharedEngine>, text: String) -> Result<PotaStampResult, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let (stamped, already, unmatched) = eng.import_pota_log(&text);
    Ok(PotaStampResult {
        stamped,
        already,
        unmatched,
    })
}

/// Open a station's QRZ.com profile in the operator's default browser (the roster /
/// logbook "who is this?" affordance). The call is sanitized to callsign characters
/// so a crafted string can never smuggle a different URL through; a portable suffix
/// ("PJ4/K1ABC") keeps only its base call, which is what QRZ's db pages key on.
#[tauri::command]
fn open_qrz_page(app: tauri::AppHandle, call: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // Longest slash-separated token = the base call (portable prefixes/suffixes are
    // shorter than the call itself in practice: PJ4/K1ABC, K1ABC/P).
    let base = call
        .split('/')
        .max_by_key(|t| t.len())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if base.is_empty() {
        return Err("no callsign".into());
    }
    app.opener()
        .open_url(format!("https://www.qrz.com/db/{base}"), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's default DMABUF/GBM compositing renderer degrades to a slow software
    // readback path on many Intel-iGPU / hybrid-graphics / newer-Mesa laptops, which
    // makes EVERY webview paint (clicks, scroll, hover) crawl — the "super slow on a
    // laptop, fine on a desktop dGPU" reports. Disabling it forces the reliable path.
    // Must be set before the WebKitWebProcess is forked (it inherits this env), so this
    // is the very first thing run() does. Harmless on machines where DMABUF worked.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let mut settings = Settings::load(&settings_path());

    // One-time migration: an older build kept the Cloudlog/Wavelog API key in
    // plaintext in settings.json. Move any legacy key into the OS keychain and
    // scrub it from the file at rest (the field is now skip-serialized too).
    if !settings.cloudlog_key.trim().is_empty() {
        if let Ok(entry) = cloudlog_keychain() {
            let _ = entry.set_password(settings.cloudlog_key.trim());
        }
        settings.cloudlog_key.clear();
        if let Err(e) = settings.save(&settings_path()) {
            eprintln!("tempo: couldn't re-save settings after Cloudlog key migration: {e}");
        }
    }

    // Build the radio config from settings before the engine takes ownership.
    #[cfg(feature = "radio")]
    let radio_cfg = tempo_audio::service::RadioConfig {
        ptt_method: settings.ptt_method.clone(),
        rig_model: settings.rig_model,
        serial_port: settings.serial_port.clone(),
        baud: settings.baud,
        rig_conn: settings.rig_conn.clone(),
        rig_addr: settings.rig_addr.clone(),
        rigctld_port: settings.rigctld_port,
        icom_native_cat: settings.icom_native_cat,
        broker_self_port: if settings.cat_broker {
            Some(settings.cat_broker_port)
        } else {
            None
        },
        dial_hz: settings.dial_hz(),
        mode: settings.rig_mode(), // DATA submode (PKTUSB/…) for FT8, not voice USB
        wsjtx_udp: settings.wsjtx_udp,
        wsjtx_addr: settings.wsjtx_udp_addr.clone(),
        pskreporter: settings.pskreporter,
        audio_in: settings.audio_in.clone(),
        audio_out: settings.audio_out.clone(),
        tx_level: settings.tx_level,
        rx_gain: settings.rx_gain,
    };

    // The engine boots on the native source; restore a persisted Companion choice
    // below (best-effort — a failed UDP bind falls back to native).
    let persisted_source = settings.source;
    // Cluster/RBN feed config (captured before `settings` moves into the engine).
    let cluster_enabled = settings.cluster_enabled;
    let cluster_hosts = settings.cluster_hosts.clone();
    let cluster_call = settings.mycall.clone();
    let region_grid = settings.mygrid.clone();
    let region_enabled = settings.opening_regional;
    // Which RadioProfile the one chain is registered under. `Settings::load` runs
    // `ensure_radio_profiles`, so this always names a real profile (0 for a migrated
    // single-radio station).
    //
    // KNOWN AND DELIBERATE: this is a BOOT SNAPSHOT. Today the single engine follows whichever
    // radio is active, so switching radios in Settings leaves the registry keyed under the old
    // profile. That is harmless only because nothing reads the registry and the cap forbids a
    // second chain — and re-registering chains when the radio set changes is the first thing the
    // cap-lift has to solve. It is not papered over here, where it would be untestable.
    let engine: SharedEngine = Arc::new(Mutex::new(Engine::with_settings(settings)));
    // Re-seed the decoder's hash table from the logbook so <...> compound-call
    // tokens resolve right after launch (the Fortran table dies with the process).
    if let Ok(eng) = engine.lock() {
        eng.seed_hash_table();
    }

    // Live network feeds (DX-cluster / RBN spots + the PSK Reporter MQTT firehose).
    // Each is spawned once per process (the *_STARTED latches), gated on a real
    // callsign so we never log in to a public service under the `KD9TAW` placeholder.
    // The same start_*_feed helpers are reused by `set_settings`, so entering a real
    // callsign (or enabling the cluster) in Settings starts the feed immediately —
    // no restart needed. The cluster is opt-in (`cluster_enabled`); the firehose
    // mirrors the nowcast's existing PSK Reporter use, so it has no extra toggle.
    let spots: SharedSpots = Arc::new(Mutex::new(tempo_net::cluster::SpotBuffer::default()));
    let live_paths: SharedLivePaths = Arc::new(Mutex::new(propagation::LiveSpots::default()));
    // Live POTA/SOTA activator cache — warmed by a background poller (below) so the Needed
    // board's park/summit chase rows appear from launch, and read by both the hunter panel
    // and the need scorer. Created here (not inline at `.manage`) so the poller can share it.
    let ota_spots: SharedOtaSpots = Arc::new(Mutex::new(std::collections::HashMap::new()));
    // Local park directory — load the cached CSV (if any) so offline search works from launch.
    let parks: SharedParks = Arc::new(Mutex::new(tempo_core::pota::ParkIndex::default()));
    load_parks_cache(&parks);
    // Seed the imported hunted-parks worked-set (POTA "Hunted Parks.CSV") so new-park flags
    // are right from launch — including CW hunts the log can't know about.
    load_hunted_parks_cache(&engine);
    let region_paths = SharedRegionPaths(Arc::new(Mutex::new(propagation::LiveSpots::new(
        propagation::REGION_SPOT_CAP,
    ))));
    let health: SharedHealth = Arc::new(FeedHealthState::default());
    if cluster_enabled {
        start_cluster_feeds(&spots, &cluster_hosts, &cluster_call, &health);
    }
    start_pskr_feed(&live_paths, &cluster_call, &health);
    start_wspr_feed(&live_paths, &cluster_call);
    // Integrated rotator: launch the bundled rotctld when a model is configured.
    if let Ok(eng) = engine.lock() {
        sync_rotctld(eng.settings());
    }
    if region_enabled {
        start_pskr_region_feed(&region_paths, &cluster_call, &region_grid);
    }
    // Record the call the feeds were started under, so a later Settings rename
    // knows to tear them down and reconnect (topics/login are call-bound).
    if let Ok(mut c) = PREV_FEED_CALL.lock() {
        *c = cluster_call.trim().to_string();
    }
    // Feed the live DXpedition layer the ClubLog key (most-wanted ranks). Pushed,
    // not pulled — keeps the propagation crate decoupled from settings IO.
    if let Ok(eng) = engine.lock() {
        propagation::live::dxped::set_clublog_key(&effective_clublog_key(
            &eng.settings().clublog_api_key,
        ));
    }
    // Warm the DXpedition plan cache in the background so the Needed board's
    // expedition tagging works from launch — without it the cache stays cold until
    // the operator first opens Connect/DXpeditions.
    std::thread::spawn(|| {
        let _ = propagation::live::dxped::fetch_plans();
    });
    // Warm the POTA/SOTA activator cache in the background on a slow cadence (~3 min;
    // activations run minutes-to-hours), so the Needed board surfaces live park/summit
    // chase rows from launch — without waiting for the operator to open the POTA/SOTA
    // panel. Both programs each cycle; failures are ignored (transient feed hiccup) and
    // retried next tick. Lock-only writes to the same per-program cache the panel + the
    // need scorer read. SOTA fetches a wider window (50) since it's sparse and count-based.
    {
        let ota = ota_spots.clone();
        std::thread::spawn(move || loop {
            for (prog, fetched) in [
                ("POTA", propagation::live::pota::fetch_pota_spots()),
                ("SOTA", propagation::live::pota::fetch_sota_spots(50)),
            ] {
                if let Ok(v) = fetched {
                    if let Ok(mut c) = ota.lock() {
                        c.insert(prog.to_string(), (now_unix(), v));
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(180));
        });
    }

    // Point the logbook at its ADIF file and load prior contacts (so worked-
    // before highlighting and the log view reflect previous sessions), and
    // restore the persisted signal source.
    if let Ok(mut eng) = engine.lock() {
        // Wire the DXCC entity resolver (cty.dat lives in the propagation crate)
        // so new-DXCC decode highlighting works; set it BEFORE loading the log so
        // the initial worked-entity index is populated.
        eng.set_dxcc_resolver(|call| {
            propagation::dxcc::resolve(call).map(|i| i.entity.to_string())
        });
        // Grid-rarity gems: geography table + the measured-activity census
        // (demote-only refinement). Restore the persisted census BEFORE any
        // stamping so the first snapshot already shows refined tiers.
        if let Ok(text) = std::fs::read_to_string(census_path()) {
            if let Ok(c) = serde_json::from_str::<propagation::gridrarity::RarityCensus>(&text) {
                if let Ok(mut g) = propagation::gridrarity::census().write() {
                    *g = c;
                }
            }
        }
        // Openings log: restore the persisted episode history (missing/corrupt →
        // empty, same best-effort contract as the census).
        if let Ok(text) = std::fs::read_to_string(openings_log_path()) {
            if let Ok(eps) = serde_json::from_str::<Vec<propagation::OpeningEpisode>>(&text) {
                *OPENINGS_LOG.lock().unwrap_or_else(|e| e.into_inner()) = eps;
            }
        }
        eng.set_grid_rarity_resolver(propagation::gridrarity::effective_tier_u8);
        // LoTW-user marks: restore the persisted ARRL activity list (if the
        // operator ever fetched it) and wire the recency-windowed resolver.
        if let Ok(csv) = std::fs::read_to_string(lotw_users_path()) {
            let map = propagation::live::lotw_users::parse_user_activity(&csv);
            if !map.is_empty() {
                if let Ok(meta) = std::fs::read_to_string(lotw_users_meta_path()) {
                    if let Ok(m) = serde_json::from_str::<LotwUsersMeta>(&meta) {
                        LOTW_FETCHED_AT.store(m.fetched_at, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                if let Ok(mut g) = LOTW_ACTIVITY.write() {
                    *g = map;
                }
            }
        }
        LOTW_MAX_AGE_DAYS.store(
            eng.settings().lotw_max_age_days,
            std::sync::atomic::Ordering::Relaxed,
        );
        eng.set_lotw_resolver(|call| {
            let max_secs =
                LOTW_MAX_AGE_DAYS.load(std::sync::atomic::Ordering::Relaxed) as i64 * 86_400;
            LOTW_ACTIVITY
                .read()
                .ok()
                .and_then(|m| m.get(&call.to_uppercase()).copied())
                .is_some_and(|t| now_unix() - t <= max_secs)
        });
        eng.set_log_path(logbook_path());
        // The Field Day contest log journals to its own ADIF beside the logbook —
        // written per contact and restored when FD mode starts, so a mid-event
        // restart loses nothing.
        eng.set_fd_log_path(fd_backup_path());
        // A contact left in the confirm-before-log popup by a previous session (crash, power
        // loss, or a quit with the popup open) is a REAL QSO — the other station logged it.
        // Restore the hold so the operator can still log it. Best-effort: a missing or corrupt
        // journal just means there was nothing pending.
        eng.set_pending_qso_path(pending_qso_path());
        if let Ok(text) = std::fs::read_to_string(pending_qso_path()) {
            if let Ok(q) = serde_json::from_str::<tempo_app::dto::LoggedQso>(&text) {
                eng.load_pending_qso(q.into());
            }
        }
        // Restore-on-launch (spec §1.1): if the operator left the Field Day
        // master switch on, re-enter FD (passive S&P) so a crash/restart during a
        // 24-hour contest comes back operating and the durable journal (set just
        // above) restores the contest log. Must follow set_fd_log_path so the
        // merge finds the journal. This is the ONLY auto-entry, and only because
        // the operator left `fd_active` on — no date/default path ever sets it.
        eng.restore_field_day_if_enabled();
        // Saved RX-period WAVs (settings.save_wav) land beside the QSO recordings.
        eng.set_periods_dir(&recordings_dir().join("periods").to_string_lossy());
        // Restore the store-and-forward outbound queue BEFORE the conversation
        // threads: each restored bubble's held-vs-abandoned decision reads the live
        // queue (a held message whose journal entry survived stays "waiting to send"
        // and transmits when its peer is next heard). Best-effort like the others.
        eng.set_pending_msgs_path(pending_msgs_path());
        if let Ok(text) = std::fs::read_to_string(pending_msgs_path()) {
            eng.load_pending_msgs(&text);
        }
        // Restore persisted Tempo conversation threads so chat history (and the `*`
        // band feed) survives an app restart. Best-effort: a missing/corrupt file
        // just yields an empty roster of threads.
        if let Ok(text) = std::fs::read_to_string(conversations_path()) {
            if let Ok(convs) = serde_json::from_str::<Vec<tempo_app::dto::Conversation>>(&text) {
                eng.load_conversations(convs);
            }
        }
        if persisted_source == SourceKind::Companion {
            if let Err(e) = eng.set_source(SourceKind::Companion) {
                eprintln!("tempo: could not restore Companion source ({e}); using native");
            }
        }
    }

    // Decay + persist the grid-activity census on a slow cadence (10 min): the
    // decay keeps a one-off DXpedition from permanently un-raring a water grid,
    // and the small JSON survives restarts. Skips the write when nothing is
    // tracked (no idle disk churn).
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(600));
        let json = propagation::gridrarity::census().write().ok().map(|mut c| {
            c.decay(now_unix());
            (c.is_empty(), serde_json::to_string(&*c))
        });
        if let Some((empty, Ok(text))) = json {
            if !empty {
                let _ = std::fs::write(census_path(), text);
            }
        }
    });

    // Persist Tempo conversation threads to disk on a slow cadence so chat history
    // survives a restart — only writes when something changed (no idle disk churn).
    {
        let save_engine = engine.clone();
        std::thread::spawn(move || {
            let mut last: Option<String> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(15));
                let convs = save_engine
                    .lock()
                    .map(|e| e.export_conversations())
                    .unwrap_or_else(|e| e.into_inner().export_conversations());
                if let Ok(text) = serde_json::to_string(&convs) {
                    if last.as_deref() != Some(text.as_str()) && write_conversations_atomic(&text) {
                        last = Some(text);
                    }
                }
            }
        });
    }

    // Connector auto-upload worker — THE single funnel for QRZ / ClubLog / eQSL
    // pushes. Every `Engine::log_qso` path queues its record (the engine
    // auto-log included — the path that used to silently skip every connector);
    // this thread drains the queue and pushes per the Settings toggles, stamping
    // the per-QSO upload state and an operator-facing upload note (UI toast).
    // Runs regardless of the `radio` feature: Companion-sourced QSOs upload too.
    {
        let push_engine = engine.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let (recs, qrz_on, clublog_on, eqsl_on, hrdlog_on, n3fjp_on, cloudlog_on, dxk) = {
                // Recover a poisoned lock (conn_log pattern) — a panicked command
                // holding the engine must not silently kill auto-upload forever.
                let mut eng = push_engine.lock().unwrap_or_else(|e| e.into_inner());
                let (q, c, e, h, n, cl, dxk) = {
                    let s = eng.settings();
                    (
                        s.qrz_logbook_upload,
                        s.clublog_upload,
                        s.eqsl_upload,
                        s.hrdlog_upload,
                        s.n3fjp_upload && !s.n3fjp_host.trim().is_empty(),
                        s.cloudlog_upload && !s.cloudlog_url.trim().is_empty(),
                        // DXKeeper: empty host = off. Carried as (host, base_port, uploads)
                        // rather than a bool because the push needs all three and re-locking
                        // the engine per record just to re-read them would be wasteful.
                        (!s.dxkeeper_host.trim().is_empty()).then(|| {
                            (
                                s.dxkeeper_host.trim().to_string(),
                                s.dxkeeper_base_port,
                                s.dxkeeper_uploads,
                            )
                        }),
                    )
                };
                // DXKeeper counts toward "anything enabled" — otherwise a station using ONLY
                // DXKeeper would drain nothing and never forward a single QSO.
                if !(q || c || e || h || n || cl || dxk.is_some()) {
                    // Nothing enabled: LEAVE the queue intact (bounded at 256) so
                    // flipping a toggle on later still uploads this session's
                    // recent QSOs — log-first-configure-later must not lose them.
                    continue;
                }
                (eng.take_pending_uploads(), q, c, e, h, n, cl, dxk)
            };
            // ClubLog suspended (403 latch): skip that leg instead of erroring
            // per QSO — the suspension was announced once; re-push covers later.
            let clublog_live =
                clublog_on && !CLUBLOG_SUSPENDED.load(std::sync::atomic::Ordering::Relaxed);
            for p in recs {
                let rec = p.rec.clone();
                // DXKeeper is deliberately OUTSIDE the legs/retry machinery: it never
                // acknowledges, so there is no success to distinguish from failure and
                // nothing meaningful to retry. A failed connect almost always just means
                // DXLab is not running. Fire it and move on.
                if let Some((host, base, uploads)) = dxk.clone() {
                    dxkeeper_push_async(
                        host,
                        base,
                        uploads,
                        tempo_core::logbook::adif_record(&rec),
                    );
                }
                let failed = auto_push_one(
                    &push_engine,
                    LoggedQso::from(p.rec),
                    qrz_on,
                    clublog_live,
                    eqsl_on,
                    hrdlog_on,
                    n3fjp_on,
                    cloudlog_on,
                    p.legs,
                );
                // Transient failures (network down / service busy) → re-queue ONLY
                // the legs that failed so the next tick retries them, without
                // re-pushing the legs that already succeeded (no double-upload).
                if failed != 0 {
                    let mut eng = push_engine.lock().unwrap_or_else(|e| e.into_inner());
                    eng.requeue_upload(rec, failed, p.attempts.saturating_add(1));
                }
            }
        });
    }

    // With the `radio` feature, drive the real sound card + rig (and the WSJT-X
    // UDP / PSK Reporter outputs) on a background thread, sharing the engine the
    // UI commands lock.
    #[cfg(feature = "radio")]
    {
        let radio_engine = engine.clone();
        std::thread::spawn(move || {
            // The radio loop is the heartbeat — if it dies (error OR panic), TX/RX
            // is silently dead. Surface it loudly in the UI (the audio_error lane
            // renders as a persistent banner) instead of an invisible eprintln.
            let eng_for_report = radio_engine.clone();
            let eng_for_loop = radio_engine;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                tempo_audio::service::run_radio(eng_for_loop, radio_cfg)
            }));
            let msg = match result {
                Ok(Ok(())) => "Radio engine stopped unexpectedly — restart Nexus.".to_string(),
                Ok(Err(e)) => {
                    format!("RADIO ENGINE STOPPED — TX/RX is dead until you restart Nexus ({e})")
                }
                Err(_) => {
                    "RADIO ENGINE CRASHED — TX/RX is dead until you restart Nexus.".to_string()
                }
            };
            eprintln!("tempo: {msg}");
            let _ = eng_for_report
                .lock()
                .map(|mut eng| eng.set_audio_error(Some(msg)));
        });
    }

    // AI CW decoder (beta) is started in the Tauri `.setup()` below, where the app's
    // cross-platform resource dir is available — the DeepCW model ships as a bundled
    // resource, and resolving it by hand from the exe path only worked on Windows.

    // CAT broker: let other apps (WSJT-X / N1MM / loggers) share the radio THROUGH
    // Nexus over the rigctld protocol. Localhost-only (never expose the rig to the
    // network). A manager thread HOT-APPLIES the setting — turning "Share my radio" on
    // starts the broker with no restart, turning it off (or changing the port) stops/
    // rebinds it. Works whether Nexus owns rigctld or is coexisting onto an external one
    // (the broker reads the shared radio through the Engine, not the daemon).
    #[cfg(feature = "radio")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mgr_engine = engine.clone();
        std::thread::spawn(move || {
            // The running broker as (port, shutdown flag); None = not serving.
            let mut running: Option<(u16, std::sync::Arc<AtomicBool>)> = None;
            loop {
                let want = match mgr_engine.lock() {
                    Ok(e) => e
                        .settings()
                        .cat_broker
                        .then_some(e.settings().cat_broker_port),
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    }
                };
                let have = running.as_ref().map(|(p, _)| *p);
                if want != have {
                    // Stop the current broker (disabled, or the port changed).
                    if let Some((_, shutdown)) = running.take() {
                        shutdown.store(true, Ordering::Relaxed);
                    }
                    // Start on the wanted port.
                    if let Some(port) = want {
                        match std::net::TcpListener::bind(("127.0.0.1", port)) {
                            Ok(l) => {
                                let shutdown = std::sync::Arc::new(AtomicBool::new(false));
                                let backend: std::sync::Arc<
                                    dyn tempo_audio::rigctld_server::RigBackend,
                                > = std::sync::Arc::new(EngineRig(mgr_engine.clone()));
                                let sd = shutdown.clone();
                                std::thread::spawn(move || {
                                    tempo_audio::rigctld_server::serve_until(l, backend, sd)
                                });
                                running = Some((port, shutdown));
                                conn_log(
                                    "CAT broker",
                                    "info",
                                    format!("sharing this radio on 127.0.0.1:{port}"),
                                );
                            }
                            Err(e) => conn_log(
                                "CAT broker",
                                "error",
                                format!("couldn't bind 127.0.0.1:{port}: {e}"),
                            ),
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        });
    }

    let prop_cache: PropCache = Arc::new(Mutex::new(None));
    let aurora_cache: AuroraCache = Arc::new(Mutex::new(None));
    let kc2g_cache: Kc2gCache = Arc::new(Mutex::new(None));
    let proton_cache: ProtonCache = Arc::new(Mutex::new(None));
    let scales_cache: ScalesCache = Arc::new(Mutex::new(None));

    // NOT registered as managed state, deliberately. `Chains` would have to be keyed by
    // `RadioProfile::id`, and the only id available here is a BOOT SNAPSHOT of
    // `settings.active_radio`. Switching radios in Settings does not rebuild the engine —
    // `Engine::set_active_radio` mutates settings in place on the same engine — so the entry
    // would sit filed under a dead profile id the moment the operator switches, with no refresh
    // hook and no assertion. The first caller writing the obvious
    // `chains.get(chain_of(w).unwrap_or(active))` would then get `None` for the LIVE radio: the
    // exact wrong-rig class this addressing layer exists to make unrepresentable, reintroduced
    // one layer down.
    //
    // Nothing reads the registry yet, so managing it buys nothing and stores a fact that is
    // knowably wrong. Re-keying on radio-switch is the cap-lift's problem, and the cap-lift is
    // where the registry acquires its first reader. The type and its tests stay — they are the
    // proven artifact that branch starts from.
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(engine)
        .manage(prop_cache)
        .manage(aurora_cache)
        .manage(kc2g_cache)
        .manage(proton_cache)
        .manage(scales_cache)
        .manage(spots)
        .manage(live_paths)
        .manage(ota_spots)
        .manage(parks)
        .manage(region_paths)
        .manage(health)
        .manage(SharedOpeningTracker::default())
        .manage(SharedWxHistory::default())
        .manage(SharedQrzSession::default())
        .manage(SharedHamQthSession::default())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            send_message,
            select_peer,
            archive_conversation,
            set_tier,
            set_source,
            get_spectrum_row,
            set_mode,
            get_settings,
            set_settings,
            export_log,
            export_general_log,
            save_text_to_downloads,
            civ_diagnostic_log,
            all_txt_location,
            reveal_all_txt,
            open_qrz_page,
            import_pota_log,
            set_skip_tx1,
            civ_diagnostic_status,
            broadcast,
            get_serial_ports,
            get_serial_ports_detailed,
            get_audio_devices,
            detect_rigs,
            probe_cat_ports,
            point_rotator,
            stop_rotator,
            discover_flex,
            get_sat_schedule,
            get_iss_pass,
            get_sat_detail,
            start_sat_track,
            stop_sat_track,
            sat_track_status,
            point_rotator_at_call,
            read_rotator,
            cw_decode,
            set_ai_cw,
            cw_clear,
            preview_cw,
            cw_skim,
            rtty_arm,
            get_rtty_state,
            rtty_send,
            rtty_stop,
            rtty_clear,
            rtty_afc_reset,
            rtty_net,
            rtty_set_auto,
            rtty_auto_cq,
            rtty_auto_answer,
            rtty_auto_abort,
            sstv_arm,
            get_sstv_state,
            sstv_send,
            sstv_stop,
            get_rig_models,
            get_all_rig_models,
            get_band_plan,
            set_license_class,
            get_licensed_band_plan,
            set_frequency,
            set_operating_mode,
            work_spot,
            get_connection_log,
            get_openings_log,
            get_credentials_status,
            send_cw,
            set_cw_peer_info,
            set_cw_wpm,
            stop_cw,
            set_cw_keyer,
            set_ptt,
            set_rf_power,
            set_mic_gain,
            set_nr_level,
            set_agc,
            set_split,
            set_rig_func,
            set_sideband_override,
            set_filter_width,
            set_scope_span,
            set_scope_ref,
            set_flex_pan_span,
            set_flex_pan_ref,
            set_scope_fixed,
            set_rit,
            set_xit,
            set_vfo,
            swap_vfo,
            play_voice_message,
            stop_voice,
            start_voice_recording,
            cancel_voice_recording,
            stop_voice_recording,
            import_voice_message,
            set_voice_label,
            clear_voice_message,
            get_voice_messages,
            start_qso_recording,
            stop_qso_recording,
            set_tx_enabled,
            set_tx_level,
            set_rx_gain,
            set_active_radio,
            set_peg_lock,
            add_radio,
            remove_radio,
            rename_radio,
            set_radio_bands,
            set_tune,
            halt_tx,
            test_cat,
            set_tx_even,
            set_tx_cycle_auto,
            set_beacon,
            set_rx_offset,
            set_tx_offset,
            override_next_tx,
            redecode,
            start_cq,
            call_cq,
            set_chat_cq,
            resume_chat_cq,
            notify_erase,
            qrz_test_connection,
            set_hunt_target,
            clear_hunt_target,
            fd_log_manual,
            n3fjp_test_connection,
            set_hold_tx_freq,
            call_station,
            open_panel_window,
            dock_bandmap_window,
            set_area,
            qso_resend,
            qso_freetext,
            log_current_qso,
            confirm_pending_log,
            discard_pending_log,
            log_qso,
            get_log,
            edit_qso,
            mark_qsl_sent,
            delete_qso,
            purge_log,
            get_awards,
            get_journey,
            get_log_stats,
            get_confirmation_diagnostics,
            import_adif,
            sync_lotw_report,
            set_lotw_password,
            clear_lotw_password,
            download_lotw_report,
            upload_lotw_report,
            mark_lotw_uploaded,
            set_eqsl_password,
            clear_eqsl_password,
            download_eqsl_report,
            set_qrz_password,
            clear_qrz_password,
            set_hamqth_password,
            clear_hamqth_password,
            qrz_lookup,
            check_for_update,
            open_download_page,
            set_qrz_logbook_key,
            clear_qrz_logbook_key,
            set_cloudlog_key,
            clear_cloudlog_key,
            qrz_push_qso,
            sync_qrz,
            set_clublog_password,
            clear_clublog_password,
            clublog_push_qso,
            eqsl_push_qso,
            set_hrdlog_code,
            clear_hrdlog_code,
            hrdlog_push_qso,
            get_ota_spots,
            search_parks,
            parks_count,
            hunted_parks_count,
            import_parks_csv,
            import_hunted_parks_csv,
            download_parks,
            lookup_park,
            lookup_park_live,
            set_repeaterbook_token,
            repeater_search,
            geocode_city,
            radioprog_list_projects,
            radioprog_save_project,
            radioprog_delete_project,
            export_channels,
            set_activation,
            clear_activation,
            get_activation,
            get_need_alerts,
            get_all_spots,
            get_propagation,
            get_path_outlook,
            get_band_outlook,
            get_getting_out,
            get_aurora,
            get_pca,
            get_declination,
            get_satellites,
            get_contests,
            post_spot,
            get_lotw_users_status,
            fetch_lotw_users,
            get_kc2g_muf,
            get_space_wx_scales,
            get_xray_now,
            get_dxped_windows,
            get_feed_health,
            qsy_set_enabled,
            qsy_configure,
            qsy_move_now,
            qsy_pause,
            qsy_stop
        ])
        .setup(|app| {
            // Classic startup splash: the borderless `splashscreen` window shows on top for ~3s
            // while the `main` window (declared hidden) loads behind it; then reveal main and
            // close the splash. A plain thread timer — no dependency on the frontend being ready.
            use tauri::Manager;
            // Enforce the minimum window size at runtime too. The tauri.conf `minWidth`/
            // `minHeight` are set (both — Tauri no-ops minWidth alone), but re-applying here
            // in DPI-aware logical px is belt-and-suspenders across platforms. 900x600 is the
            // smallest window at which auto fit-scaling stays usable.
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.set_min_size(Some(tauri::LogicalSize::new(900.0, 600.0)));
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if let Some(main) = handle.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
                if let Some(splash) = handle.get_webview_window("splashscreen") {
                    let _ = splash.close();
                }
            });
            // AI CW decoder (beta): resolve the DeepCW model from the app's cross-platform
            // RESOURCE dir (where Tauri bundles `resources/deepcw/*`) — next to the exe on
            // Windows, under usr/lib/<app>/resources on a Linux .deb/AppImage. The prior
            // exe-adjacent lookup only worked on Windows, so Linux reported "model not
            // installed" though the model ships in the bundle. Try the resource-dir candidates
            // (and the exe-adjacent one) and pick the first that exists; a genuinely missing
            // model just surfaces the honest status.
            #[cfg(feature = "radio")]
            {
                let mut candidates: Vec<std::path::PathBuf> = Vec::new();
                if let Ok(res) = app.path().resource_dir() {
                    candidates.push(res.join("resources").join("deepcw"));
                    candidates.push(res.join("deepcw"));
                }
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(d) = exe.parent() {
                        candidates.push(d.join("resources").join("deepcw"));
                    }
                }
                let dir = candidates
                    .iter()
                    .find(|d| d.is_dir())
                    .cloned()
                    .unwrap_or_else(|| std::path::PathBuf::from("resources/deepcw"));
                let eng = app.state::<SharedEngine>().inner().clone();
                tempo_audio::aicw::spawn_ai_cw(eng, dir);
            }
            // Seed the SSTV gallery session list from the persisted gallery.json
            // so past images are browsable immediately (the decode thread only
            // appends). Parsed here (not via tempo-audio) so non-radio builds
            // still show the gallery.
            {
                let entries: Vec<tempo_app::dto::SstvGalleryEntry> =
                    std::fs::read_to_string(sstv_gallery_dir().join("gallery.json"))
                        .ok()
                        .and_then(|text| serde_json::from_str(&text).ok())
                        .unwrap_or_default();
                if !entries.is_empty() {
                    if let Ok(mut eng) = app.state::<SharedEngine>().inner().lock() {
                        eng.load_sstv_gallery(entries);
                    }
                }
            }
            // RTTY + SSTV RX decode threads — RX ONLY (arming is per-session
            // runtime state; nothing here can key PTT or emit TX audio).
            #[cfg(feature = "radio")]
            {
                tempo_audio::rttyrx::spawn_rtty_rx(app.state::<SharedEngine>().inner().clone());
                tempo_audio::sstvrx::spawn_sstv_rx(
                    app.state::<SharedEngine>().inner().clone(),
                    sstv_gallery_dir(),
                );
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let app = window.app_handle();
                // Remember the band-map window's size+position so it reopens where it was.
                // Captured on close (not per move/resize) so we write the file once, not on
                // every drag frame. Fetch the WebviewWindow by label (the event gives the base
                // Window); no-op for any non-band-map window.
                if let Some(w) = app.get_webview_window(window.label()) {
                    capture_bandmap_window(&w);
                }
                // Closing the MAIN window tears down the whole app: close any torn-off panel
                // windows too, so they don't linger — persisting each band map's geometry first.
                if window.label() == "main" {
                    for (label, w) in app.webview_windows() {
                        if label != "main" {
                            capture_bandmap_window(&w);
                            let _ = w.close();
                        }
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the Nexus application")
        .run(|app_handle, event| {
            // Flush Tempo conversation history on app exit so a quit within the 15 s
            // periodic-save window doesn't lose recent chat or resurrect an archived
            // thread. ExitRequested fires on the app-level quit (Alt+F4 / menu quit).
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Unkey the transmitter before the process dies: signal the radio
                // loop to drop PTT and give it a brief window to flush the un-key
                // command to the rig. A stuck carrier on quit is a TX-safety
                // hazard, so this blocks the exit for up to ~250 ms.
                #[cfg(feature = "radio")]
                {
                    use std::sync::atomic::Ordering;
                    tempo_audio::service::SHUTDOWN.store(true, Ordering::Relaxed);
                    // Wait until the loop has actually unkeyed (SHUTDOWN_DONE),
                    // not a fixed sleep: the loop only reaches the un-key after
                    // its current step() returns, and a step can be blocked in a
                    // CAT read for up to ~2.5 s. Poll so the common case returns
                    // in tens of ms while the worst case still flushes the
                    // un-key before we let the process exit.
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(3_000);
                    while !tempo_audio::service::SHUTDOWN_DONE.load(Ordering::Relaxed)
                        && std::time::Instant::now() < deadline
                    {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
                persist_conversations(app_handle.state::<SharedEngine>().inner());
                persist_field_day_log(app_handle.state::<SharedEngine>().inner());
                // An opening still in progress at quit becomes a journaled episode —
                // a 6m Es evening isn't lost because the app closed mid-opening.
                if let Ok(mut tr) = app_handle.state::<SharedOpeningTracker>().lock() {
                    record_opening_episodes(tr.close_all(now_unix()));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{
        b64_decode, b64_encode, is_complete_lotw_body, iss_pass_from_tles, parse_sstv_mode,
        profile_dir_name, sanitize_profile,
    };

    #[test]
    fn profile_names_are_sanitized_to_safe_dirs() {
        // Good names pass through untouched.
        assert_eq!(sanitize_profile("radio-a").as_deref(), Some("radio-a"));
        assert_eq!(sanitize_profile("K9_2m").as_deref(), Some("K9_2m"));
        assert_eq!(sanitize_profile("  hf  ").as_deref(), Some("hf")); // trimmed
        // Anything that could escape the config root or split it into junk → None (default).
        assert_eq!(sanitize_profile(""), None);
        assert_eq!(sanitize_profile("../evil"), None); // path escape
        assert_eq!(sanitize_profile("a/b"), None);
        assert_eq!(sanitize_profile("space bar"), None);
        assert_eq!(sanitize_profile(&"x".repeat(33)), None); // too long
    }

    #[test]
    fn profile_dir_name_default_matches_the_shared_root() {
        // Default profile's config dir == the shared "tempo" dir → single instance is unchanged
        // and shares the log with itself (no migration). A named profile is isolated but the
        // shared log dir stays "tempo", so the two never share config yet always share the log.
        assert_eq!(profile_dir_name(None), "tempo");
        assert_eq!(profile_dir_name(Some("radio-a")), "tempo-radio-a");
        assert_ne!(profile_dir_name(Some("radio-a")), profile_dir_name(None));
    }

    // Well-known 2008 ISS element set (NORAD 25544) — the same vectors the
    // propagation sat tests use. Pure geometry, so a fixed epoch is deterministic.
    const ISS_NAME: &str = "ISS (ZARYA)";
    const ISS_L1: &str = "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927";
    const ISS_L2: &str = "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";
    const ISS_EPOCH_UNIX: i64 = 1_221_913_539;
    // EN52 — south-central Wisconsin, well inside the ISS ground track.
    const EN52: (f64, f64) = (42.5, -89.0);

    fn iss_tle() -> propagation::sat::Tle {
        propagation::sat::Tle {
            name: ISS_NAME.into(),
            line1: ISS_L1.into(),
            line2: ISS_L2.into(),
        }
    }

    #[test]
    fn iss_pass_requires_the_iss_by_norad_number() {
        // A set with only a NON-ISS bird (NORAD 43017) — the 25544 filter rejects
        // it, so the auto-arm can never tune off a name collision or wrong bird.
        let ao91 = propagation::sat::Tle {
            name: "AO-91".into(),
            line1: "1 43017U 17073E   08264.51782528  .00000000  00000-0  00000-0 0  0000".into(),
            line2: "2 43017  97.7000 000.0000 0000000 000.0000 000.0000 14.80000000000000".into(),
        };
        assert!(iss_pass_from_tles(&[ao91], EN52, ISS_EPOCH_UNIX).is_none());
        // No elements at all → None (the no-TLE path get_iss_pass short-circuits).
        assert!(iss_pass_from_tles(&[], EN52, ISS_EPOCH_UNIX).is_none());
    }

    #[test]
    fn iss_pass_returns_the_in_progress_pass() {
        // Anchor `now` one minute into the ISS's first modelled pass over EN52;
        // the auto-arm helper must report that same still-open pass (LOS ahead).
        let first = propagation::sat::passes(&iss_tle(), EN52, ISS_EPOCH_UNIX, 24)
            .into_iter()
            .next()
            .expect("at least one ISS pass in 24 h");
        let now = first.aos_unix + 60;
        let got = iss_pass_from_tles(&[iss_tle()], EN52, now).expect("ISS pass found mid-pass");
        assert_eq!(got.name, ISS_NAME);
        assert!(got.los_unix > now, "the reported pass is still above the horizon");
        assert!(got.status.is_none(), "geometry only — no status stamped");
    }

    // A full report ends with the documented `<APP_LoTW_EOF>` trailer.
    const COMPLETE_REPORT: &str = "ARRL Logbook of the World Status Report\n\
<PROGRAMID:4>LoTW\n\
<APP_LoTW_LASTQSL:19>2026-03-01 12:34:56\n\
<APP_LoTW_NUMREC:1>1\n\
<eoh>\n\
<CALL:5>W1AW/4 <BAND:3>20m <MODE:3>FT8 <QSO_DATE:8>20260228 <QSL_RCVD:1>Y <eor>\n\
<APP_LoTW_EOF>\n";

    // The SAME report cut off mid-stream: HTTP-200'd but the `<APP_LoTW_EOF>` trailer
    // (and the tail records before it) never arrived. Advancing the cursor here would
    // skip every confirmation in the truncated-away tail forever — the data-loss bug.
    const TRUNCATED_REPORT: &str = "ARRL Logbook of the World Status Report\n\
<PROGRAMID:4>LoTW\n\
<APP_LoTW_LASTQSL:19>2026-03-01 12:34:56\n\
<APP_LoTW_NUMREC:2>2\n\
<eoh>\n\
<CALL:5>W1AW/4 <BAND:3>20m <MODE:3>FT8 <QSO_DATE:8>20260228 <QSL_RCVD:1>Y <eor>\n\
<CALL:4>K1AB <BAND:3>40m <MOD";

    #[test]
    fn complete_lotw_body_has_eof_trailer() {
        assert!(is_complete_lotw_body(COMPLETE_REPORT));
    }

    #[test]
    fn truncated_lotw_body_is_incomplete_so_cursor_holds() {
        // Regression: the cursor advance MUST be gated on this returning false.
        assert!(!is_complete_lotw_body(TRUNCATED_REPORT));
    }

    #[test]
    fn eof_trailer_match_is_case_insensitive() {
        assert!(is_complete_lotw_body("...<eor>\n<app_lotw_eof>\n"));
    }

    // RFC 4648 vectors — the SSTV preview encoder must match standard base64
    // exactly (the UI decodes it with atob()).
    #[test]
    fn b64_matches_rfc4648_vectors() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_encode(&[0xFF, 0xEF, 0xBE]), "/+++");
    }

    #[test]
    fn b64_decode_inverts_encode_including_the_rfc_vectors() {
        // RFC 4648 vectors, the inverse of the encode test above.
        assert_eq!(b64_decode("").unwrap(), b"");
        assert_eq!(b64_decode("Zg==").unwrap(), b"f");
        assert_eq!(b64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(b64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(b64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(b64_decode("/+++").unwrap(), vec![0xFF, 0xEF, 0xBE]);
        // Padding-optional + round-trips arbitrary bytes (the SSTV RGB payload).
        let payload: Vec<u8> = (0u16..300).map(|n| (n * 7 % 256) as u8).collect();
        assert_eq!(b64_decode(&b64_encode(&payload)).unwrap(), payload);
        // Rejects a stray non-alphabet byte rather than silently mangling the image.
        assert!(b64_decode("Zm9v!").is_err());
    }

    #[test]
    fn parse_sstv_mode_resolves_slugs_case_insensitively() {
        assert!(parse_sstv_mode("scottie1").is_some());
        assert!(parse_sstv_mode("ScottieDX").is_some()); // case-insensitive
        assert!(parse_sstv_mode("pd120").is_some());
        assert!(parse_sstv_mode("martin2").is_some());
        assert!(parse_sstv_mode("robot36").is_some());
        assert!(parse_sstv_mode("nonsense").is_none());
    }
}
