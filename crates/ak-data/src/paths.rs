//! Where the pinned data lives on disk.

use std::path::PathBuf;

/// Environment variable that overrides the data root at runtime.
pub const DATA_DIR_ENV: &str = "AK_DATA_DIR";

/// The workspace `data/` directory.
///
/// Resolution order: `AK_DATA_DIR` if set, else the path baked in at compile
/// time relative to this crate (`<workspace>/data`). The latter is right for
/// development and tests; deployed binaries should set the variable.
pub fn default_data_root() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV) {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}
