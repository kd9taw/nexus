//! Tempo network interoperability: WSJT-X-compatible UDP telemetry + PSK Reporter.
//!
//! This crate lets Tempo speak the wire protocols the amateur-radio ecosystem
//! already understands, so third-party apps interoperate with it unmodified:
//!
//! - [`wsjtx`] / [`server`] — the WSJT-X `NetworkMessage` UDP protocol. Loggers
//!   and helpers (JTAlert, GridTracker, N1MM+, log4om, …) listen for these
//!   datagrams (WSJT-X's default sink is `127.0.0.1:2237`). Tempo emits
//!   Heartbeat / Status / Decode / QSOLogged / Close, and parses the inbound
//!   Reply / HaltTx / FreeText control datagrams.
//! - [`pskreporter`] — the IPFIX-like UDP spot upload to
//!   `report.pskreporter.info:4739`, the same one WSJT-X uses to report heard
//!   stations.
//! - [`qds`] — the shared Qt `QDataStream` (big-endian) byte codec the WSJT-X
//!   protocol is framed with.
//!
//! Everything is pure Rust over `std` UDP and byte buffers; encoders take plain
//! field arguments so there is no dependency on the rest of the workspace. The
//! datagram layouts are exhaustively unit-tested (build-bytes / loopback only —
//! no test ever touches the real network).

pub mod cluster;
pub mod mqtt;
pub mod pskreporter;
pub mod qds;
pub mod server;
pub mod sntp;
pub mod wsjtx;

// Convenience re-exports for the common entry points.
pub use cluster::{parse_dx_spot, ClusterSpot};
pub use mqtt::subscribe as mqtt_subscribe;

/// Upper bound (secs) on how long a feed loop (cluster telnet / MQTT) can take to
/// observe its stop flag — both use 2 s socket read timeouts. Restart orchestration
/// sleeps `this + 1` so the coupling is explicit, not folklore.
pub const FEED_STOP_OBSERVE_SECS: u64 = 2;
pub use pskreporter::{PskReporter, Spot};
pub use server::{WsjtxServer, APP_ID};
pub use wsjtx::{Decode, Inbound, QsoLogged, Status};
