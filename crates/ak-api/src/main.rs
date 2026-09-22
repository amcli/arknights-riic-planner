//! `ak-api`: the HTTP layer.
//!
//! ```text
//! GET    /healthz
//! GET    /api/v1/gamedata/version          provenance + parser stats
//! GET    /api/v1/gamedata/operators        summary list
//! GET    /api/v1/gamedata/operators/{id}   full operator
//! GET    /api/v1/gamedata/skills/{id}      full skill tier
//! POST   /api/v1/evaluate                  SimRequest → Snapshot (Layer 4)
//! POST   /api/v1/simulate                  SimRequest → SimResult (Layer 4)
//! POST   /api/v1/rosters                   { name?, roster } → stored document (Layer 6)
//! GET    /api/v1/rosters                   listing
//! GET    /api/v1/rosters/{id}              the roster
//! PUT    /api/v1/rosters/{id}              replace
//! DELETE /api/v1/rosters/{id}
//! POST   /api/v1/bases                     { name?, base } → stored document
//! GET    /api/v1/bases                     listing
//! GET    /api/v1/bases/{id}                the base
//! PUT    /api/v1/bases/{id}                replace
//! DELETE /api/v1/bases/{id}
//! POST   /api/v1/solves                    { name?, request } → job, 202 (Layer 5)
//! GET    /api/v1/solves                    listing with status
//! GET    /api/v1/solves/{id}               status, request, and result or error
//! DELETE /api/v1/solves/{id}
//! ```
//!
//! A solve runs in the background: the job document is written as
//! `pending`, moves to `running`, and ends `done` with the result or
//! `failed` with the reason. Poll `GET /api/v1/solves/{id}`.

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
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use ak_data::stats::DataStats;
use ak_data::{Strictness, TransformReport, load_source, stats};
use ak_domain::{
    Assignment, BaseConfig, BaseSkill, DataVersion, GameData, Operator, PowerId, Profession,
    Rarity, Roster,
};
use ak_solver::{SolveRequest, SolveResult, Space};
use ak_store::{Collection, Document, DocumentMeta, FileStore, Store, StoreError};

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
    /// Directory for saved rosters, bases and solves (default:
    /// <workspace>/store).
    #[arg(long, env = "AK_STORE_DIR")]
    store_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct AppState {
    data: Arc<GameData>,
    stats: Arc<DataStats>,
    report: Arc<TransformReport>,
    store: Arc<dyn Store>,
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

    let store_dir = cli.store_dir.unwrap_or_else(|| {
        root.parent()
            .map_or_else(|| PathBuf::from("store"), |p| p.join("store"))
    });
    let store = FileStore::open(&store_dir)
        .with_context(|| format!("opening store at {}", store_dir.display()))?;
    tracing::info!(dir = %store_dir.display(), "store opened");

    let state = AppState {
        data: Arc::new(loaded.data),
        stats: Arc::new(stats),
        report: Arc::new(loaded.report),
        store: Arc::new(store),
    };

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/gamedata/version", get(version))
        .route("/api/v1/gamedata/operators", get(list_operators))
        .route("/api/v1/gamedata/operators/{id}", get(get_operator))
        .route("/api/v1/gamedata/skills/{id}", get(get_skill))
        .route("/api/v1/evaluate", axum::routing::post(evaluate_route))
        .route("/api/v1/simulate", axum::routing::post(simulate_route))
        .route("/api/v1/rosters", get(list_rosters).post(post_roster))
        .route(
            "/api/v1/rosters/{id}",
            get(get_roster).put(put_roster).delete(delete_roster),
        )
        .route("/api/v1/bases", get(list_bases).post(post_base))
        .route(
            "/api/v1/bases/{id}",
            get(get_base).put(put_base).delete(delete_base),
        )
        .route("/api/v1/solves", get(list_solves).post(post_solve))
        .route("/api/v1/solves/{id}", get(get_solve).delete(delete_solve))
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

// ---- errors ------------------------------------------------------------------

/// An error response: a status and a JSON `{ "error": message }` body.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

fn bad_request(message: impl ToString) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.to_string())
}

