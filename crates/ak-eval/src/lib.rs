//! Evaluator and mood-aware base simulator (Layer 4).
//!
//! **Status: placeholder.** This crate exists so the workspace layout and
//! dependency direction are fixed from day one:
//!
//! ```text
//! ak-domain  ←  ak-eval  ←  ak-solver  ←  ak-api / ak-cli
//! ```
//!
//! It must stay pure: no async, no I/O, no framework dependencies. The
//! intended entry point is a plain function
//!
//! ```text
//! simulate(&GameData, &Assignment, &SimConfig) -> SimResult
//! ```
//!
//! driven by a fixed-tick discrete-time loop with a pluggable
//! `RotationPolicy`, deterministic (any RNG seeded from `SimConfig`).
//! Hundreds of unit tests will live here, so keep compile times short.

#![forbid(unsafe_code)]
