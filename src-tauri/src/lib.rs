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
use tauri::State;
use tempo_app::dto::{
    AppSnapshot, DiagnosticsReportDto, ImportStats, LoggedQso, LotwSyncResult, SourceKind,
    Spectrum, Tier, UploadReportDto,
};
use tempo_app::engine::Engine;
use tempo_app::settings::Settings;

/// The engine, shared between UI commands and the radio loop.
type SharedEngine = Arc<Mutex<Engine>>;

/// Cached propagation nowcast: `(fetched_at, snapshot)`. Caching enforces PSK
/// Reporter's ≥5-minute-per-dataset query limit across UI polls.
type PropCache = Arc<Mutex<Option<(std::time::Instant, propagation::PropagationSnapshot)>>>;
/// TTL cache for the OVATION aurora oval (distinct payload type from PropCache, so
/// a distinct TypeId for `.manage()`).
type AuroraCache = Arc<Mutex<Option<(std::time::Instant, Vec<propagation::live::aurora::AuroraPoint>)>>>;

/// Recent DX-cluster / RBN spots, fed by the background cluster thread and read
/// by `get_need_alerts`.
type SharedSpots = Arc<Mutex<tempo_net::cluster::SpotBuffer>>;

/// The cluster thread runs for the process lifetime (a desktop daemon thread);
/// this stop flag exists only so `cluster::run`'s signature is satisfied.
static CLUSTER_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One-shot latch: the cluster daemon thread is spawned at most once per process,
/// whether at startup or lazily when a real callsign is first entered in Settings.
static CLUSTER_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Recent live PSK Reporter MQTT reception reports (the "getting out" firehose),
/// fed by the background MQTT thread and merged into the propagation nowcast.
type SharedLivePaths = Arc<Mutex<propagation::LiveSpots>>;

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

/// One-shot latch for the PSK Reporter MQTT daemon thread (see CLUSTER_STARTED).
static PSKR_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One-shot latch for the near-region opening MQTT daemon thread (Phase 2).
static PSKR_REGION_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Keep a near-region spot only if an end is within this many km of the operator
/// (~one Es hop — "openings I could plausibly join"). The per-band global stream
/// is gated to this radius client-side (the broker can't filter by region).
const REGION_RADIUS_KM: f64 = 2000.0;

/// Liveness of the background live feeds, updated from their daemon threads and
/// read by `get_feed_health` for the Now-Bar connector pills. Timestamps are Unix
/// secs of the last *successfully parsed* event; `0` = none yet this session.
#[derive(Default)]
struct FeedHealthState {
    /// Last parsed DX-cluster / RBN spot.
    cluster_last: std::sync::atomic::AtomicI64,
    /// Last successfully parsed PSK Reporter MQTT report.
    pskr_last_event: std::sync::atomic::AtomicI64,
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

/// Spawn the DX-cluster / RBN daemon thread once per process (the [`CLUSTER_STARTED`]
/// latch makes repeat calls a no-op, so it is safe to call both at startup and from
/// `set_settings`). No-op unless `mycall` is a [`is_real_call`]; the caller is
/// responsible for the `cluster_enabled` gate. Parsed spots flow into `spots`, and
/// each one stamps `health.cluster_last` for the Now-Bar liveness pill.
fn start_cluster_feed(
    spots: &SharedSpots,
    cluster_host: &str,
    mycall: &str,
    health: &SharedHealth,
) {
    if !is_real_call(mycall) || CLUSTER_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let buf = spots.clone();
    let hp = health.clone();
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
    let buf = live_paths.clone();
    let hp = health.clone();
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
    /// "off" | "waiting" | "live" | "idle" (only meaningful when `enabled`).
    state: String,
}

/// Liveness of both background live feeds.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedHealth {
    cluster: FeedStatus,
    pskr: FeedStatus,
}

