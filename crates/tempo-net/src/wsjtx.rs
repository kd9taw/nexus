//! WSJT-X UDP "telemetry" protocol (the `NetworkMessage` interface).
//!
//! Third-party ham apps — JTAlert, GridTracker, N1MM+, log4om and other loggers
//! — listen for these datagrams (WSJT-X's default is UDP `127.0.0.1:2237`).
//! Emitting them lets Tempo masquerade as WSJT-X so that whole ecosystem
//! interoperates.
//!
//! ## Frame layout
//! Every datagram is a Qt `QDataStream` (big-endian, see [`crate::qds`]) of:
//! ```text
//! quint32 magic   = 0xADBCCBDA
//! quint32 schema  = 3            (WSJT-X negotiable schema; 3 is widely used)
//! quint32 type    (message id)
//! QString id      (the sender's "key" — Tempo uses "Tempo")
//! ... per-message payload ...
//! ```
//!
//! Field order and types follow the documented `NetworkMessage.hpp` from the
//! WSJT-X source for schema 2/3.
//!
//! ## Notes / uncertainties
//! - WSJT-X negotiates the highest *common* schema between client and server.
//!   We pin schema 3, which all current consumers accept; schema 3 added the
//!   `Status` trailing fields (`tx_message` etc.) that GridTracker reads.
//! - `Status.special_op` is a `quint8` enum (0 = NONE, 1 = NA VHF, 2 = EU VHF,
//!   3 = FIELD DAY, 4 = RTTY RU, 5 = WW DIGI, 6 = FOX, 7 = HOUND). Tempo would
//!   typically send `3` (Field Day).
//! - `Decode.delta_freq` is documented as `quint32` Hz (audio offset). Some
//!   readers treat it loosely; we emit the documented `quint32`.
//! - `QSOLogged.adif_propmode` (schema 3) is the ADIF propagation mode string;
//!   left empty for HF skywave QSOs.

use crate::qds::{QdsReader, QdsWriter};

/// WSJT-X datagram magic number.
pub const MAGIC: u32 = 0xADBC_CBDA;
/// Protocol schema version Tempo speaks.
pub const SCHEMA: u32 = 3;

/// Numeric message-type identifiers from the WSJT-X `NetworkMessage` enum.
pub mod msg_type {
    pub const HEARTBEAT: u32 = 0;
    pub const STATUS: u32 = 1;
    pub const DECODE: u32 = 2;
    pub const CLEAR: u32 = 3;
    pub const REPLY: u32 = 4;
    pub const QSO_LOGGED: u32 = 5;
    pub const CLOSE: u32 = 6;
    pub const REPLAY: u32 = 7;
    // CANONICAL NetworkMessage.hpp numbering — these were shifted +1 for every
    // type >= 8 (HaltTx missing entirely): a REAL JTAlert FreeText(9) parsed as
    // our HaltTx and KILLED TX, and a real HaltTx(8) was silently ignored.
    pub const HALT_TX: u32 = 8;
    pub const FREE_TEXT: u32 = 9;
    pub const WSPR_DECODE: u32 = 10;
    pub const LOCATION: u32 = 11;
    pub const LOGGED_ADIF: u32 = 12;
    pub const HIGHLIGHT_CALLSIGN: u32 = 13;
    pub const SWITCH_CONFIGURATION: u32 = 14;
    pub const CONFIGURE: u32 = 15;
}

/// Start a datagram: write the magic, schema, message type and sender id.
fn header(id: &str, message_type: u32) -> QdsWriter {
    let mut w = QdsWriter::new();
    w.put_u32(MAGIC)
        .put_u32(SCHEMA)
        .put_u32(message_type)
        .put_utf8(Some(id));
    w
}

/// **Heartbeat (type 0).** Sent periodically so consumers learn we are alive
/// and can negotiate schema. Fields: `id`, max schema, version, revision.
pub fn encode_heartbeat(id: &str, max_schema: u32, version: &str, revision: &str) -> Vec<u8> {
    let mut w = header(id, msg_type::HEARTBEAT);
    w.put_u32(max_schema)
        .put_utf8(Some(version))
        .put_utf8(Some(revision));
    w.into_bytes()
}

