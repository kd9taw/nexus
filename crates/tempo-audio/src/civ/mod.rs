//! Native Icom CI-V support (Wave 8).
//!
//! Icom radios (IC-7300/7610/9700/705/905) speak **CI-V**, a compact serial protocol
//! that — unlike the generic Hamlib path — can stream a real **spectrum-scope waveform**
//! and push instant frequency updates (transceive). Nexus's bundled `rigctld` owns the
//! serial port exclusively and discards the async scope stream, so to get a native
//! panadapter Nexus must own the CI-V port itself.
//!
//! This module is layered so the protocol core is testable without any hardware:
//! - [`frame`] — the pure CI-V frame + BCD codec layer (no I/O, no `serialport` dep;
//!   builds and tests on every target). This is the foundation.
//!
//! The serial engine, command table, scope reassembler, and the rigctld-compatible
//! broker are built on top of `frame` and land behind the `serial` feature.

pub mod commands;
pub mod frame;
