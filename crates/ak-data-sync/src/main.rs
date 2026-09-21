//! `ak-data-sync`: fetch the pinned upstream commit into `data/`, then prove
//! the result still parses.
//!
//! ```text
//! ak-data-sync sync            # fetch every enabled source, validate
//! ak-data-sync sync --force    # re-fetch even if the sidecar matches
//! ak-data-sync check           # validate what is on disk against the pins
//! ak-data-sync schema          # only the raw-JSON drift report
//! ```
//!
//! Bumping a pin is: edit `data/manifest.toml`, run `sync`, run the
//! workspace tests, commit all of it together.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

use ak_data::manifest::{
    BUILDING_FILE, CHARACTER_FILE, DataSource, FileRecord, MANIFEST_FILE, Manifest,
    SYNC_RECORD_FILE, SyncRecord, TEAM_FILE,
};
use ak_data::{Strictness, loader, schema, stats};

#[derive(Parser)]
#[command(name = "ak-data-sync", version, about)]
struct Cli {
    /// Data root containing manifest.toml (default: <workspace>/data).
    #[arg(long, env = ak_data::paths::DATA_DIR_ENV)]
    data_root: Option<PathBuf>,
    /// Only act on this manifest source (default: every enabled source).
    #[arg(long)]
    source: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Fetch pinned files from GitHub, write the sidecar, then validate.
    Sync {
        /// Re-download even if the on-disk sidecar already matches the pin.
        #[arg(long)]
        force: bool,
    },
    /// Verify on-disk files match the manifest pin and digests, then validate.
    Check,
    /// Print only the raw-JSON schema drift report.
    Schema,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let root = cli
        .data_root
        .unwrap_or_else(ak_data::paths::default_data_root);
    let manifest = Manifest::load(&root.join(MANIFEST_FILE))?;
    let sources: Vec<&DataSource> = match &cli.source {
        Some(name) => vec![
            manifest
                .source(name)
                .with_context(|| format!("no source named {name:?} in manifest"))?,
        ],
        None => manifest.enabled_sources().collect(),
    };
    if sources.is_empty() {
        bail!("no enabled sources in manifest");
    }

    let mut failed = false;
    for source in sources {
        let outcome = match &cli.cmd {
            Cmd::Sync { force } => {
                sync(&root, source, *force).and_then(|_| validate(&root, source))
            }
            Cmd::Check => check_pins(&root, source).and_then(|_| validate(&root, source)),
            Cmd::Schema => schema_report(&root, source).map(|_| ()),
        };
        if let Err(err) = outcome {
            tracing::error!(source = %source.name, "{err:#}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn sync(root: &Path, source: &DataSource, force: bool) -> anyhow::Result<()> {
    let dir = source.dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    if !force && on_disk_matches(&dir, source)? {
        tracing::info!(source = %source.name, sha = %source.sha, "already up to date");
        return Ok(());
    }

    let mut files = BTreeMap::new();
    for file in &source.files {
        let url = source.raw_url(file);
        tracing::info!(%url, "fetching");
        let bytes = fetch(&url)?;
        let path = dir.join(file);
        fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        files.insert(
            file.clone(),
            FileRecord {
                sha256: hex::encode(Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
            },
        );
        tracing::info!(file, bytes = bytes.len(), "written");
    }

    let record = SyncRecord {
        repo: source.repo.clone(),
        sha: source.sha.clone(),
        locale: source.locale.clone(),
        fetched_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        files,
    };
    let sidecar = dir.join(SYNC_RECORD_FILE);
    fs::write(&sidecar, serde_json::to_string_pretty(&record)? + "\n")
        .with_context(|| format!("writing {}", sidecar.display()))?;
    tracing::info!(source = %source.name, sha = %source.sha, "synced");
    Ok(())
}

fn fetch(url: &str) -> anyhow::Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()
        .with_context(|| format!("reading body of {url}"))?;
    Ok(bytes)
}

/// True when the sidecar pins the manifest sha and every file's digest
/// matches what is on disk.
fn on_disk_matches(dir: &Path, source: &DataSource) -> anyhow::Result<bool> {
    let Some(record) = loader::read_sync_record(dir)? else {
        return Ok(false);
    };
    if record.sha != source.sha {
        return Ok(false);
    }
    for file in &source.files {
        let Some(expected) = record.files.get(file) else {
            return Ok(false);
        };
        let path = dir.join(file);
        if !path.is_file() || digest_file(&path)? != expected.sha256 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn check_pins(root: &Path, source: &DataSource) -> anyhow::Result<()> {
    let dir = source.dir(root);
    let record = loader::read_sync_record(&dir)?
        .with_context(|| format!("{} has no {SYNC_RECORD_FILE}; run sync", dir.display()))?;
    if record.sha != source.sha {
        bail!(
            "{} was synced from {} but manifest pins {}; run sync",
            dir.display(),
            record.sha,
            source.sha
        );
    }
    for file in &source.files {
        let expected = record
            .files
            .get(file)
            .with_context(|| format!("sidecar has no digest for {file}; run sync --force"))?;
        let path = dir.join(file);
        let actual = digest_file(&path)?;
        if actual != expected.sha256 {
            bail!(
                "{} digest {} does not match sidecar {}; run sync --force",
                path.display(),
                actual,
                expected.sha256
            );
        }
    }
    tracing::info!(source = %source.name, sha = %source.sha, "pins and digests match");
    Ok(())
}

fn digest_file(path: &Path) -> anyhow::Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn schema_report(root: &Path, source: &DataSource) -> anyhow::Result<schema::SchemaReport> {
    let dir = source.dir(root);
    let building: serde_json::Value = loader::read_json(&dir.join(BUILDING_FILE))?;
    let characters: serde_json::Value = loader::read_json(&dir.join(CHARACTER_FILE))?;
    let teams: serde_json::Value = loader::read_json(&dir.join(TEAM_FILE))?;
    let report = schema::check_bundle(&building, &characters, &teams);
    for warning in &report.warnings {
        tracing::warn!(source = %source.name, "schema: {warning}");
    }
    for error in &report.errors {
        tracing::error!(source = %source.name, "schema: {error}");
    }
    if report.is_ok() {
        tracing::info!(
            source = %source.name,
            warnings = report.warnings.len(),
            "schema check passed"
        );
    }
    Ok(report)
}

fn validate(root: &Path, source: &DataSource) -> anyhow::Result<()> {
    let report = schema_report(root, source)?;
    if !report.is_ok() {
        bail!("{} schema error(s); see log", report.errors.len());
    }
    let loaded = loader::load_dir(&source.dir(root), Strictness::Strict)
        .context("strict transform failed")?;
    let s = stats::compute(&loaded.data);
    tracing::info!(
        source = %source.name,
        operators = s.operators,
        skill_tiers = s.skill_tiers,
        skill_families = s.skill_families,
        powers = s.powers,
        formulas = s.manufacture_formulas,
        "strict transform passed"
    );
    Ok(())
}