/// All `Status` (type 1) fields. Mirrors the operating state WSJT-X broadcasts
/// whenever something changes (frequency, mode, TX/RX state, calls in the
/// QSO fields, etc.).
#[derive(Debug, Clone, Default)]
pub struct Status<'a> {
    pub dial_freq: u64,
    pub mode: &'a str,
    pub dx_call: &'a str,
    pub report: &'a str,
    pub tx_mode: &'a str,
    pub tx_enabled: bool,
    pub transmitting: bool,
    pub decoding: bool,
    pub rx_df: u32,
    pub tx_df: u32,
    pub de_call: &'a str,
    pub de_grid: &'a str,
    pub dx_grid: &'a str,
    pub tx_watchdog: bool,
    pub sub_mode: &'a str,
    pub fast_mode: bool,
    /// Special operation mode enum (3 = Field Day).
    pub special_op: u8,
    pub freq_tol: u32,
    pub tr_period: u32,
    pub config_name: &'a str,
    pub tx_message: &'a str,
}

/// **Status (type 1).**
pub fn encode_status(id: &str, s: &Status) -> Vec<u8> {
    let mut w = header(id, msg_type::STATUS);
    w.put_u64(s.dial_freq)
        .put_utf8(Some(s.mode))
        .put_utf8(Some(s.dx_call))
        .put_utf8(Some(s.report))
        .put_utf8(Some(s.tx_mode))
        .put_bool(s.tx_enabled)
        .put_bool(s.transmitting)
        .put_bool(s.decoding)
        .put_u32(s.rx_df)
        .put_u32(s.tx_df)
        .put_utf8(Some(s.de_call))
        .put_utf8(Some(s.de_grid))
        .put_utf8(Some(s.dx_grid))
        .put_bool(s.tx_watchdog)
        .put_utf8(Some(s.sub_mode))
        .put_bool(s.fast_mode)
        .put_u8(s.special_op)
        .put_u32(s.freq_tol)
        .put_u32(s.tr_period)
        .put_utf8(Some(s.config_name))
        .put_utf8(Some(s.tx_message));
    w.into_bytes()
}

/// All `Decode` (type 2) fields — one decoded signal from a slot.
#[derive(Debug, Clone, Default)]
pub struct Decode<'a> {
    /// True for a "new" decode (vs. a replayed one).
    pub new: bool,
    /// Milliseconds since midnight (UTC) of the decode.
    pub time_ms: u32,
    pub snr: i32,
    pub delta_time: f64,
    pub delta_freq: u32,
    pub mode: &'a str,
    pub message: &'a str,
    pub low_confidence: bool,
    pub off_air: bool,
}

/// **Decode (type 2).**
pub fn encode_decode(id: &str, d: &Decode) -> Vec<u8> {
    let mut w = header(id, msg_type::DECODE);
    w.put_bool(d.new)
        .put_qtime(d.time_ms)
        .put_i32(d.snr)
        .put_f64(d.delta_time)
        .put_u32(d.delta_freq)
        .put_utf8(Some(d.mode))
        .put_utf8(Some(d.message))
        .put_bool(d.low_confidence)
        .put_bool(d.off_air);
    w.into_bytes()
}

/// All `QSOLogged` (type 5) fields — a completed contact for logging apps.
#[derive(Debug, Clone, Default)]
pub struct QsoLogged<'a> {
    /// QSO end time, Unix seconds (UTC).
    pub time_off: i64,
    pub dx_call: &'a str,
    pub dx_grid: &'a str,
    pub tx_freq: u64,
    pub mode: &'a str,
    pub report_sent: &'a str,
    pub report_recvd: &'a str,
    pub tx_power: &'a str,
    pub comments: &'a str,
    pub name: &'a str,
    /// QSO start time, Unix seconds (UTC).
    pub time_on: i64,
    pub op_call: &'a str,
    pub my_call: &'a str,
    pub my_grid: &'a str,
    pub exchange_sent: &'a str,
    pub exchange_recvd: &'a str,
    pub adif_propmode: &'a str,
}

