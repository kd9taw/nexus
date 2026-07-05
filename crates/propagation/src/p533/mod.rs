//! Native implementation of the ITU-R P.533 HF circuit-reliability method (with
//! the P.372 radio-noise model) — the app's VOACAP-class prediction engine.
//!
//! This is an ORIGINAL Rust implementation of the published Recommendation
//! methods, written against Rec. ITU-R P.533 / P.372 / P.1239 with the ITU-R
//! Study Group 3 reference C (`ITU-R-HF`) and the public-domain VOACAP Fortran
//! as cross-references; no foreign code is included. The embedded CCIR/ITU-R
//! coefficient data are attributed in `data/itu/itu_copyright.txt` + NOTICE.
//!
//! Build-out is incremental and gated (see `tasks/todo.md`): the engine does
//! NOT surface to the operator until its full chain validates against the ITU
//! reference fixtures. Module map (mirrors the spec/reference decomposition):
//! - [`coeffs`] — embedded CCIR/P.372 coefficient data + parsers.
//! - `geometry` / `magfield` / `ionosphere` — path control points, magnetic
//!   dip/gyrofrequency, and the CCIR numerical-map expansion (foF2, M(3000)F2).
//! - `muf` / `fieldstrength` / `noise` / `reliability` — the P.533 chain.

pub mod coeffs;
pub mod geometry;
pub mod ionosphere;
pub mod magfield;
