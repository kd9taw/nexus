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
//! - `set_tier { tier }` -> `AppSnapshot`  (tier is "FT1" | "FT8" | "FT4" | "DX1")
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

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
type AuroraCache = Arc<Mutex<Option<(std::time::Instant, Vec<propagation::live::aurora::AuroraPoint>)>>>;
/// TTL cache for the KC2G ionosonde MUF map. Distinct payload type → distinct
/// TypeId for `.manage()`.
type Kc2gCache = Arc<Mutex<Option<(std::time::Instant, Vec<propagation::MufStation>)>>>;
/// TTL cache for the NOAA R/S/G scales + recent SWPC alerts (one fetch pair).
/// Distinct payload type → distinct TypeId for `.manage()`.
type ScalesCache =
    Arc<Mutex<Option<(std::time::Instant, (propagation::NoaaScalesView, Vec<propagation::AlertView>))>>>;

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
type SharedOtaSpots = Arc<Mutex<std::collections::HashMap<String, (i64, Vec<propagation::OtaSpot>)>>>;

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
    /// Last parsed spot from ANY HUMAN DX-cluster node — the SSB/phone source. The RBN
    /// skimmer firehoses (CW/digital) do NOT stamp this, so it answers "is my phone source
    /// actually up?" independently of the always-busy RBN feeds (whose traffic otherwise
    /// keeps `cluster_last` green even when every human node is down). Per-node connected
    /// state lives in [`PHONE_NODE_CONNS`]; this is the shared aggregate freshness.
    phone_cluster_last: std::sync::atomic::AtomicI64,
}
type SharedHealth = Arc<FeedHealthState>;

/// Cached QRZ XML session key (in-memory only — it's IP-bound, short-lived, and
/// re-derivable from the keychain password, so it never persists). `None` = not
/// logged in yet / expired.
type SharedQrzSession = Arc<Mutex<Option<String>>>;

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
    start_cluster_feed(spots, RBN_DIGITAL_HOST, mycall, health, &RBN_DIGITAL_STARTED);
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
        format!("connecting to {} as {}", cluster_host, mycall.trim().to_uppercase()),
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
                    b.push(sp.clone());
                }
            },
            &CLUSTER_STOP,
            &hp_conn.cluster_connected,
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
                hp.phone_cluster_last
                    .store(ts, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut b) = buf.lock() {
                    b.push(sp.clone());
                }
            },
            &CLUSTER_STOP,
            &conn,
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
            |topic, _payload| {
                if let Some(spot) = propagation::parse_pskr_mqtt(topic, now_unix()) {
                    hp.pskr_last_event
                        .store(now_unix(), std::sync::atomic::Ordering::Relaxed);
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
        let topics = propagation::pskr_region_topics();
        let topic_refs: Vec<&str> = topics.iter().map(|s| s.as_str()).collect();
        let base_w = propagation::OpeningConfig::default().base_w;
        // No Now-Bar pill for the region feed — session state isn't surfaced.
        let connected = std::sync::atomic::AtomicBool::new(false);
        tempo_net::mqtt::subscribe(
            PSKR_MQTT_ADDR,
            &format!("nexus-rgn-{call}"),
            &topic_refs,
            |topic, _payload| {
                let now = now_unix();
                let Some(spot) = propagation::parse_pskr_mqtt(topic, now) else {
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
    }
}

/// How long a live propagation nowcast is reused before refetching (seconds).
const PROP_TTL_SECS: u64 = 300;

/// When the last live-propagation refetch was ATTEMPTED (success or failure).
/// Rate-limits refetches to one per [`PROP_TTL_SECS`] independent of the UI poll
/// cadence, so a failing fetch on a cold cache can't storm PSK Reporter into 429s.
static PROP_FETCH_BACKOFF: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Where settings are persisted: `%APPDATA%\tempo\settings.json` on Windows,
/// `$XDG_CONFIG_HOME`/`~/.config/tempo/settings.json` on Unix, else CWD.
fn settings_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("tempo")
        .join("settings.json")
}

/// Where the ADIF logbook is persisted: the same base dir as [`settings_path`],
/// i.e. `%APPDATA%\tempo\log.adi` on Windows / `~/.config/tempo/log.adi` on Unix.
fn logbook_path() -> PathBuf {
    settings_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("log.adi")
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
    let eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.snapshot())
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

/// Archive (hide) a conversation thread (the recents-list hide affordance).
/// Returns the refreshed snapshot.
#[tauri::command]
fn archive_conversation(
    state: State<'_, SharedEngine>,
    peer: String,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.archive_conversation(&peer);
    Ok(eng.snapshot())
}

/// Switch the waveform mode/tier ("FT1" | "FT8" | "FT4" | "DX1"). Operator-
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
            needs.add(&q.call, &q.band, &q.mode, q.award_confirmed);
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
            // rate-limited XML query. Refetch (blocking HTTP).
            let extra = live_paths
                .lock()
                .map(|b| b.recent(now, 1800))
                .unwrap_or_default();
            propagation::live::snapshot_with_spots(&mycall, &mygrid, 1800, &needs, &extra)
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
                            .is_some_and(|rx| propagation::geo::haversine_km(me, rx) <= REGION_RADIUS_KM),
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
                    // spot frequency + (usually) the mode — what map click-to-work needs.
                    mode: cs.mode().map(str::to_string),
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
    }
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
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
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
                kp: s.space_wx.kp,
                a_index: s.space_wx.a_index,
                xray_long: if s.space_wx.flare { 1e-5 } else { 1e-7 },
            })
            .unwrap_or_default()
    };
    use propagation::PathPredictor as _; // bring the trait's `predict` into scope
    let eng = propagation::HeuristicEngine::new(me);
    Ok(eng.predict(dx, now_unix(), &wx))
}