/// **QSOLogged (type 5).**
pub fn encode_qso_logged(id: &str, q: &QsoLogged) -> Vec<u8> {
    let mut w = header(id, msg_type::QSO_LOGGED);
    w.put_qdatetime(q.time_off)
        .put_utf8(Some(q.dx_call))
        .put_utf8(Some(q.dx_grid))
        .put_u64(q.tx_freq)
        .put_utf8(Some(q.mode))
        .put_utf8(Some(q.report_sent))
        .put_utf8(Some(q.report_recvd))
        .put_utf8(Some(q.tx_power))
        .put_utf8(Some(q.comments))
        .put_utf8(Some(q.name))
        .put_qdatetime(q.time_on)
        .put_utf8(Some(q.op_call))
        .put_utf8(Some(q.my_call))
        .put_utf8(Some(q.my_grid))
        .put_utf8(Some(q.exchange_sent))
        .put_utf8(Some(q.exchange_recvd))
        .put_utf8(Some(q.adif_propmode));
    w.into_bytes()
}

/// **Close (type 6).** Tells consumers Tempo is shutting down. Fields: just `id`.
pub fn encode_close(id: &str) -> Vec<u8> {
    header(id, msg_type::CLOSE).into_bytes()
}

/// Build a **Clear (type 3)** datagram: window 0 = Band Activity, 1 = Rx
/// Frequency, 2 = both — sent when the operator hits Erase so cooperating
/// apps (JTAlert/GridTracker) clear their mirrored windows too.
pub fn encode_clear(id: &str, window: u8) -> Vec<u8> {
    let mut w = header(id, msg_type::CLEAR);
    w.put_u8(window);
    w.into_bytes()
}

/// A parsed inbound datagram from a controlling app (e.g. GridTracker telling
/// us to reply to a CQ, halt TX, or send free text).
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// **Reply (type 4)** — the user double-clicked a decode in the consumer;
    /// it asks WSJT-X to call that station. We surface the offered decode.
    Reply {
        id: String,
        time_ms: u32,
        snr: i32,
        delta_time: f64,
        delta_freq: u32,
        mode: String,
        message: String,
        low_confidence: bool,
        modifiers: u8,
    },
    /// **HaltTx (type 8)** — stop transmitting (`auto_only` = true means only
    /// turn off auto-TX, finishing the current transmission).
    HaltTx { id: String, auto_only: bool },
    /// **Clear (type 3)** — the consumer asks us to clear decode windows.
    /// `window`: 0 = Band Activity, 1 = Rx Frequency, 2 = both.
    Clear { id: String, window: u8 },
    /// **Replay (type 7)** — re-send the current period's Decode datagrams
    /// (a consumer that just connected wants to catch up).
    Replay { id: String },
    /// **Location (type 11)** — a GPS feeder updates our Maidenhead grid.
    Location { id: String, location: String },
    /// **HighlightCallsign (type 13)** — JTAlert-style row coloring for a
    /// callsign in Band Activity. Empty/invalid colors clear the highlight.
    HighlightCallsign {
        id: String,
        call: String,
        /// CSS hex like "#rrggbb", or None = clear.
        bg: Option<String>,
        fg: Option<String>,
        highlight_last: bool,
    },
    /// **FreeText (type 9)** — set the free-text TX message (`send` = transmit
    /// it now vs. just stage it).
    FreeText {
        id: String,
        text: String,
        send: bool,
    },
    /// **Decode (type 2)** — a decoded signal from an *upstream* decoder
    /// (WSJT-X / JTDX / MSHV) that Tempo/Nexus consumes in companion mode. This
    /// is the same payload [`encode_decode`] writes, parsed back from the wire so
    /// a [`Decode`]-producing app can be used as a signal source.
    Decode {
        id: String,
        new: bool,
        time_ms: u32,
        snr: i32,
        delta_time: f64,
        delta_freq: u32,
        mode: String,
        message: String,
        low_confidence: bool,
        off_air: bool,
    },
    /// A datagram we recognized the header of but do not act on.
    Other { id: String, message_type: u32 },
}

