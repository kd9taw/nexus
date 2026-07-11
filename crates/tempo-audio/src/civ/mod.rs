//! Native Icom CI-V support (Wave 8).
//!
//! Icom radios (IC-7300/7610/9700/705/905) speak **CI-V**, a compact serial protocol
//! that — unlike the generic Hamlib path — can stream a real **spectrum-scope waveform**
//! and push instant frequency updates (transceive). Nexus's bundled `rigctld` owns the
//! serial port exclusively and discards the async scope stream, so to get a native
//! panadapter Nexus must own the CI-V port itself.
//!
//! This module is layered so the protocol core is testable without any hardware:
//! - [`frame`] — the pure CI-V frame + BCD codec layer (the foundation);
//! - [`commands`] — the verb table (freq/mode/PTT/meters…) as pure encode/decode;
//! - [`state`] — folds replies + transceive pushes into a live snapshot;
//! - [`scope`] — reassembles `27 00` waveform bursts into normalized sweeps;
//! - [`engine`] — the one thread that owns the CI-V byte stream, generic over
//!   `Read + Write` so the whole path unit-tests against an in-memory fake radio;
//! - [`broker`] — serves the rigctld text protocol on the radio's TCP port, backed by
//!   the engine — so `Rig`, the monitors, and the handoff all work UNCHANGED. Only the
//!   constructor that opens a real COM port needs the `serial` feature.

pub mod broker;
pub mod commands;
pub mod engine;
pub mod frame;
pub mod scope;
pub mod state;