/// The no-selection "Band outlook (modelled)": modeled per-band workability + MUF to
/// a ring of representative long-haul DX directions (best per band over the ring), so
/// the Connect view can answer "which bands are modeled-open for DX right now" without
/// a selected station. Needs only the operator's grid; empty if it's unset.
#[tauri::command]
async fn get_band_outlook(
    state: State<'_, SharedEngine>,
    cache: State<'_, PropCache>,
) -> Result<propagation::PathPrediction, String> {
    let mygrid = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().mygrid.clone()
    };
    let Some(me) = propagation::geo::maidenhead_to_latlon(mygrid.trim()) else {
        return Ok(propagation::PathPrediction {
            engine: "heuristic".to_string(),
            bands: Vec::new(),
            muf_now: 0.0,
            muf_hourly: Vec::new(),
        });
    };
    let wx = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .map(|(_, s)| propagation::SpaceWx {
                sfi: s.space_wx.sfi,
                kp: s.space_wx.kp,
                a_index: s.space_wx.a_index,
                xray_long: if s.space_wx.flare { 1e-5 } else { 1e-7 },
            })
            .unwrap_or_default()
    };
    // 8 azimuths at ~9000 km — direction-agnostic "best band to ANY far DX now".
    Ok(propagation::band_outlook_ring(me, 9000.0, 8, now_unix(), &wx))
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