/// Parse an inbound WSJT-X datagram. Returns `None` if the magic/header is not
/// a valid WSJT-X frame (so non-WSJT-X traffic on the port is ignored safely).
pub fn parse_inbound(bytes: &[u8]) -> Option<Inbound> {
    let mut r = QdsReader::new(bytes);
    if r.read_u32()? != MAGIC {
        return None;
    }
    let _schema = r.read_u32()?;
    let message_type = r.read_u32()?;
    let id = r.read_utf8()?.unwrap_or_default();

    match message_type {
        msg_type::REPLY => Some(Inbound::Reply {
            id,
            time_ms: r.read_u32()?,
            snr: r.read_i32()?,
            delta_time: r.read_f64()?,
            delta_freq: r.read_u32()?,
            mode: r.read_utf8()?.unwrap_or_default(),
            message: r.read_utf8()?.unwrap_or_default(),
            // `low_confidence` (bool) + `modifiers` (quint8) were added in
            // schema 3; tolerate their absence on older senders.
            low_confidence: r.read_bool().unwrap_or(false),
            modifiers: r.read_u8().unwrap_or(0),
        }),
        msg_type::CLEAR => Some(Inbound::Clear {
            id,
            // The window byte was added later; absent = Band Activity (0).
            window: r.read_u8().unwrap_or(0),
        }),
        msg_type::REPLAY => Some(Inbound::Replay { id }),
        msg_type::LOCATION => Some(Inbound::Location {
            id,
            location: r.read_utf8()?.unwrap_or_default(),
        }),
        msg_type::HIGHLIGHT_CALLSIGN => {
            let call = r.read_utf8()?.unwrap_or_default();
            let bg = r.read_qcolor()?;
            let fg = r.read_qcolor()?;
            let highlight_last = r.read_bool().unwrap_or(false);
            Some(Inbound::HighlightCallsign { id, call, bg, fg, highlight_last })
        }
        msg_type::HALT_TX => Some(Inbound::HaltTx {
            id,
            auto_only: r.read_bool()?,
        }),
        msg_type::FREE_TEXT => Some(Inbound::FreeText {
            id,
            text: r.read_utf8()?.unwrap_or_default(),
            send: r.read_bool()?,
        }),
        msg_type::DECODE => Some(Inbound::Decode {
            id,
            new: r.read_bool()?,
            // `put_qtime` writes ms-since-midnight as a quint32.
            time_ms: r.read_u32()?,
            snr: r.read_i32()?,
            delta_time: r.read_f64()?,
            delta_freq: r.read_u32()?,
            mode: r.read_utf8()?.unwrap_or_default(),
            message: r.read_utf8()?.unwrap_or_default(),
            // `low_confidence` / `off_air` are tolerated absent on older senders.
            low_confidence: r.read_bool().unwrap_or(false),
            off_air: r.read_bool().unwrap_or(false),
        }),
        other => Some(Inbound::Other {
            id,
            message_type: other,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qds::{QdsReader, QdsWriter};

    /// Read and assert the common 4-field header, returning the reader
    /// positioned at the payload.
    fn check_header<'a>(bytes: &'a [u8], expect_type: u32) -> QdsReader<'a> {
        let mut r = QdsReader::new(bytes);
        assert_eq!(r.read_u32(), Some(MAGIC));
        assert_eq!(r.read_u32(), Some(SCHEMA));
        assert_eq!(r.read_u32(), Some(expect_type));
        assert_eq!(r.read_utf8(), Some(Some("Tempo".to_string())));
        r
    }

    #[test]
    fn heartbeat_layout() {
        let bytes = encode_heartbeat("Tempo", 3, "1.0", "g0");
        let mut r = check_header(&bytes, msg_type::HEARTBEAT);
        assert_eq!(r.read_u32(), Some(3));
        assert_eq!(r.read_utf8(), Some(Some("1.0".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("g0".to_string())));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn status_layout_full() {
        let s = Status {
            dial_freq: 14_074_000,
            mode: "FT8",
            dx_call: "W1AW",
            report: "-10",
            tx_mode: "FT8",
            tx_enabled: true,
            transmitting: false,
            decoding: true,
            rx_df: 1500,
            tx_df: 1500,
            de_call: "KD9TAW",
            de_grid: "EN52",
            dx_grid: "FN31",
            tx_watchdog: false,
            sub_mode: "",
            fast_mode: false,
            special_op: 3, // Field Day
            freq_tol: 0,
            tr_period: 15,
            config_name: "Default",
            tx_message: "W1AW KD9TAW EN52",
        };
        let bytes = encode_status("Tempo", &s);
        let mut r = check_header(&bytes, msg_type::STATUS);
        assert_eq!(r.read_u64(), Some(14_074_000));
        assert_eq!(r.read_utf8(), Some(Some("FT8".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("W1AW".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("-10".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("FT8".to_string()))); // tx_mode
        assert_eq!(r.read_bool(), Some(true)); // tx_enabled
        assert_eq!(r.read_bool(), Some(false)); // transmitting
        assert_eq!(r.read_bool(), Some(true)); // decoding
        assert_eq!(r.read_u32(), Some(1500)); // rx_df
        assert_eq!(r.read_u32(), Some(1500)); // tx_df
        assert_eq!(r.read_utf8(), Some(Some("KD9TAW".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("EN52".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("FN31".to_string())));
        assert_eq!(r.read_bool(), Some(false)); // tx_watchdog
        assert_eq!(r.read_utf8(), Some(Some("".to_string()))); // sub_mode
        assert_eq!(r.read_bool(), Some(false)); // fast_mode
        assert_eq!(r.read_u8(), Some(3)); // special_op
        assert_eq!(r.read_u32(), Some(0)); // freq_tol
        assert_eq!(r.read_u32(), Some(15)); // tr_period
        assert_eq!(r.read_utf8(), Some(Some("Default".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("W1AW KD9TAW EN52".to_string())));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn decode_layout() {
        let d = Decode {
            new: true,
            time_ms: 45_296_000,
            snr: -7,
            delta_time: 0.2,
            delta_freq: 1234,
            mode: "FT8",
            message: "CQ KD9TAW EN52",
            low_confidence: false,
            off_air: false,
        };
        let bytes = encode_decode("Tempo", &d);
        let mut r = check_header(&bytes, msg_type::DECODE);
        assert_eq!(r.read_bool(), Some(true)); // new
        assert_eq!(r.read_u32(), Some(45_296_000)); // qtime
        assert_eq!(r.read_i32(), Some(-7)); // snr
        assert_eq!(r.read_f64(), Some(0.2)); // delta_time
        assert_eq!(r.read_u32(), Some(1234)); // delta_freq
        assert_eq!(r.read_utf8(), Some(Some("FT8".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("CQ KD9TAW EN52".to_string())));
        assert_eq!(r.read_bool(), Some(false)); // low_confidence
        assert_eq!(r.read_bool(), Some(false)); // off_air
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn qso_logged_layout() {
        let q = QsoLogged {
            time_off: 1_577_836_800, // 2020-01-01T00:00:00Z
            dx_call: "W1AW",
            dx_grid: "FN31",
            tx_freq: 14_075_500,
            mode: "FT8",
            report_sent: "-10",
            report_recvd: "-12",
            tx_power: "50",
            comments: "FD",
            name: "ARRL",
            time_on: 1_577_836_860, // 2020-01-01T00:01:00Z (same day, +60s)
            op_call: "KD9TAW",
            my_call: "KD9TAW",
            my_grid: "EN52",
            exchange_sent: "1D WI",
            exchange_recvd: "3A EMA",
            adif_propmode: "",
        };
        let bytes = encode_qso_logged("Tempo", &q);
        let mut r = check_header(&bytes, msg_type::QSO_LOGGED);
        // time_off as QDateTime: JD(2020-01-01)=2458850, ms=0, utc=1
        assert_eq!(r.read_i64(), Some(2_458_850));
        assert_eq!(r.read_u32(), Some(0));
        assert_eq!(r.read_i8(), Some(1));
        assert_eq!(r.read_utf8(), Some(Some("W1AW".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("FN31".to_string())));
        assert_eq!(r.read_u64(), Some(14_075_500));
        assert_eq!(r.read_utf8(), Some(Some("FT8".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("-10".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("-12".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("50".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("FD".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("ARRL".to_string())));
        // time_on: JD 2458850, ms = 60_000 (00:01:00)
        assert_eq!(r.read_i64(), Some(2_458_850));
        assert_eq!(r.read_u32(), Some(60_000));
        assert_eq!(r.read_i8(), Some(1));
        assert_eq!(r.read_utf8(), Some(Some("KD9TAW".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("KD9TAW".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("EN52".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("1D WI".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("3A EMA".to_string())));
        assert_eq!(r.read_utf8(), Some(Some("".to_string())));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn close_layout() {
        let bytes = encode_close("Tempo");
        let r = check_header(&bytes, msg_type::CLOSE);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn parse_reply_roundtrip() {
        // Build a Reply datagram by hand the way a consumer would.
        let mut w = QdsWriter::new();
        w.put_u32(MAGIC)
            .put_u32(SCHEMA)
            .put_u32(msg_type::REPLY)
            .put_utf8(Some("GridTracker"))
            .put_u32(45_296_000)
            .put_i32(-7)
            .put_f64(0.2)
            .put_u32(1234)
            .put_utf8(Some("FT8"))
            .put_utf8(Some("CQ W1AW FN31"))
            .put_bool(false)
            .put_u8(0);
        let bytes = w.into_bytes();
        match parse_inbound(&bytes).unwrap() {
            Inbound::Reply {
                id,
                time_ms,
                snr,
                delta_time,
                delta_freq,
                mode,
                message,
                low_confidence,
                modifiers,
            } => {
                assert_eq!(id, "GridTracker");
                assert_eq!(time_ms, 45_296_000);
                assert_eq!(snr, -7);
                assert_eq!(delta_time, 0.2);
                assert_eq!(delta_freq, 1234);
                assert_eq!(mode, "FT8");
                assert_eq!(message, "CQ W1AW FN31");
                assert!(!low_confidence);
                assert_eq!(modifiers, 0);
            }
            other => panic!("expected Reply, got {other:?}"),
        }
    }

    #[test]
    fn parse_halt_tx() {
        let mut w = QdsWriter::new();
        w.put_u32(MAGIC)
            .put_u32(SCHEMA)
            .put_u32(msg_type::HALT_TX)
            .put_utf8(Some("JTAlert"))
            .put_bool(true);
        assert_eq!(
            parse_inbound(&w.into_bytes()),
            Some(Inbound::HaltTx {
                id: "JTAlert".to_string(),
                auto_only: true,
            })
        );
    }

    #[test]
    fn parse_free_text() {
        let mut w = QdsWriter::new();
        w.put_u32(MAGIC)
            .put_u32(SCHEMA)
            .put_u32(msg_type::FREE_TEXT)
            .put_utf8(Some("N1MM"))
            .put_utf8(Some("73 GL"))
            .put_bool(true);
        assert_eq!(
            parse_inbound(&w.into_bytes()),
            Some(Inbound::FreeText {
                id: "N1MM".to_string(),
                text: "73 GL".to_string(),
                send: true,
            })
        );
    }

    /// A Decode (type 2) written by [`encode_decode`] must parse back to an
    /// equivalent [`Inbound::Decode`] — this is the wire contract a companion
    /// SignalSource relies on to ingest an upstream WSJT-X/JTDX/MSHV stream.
    #[test]
    fn parse_decode_roundtrip() {
        let d = Decode {
            new: true,
            time_ms: 123_000,
            snr: -12,
            delta_time: 0.2,
            delta_freq: 1500,
            mode: "~",
            message: "CQ KD9TAW EN52",
            low_confidence: false,
            off_air: false,
        };
        let bytes = encode_decode("WSJT-X", &d);
        assert_eq!(
            parse_inbound(&bytes),
            Some(Inbound::Decode {
                id: "WSJT-X".to_string(),
                new: true,
                time_ms: 123_000,
                snr: -12,
                delta_time: 0.2,
                delta_freq: 1500,
                mode: "~".to_string(),
                message: "CQ KD9TAW EN52".to_string(),
                low_confidence: false,
                off_air: false,
            })
        );
    }

    #[test]
    fn parse_rejects_bad_magic() {
        let mut w = QdsWriter::new();
        w.put_u32(0xDEADBEEF).put_u32(SCHEMA).put_u32(0);
        assert_eq!(parse_inbound(&w.into_bytes()), None);
    }

    #[test]
    fn parse_unknown_type_is_other() {
        // type 1 (Status) inbound is not one we act on -> Other.
        let mut w = QdsWriter::new();
        w.put_u32(MAGIC)
            .put_u32(SCHEMA)
            .put_u32(msg_type::STATUS)
            .put_utf8(Some("WSJT-X"));
        assert_eq!(
            parse_inbound(&w.into_bytes()),
            Some(Inbound::Other {
                id: "WSJT-X".to_string(),
                message_type: msg_type::STATUS,
            })
        );
    }
    #[test]
    fn message_type_numbers_are_canonical_networkmessage_hpp() {
        // The interop contract with EVERY cooperating app. These were shifted
        // +1 for types >= 8 once — a real JTAlert FreeText(9) parsed as our
        // HaltTx and killed TX. Pin the canon.
        assert_eq!(msg_type::HEARTBEAT, 0);
        assert_eq!(msg_type::STATUS, 1);
        assert_eq!(msg_type::DECODE, 2);
        assert_eq!(msg_type::CLEAR, 3);
        assert_eq!(msg_type::REPLY, 4);
        assert_eq!(msg_type::QSO_LOGGED, 5);
        assert_eq!(msg_type::CLOSE, 6);
        assert_eq!(msg_type::REPLAY, 7);
        assert_eq!(msg_type::HALT_TX, 8);
        assert_eq!(msg_type::FREE_TEXT, 9);
        assert_eq!(msg_type::WSPR_DECODE, 10);
        assert_eq!(msg_type::LOCATION, 11);
        assert_eq!(msg_type::LOGGED_ADIF, 12);
        assert_eq!(msg_type::HIGHLIGHT_CALLSIGN, 13);
        assert_eq!(msg_type::SWITCH_CONFIGURATION, 14);
        assert_eq!(msg_type::CONFIGURE, 15);
    }

    #[test]
    fn parses_clear_replay_location_highlight() {
        let dg = |t: u32, body: &dyn Fn(&mut crate::qds::QdsWriter)| {
            let mut w = crate::qds::QdsWriter::new();
            w.put_u32(MAGIC);
            w.put_u32(SCHEMA);
            w.put_u32(t);
            w.put_utf8(Some("JTAlert"));
            body(&mut w);
            w.into_bytes()
        };
        match parse_inbound(&dg(msg_type::CLEAR, &|w| {
            w.put_u8(2);
        })) {
            Some(Inbound::Clear { window: 2, .. }) => {}
            other => panic!("expected Clear, got {other:?}"),
        }
        match parse_inbound(&dg(msg_type::REPLAY, &|_| {})) {
            Some(Inbound::Replay { .. }) => {}
            other => panic!("expected Replay, got {other:?}"),
        }
        match parse_inbound(&dg(msg_type::LOCATION, &|w| {
            w.put_utf8(Some("GRID:EN52"));
        })) {
            Some(Inbound::Location { location, .. }) => assert_eq!(location, "GRID:EN52"),
            other => panic!("expected Location, got {other:?}"),
        }
        // HighlightCallsign with a red bg, valid-RGB fg, and the QColor layout
        // (qint8 spec, then a/r/g/b/pad quint16s).
        let qcolor = |w: &mut crate::qds::QdsWriter, spec: i8, r: u16, g: u16, b: u16| {
            w.put_u8(spec as u8);
            w.put_u16(0xffff); // alpha
            w.put_u16(r);
            w.put_u16(g);
            w.put_u16(b);
            w.put_u16(0); // pad
        };
        // A non-RGB spec (Hsv = 2) must keep the stream ALIGNED — all five
        // quint16s are consumed even when the color is reported as None, so the
        // fields AFTER the QColors still parse. (Misalignment would corrupt
        // every later field silently.)
        match parse_inbound(&dg(msg_type::HIGHLIGHT_CALLSIGN, &|w| {
            w.put_utf8(Some("HSV1AB"));
            qcolor(w, 2, 0x1234, 0x5678, 0x9abc); // Hsv — unsupported spec
            qcolor(w, 1, 0x0000, 0xffff, 0x0000); // green fg AFTER it
            w.put_u8(1);
        })) {
            Some(Inbound::HighlightCallsign { bg, fg, highlight_last, .. }) => {
                assert_eq!(bg, None, "non-RGB spec reads as None");
                assert_eq!(fg.as_deref(), Some("#00ff00"), "stream stayed aligned");
                assert!(highlight_last);
            }
            other => panic!("expected HighlightCallsign, got {other:?}"),
        }
        match parse_inbound(&dg(msg_type::HIGHLIGHT_CALLSIGN, &|w| {
            w.put_utf8(Some("K1ABC"));
            qcolor(w, 1, 0xffff, 0x0000, 0x0000); // red bg
            qcolor(w, 0, 0, 0, 0); // Invalid fg = none
            w.put_u8(1);
        })) {
            Some(Inbound::HighlightCallsign { call, bg, fg, highlight_last, .. }) => {
                assert_eq!(call, "K1ABC");
                assert_eq!(bg.as_deref(), Some("#ff0000"));
                assert_eq!(fg, None);
                assert!(highlight_last);
            }
            other => panic!("expected HighlightCallsign, got {other:?}"),
        }
    }

}
