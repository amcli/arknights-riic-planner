//! `ak-api`: the HTTP layer.
//!
//! Layer 1 exposes read-only game-data endpoints so the frontend has
//! something real to render and the data version is visible in production:
//!
//! ```text
//! GET /healthz
//! GET /api/v1/gamedata/version          provenance + parser stats
//! GET /api/v1/gamedata/operators        summary list
//! GET /api/v1/gamedata/operators/{id}   full operator
//! GET /api/v1/gamedata/skills/{id}      full skill tier
//! ```
//!
//! Rosters, solves, and persistence arrive with later layers.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use ak_data::stats::DataStats;
use ak_data::{Strictness, TransformReport, load_source, stats};
use ak_domain::{BaseSkill, DataVersion, GameData, Operator, PowerId, Profession, Rarity};

#[derive(Parser)]
#[command(name = "ak-api", version, about)]
struct Cli {
    /// Address to listen on.
    #[arg(long, env = "AK_API_ADDR", default_value = "127.0.0.1:8080")]
    addr: SocketAddr,
    /// Data root containing manifest.toml (default: <workspace>/data).
    #[arg(long, env = ak_data::paths::DATA_DIR_ENV)]
    data_root: Option<PathBuf>,
    /// Manifest source to load (default: the manifest's default_source).
    #[arg(long, env = "AK_DATA_SOURCE")]
    source: Option<String>,
}

#[derive(Clone)]
struct AppState {
    data: Arc<GameData>,
    stats: Arc<DataStats>,
    report: Arc<TransformReport>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let root = cli
        .data_root
        .unwrap_or_else(ak_data::paths::default_data_root);
    let loaded = load_source(&root, cli.source.as_deref(), Strictness::Lenient)
        .context("loading game data")?;
    let stats = stats::compute(&loaded.data);
    tracing::info!(
        sha = loaded.data.version.short_sha(),
        operators = stats.operators,
        skipped = loaded.report.skipped.len(),
        "game data loaded"
    );

    let state = AppState {
        data: Arc::new(loaded.data),
        stats: Arc::new(stats),
        report: Arc::new(loaded.report),
    };

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/gamedata/version", get(version))
        .route("/api/v1/gamedata/operators", get(list_operators))
        .route("/api/v1/gamedata/operators/{id}", get(get_operator))
        .route("/api/v1/gamedata/skills/{id}", get(get_skill))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(cli.addr)
        .await
        .with_context(|| format!("binding {}", cli.addr))?;
    tracing::info!("listening on http://{}", cli.addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

#[derive(Serialize)]
struct VersionResponse<'a> {
    #[serde(flatten)]
    version: &'a DataVersion,
    stats: &'a DataStats,
    operators_skipped: usize,
}

async fn version(State(s): State<AppState>) -> Response {
    Json(VersionResponse {
        version: &s.data.version,
        stats: &s.stats,
        operators_skipped: s.report.skipped.len(),
    })
    .into_response()
}

#[derive(Serialize)]
struct OperatorSummary<'a> {
    id: &'a str,
    name: &'a str,
    rarity: Rarity,
    stars: u8,
    profession: Profession,
    sub_profession: &'a str,
    nation: Option<&'a PowerId>,
    group: Option<&'a PowerId>,
    team: Option<&'a PowerId>,
}

impl<'a> From<&'a Operator> for OperatorSummary<'a> {
    fn from(op: &'a Operator) -> Self {
        OperatorSummary {
            id: op.id.as_str(),
            name: &op.name,
            rarity: op.rarity,
            stars: op.rarity.stars(),
            profession: op.profession,
            sub_profession: op.sub_profession.as_str(),
            nation: op.nation.as_ref(),
            group: op.group.as_ref(),
            team: op.team.as_ref(),
        }
    }
}

async fn list_operators(State(s): State<AppState>) -> Response {
    let list: Vec<OperatorSummary<'_>> = s.data.operators.values().map(Into::into).collect();
    Json(list).into_response()
}

async fn get_operator(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match s.data.operator(&id) {
        Some(op) => Json::<&Operator>(op).into_response(),
        None => not_found(format!("no operator {id:?}")),
    }
}

async fn get_skill(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    match s.data.skill(&id) {
        Some(skill) => Json::<&BaseSkill>(skill).into_response(),
        None => not_found(format!("no skill {id:?}")),
    }
}

fn not_found(message: String) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}
