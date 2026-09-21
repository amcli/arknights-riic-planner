//! Reads the pinned files from disk and runs the transform.

use std::fs;
use std::path::{Path, PathBuf};

use ak_domain::{DataVersion, GameData};

use crate::PARSER_VERSION;
use crate::manifest::{
    BUILDING_FILE, CHARACTER_FILE, DataSource, MANIFEST_FILE, Manifest, ManifestError,
    SYNC_RECORD_FILE, SyncRecord, TEAM_FILE,
};
use crate::raw::RawBundle;
use crate::transform::{Strictness, TransformError, TransformReport, transform};

/// Anything that can go wrong between disk and [`GameData`].
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error(
        "data in {dir} was synced from commit {found} but the manifest pins {expected}; run `cargo run -p ak-data-sync -- sync`"
    )]
    StaleData {
        dir: PathBuf,
        expected: String,
        found: String,
    },
    #[error("no data directory at {0}; run `cargo run -p ak-data-sync -- sync`")]
    MissingDataDir(PathBuf),
}

/// A successful load.
#[derive(Debug)]
pub struct Loaded {
    /// The model.
    pub data: GameData,
    /// What the transform had to tolerate.
    pub report: TransformReport,
}

/// Reads and deserialises one JSON file.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LoadError> {
    let bytes = fs::read(path).map_err(|source| LoadError::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| LoadError::Json {
        path: path.to_owned(),
        source,
    })
}

/// Reads the three upstream files from a source directory.
pub fn read_raw_bundle(dir: &Path) -> Result<RawBundle, LoadError> {
    if !dir.is_dir() {
        return Err(LoadError::MissingDataDir(dir.to_owned()));
    }
    Ok(RawBundle {
        building: read_json(&dir.join(BUILDING_FILE))?,
        characters: read_json(&dir.join(CHARACTER_FILE))?,
        teams: read_json(&dir.join(TEAM_FILE))?,
    })
}

/// Reads the `.sync.json` sidecar if present.
pub fn read_sync_record(dir: &Path) -> Result<Option<SyncRecord>, LoadError> {
    let path = dir.join(SYNC_RECORD_FILE);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map(Some)
}

/// Loads from a directory without consulting the manifest. Provenance comes
/// from the sidecar when present, otherwise it is marked unknown.
pub fn load_dir(dir: &Path, strictness: Strictness) -> Result<Loaded, LoadError> {
    let record = read_sync_record(dir)?;
    let version = version_from(dir, record.as_ref(), None);
    load_with_version(dir, version, strictness)
}

/// Loads a named source (or the manifest default) from a data root, refusing
/// to proceed if the on-disk sidecar disagrees with the manifest pin.
pub fn load_source(
    data_root: &Path,
    source_name: Option<&str>,
    strictness: Strictness,
) -> Result<Loaded, LoadError> {
    let manifest = Manifest::load(&data_root.join(MANIFEST_FILE))?;
    let source = match source_name {
        Some(name) => manifest
            .source(name)
            .ok_or_else(|| ManifestError::UnknownSource(name.to_owned()))?,
        None => manifest.default_source()?,
    };
    let dir = source.dir(data_root);
    let record = read_sync_record(&dir)?;
    if let Some(record) = &record
        && record.sha != source.sha
    {
        return Err(LoadError::StaleData {
            dir,
            expected: source.sha.clone(),
            found: record.sha.clone(),
        });
    }
    let version = version_from(&dir, record.as_ref(), Some(source));
    load_with_version(&dir, version, strictness)
}

/// Loads the manifest default source from [`crate::paths::default_data_root`].
pub fn load_default(strictness: Strictness) -> Result<Loaded, LoadError> {
    load_source(&crate::paths::default_data_root(), None, strictness)
}

fn load_with_version(
    dir: &Path,
    version: DataVersion,
    strictness: Strictness,
) -> Result<Loaded, LoadError> {
    let raw = read_raw_bundle(dir)?;
    let (data, report) = transform(&raw, version, strictness)?;
    Ok(Loaded { data, report })
}

fn version_from(
    dir: &Path,
    record: Option<&SyncRecord>,
    source: Option<&DataSource>,
) -> DataVersion {
    let unknown = || "unknown".to_owned();
    DataVersion {
        source: source
            .map(|s| s.name.clone())
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(unknown),
        repo: source
            .map(|s| s.repo.clone())
            .or_else(|| record.map(|r| r.repo.clone()))
            .unwrap_or_else(unknown),
        sha: source
            .map(|s| s.sha.clone())
            .or_else(|| record.map(|r| r.sha.clone()))
            .unwrap_or_else(unknown),
        locale: source
            .map(|s| s.locale.clone())
            .or_else(|| record.map(|r| r.locale.clone()))
            .unwrap_or_else(unknown),
        fetched_at: record.map(|r| r.fetched_at.clone()),
        parser_version: PARSER_VERSION.to_owned(),
    }
}
