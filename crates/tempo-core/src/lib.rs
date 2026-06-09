//! Tempo transceiver core.
//!
//! This crate sits between the [`ft1`] modem (encode/decode of a single 4-second
//! frame) and the application: it handles **slot timing**, a **virtual-air
//! channel** for headless loopback testing, and the **TX** path. The RX
//! acquisition path is wired in once `libft1` exposes the full
//! sync+search decoder (`ft1_decode_frame`).
//!
//! Audio devices are intentionally abstracted away: on this development host
//! there is no sound hardware, so the M2 milestone is proven over an in-process
//! [`channel::VirtualAir`]. A real `cpal` audio backend slots in behind the same
//! frame-in/frame-out boundary later (see task #10).

pub mod channel;
pub mod clublog;
pub mod cw;
pub mod diagnostics;
pub mod eqsl;
pub mod fieldday;
pub mod inbox;
pub mod logbook;
pub mod lotw;
pub mod lotw_upload;
pub mod message;
pub mod pota;
pub mod qrz;
pub mod qso;
pub mod qsy;
pub mod reconcile;
pub mod roster;
pub mod spectrum;
pub mod store;
pub mod text;
pub mod timing;
pub mod tx;
pub mod wavfile;

pub use ft1;
pub use modes;