fn feed_status(started: bool, last: i64, now: i64) -> FeedStatus {
    if !started {
        return FeedStatus {
            enabled: false,
            last_event_secs: None,
            state: "off".into(),
        };
    }
    if last == 0 {
        return FeedStatus {
            enabled: true,
            last_event_secs: None,
            state: "waiting".into(),
        };
    }
    let age = (now - last).max(0);
    FeedStatus {
        enabled: true,
        last_event_secs: Some(age),
        state: if age <= FEED_FRESH_SECS {
            "live"
        } else {
            "idle"
        }
        .into(),
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
    FeedHealth {
        cluster: feed_status(
            CLUSTER_STARTED.load(Relaxed),
            health.cluster_last.load(Relaxed),
            now,
        ),
        pskr: feed_status(
            PSKR_STARTED.load(Relaxed),
            health.pskr_last_event.load(Relaxed),
            now,
        ),
    }
}

/// How long a live propagation nowcast is reused before refetching (seconds).
const PROP_TTL_SECS: u64 = 300;

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

/// Full UI snapshot (`AppSnapshot`) — the UI renders all three zones from this.
#[tauri::command]
fn get_snapshot(state: State<'_, SharedEngine>) -> Result<AppSnapshot, String> {
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
fn select_peer(state: State<'_, SharedEngine>, peer: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.select_peer(&peer);
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
/// (then a demo scene) if a fetch fails or the operator hasn't set a callsign.
#[tauri::command]
fn get_propagation(
    state: State<'_, SharedEngine>,
    cache: State<'_, PropCache>,
    live_paths: State<'_, SharedLivePaths>,
    region_paths: State<'_, SharedRegionPaths>,
    opening_tracker: State<'_, SharedOpeningTracker>,
    spots: State<'_, SharedSpots>,
) -> Result<propagation::PropagationSnapshot, String> {
    let (mycall, mygrid, needs) = {
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
        (mycall, mygrid, needs)
    };

    let now = now_unix();

    // No callsign → can't query "who hears me"; show the demo scene (which keeps
    // its own openings). Checked BEFORE the cache so a cleared callsign can't keep
    // serving the previous identity's live-labeled openings from a warm cache.
    if mycall.trim().is_empty() {
        return Ok(propagation::demo());
    }

    // --- base snapshot: fresh cache, else a live refetch, else last-good/demo ---
    let cached = {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        guard
            .as_ref()
            .filter(|(when, _)| when.elapsed().as_secs() < PROP_TTL_SECS)
            .map(|(_, snap)| snap.clone())
    };

    let mut snap = if let Some(s) = cached {
        s
    } else {
        // Live PSK Reporter MQTT spots since the last rebuild, merged with the
        // rate-limited XML query. Refetch (blocking HTTP); on failure keep the
        // last good snapshot (marked stale), else demo.
        let extra = live_paths
            .lock()
            .map(|b| b.recent(now, 1800))
            .unwrap_or_default();
        match propagation::live::snapshot_with_spots(&mycall, &mygrid, 1800, &needs, &extra) {
            Ok(snap) => {
                if let Ok(mut guard) = cache.lock() {
                    *guard = Some((std::time::Instant::now(), snap.clone()));
                }
                snap
            }
            Err(_) => {
                let guard = cache.lock().map_err(|e| e.to_string())?;
                guard
                    .as_ref()
                    .map(|(_, s)| {
                        let mut s = s.clone();
                        s.source = "cached".to_string();
                        s
                    })
                    .unwrap_or_else(propagation::demo)
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
        xray_long: if snap.space_wx.flare { 1e-5 } else { 1e-7 },
    };
    // Merge the operator's own-call window with the near-region window (disjoint —
    // the region feed drops own-call spots). The regional opening gate is enabled
    // only when there's regional data, so a band can light up from activity around
    // the operator, not just their own contacts.
    let mut wide = live_paths
        .lock()
        .map(|b| b.recent(now, cfg.base_w))
        .unwrap_or_default();
    let mut regional_scope = false;
    if let Ok(r) = region_paths.0.lock() {
        let regional = r.recent(now, cfg.base_w);
        if !regional.is_empty() {
            regional_scope = true;
            wide.extend(regional);
        }
    }
    // Bridge the DX-cluster / RBN firehose (a continent-wide who-hears-whom stream
    // on every band) into the SAME window so the band ladder + opening detector see
    // real band activity, not just the operator's own-call traffic. RBN/cluster
    // lines rarely carry grids, so these light up band LIVENESS (the "activity"
    // census) even if region/bearing still needs gridded PSKR spots.
    if let Ok(buf) = spots.lock() {
        let cluster = buf.recent_within(
            std::time::Instant::now(),
            std::time::Duration::from_secs(cfg.base_w as u64),
        );
        for cs in cluster {
            if let Some(band) = propagation::model::Band::from_mhz(cs.freq_mhz()) {
                wide.push(propagation::PathSpot {
                    time: now,
                    tx_call: cs.dx_call.to_uppercase(),
                    tx_grid: None,
                    rx_call: cs.spotter.to_uppercase(),
                    rx_grid: None,
                    band,
                    mode: None,
                    snr: None,
                });
                regional_scope = true; // we now have wide-area data → advisor uses it
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

    // Rebuild the band advisor on the SAME merged window (own paths + near-region
    // census) the opening detector uses — so the "Bands — what's open now" list
    // reflects activity AROUND the operator, not only bands they've personally
    // been heard on. (The cached snapshot's advisory was built from own-call spots
    // alone, which is why bands you weren't using all read "Closed".)
    if regional_scope {
        snap.advisory = propagation::PropAdvisor::new(&mycall, &mygrid).advise(now, &wide, &wx);
    }

    Ok(snap)
}

/// Per-path HF outlook to a selected station's `grid` — the heuristic
/// PathPredictor (the VOACAP-ready seam) over the operator↔DX great circle, under
/// the current space weather. Answers "is THIS path workable, which band, when"
/// for a station you may have no live spots on. Empty bands if either grid is
/// unknown (operator hasn't set a grid, or the station has none).
#[tauri::command]
fn get_path_outlook(
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

/// "Am I getting out?" — who is hearing the operator right now, from the live PSK
/// Reporter / RBN firehose (spots where the operator is the TX side). Pure
/// observed data — the most reassuring live answer a station can get.
#[tauri::command]
fn get_getting_out(
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
fn get_aurora(
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
    mut settings: Settings,
) -> Result<AppSnapshot, String> {
    // Capture the feed config before `settings` moves into the engine.
    let cluster_enabled = settings.cluster_enabled;
    let cluster_host = settings.cluster_host.clone();
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
        if let Err(e) = settings.save(&settings_path()) {
            eprintln!("tempo: failed to persist settings: {e}");
        }
        eng.apply_settings(settings);
        eng.snapshot()
    }; // release the engine lock before spawning feed threads

    if cluster_enabled {
        start_cluster_feed(spots.inner(), &cluster_host, &mycall, health.inner());
    }
    start_pskr_feed(live_paths.inner(), &mycall, health.inner());
    if opening_regional {
        start_pskr_region_feed(region_paths.inner(), &mycall, &mygrid);
    }
    Ok(snap)
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
fn get_serial_ports() -> Vec<String> {
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
fn get_audio_devices() -> AudioDevices {
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
fn detect_rigs() -> Vec<DetectedRigDto> {
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
fn test_cat(state: State<'_, SharedEngine>) -> Result<CatTestResult, String> {
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
    // Tier-aware: FT8/FT4 → the standard WSJT-X watering holes; FT1/DX1 → native plan.
    let tier = state.lock().map_err(|e| e.to_string())?.tier();
    Ok(tempo_app::bandplan::band_plan_for(tier))
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

/// Initiate a directed QSO with a specific station (the UI "work this station"
/// action). Enters QSO mode answering `call`. Returns the refreshed snapshot.
#[tauri::command]
fn call_station(
    state: State<'_, SharedEngine>,
    call: String,
    grid: Option<String>,
) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let g = grid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    eng.call_station_with_grid(&call, g);
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

/// Need-aware spotting: rank the stations the operator is hearing right now (the
/// roster, on the current band) by award value — new DXCC entity / CQ zone / band
/// slot / mode — so "new ones" surface from the live decodes. Offline (native
/// roster); a telnet-cluster / RBN / PSK-Reporter feed is a later increment.
#[tauri::command]
fn get_need_alerts(
    state: State<'_, SharedEngine>,
    spots: State<'_, SharedSpots>,
) -> Result<Vec<propagation::NeedAlert>, String> {
    let eng = state.lock().map_err(|e| e.to_string())?;
    let mut needs = propagation::LogNeeds::new();
    for q in eng.get_log() {
        needs.add(&q.call, &q.band, &q.mode, q.award_confirmed);
    }
    let snap = eng.snapshot();
    drop(eng); // nothing below needs the engine — don't hold the hot lock across spots
    let band = snap.radio.band.clone();
    // Stations heard natively on the current band...
    let mut heard: Vec<propagation::Heard> = snap
        .stations
        .iter()
        .map(|s| propagation::Heard {
            call: s.call.clone(),
            band: band.clone(),
            mode: "FT8".to_string(), // FT-family → Digital class; the band is what varies
        })
        .collect();
    // ...plus recent DX-cluster / RBN spots (each on its own band, derived from
    // freq). Only the last 15 min — "heard now" must mean recent.
    if let Ok(buf) = spots.lock() {
        for cs in buf.recent_within(
            std::time::Instant::now(),
            std::time::Duration::from_secs(900),
        ) {
            // Pick a real mode keyword out of the comment (RBN leads with it;
            // human spots may lead with "up 2" etc.) rather than the first token.
            let mode = cs
                .comment
                .split_whitespace()
                .find(|t| {
                    matches!(
                        t.to_ascii_uppercase().as_str(),
                        "CW" | "SSB"
                            | "USB"
                            | "LSB"
                            | "AM"
                            | "FM"
                            | "RTTY"
                            | "PSK"
                            | "FT8"
                            | "FT4"
                            | "JT65"
                            | "JT9"
                            | "MFSK"
                    )
                })
                .unwrap_or("FT8");
            if let Some(h) = propagation::heard_from_freq(&cs.dx_call, cs.freq_mhz(), mode) {
                heard.push(h);
            }
        }
    }
    Ok(propagation::rank_needs(
        &heard,
        &needs,
        needs.worked_zones(),
    ))
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
fn sync_lotw_report(
    state: State<'_, SharedEngine>,
    text: String,
) -> Result<LotwSyncResult, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.merge_lotw_report(&text).into())
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
        return clear_keychain_entry(&entry);
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))
}

/// Remove the stored LoTW password from the OS keychain (idempotent).
#[tauri::command]
fn clear_lotw_password() -> Result<(), String> {
    clear_keychain_entry(&lotw_keychain()?)
}

/// Store (or, if empty, clear) the eQSL website password in the OS keychain.
/// Write-only, like the LoTW counterpart.
#[tauri::command]
fn set_eqsl_password(password: String) -> Result<(), String> {
    let entry = eqsl_keychain()?;
    if password.is_empty() {
        return clear_keychain_entry(&entry);
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))
}

/// Remove the stored eQSL password from the OS keychain (idempotent).
#[tauri::command]
fn clear_eqsl_password() -> Result<(), String> {
    clear_keychain_entry(&eqsl_keychain()?)
}

/// Store (or, if empty, clear) the QRZ.com account password in the OS keychain.
/// Write-only, like the LoTW/eQSL counterparts.
#[tauri::command]
fn set_qrz_password(password: String) -> Result<(), String> {
    let entry = qrz_keychain()?;
    if password.is_empty() {
        return clear_keychain_entry(&entry);
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))
}

/// Remove the stored QRZ password from the OS keychain (idempotent).
#[tauri::command]
fn clear_qrz_password() -> Result<(), String> {
    clear_keychain_entry(&qrz_keychain()?)
}

/// Store (or, if empty, clear) the QRZ **Logbook API key** (distinct from the XML
/// password) in the OS keychain. Write-only.
#[tauri::command]
fn set_qrz_logbook_key(key: String) -> Result<(), String> {
    let entry = qrz_logbook_keychain()?;
    if key.is_empty() {
        return clear_keychain_entry(&entry);
    }
    entry
        .set_password(&key)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))
}

/// Remove the stored QRZ Logbook API key from the OS keychain (idempotent).
#[tauri::command]
fn clear_qrz_logbook_key() -> Result<(), String> {
    clear_keychain_entry(&qrz_logbook_keychain()?)
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
                    eprintln!("tempo: failed to persist LoTW cursor: {e}");
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
            Ok(_) => eprintln!("tempo: LoTW own-echo pull returned a non-ADIF body; skipped"),
            Err(e) => eprintln!("tempo: LoTW own-echo pull failed (confirmations still synced): {e}"),
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
fn upload_lotw_report(
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
fn qrz_lookup(
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
fn qrz_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
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
        let mut eng = state.lock().map_err(|e| e.to_string())?;
        eng.stamp_qrz_upload(&rec, outcome, now_unix(), detail);
    }
    Ok(push.into())
}

// ----- ClubLog realtime QSO push --------------------------------------------

/// Store (or, if empty, clear) the ClubLog **Application Password** in the OS
/// keychain. Also re-arms ClubLog auto-push (a credential change clears the 403
/// suspend latch). Write-only.
#[tauri::command]
fn set_clublog_password(password: String) -> Result<(), String> {
    CLUBLOG_SUSPENDED.store(false, std::sync::atomic::Ordering::Relaxed);
    let entry = clublog_keychain()?;
    if password.is_empty() {
        return clear_keychain_entry(&entry);
    }
    entry
        .set_password(&password)
        .map_err(|e| format!("couldn't save to the system keychain: {e}"))
}

/// Remove the stored ClubLog app-password from the OS keychain (idempotent); also
/// re-arms auto-push.
#[tauri::command]
fn clear_clublog_password() -> Result<(), String> {
    CLUBLOG_SUSPENDED.store(false, std::sync::atomic::Ordering::Relaxed);
    clear_keychain_entry(&clublog_keychain()?)
}

/// Push one logged QSO to ClubLog (realtime). Resolves the 4 credentials (email +
/// callsign∥mycall + api-key from Settings or the build-time `option_env!` + the
/// keychain app-password), uploads, and classifies the HTTP-status response. A 403
/// **suspends** further auto-pushes this session (ClubLog IP-blocks hammering)
/// until a credential changes. No lock over the network.
#[tauri::command]
fn clublog_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
) -> Result<tempo_app::dto::ClubLogPushResultDto, String> {
    use std::sync::atomic::Ordering;
    if CLUBLOG_SUSPENDED.load(Ordering::Relaxed) {
        return Err(
            "ClubLog auto-upload paused after an auth failure — fix your credentials in Settings."
                .to_string(),
        );
    }
    let (email, callsign_setting, api_setting, mycall) = {
        let eng = state.lock().map_err(|e| e.to_string())?;
        let s = eng.settings();
        (
            s.clublog_email.trim().to_string(),
            s.clublog_callsign.trim().to_string(),
            s.clublog_api_key.trim().to_string(),
            s.mycall.trim().to_string(),
        )
    };
    // API key: Settings first, else the build-time baked key (official installer).
    let api_key = if !api_setting.is_empty() {
        api_setting
    } else {
        option_env!("CLUBLOG_API_KEY").unwrap_or("").to_string()
    };
    if api_key.is_empty() {
        return Err("No ClubLog API key set — Nexus ships none (open-source). Get a free key at clublog.org/requestapikey.php and add it in Settings.".to_string());
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
        CLUBLOG_SUSPENDED.store(true, Ordering::Relaxed);
    }
    // Record the outcome on the just-pushed QSO so diagnostics can surface R1 (never
    // pushed to ClubLog) / R9 (bounced). Transient results (ServerError/Unknown) map
    // to None → leave it unstamped for a clean retry.
    if let Some(outcome) = push.result.to_upload_outcome() {
        let detail = push
            .message
            .as_deref()
            .and_then(tempo_core::lotw_upload::sanitize_detail);
        let mut eng = state.lock().map_err(|e| e.to_string())?;
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
fn eqsl_push_qso(
    record: LoggedQso,
    state: State<'_, SharedEngine>,
) -> Result<UploadReportDto, String> {
    let user = {
        let eng = state.lock().map_err(|e| e.to_string())?;
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
            let mut eng = state.lock().map_err(|e| e.to_string())?;
            eng.stamp_eqsl_upload(&rec, outcome, now_unix(), None);
            Ok(UploadReportDto {
                dispatched: 1,
                outcome: outcome.code().to_string(),
                detail: None,
            })
        }
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
#[tauri::command]
fn get_ota_spots(program: String) -> Result<Vec<propagation::OtaSpot>, String> {
    match program.to_ascii_uppercase().as_str() {
        "POTA" => propagation::live::pota::fetch_pota_spots(),
        "SOTA" => propagation::live::pota::fetch_sota_spots(30),
        other => Err(format!("Unknown program '{other}' — use POTA or SOTA.")),
    }
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
        let m = self.0.lock().map(|e| e.settings().sideband.clone()).unwrap_or_default();
        (if m.is_empty() { "USB".into() } else { m }, 2700)
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
        mode: settings.sideband.clone(),
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
    let cluster_host = settings.cluster_host.clone();
    let cluster_call = settings.mycall.clone();
    let region_grid = settings.mygrid.clone();
    let region_enabled = settings.opening_regional;
    let engine: SharedEngine = Arc::new(Mutex::new(Engine::with_settings(settings)));

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
        start_cluster_feed(&spots, &cluster_host, &cluster_call, &health);
    }
    start_pskr_feed(&live_paths, &cluster_call, &health);
    if region_enabled {
        start_pskr_region_feed(&region_paths, &cluster_call, &region_grid);
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
        eng.set_log_path(logbook_path());
        if persisted_source == SourceKind::Companion {
            if let Err(e) = eng.set_source(SourceKind::Companion) {
                eprintln!("tempo: could not restore Companion source ({e}); using native");
            }
        }
    }

    // With the `radio` feature, drive the real sound card + rig (and the WSJT-X
    // UDP / PSK Reporter outputs) on a background thread, sharing the engine the
    // UI commands lock.
    #[cfg(feature = "radio")]
    {
        let radio_engine = engine.clone();
        std::thread::spawn(move || {
            if let Err(e) = tempo_audio::service::run_radio(radio_engine, radio_cfg) {
                eprintln!("tempo: radio loop stopped: {e}");
            }
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
                    Err(e) => eprintln!("tempo: CAT broker couldn't bind 127.0.0.1:{broker_port}: {e}"),
                }
            });
        }
    }

    let prop_cache: PropCache = Arc::new(Mutex::new(None));
    let aurora_cache: AuroraCache = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .manage(engine)
        .manage(prop_cache)
        .manage(aurora_cache)
        .manage(spots)
        .manage(live_paths)
        .manage(region_paths)
        .manage(health)
        .manage(SharedOpeningTracker::default())
        .manage(SharedQrzSession::default())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            send_message,
            select_peer,
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
            set_frequency,
            set_tx_enabled,
            set_tx_level,
            set_tune,
            halt_tx,
            test_cat,
            set_tx_even,
            set_rx_offset,
            set_tx_offset,
            set_hold_tx_freq,
            call_station,
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
            get_awards,
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
            get_getting_out,
            get_aurora,
            get_feed_health,
            qsy_set_enabled,
            qsy_configure,
            qsy_move_now,
            qsy_pause,
            qsy_stop
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Tempo application");
}
