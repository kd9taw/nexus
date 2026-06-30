//! Tempo real-radio transport.
//!
//! Bridges the transport-agnostic [`tempo_app::engine::Engine`] to a real
//! station: sound-card audio (via `cpal`, behind the `device` feature) and
//! PTT/CAT via Hamlib's `rigctld` daemon over TCP (no `libhamlib` build
//! dependency — just run `rigctld`). The slot-clock loop ([`runtime::Transceiver`])
//! transmits the engine's `poll_tx` waveforms on TX slots (keying PTT) and feeds
//! captured 4-second frames to `ingest` on RX slots.
//!
//! ## Layers
//! - [`rig`] — PTT/CAT via rigctld TCP, serial RTS/DTR, or VOX no-op. Pure std; unit-tested.
//! - [`rigctld_proc`] — builds the `rigctld` command line and launches the daemon. Pure args; unit-tested.
//! - [`rigmodels`] — curated Hamlib rig-model table + name lookup for the UI. Pure; tested.
//! - [`ports`] — serial-port enumeration for the UI (feature `serial`; empty Vec otherwise).
//! - [`frames::RxRing`] — rolling buffer of the latest 4 s of audio. Pure; tested.
//! - [`backend::AudioBackend`] — capture/play seam; [`backend::MockBackend`] for tests.
//! - [`runtime::Transceiver`] — the slot loop tying it all together. Tested with a mock backend.
//! - `device::CpalBackend` (feature `device`) — real sound-card I/O via cpal.
//!
//! The pure layers compile and test with no audio libraries. Build the device
//! backend on the station PC with `--features device` (needs ALSA/CoreAudio/WASAPI
//! at build time and a sound card at runtime).

pub mod backend;
pub mod frames;
pub mod port_prober;
pub mod ports;
pub mod resample;
pub mod rig;
pub mod rotator;
pub mod rigctld_proc;
pub mod rigctld_server;
pub mod rigmodels;
pub mod runtime;
pub mod slot;
pub mod usbrig;
pub mod voice;

#[cfg(feature = "device")]
pub mod device;
#[cfg(feature = "device")]
pub mod service;