fn not_found(message: impl ToString) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, message.to_string())
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::BadId(_) => bad_request(e),
            other => ApiError(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

/// Runs file or CPU work off the async executor.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

// ---- game data -----------------------------------------------------------

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

async fn get_operator(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    match s.data.operator(&id) {
        Some(op) => Ok(Json::<&Operator>(op).into_response()),
        None => Err(not_found(format!("no operator {id:?}"))),
    }
}

async fn get_skill(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    match s.data.skill(&id) {
        Some(skill) => Ok(Json::<&BaseSkill>(skill).into_response()),
        None => Err(not_found(format!("no skill {id:?}"))),
    }
}

// ---- simulation ------------------------------------------------------------

/// Runs the Layer 4 simulator. CPU-bound, so it runs off the async executor.
async fn simulate_route(
    State(s): State<AppState>,
    Json(req): Json<ak_eval::SimRequest>,
) -> Result<Response, ApiError> {
    let data = s.data.clone();
    let result = blocking(move || ak_eval::simulate(&data, &req))
        .await?
        .map_err(bad_request)?;
    Ok(Json(result).into_response())
}

/// Instantaneous room stats and morale rates for a request.
async fn evaluate_route(
    State(s): State<AppState>,
    Json(req): Json<ak_eval::SimRequest>,
) -> Result<Response, ApiError> {
    let snapshot = ak_eval::evaluate(
        &s.data,
        &req.base,
        &req.assignment,
        &req.roster,
        &req.config,
    )
    .map_err(bad_request)?;
    Ok(Json(snapshot).into_response())
}

// ---- stored documents ------------------------------------------------------

/// A stored roster or base with its metadata.
#[derive(Serialize)]
struct Stored<T> {
    #[serde(flatten)]
    meta: DocumentMeta,
    #[serde(flatten)]
    body: T,
}

#[derive(Deserialize)]
struct RosterBody {
    #[serde(default)]
    name: Option<String>,
    roster: Roster,
}

#[derive(Serialize, Deserialize)]
struct RosterView {
    roster: Roster,
}

#[derive(Deserialize)]
struct BaseBody {
    #[serde(default)]
    name: Option<String>,
    base: BaseConfig,
}

#[derive(Serialize, Deserialize)]
struct BaseView {
    base: BaseConfig,
}

fn check_roster(data: &GameData, roster: &Roster) -> Result<(), ApiError> {
    let unknown: Vec<&str> = roster
        .entries
        .keys()
        .filter(|id| !data.operators.contains_key(id.as_str()))
        .map(|id| id.as_str())
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(bad_request(format!(
            "unknown operators: {}",
            unknown.join(", ")
        )))
    }
}

async fn create(
    s: &AppState,
    collection: Collection,
    name: Option<String>,
    body: impl Serialize,
) -> Result<Response, ApiError> {
    let body = serde_json::to_value(body).map_err(|e| bad_request(e.to_string()))?;
    let doc = Document::new(name, body);
    let store = s.store.clone();
    let meta = doc.meta();
    blocking(move || store.put(collection, &doc)).await??;
    Ok((StatusCode::CREATED, Json(meta)).into_response())
}

async fn replace(
    s: &AppState,
    collection: Collection,
    id: String,
    name: Option<String>,
    body: impl Serialize,
) -> Result<Response, ApiError> {
    let body = serde_json::to_value(body).map_err(|e| bad_request(e.to_string()))?;
    let store = s.store.clone();
    let key = id.clone();
    let meta = blocking(move || -> Result<Option<DocumentMeta>, StoreError> {
        let Some(mut doc) = store.get(collection, &key)? else {
            return Ok(None);
        };
        if name.is_some() {
            doc.name = name;
        }
        doc.body = body;
        doc.touch();
        store.put(collection, &doc)?;
        Ok(Some(doc.meta()))
    })
    .await??;
    match meta {
        Some(meta) => Ok(Json(meta).into_response()),
        None => Err(not_found(format!("no {collection} document {id}"))),
    }
}

async fn fetch(s: &AppState, collection: Collection, id: String) -> Result<Document, ApiError> {
    let store = s.store.clone();
    let doc = blocking(move || store.get(collection, &id)).await??;
    doc.ok_or_else(|| not_found(format!("no {collection} document")))
}

async fn listing(s: &AppState, collection: Collection) -> Result<Vec<DocumentMeta>, ApiError> {
    let store = s.store.clone();
    Ok(blocking(move || store.list(collection)).await??)
}

async fn remove(s: &AppState, collection: Collection, id: String) -> Result<Response, ApiError> {
    let store = s.store.clone();
    let removed = blocking(move || store.delete(collection, &id)).await??;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(not_found(format!("no {collection} document")))
    }
}

fn view<T: for<'de> Deserialize<'de>>(doc: Document) -> Result<Stored<T>, ApiError> {
    let body: T = serde_json::from_value(doc.body.clone())
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Stored {
        meta: doc.meta(),
        body,
    })
}

async fn post_roster(
    State(s): State<AppState>,
    Json(body): Json<RosterBody>,
) -> Result<Response, ApiError> {
    check_roster(&s.data, &body.roster)?;
    create(
        &s,
        Collection::Rosters,
        body.name,
        RosterView {
            roster: body.roster,
        },
    )
    .await
}

async fn list_rosters(State(s): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(listing(&s, Collection::Rosters).await?).into_response())
}

async fn get_roster(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = fetch(&s, Collection::Rosters, id).await?;
    Ok(Json(view::<RosterView>(doc)?).into_response())
}

async fn put_roster(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RosterBody>,
) -> Result<Response, ApiError> {
    check_roster(&s.data, &body.roster)?;
    replace(
        &s,
        Collection::Rosters,
        id,
        body.name,
        RosterView {
            roster: body.roster,
        },
    )
    .await
}

async fn delete_roster(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    remove(&s, Collection::Rosters, id).await
}