/// Real-time KC2G ionosonde MUF/foF2 station fixes for the Connect map's MUF
/// overlay. Cached `KC2G_TTL_SECS`; serves the last-good set on a fetch failure,
/// empty if we never had one (never fabricated).
#[tauri::command]
async fn get_kc2g_muf(
    cache: State<'_, Kc2gCache>,
) -> Result<Vec<propagation::MufStation>, String> {
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
        (Ok(scales), Ok(alerts)) => {
            let pair = (scales, alerts);
            if let Ok(mut g) = cache.lock() {
                *g = Some((std::time::Instant::now(), pair.clone()));
            }
            Ok(pair)
        }
        _ => {
            // Any feed down → serve last-good if we have it, else quiet defaults.
            let g = cache.lock().map_err(|e| e.to_string())?;
            Ok(g.as_ref()
                .map(|(_, pair)| pair.clone())
                .unwrap_or_default())
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
    Ok(eng.settings().clone())
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
        if let Err(e) = settings.save(&settings_path()) {
            eprintln!("tempo: failed to persist settings: {e}");
        }
        eng.apply_settings(settings);
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
        let changed =
            !prev.is_empty() && prev.to_uppercase() != mycall.trim().to_uppercase();
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
        "callsign changed — restarting cluster + PSK Reporter feeds under the new call",
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
        let (cluster_enabled, cluster_hosts, mycall, mygrid, opening_regional) =
            match engine.lock() {
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

/// Tempo's proposed calling-frequency band plan (HF + VHF/UHF), for the band
/// selector. Each entry is General-legal + clear of the existing watering holes.
#[tauri::command]
fn get_band_plan(state: State<'_, SharedEngine>) -> Result<Vec<tempo_app::bandplan::BandChannel>, String> {
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
        ("160m", "HF"), ("80m", "HF"), ("40m", "HF"), ("30m", "HF"), ("20m", "HF"),
        ("17m", "HF"), ("15m", "HF"), ("12m", "HF"), ("10m", "HF"), ("6m", "VHF"),
        ("2m", "VHF"), ("1.25m", "VHF"), ("70cm", "UHF"),
    ];
    let eng = state.lock().map_err(|e| e.to_string())?;
    let class = eng.settings().license_class;
    // The caller (the cockpit) passes its mode explicitly — the engine's operating_mode is
    // set asynchronously on section entry, so reading it here would race the first mount.
    let mode = match mode.to_ascii_lowercase().as_str() {
        "phone" => OperatingMode::Phone,
        "cw" => OperatingMode::Cw,
        _ => OperatingMode::Digital,
    };
    let mut out = Vec::new();
    for (band, group) in BANDS {
        if let Some(dial) = tempo_app::privileges::segment_start(class, band, mode) {
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

/// Set the CW keyer speed in WPM (5–50).
#[tauri::command]
fn set_cw_wpm(state: State<'_, SharedEngine>, wpm: u32) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_cw_wpm(wpm);
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
        return Err(format!("No recording in F{slot} yet — record or import one first"));
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
#[tauri::command]
async fn open_panel_window(app: tauri::AppHandle, panel: String) -> Result<(), String> {
    let slug: String = panel.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if slug.is_empty() {
        return Err("invalid panel".into());
    }
    let label = format!("panel-{slug}");
    // Already open → just focus it (one window per panel).
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
        other => format!("Nexus — {other}"),
    };
    // The Operate cockpit (waterfall + Band Activity + roster) needs more room than the
    // narrower insight panels.
    let (w, h) = if slug == "operate" {
        (1140.0, 760.0)
    } else {
        (760.0, 660.0)
    };
    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(format!("index.html?panel={slug}").into()),
    )
    .title(title)
    .inner_size(w, h)
    .min_inner_size(420.0, 360.0)
    .build()
    .map_err(|e| e.to_string())?;
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
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.log_qso(record.into());
    Ok(eng.snapshot())
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
        );
    }
    Ok(awards.summary())
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
                r.rst_rcvd.as_deref().and_then(|s| s.trim().parse::<i32>().ok())
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
    let eng = state.lock().map_err(|e| e.to_string())?;
    let mut needs = propagation::LogNeeds::new();
    for q in eng.get_log() {
        needs.add(&q.call, &q.band, &q.mode, q.award_confirmed);
    }
    let snap = eng.snapshot();
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
    // CW / Phone (all bands) AND digital on HF come from the DX-cluster + RBN spot
    // firehose, which carries an EXACT frequency (→ click-to-work can QSY to the spot).
    // Cluster spots have no structured mode, so classify each from its comment token /
    // band segment. Digital is admitted on HF ONLY: a pure-digital operator's near-me
    // PSKR census is hardcoded to 10m/6m/4m/2m (region_topics), so without this an HF
    // FT8 new-one (20/40/15m) that's actively spotted has NO path to the Needed board —
    // the "I have HF needs that never show" gap. VHF digital stays out (VHF locality is
    // gated above and Es/MS digital adds noise); the frontend's mode gating still hides
    // CW/Phone rows unless those features are on, so a digital board gains only HF digital.
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
                    .filter(|sp| {
                        match (propagation::skimmer_grid(sp), me_ll) {
                            (Some(g), Some(me)) => {
                                propagation::geo::maidenhead_to_latlon(g).is_some_and(|rx| {
                                    propagation::geo::haversine_km(me, rx)
                                        <= propagation::near_me_radius_km(band)
                                })
                            }
                            _ => false,
                        }
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
                        let my_entity =
                            propagation::dxcc::resolve(&snap.mycall).map(|i| i.entity);
                        my_entity != Some(info.entity)
                            || propagation::geo::haversine_km(me, (info.lat, info.lon))
                                >= propagation::VHF_MIN_DX_KM
                    }
                    _ => false,
                };
                if !dx_far {
                    continue;
                }
            }
            let class = propagation::classify_spot_mode(freq, &cs.comment);
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
                    mode: class.label().to_string(),
                    freq_mhz: Some(freq),
                    // The spot's REAL receive time — stamping poll-time made
                    // every cluster row read "just now" forever.
                    admitted_at: (cs.received_unix > 0)
                        .then_some(cs.received_unix as i64),
                    evidence: Some(format!(
                        "spotted by {} via cluster/RBN",
                        spotters.join(" + ")
                    )),
                });
            }
        }
    }
    let mut alerts = propagation::rank_needs(&heard, &needs, needs.worked_zones());
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
                        a.headline =
                            format!("{} · {} {}", a.headline, sp.program, sp.reference);
                    }
                }
            }
        }
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
const CLUBLOG_KEYCHAIN_USER: &str = "clublog-password";

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
    let (lotw_user, eqsl_user, qrz_user, clublog_email, clublog_key) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let st = eng.settings();
        (
            st.lotw_username.clone(),
            st.eqsl_username.clone(),
            st.qrz_username.clone(),
            st.clublog_email.clone(),
            // The API key is an APPLICATION (developer) credential, not a user
            // one: a Settings override OR the key baked into official installer
            // builds (CLUBLOG_API_KEY at build time) both satisfy it. The user's
            // own credentials are only email + app-password.
            !effective_clublog_key(&st.clublog_api_key).is_empty(),
        )
    };
    let has = |entry: Result<keyring::Entry, String>| {
        entry.and_then(|e| e.get_password().map_err(|er| er.to_string())).is_ok()
    };
    Ok(vec![
        CredStatus {
            connector: "LoTW".into(),
            stored: has(lotw_keychain()),
            identity: lotw_user,
        },
        CredStatus {
            connector: "QRZ XML".into(),
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
    option_env!("CLUBLOG_API_KEY").unwrap_or("").trim().to_string()
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

fn clublog_keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(LOTW_KEYCHAIN_SERVICE, CLUBLOG_KEYCHAIN_USER)
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

/// Store (or, if empty, clear) the QRZ **Logbook API key** (distinct from the XML
/// password) in the OS keychain. Write-only. Saving a key also switches QRZ
/// auto-upload ON: entering the key IS the intent ("upload my QSOs to QRZ") —
/// previously the separate toggle silently stayed off and nothing uploaded.
#[tauri::command]
fn set_qrz_logbook_key(key: String, state: State<'_, SharedEngine>) -> Result<(), String> {
    let entry = qrz_logbook_keychain()?;
    if key.is_empty() {
        clear_keychain_entry(&entry)?;
        conn_log("QRZ Logbook", "info", "API key cleared from the OS keychain");
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
            }
        };
        if already == on {
            return;
        }
        let updated = match which {
            UploadToggle::Qrz => eng.set_upload_toggles(Some(on), None, None),
            UploadToggle::Clublog => eng.set_upload_toggles(None, Some(on), None),
            UploadToggle::Eqsl => eng.set_upload_toggles(None, None, Some(on)),
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
        conn_log("QRZ Logbook", "info", "API key cleared from the OS keychain");
        set_upload_toggle(&state, UploadToggle::Qrz, false);
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
            // Only advance the cursor if the username is still the one this download
            // used. If `set_settings` changed it during the (lock-free) fetch, it
            // already reset the cursor to a full pull for the new identity — this
            // high-water belongs to the old query, so binding it would risk skipping
            // records on the next incremental pull. Persist via a narrow setter so the
            // sync never disturbs live operation (no mode reset / TX-queue clear).
            if eng.settings().lotw_username.trim() == used_username.trim() {
                let updated = eng.set_lotw_cursor(high_water);
                if let Err(e) = updated.save(&settings_path()) {
                    conn_log("LoTW", "error", format!("failed to persist the sync cursor: {e}"));
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
            Ok(_) => conn_log("LoTW", "error", "own-echo pull returned a non-ADIF body; skipped"),
            Err(e) => conn_log("LoTW", "error", format!("own-echo pull failed (confirmations still synced): {e}")),
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
        let location = eng.settings().lotw_station_location.trim().to_string();
        if location.is_empty() {
            return Err("Set your LoTW Station Location in Settings before uploading.".into());
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
        (batch, adif, location, tqsl_path)
    };

    // Write the batch ADIF to a temp file for TQSL to sign.
    let path = std::env::temp_dir().join("nexus_lotw_upload.adi");
    std::fs::write(&path, adif).map_err(|e| format!("Couldn't write the upload file: {e}"))?;
    let path_str = path.to_string_lossy().to_string();

    // Resolve + run TQSL one-shot, capturing its result.
    let tqsl = resolve_tqsl(&tqsl_path);
    let args = tempo_core::lotw_upload::tqsl_args(&location, &path_str);
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

// ----- QRZ.com callsign lookup (session-key XML API) ------------------------

/// Outcome of one lookup attempt with a given session key.
enum QrzOutcome {
    Found(tempo_app::dto::QrzLookupDto),
    NotFound,
    NeedLogin, // the session key is expired/invalid → (re)login
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

/// Look up a callsign on QRZ, enriching with name / (subscriber) grid / QTH /
/// state. Uses the cached session key if valid; on expiry logs in **once** and
/// retries (bounded — never loops). Network runs without any lock held.
#[tauri::command]
async fn qrz_lookup(
    callsign: String,
    state: State<'_, SharedEngine>,
    qrz_session: State<'_, SharedQrzSession>,
) -> Result<tempo_app::dto::QrzLookupDto, String> {
    let call = callsign.trim().to_string();
    if call.is_empty() {
        return Err("Enter a callsign to look up.".to_string());
    }
    let username = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        eng.settings().qrz_username.trim().to_string()
    };
    if username.is_empty() {
        return Err("Set your QRZ username in Settings first.".to_string());
    }
    let password = qrz_keychain()?
        .get_password()
        .map_err(|_| "No QRZ password stored — set it in Settings.".to_string())?;

    let not_found = || format!("{} is not in the QRZ database.", call.to_uppercase());

    // 1) Try the cached key, if any.
    let cached = qrz_session.lock().ok().and_then(|g| g.clone());
    if let Some(key) = cached {
        match qrz_try_lookup(&key, &call)? {
            QrzOutcome::Found(dto) => return Ok(dto),
            QrzOutcome::NotFound => return Err(not_found()),
            QrzOutcome::NeedLogin => {} // fall through to a single re-login
        }
    }

    // 2) Log in once, cache the new key, retry the lookup once (bounded).
    let key = qrz_login(&username, &password)?;
    if let Ok(mut g) = qrz_session.lock() {
        *g = Some(key.clone());
    }
    match qrz_try_lookup(&key, &call)? {
        QrzOutcome::Found(dto) => Ok(dto),
        QrzOutcome::NotFound => Err(not_found()),
        // A fresh key still reporting expiry is anomalous — fail without looping.
        QrzOutcome::NeedLogin => Err("QRZ session error — please try again.".to_string()),
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
    conn_logged("QRZ Logbook", |r| format!("pushed {} — {}", who, r.result), res)
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
        Ok(format!("{owner}{book} — {} QSOs in the online logbook", st.count))
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
        conn_log("ClubLog", "info", "app-password cleared from the OS keychain");
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
    conn_log("ClubLog", "info", "app-password cleared from the OS keychain");
    set_upload_toggle(&state, UploadToggle::Clublog, false);
    Ok(())
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
    conn_logged("eQSL", |r| format!("pushed {} — outcome: {}", who, r.outcome), res)
}

fn eqsl_push_qso_impl(
    record: LoggedQso,
    engine: &SharedEngine,
) -> Result<UploadReportDto, String> {
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
        propagation::live::qrz::post_form(tempo_core::eqsl::EQSL_IMPORT_URL, body)?
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
fn auto_push_one(
    engine: &SharedEngine,
    dto: LoggedQso,
    qrz_on: bool,
    clublog_on: bool,
    eqsl_on: bool,
) {
    let call = dto.call.clone();
    let mut parts: Vec<String> = Vec::new();
    let mut all_ok = true;
    if qrz_on {
        let (part, ok) = match qrz_push_qso_impl(dto.clone(), engine) {
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
                (part, ok)
            }
            Err(e) => {
                conn_log("QRZ Logbook", "error", format!("auto-push {call} — {e}"));
                (format!("QRZ ✗ {e}"), false)
            }
        };
        parts.push(part);
        all_ok &= ok;
    }
    if clublog_on {
        let (part, ok) = match clublog_push_qso_impl(dto.clone(), engine) {
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
                (part, ok)
            }
            Err(e) => {
                conn_log("ClubLog", "error", format!("auto-push {call} — {e}"));
                (format!("ClubLog ✗ {e}"), false)
            }
        };
        parts.push(part);
        all_ok &= ok;
    }
    if eqsl_on {
        let (part, ok) = match eqsl_push_qso_impl(dto, engine) {
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
                (part, ok)
            }
            Err(e) => {
                conn_log("eQSL", "error", format!("auto-push {call} — {e}"));
                (format!("eQSL ✗ {e}"), false)
            }
        };
        parts.push(part);
        all_ok &= ok;
    }
    if !parts.is_empty() {
        note_upload_shared(engine, format!("{call} → {}", parts.join(" · ")), all_ok);
    }
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
            OtaSpotDto { spot: sp, new_park, band_open }
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
/// the dial/mode/PTT and can retune Nexus. v1 is CAT-sharing (freq/mode) for loggers
/// + panadapters; it does NOT key the rig on a foreign app's behalf — Nexus owns TX
/// timing (external-PTT / TX arbitration is a flagged v2 item).
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
        self.0.lock().map(|e| e.snapshot().radio.transmitting).unwrap_or(false)
    }
    fn set_freq(&self, hz: u64) -> bool {
        let Ok(mut e) = self.0.lock() else { return false };
        let mhz = hz as f64 / 1_000_000.0;
        // Derive the band label from the freq; keep the current band if off-plan.
        let band = propagation::model::Band::from_mhz(mhz)
            .map(|b| b.label().to_string())
            .unwrap_or_else(|| e.settings().band.clone());
        let mode = {
            let m = e.settings().sideband.clone();
            if m.is_empty() { "USB".to_string() } else { m }
        };
        e.set_frequency(mhz, &band, &mode);
        true
    }
    fn set_mode(&self, mode: &str, _passband_hz: u32) -> bool {
        let Ok(mut e) = self.0.lock() else { return false };
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
    fn set_ptt(&self, _on: bool) -> bool {
        false // v1: Nexus owns TX; it won't key the rig for a foreign app (v2: arbitration).
    }
}

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings = Settings::load(&settings_path());

    // Build the radio config from settings before the engine takes ownership.
    #[cfg(feature = "radio")]
    let radio_cfg = tempo_audio::service::RadioConfig {
        ptt_method: settings.ptt_method.clone(),
        rig_model: settings.rig_model,
        serial_port: settings.serial_port.clone(),
        baud: settings.baud,
        rigctld_port: settings.rigctld_port,
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
    let region_paths = SharedRegionPaths(Arc::new(Mutex::new(propagation::LiveSpots::new(
        propagation::REGION_SPOT_CAP,
    ))));
    let health: SharedHealth = Arc::new(FeedHealthState::default());
    if cluster_enabled {
        start_cluster_feeds(&spots, &cluster_hosts, &cluster_call, &health);
    }
    start_pskr_feed(&live_paths, &cluster_call, &health);
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
        eng.set_log_path(logbook_path());
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
                    if last.as_deref() != Some(text.as_str())
                        && write_conversations_atomic(&text)
                    {
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
            let (recs, qrz_on, clublog_on, eqsl_on) = {
                // Recover a poisoned lock (conn_log pattern) — a panicked command
                // holding the engine must not silently kill auto-upload forever.
                let mut eng = push_engine.lock().unwrap_or_else(|e| e.into_inner());
                let (q, c, e) = {
                    let s = eng.settings();
                    (s.qrz_logbook_upload, s.clublog_upload, s.eqsl_upload)
                };
                if !(q || c || e) {
                    // Nothing enabled: LEAVE the queue intact (bounded at 256) so
                    // flipping a toggle on later still uploads this session's
                    // recent QSOs — log-first-configure-later must not lose them.
                    continue;
                }
                (eng.take_pending_uploads(), q, c, e)
            };
            // ClubLog suspended (403 latch): skip that leg instead of erroring
            // per QSO — the suspension was announced once; re-push covers later.
            let clublog_live = clublog_on
                && !CLUBLOG_SUSPENDED.load(std::sync::atomic::Ordering::Relaxed);
            for rec in recs {
                auto_push_one(&push_engine, LoggedQso::from(rec), qrz_on, clublog_live, eqsl_on);
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
                Ok(Err(e)) => format!("RADIO ENGINE STOPPED — TX/RX is dead until you restart Nexus ({e})"),
                Err(_) => "RADIO ENGINE CRASHED — TX/RX is dead until you restart Nexus.".to_string(),
            };
            eprintln!("tempo: {msg}");
            let _ = eng_for_report
                .lock()
                .map(|mut eng| eng.set_audio_error(Some(msg)));
        });
    }

    // CAT broker: let other apps (WSJT-X / N1MM / loggers) share the radio THROUGH
    // Nexus over the rigctld protocol. Localhost-only (never expose the rig to the
    // network). Boot-time start when enabled; toggling needs a restart for now.
    #[cfg(feature = "radio")]
    {
        let (broker_on, broker_port) = engine
            .lock()
            .map(|e| (e.settings().cat_broker, e.settings().cat_broker_port))
            .unwrap_or((false, 4532));
        if broker_on {
            let backend: std::sync::Arc<dyn tempo_audio::rigctld_server::RigBackend> =
                std::sync::Arc::new(EngineRig(engine.clone()));
            std::thread::spawn(move || {
                match std::net::TcpListener::bind(("127.0.0.1", broker_port)) {
                    Ok(l) => tempo_audio::rigctld_server::serve(l, backend),
                    Err(e) => conn_log("CAT broker", "error", format!("couldn't bind 127.0.0.1:{broker_port}: {e}")),
                }
            });
        }
    }

    let prop_cache: PropCache = Arc::new(Mutex::new(None));
    let aurora_cache: AuroraCache = Arc::new(Mutex::new(None));
    let kc2g_cache: Kc2gCache = Arc::new(Mutex::new(None));
    let scales_cache: ScalesCache = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .manage(engine)
        .manage(prop_cache)
        .manage(aurora_cache)
        .manage(kc2g_cache)
        .manage(scales_cache)
        .manage(spots)
        .manage(live_paths)
        .manage(SharedOtaSpots::new(Mutex::new(std::collections::HashMap::new())))
        .manage(region_paths)
        .manage(health)
        .manage(SharedOpeningTracker::default())
        .manage(SharedWxHistory::default())
        .manage(SharedQrzSession::default())
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
            broadcast,
            get_serial_ports,
            get_audio_devices,
            detect_rigs,
            get_rig_models,
            get_band_plan,
            set_license_class,
            get_licensed_band_plan,
            set_frequency,
            set_operating_mode,
            work_spot,
            get_connection_log,
            get_credentials_status,
            send_cw,
            set_cw_wpm,
            stop_cw,
            set_cw_keyer,
            set_ptt,
            set_rf_power,
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
            notify_erase,
            qrz_test_connection,
            set_hunt_target,
            clear_hunt_target,
            fd_log_manual,
            n3fjp_test_connection,
            set_hold_tx_freq,
            call_station,
            open_panel_window,
            set_area,
            qso_resend,
            qso_freetext,
            log_current_qso,
            confirm_pending_log,
            discard_pending_log,
            log_qso,
            get_log,
            edit_qso,
            delete_qso,
            purge_log,
            get_awards,
            get_journey,
            get_confirmation_diagnostics,
            import_adif,
            sync_lotw_report,
            set_lotw_password,
            clear_lotw_password,
            download_lotw_report,
            upload_lotw_report,
            set_eqsl_password,
            clear_eqsl_password,
            download_eqsl_report,
            set_qrz_password,
            clear_qrz_password,
            qrz_lookup,
            set_qrz_logbook_key,
            clear_qrz_logbook_key,
            qrz_push_qso,
            set_clublog_password,
            clear_clublog_password,
            clublog_push_qso,
            eqsl_push_qso,
            get_ota_spots,
            set_activation,
            clear_activation,
            get_activation,
            get_need_alerts,
            get_propagation,
            get_path_outlook,
            get_band_outlook,
            get_getting_out,
            get_aurora,
            get_kc2g_muf,
            get_space_wx_scales,
            get_feed_health,
            qsy_set_enabled,
            qsy_configure,
            qsy_move_now,
            qsy_pause,
            qsy_stop
        ])
        .on_window_event(|window, event| {
            // Closing the MAIN window tears down the whole app: close any torn-off
            // panel windows too, so they don't linger (and keep the process alive).
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                let app = window.app_handle();
                for (label, w) in app.webview_windows() {
                    if label != "main" {
                        let _ = w.close();
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
                persist_conversations(app_handle.state::<SharedEngine>().inner());
            }
        });
}
