//! `data/manifest.toml`: the pinned upstream sources, and the `.sync.json`
//! sidecar that records what was actually fetched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name of the manifest inside the data root.
pub const MANIFEST_FILE: &str = "manifest.toml";
/// File name of the per-source sidecar written by `ak-data-sync`.
pub const SYNC_RECORD_FILE: &str = ".sync.json";
/// Upstream file names we ingest.
pub const BUILDING_FILE: &str = "building_data.json";
pub const CHARACTER_FILE: &str = "character_table.json";
pub const TEAM_FILE: &str = "handbook_team_table.json";
/// Every file a source directory must contain.
pub const REQUIRED_FILES: &[&str] = &[BUILDING_FILE, CHARACTER_FILE, TEAM_FILE];

/// The manifest schema version this crate understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Errors reading or validating the manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        /// Boxed: `toml::de::Error` is large and would bloat every `Result`.
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("manifest schema_version {0} is not supported (expected {SUPPORTED_SCHEMA_VERSION})")]
    UnsupportedSchema(u32),
    #[error("no source named {0:?} in manifest")]
    UnknownSource(String),
    #[error("source {name}: sha {sha:?} is not a 40-character hex commit id")]
    BadSha { name: String, sha: String },
    #[error("source {name}: missing required file {file}")]
    MissingRequiredFile { name: String, file: &'static str },
}

/// `data/manifest.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Manifest format version.
    pub schema_version: u32,
    /// Name of the source loaded when none is specified.
    pub default_source: String,
    /// Every pinned source.
    pub sources: Vec<DataSource>,
}

/// One pinned upstream source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSource {
    /// Directory name under the data root, e.g. `en_US`.
    pub name: String,
    /// GitHub `owner/repo`.
    pub repo: String,
    /// Full commit id.
    pub sha: String,
    /// Locale directory inside the repository, e.g. `en_US`.
    pub locale: String,
    /// Whether `ak-data-sync` fetches this source by default.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Free-form note about why this pin was chosen.
    #[serde(default)]
    pub note: Option<String>,
    /// Files to fetch from `<locale>/gamedata/excel/`.
    #[serde(default = "default_files")]
    pub files: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_files() -> Vec<String> {
    REQUIRED_FILES.iter().map(|s| (*s).to_owned()).collect()
}

impl Manifest {
    /// Reads and validates a manifest file.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_owned(),
            source,
        })?;
        let manifest: Manifest = toml::from_str(&text).map_err(|source| ManifestError::Parse {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Structural checks that do not touch the filesystem.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema(self.schema_version));
        }
        for source in &self.sources {
            if source.sha.len() != 40 || !source.sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ManifestError::BadSha {
                    name: source.name.clone(),
                    sha: source.sha.clone(),
                });
            }
            for required in REQUIRED_FILES {
                if !source.files.iter().any(|f| f == required) {
                    return Err(ManifestError::MissingRequiredFile {
                        name: source.name.clone(),
                        file: required,
                    });
                }
            }
        }
        self.default_source()?;
        Ok(())
    }

    /// Looks up a source by name.
    pub fn source(&self, name: &str) -> Option<&DataSource> {
        self.sources.iter().find(|s| s.name == name)
    }

    /// The source named by `default_source`.
    pub fn default_source(&self) -> Result<&DataSource, ManifestError> {
        self.source(&self.default_source)
            .ok_or_else(|| ManifestError::UnknownSource(self.default_source.clone()))
    }

    /// Sources with `enabled = true`.
    pub fn enabled_sources(&self) -> impl Iterator<Item = &DataSource> {
        self.sources.iter().filter(|s| s.enabled)
    }
}

impl DataSource {
    /// The raw.githubusercontent.com URL for one of this source's files.
    pub fn raw_url(&self, file: &str) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{}/gamedata/excel/{}",
            self.repo, self.sha, self.locale, file
        )
    }

    /// Directory holding this source's files under a data root.
    pub fn dir(&self, data_root: &Path) -> PathBuf {
        data_root.join(&self.name)
    }

    /// Browser URL of the pinned commit.
    pub fn commit_url(&self) -> String {
        format!("https://github.com/{}/commit/{}", self.repo, self.sha)
    }
}

/// Sidecar written next to the fetched files, recording provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRecord {
    /// GitHub `owner/repo` fetched from.
    pub repo: String,
    /// Commit fetched.
    pub sha: String,
    /// Locale directory fetched.
    pub locale: String,
    /// RFC 3339 timestamp of the fetch.
    pub fetched_at: String,
    /// Per-file digests.
    pub files: BTreeMap<String, FileRecord>,
}

/// Digest of one fetched file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Lower-case hex SHA-256 of the file contents.
    pub sha256: String,
    /// Size in bytes.
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema_version = 1
default_source = "en_US"

[[sources]]
name = "en_US"
repo = "Kengxxiao/ArknightsGameData_YoStar"
sha = "57010cb5b2afea112cae57daa756b58676ba6850"
locale = "en_US"
files = ["building_data.json", "character_table.json", "handbook_team_table.json"]

[[sources]]
name = "zh_CN"
repo = "Kengxxiao/ArknightsGameData"
sha = "bb8f9ac8db143a661577ed6ef5184d3c6e93d1d0"
locale = "zh_CN"
enabled = false
"#;

    #[test]
    fn parses_and_validates() {
        let m: Manifest = toml::from_str(SAMPLE).unwrap();
        m.validate().unwrap();
        assert_eq!(m.default_source().unwrap().name, "en_US");
        assert_eq!(m.enabled_sources().count(), 1);
        // `files` defaults to the required set when omitted.
        assert_eq!(m.source("zh_CN").unwrap().files, default_files());
    }

    #[test]
    fn raw_url_layout() {
        let m: Manifest = toml::from_str(SAMPLE).unwrap();
        let s = m.source("en_US").unwrap();
        assert_eq!(
            s.raw_url("building_data.json"),
            "https://raw.githubusercontent.com/Kengxxiao/ArknightsGameData_YoStar/57010cb5b2afea112cae57daa756b58676ba6850/en_US/gamedata/excel/building_data.json"
        );
    }

    #[test]
    fn rejects_bad_sha() {
        let bad = SAMPLE.replace("57010cb5b2afea112cae57daa756b58676ba6850", "main");
        let m: Manifest = toml::from_str(&bad).unwrap();
        assert!(matches!(m.validate(), Err(ManifestError::BadSha { .. })));
    }

    #[test]
    fn rejects_unknown_default() {
        let bad = SAMPLE.replace("default_source = \"en_US\"", "default_source = \"ko_KR\"");
        let m: Manifest = toml::from_str(&bad).unwrap();
        assert!(matches!(m.validate(), Err(ManifestError::UnknownSource(_))));
    }
}
