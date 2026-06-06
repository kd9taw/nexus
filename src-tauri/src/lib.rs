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
    AppSnapshot, ImportStats, LoggedQso, LotwSyncResult, SourceKind, Spectrum, Tier,
};
use tempo_app::engine::Engine;
use tempo_app::settings::Settings;

/// The engine, shared between UI commands and the radio loop.
type SharedEngine = Arc<Mutex<Engine>>;

/// Cached propagation nowcast: `(fetched_at, snapshot)`. Caching enforces PSK
/// Reporter's ≥5-minute-per-dataset query limit across UI polls.
type PropCache = Arc<Mutex<Option<(std::time::Instant, propagation::PropagationSnapshot)>>>;

/// Recent DX-cluster / RBN spots, fed by the background cluster thread and read
/// by `get_need_alerts`.
type SharedSpots = Arc<Mutex<tempo_net::cluster::SpotBuffer>>;

/// The cluster thread runs for the process lifetime (a desktop daemon thread);
/// this stop flag exists only so `cluster::run`'s signature is satisfied.
static CLUSTER_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    let tier: Tier = serde_json::from_value(serde_json::Value::String(tier.clone()))
        .map_err(|_| format!("invalid tier {tier:?}: expected \"FT1\", \"FT8\", \"FT4\", or \"DX1\""))?;
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

    // Serve a fresh cached nowcast without re-querying PSK Reporter.
    {
        let guard = cache.lock().map_err(|e| e.to_string())?;
        if let Some((when, snap)) = guard.as_ref() {
            if when.elapsed().as_secs() < PROP_TTL_SECS {
                return Ok(snap.clone());
            }
        }
    }

    // No callsign yet → can't query "who hears me"; show the demo scene.
    if mycall.trim().is_empty() {
        return Ok(propagation::demo());
    }

    // Refetch live (blocking HTTP); on failure keep the last good snapshot, else demo.
    match propagation::live::snapshot(&mycall, &mygrid, 1800, &needs) {
        Ok(snap) => {
            if let Ok(mut guard) = cache.lock() {
                *guard = Some((std::time::Instant::now(), snap.clone()));
            }
            Ok(snap)
        }
        Err(_) => {
            let guard = cache.lock().map_err(|e| e.to_string())?;
            Ok(guard
                .as_ref()
                .map(|(_, s)| {
                    // Last-good snapshot after a failed refetch — mark it stale
                    // so the UI shows a "cached" chip instead of pretending live.
                    let mut s = s.clone();
                    s.source = "cached".to_string();
                    s
                })
                .unwrap_or_else(propagation::demo))
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
#[tauri::command]
fn set_settings(
    state: State<'_, SharedEngine>,
    settings: Settings,
) -> Result<AppSnapshot, String> {
    if let Err(e) = settings.save(&settings_path()) {
        eprintln!("tempo: failed to persist settings: {e}");
    }
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.apply_settings(settings);
    Ok(eng.snapshot())
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

/// Enable/disable normal slot transmit ("Monitor"). `false` mutes transmit and
/// clears anything queued; `true` re-enables it and clears a tripped watchdog.
#[tauri::command]
fn set_tx_enabled(state: State<'_, SharedEngine>, enabled: bool) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.set_tx_enabled(enabled);
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
fn get_band_plan() -> Vec<tempo_app::bandplan::BandChannel> {
    tempo_app::bandplan::band_plan()
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
fn call_station(state: State<'_, SharedEngine>, call: String) -> Result<AppSnapshot, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    eng.call_station(&call);
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
        for cs in buf.recent_within(std::time::Instant::now(), std::time::Duration::from_secs(900)) {
            // Pick a real mode keyword out of the comment (RBN leads with it;
            // human spots may lead with "up 2" etc.) rather than the first token.
            let mode = cs
                .comment
                .split_whitespace()
                .find(|t| {
                    matches!(
                        t.to_ascii_uppercase().as_str(),
                        "CW" | "SSB" | "USB" | "LSB" | "AM" | "FM" | "RTTY" | "PSK" | "FT8" | "FT4"
                            | "JT65" | "JT9" | "MFSK"
                    )
                })
                .unwrap_or("FT8");
            if let Some(h) = propagation::heard_from_freq(&cs.dx_call, cs.freq_mhz(), mode) {
                heard.push(h);
            }
        }
    }
    Ok(propagation::rank_needs(&heard, &needs, needs.worked_zones()))
}

/// Import an external ADIF logbook (deduped merge → real "needs"). Takes the
/// file's text; the UI reads the file so no fs/dialog plugin is needed.
#[tauri::command]
fn import_adif(state: State<'_, SharedEngine>, text: String) -> Result<ImportStats, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    let (added, skipped, total) = eng.import_adif(&text);
    Ok(ImportStats { added, skipped, total })
}

/// Reconcile a confirmation/credit report (LoTW ADIF export) INTO the existing
/// log — upgrades confirmation + credit on already-logged QSOs (which a plain
/// import would skip), rewrites the log file, and returns the diff + any
/// confirmations that matched no logged QSO. Offline; the live LoTW download is a
/// later increment that feeds this the same ADIF.
#[tauri::command]
fn sync_lotw_report(state: State<'_, SharedEngine>, text: String) -> Result<LotwSyncResult, String> {
    let mut eng = state.lock().map_err(|e| e.to_string())?;
    Ok(eng.merge_lotw_report(&text).into())
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
    let engine: SharedEngine = Arc::new(Mutex::new(Engine::with_settings(settings)));

    // Recent DX-cluster / RBN spots for need-aware spotting. When enabled (and a
    // callsign is set), a background daemon thread connects, logs in, and pushes
    // parsed spots into this buffer; `get_need_alerts` reads it. Opt-in network.
    let spots: SharedSpots = Arc::new(Mutex::new(tempo_net::cluster::SpotBuffer::default()));
    // Gate on a REAL callsign — never log in to a public service under the shipped
    // placeholder (`KD9TAW` = "operator hasn't set their call yet", per the UI
    // onboarding nudge); that would key an RBN session as a third party.
    let real_call = !cluster_call.trim().is_empty()
        && !cluster_call.trim().eq_ignore_ascii_case("KD9TAW");
    if cluster_enabled && real_call {
        let buf = spots.clone();
        std::thread::spawn(move || {
            tempo_net::cluster::run(
                &cluster_host,
                &cluster_call,
                |sp| {
                    if let Ok(mut b) = buf.lock() {
                        b.push(sp.clone());
                    }
                },
                &CLUSTER_STOP,
            );
        });
    }

    // Point the logbook at its ADIF file and load prior contacts (so worked-
    // before highlighting and the log view reflect previous sessions), and
    // restore the persisted signal source.
    if let Ok(mut eng) = engine.lock() {
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

    let prop_cache: PropCache = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .manage(engine)
        .manage(prop_cache)
        .manage(spots)
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
            get_rig_models,
            get_band_plan,
            set_frequency,
            set_tx_enabled,
            set_tune,
            halt_tx,
            test_cat,
            set_tx_even,
            set_rx_offset,
            set_tx_offset,
            set_hold_tx_freq,
            call_station,
            log_qso,
            get_log,
            get_awards,
            import_adif,
            sync_lotw_report,
            get_need_alerts,
            get_propagation,
            qsy_set_enabled,
            qsy_configure,
            qsy_move_now,
            qsy_pause,
            qsy_stop
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Tempo application");
}
