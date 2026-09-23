//! Upstream game-data ingestion.
//!
//! Two-stage deserialisation:
//!
//! 1. [`raw`]: `serde_json` into structs that mirror the upstream JSON
//!    exactly. Permissive: unknown fields are ignored, only the fields we
//!    need are declared.
//! 2. [`transform`]: `raw` → [`ak_domain::GameData`]. Strict and exhaustive:
//!    every string that should be an enum is parsed, every cross-reference is
//!    checked. When upstream ships something unexpected, this is where it
//!    fails, with a message naming the entity.
//!
//! Around those: [`manifest`] pins upstream commits, [`schema`] detects
//! drift in the raw JSON before it reaches the typed structs, [`loader`]
//! wires it together, and [`richtext`] parses description markup.
//!
//! [`import`] (Layer 8) turns other tools' roster exports into the
//! canonical [`ak_domain::Roster`].

pub mod import;
pub mod loader;
pub mod manifest;
pub mod mechanics;
pub mod paths;
pub mod raw;
pub mod richtext;
pub mod schema;
pub mod stats;
pub mod transform;

pub use loader::{LoadError, Loaded, load_default, load_dir, load_source};
pub use transform::{Strictness, TransformError, TransformReport, transform};

/// Version of this crate, recorded in [`ak_domain::DataVersion`].
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");