async fn post_base(
    State(s): State<AppState>,
    Json(body): Json<BaseBody>,
) -> Result<Response, ApiError> {
    body.base.validate(&s.data).map_err(bad_request)?;
    create(
        &s,
        Collection::Bases,
        body.name,
        BaseView { base: body.base },
    )
    .await
}

async fn list_bases(State(s): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(listing(&s, Collection::Bases).await?).into_response())
}

async fn get_base(State(s): State<AppState>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let doc = fetch(&s, Collection::Bases, id).await?;
    Ok(Json(view::<BaseView>(doc)?).into_response())
}

async fn put_base(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<BaseBody>,
) -> Result<Response, ApiError> {
    body.base.validate(&s.data).map_err(bad_request)?;
    replace(
        &s,
        Collection::Bases,
        id,
        body.name,
        BaseView { base: body.base },
    )
    .await
}

async fn delete_base(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    remove(&s, Collection::Bases, id).await
}

// ---- solve jobs ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Pending,
    Running,
    Done,
    Failed,
}

/// The body of a solve document.
#[derive(Serialize, Deserialize)]
struct SolveJob {
    status: JobStatus,
    request: SolveRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<SolveResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
}

#[derive(Deserialize)]
struct SolveBody {
    #[serde(default)]
    name: Option<String>,
    request: SolveRequest,
}

#[derive(Serialize)]
struct SolveSummary {
    #[serde(flatten)]
    meta: DocumentMeta,
    status: JobStatus,
}

/// Rejects a request the solver would refuse, before a job is created.
fn check_solve(data: &GameData, req: &SolveRequest) -> Result<(), ApiError> {
    req.solver.validate().map_err(bad_request)?;
    req.config.validate().map_err(bad_request)?;
    check_roster(data, &req.roster)?;
    let initial = req
        .initial
        .clone()
        .unwrap_or_else(|| Assignment::empty(&req.base, data));
    Space::new(
        data,
        &req.base,
        &req.roster,
        req.pool.as_deref(),
        &initial,
        &req.locked,
    )
    .map_err(bad_request)?;
    Ok(())
}

async fn post_solve(
    State(s): State<AppState>,
    Json(body): Json<SolveBody>,
) -> Result<Response, ApiError> {
    check_solve(&s.data, &body.request)?;
    let job = SolveJob {
        status: JobStatus::Pending,
        request: body.request,
        result: None,
        error: None,
        started_at: None,
        finished_at: None,
    };
    let value = serde_json::to_value(&job).map_err(|e| bad_request(e.to_string()))?;
    let doc = Document::new(body.name, value);
    let meta = doc.meta();
    let id = doc.id.clone();
    let store = s.store.clone();
    blocking(move || store.put(Collection::Solves, &doc)).await??;

    let state = s.clone();
    tokio::task::spawn_blocking(move || run_job(&state, &id, job));

    Ok((
        StatusCode::ACCEPTED,
        Json(SolveSummary {
            meta,
            status: JobStatus::Pending,
        }),
    )
        .into_response())
}

/// Runs one solve job to completion, recording each status change.
fn run_job(state: &AppState, id: &str, mut job: SolveJob) {
    let save = |job: &SolveJob| -> Result<(), StoreError> {
        let Some(mut doc) = state.store.get(Collection::Solves, id)? else {
            return Ok(()); // deleted while running
        };
        doc.body = serde_json::to_value(job).unwrap_or_default();
        doc.touch();
        state.store.put(Collection::Solves, &doc)
    };
    job.status = JobStatus::Running;
    job.started_at = Some(ak_store::now());
    if let Err(e) = save(&job) {
        tracing::error!(id, "cannot mark solve running: {e}");
    }
    tracing::info!(id, "solve started");
    match ak_solver::solve(&state.data, &job.request) {
        Ok(result) => {
            tracing::info!(
                id,
                elapsed_ms = result.elapsed_ms,
                evaluations = result.evaluations,
                "solve done"
            );
            job.status = JobStatus::Done;
            job.result = Some(result);
        }
        Err(e) => {
            tracing::warn!(id, "solve failed: {e}");
            job.status = JobStatus::Failed;
            job.error = Some(e.to_string());
        }
    }
    job.finished_at = Some(ak_store::now());
    if let Err(e) = save(&job) {
        tracing::error!(id, "cannot save solve result: {e}");
    }
}

async fn list_solves(State(s): State<AppState>) -> Result<Response, ApiError> {
    let store = s.store.clone();
    let summaries = blocking(move || -> Result<Vec<SolveSummary>, StoreError> {
        let mut out = Vec::new();
        for meta in store.list(Collection::Solves)? {
            let status = store
                .get(Collection::Solves, &meta.id)?
                .and_then(|doc| serde_json::from_value(doc.body.get("status")?.clone()).ok())
                .unwrap_or(JobStatus::Failed);
            out.push(SolveSummary { meta, status });
        }
        Ok(out)
    })
    .await??;
    Ok(Json(summaries).into_response())
}

async fn get_solve(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let doc = fetch(&s, Collection::Solves, id).await?;
    Ok(Json(view::<SolveJob>(doc)?).into_response())
}

async fn delete_solve(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    remove(&s, Collection::Solves, id).await
}
