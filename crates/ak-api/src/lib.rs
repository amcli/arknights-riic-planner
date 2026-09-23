//! `ak-api`: the HTTP layer (Layer 7).
//!
//! ```text
//! GET    /healthz
//! GET    /api/v1/gamedata/version          provenance + parser stats
//! GET    /api/v1/gamedata/operators        summary list
//! GET    /api/v1/gamedata/operators/{id}   full operator
//! GET    /api/v1/gamedata/skills/{id}      full skill tier
//! GET    /api/v1/gamedata/facilities       room kinds, per-level parameters
//! GET    /api/v1/gamedata/formulas         Factory formulas
//! POST   /api/v1/evaluate                  request → Snapshot
//! POST   /api/v1/simulate                  request → SimResult
//! POST   /api/v1/rosters                   { name?, source?, roster } → 201 + metadata
//! GET    /api/v1/rosters                   listing
//! GET    /api/v1/rosters/{id}              the roster and its source
//! PUT    /api/v1/rosters/{id}              replace
//! DELETE /api/v1/rosters/{id}
//! POST   /api/v1/bases                     { name?, base } → 201 + metadata
//! GET    /api/v1/bases                     listing
//! GET    /api/v1/bases/{id}                the base
//! PUT    /api/v1/bases/{id}                replace
//! DELETE /api/v1/bases/{id}
//! POST   /api/v1/solves                    { name?, request } → 202 + job
//! GET    /api/v1/solves                    listing with status
//! GET    /api/v1/solves/{id}               status, progress, result or error
//! POST   /api/v1/solves/{id}/stop          cancel if pending, stop early if running
//! DELETE /api/v1/solves/{id}               stop and remove
//! ```
//!
//! Requests to evaluate, simulate or solve may name a stored base or
//! roster as `base_id` / `roster_id` instead of including it ([`refs`]).
//! Solves run in the background, a few at a time ([`jobs`]). Every error is
//! JSON: `{ "error": message }` ([`error`]).
//!
//! The binary (`src/main.rs`) loads the game data, opens the store and
//! serves [`router`]; tests drive the same router in-process.

#![forbid(unsafe_code)]

pub mod bases;
pub mod error;
pub mod gamedata;
pub mod jobs;
pub mod refs;
pub mod rosters;
pub mod simulation;
pub mod solves;
pub mod stored;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use ak_data::stats::DataStats;
use ak_domain::GameData;
use ak_store::Store;

pub use error::ApiError;
pub use jobs::Jobs;

/// Bounds on the work one request can ask of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Most simulation ticks (horizon ÷ tick length) a simulate or solve
    /// request may ask for.
    pub max_ticks: u64,
    /// Longest a solve may search, in milliseconds. It becomes the time
    /// budget of any solve request without a shorter one.
    pub max_solve_ms: u64,
    /// Solves that may run at once; later ones wait as `pending`.
    pub max_running_solves: usize,
}

impl Limits {
    /// Default [`max_ticks`](Self::max_ticks): about 35 days at one-minute
    /// ticks, or several seconds of simulation.
    pub const DEFAULT_MAX_TICKS: u64 = 50_000;
    /// Default [`max_solve_ms`](Self::max_solve_ms): ten minutes.
    pub const DEFAULT_MAX_SOLVE_MS: u64 = 600_000;

    /// Default [`max_running_solves`](Self::max_running_solves): half the
    /// CPU cores, at least one. Each solve uses one core.
    pub fn default_max_running_solves() -> usize {
        std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_ticks: Self::DEFAULT_MAX_TICKS,
            max_solve_ms: Self::DEFAULT_MAX_SOLVE_MS,
            max_running_solves: Self::default_max_running_solves(),
        }
    }
}

/// Everything a handler needs.
#[derive(Clone)]
pub struct AppState {
    /// The loaded game data.
    pub data: Arc<GameData>,
    /// Counts and parser coverage for `/gamedata/version`.
    pub stats: Arc<DataStats>,
    /// Operators the lenient transform had to skip.
    pub operators_skipped: usize,
    /// Rosters, bases and solve jobs.
    pub store: Arc<dyn Store>,
    /// Bounds on the work one request can ask for.
    pub limits: Arc<Limits>,
    /// Queued and running solves.
    pub jobs: Arc<Jobs>,
}

impl AppState {
    /// Builds the state. Call [`jobs::resume`] afterwards to pick up solves
    /// an earlier run left unfinished.
    pub fn new(
        data: Arc<GameData>,
        operators_skipped: usize,
        store: Arc<dyn Store>,
        limits: Limits,
    ) -> Self {
        let stats = Arc::new(ak_data::stats::compute(&data));
        let jobs = Arc::new(Jobs::new(limits.max_running_solves));
        AppState {
            data,
            stats,
            operators_skipped,
            store,
            limits: Arc::new(limits),
            jobs,
        }
    }
}

/// The API's routes, with JSON fallbacks, gzip compression and request
/// tracing. CORS is left to the binary, since it depends on deployment.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/gamedata/version", get(gamedata::version))
        .route("/api/v1/gamedata/operators", get(gamedata::list_operators))
        .route(
            "/api/v1/gamedata/operators/{id}",
            get(gamedata::get_operator),
        )
        .route("/api/v1/gamedata/skills/{id}", get(gamedata::get_skill))
        .route(
            "/api/v1/gamedata/facilities",
            get(gamedata::list_facilities),
        )
        .route("/api/v1/gamedata/formulas", get(gamedata::list_formulas))
        .route("/api/v1/evaluate", post(simulation::evaluate))
        .route("/api/v1/simulate", post(simulation::simulate))
        .route("/api/v1/rosters", get(rosters::list).post(rosters::create))
        .route(
            "/api/v1/rosters/{id}",
            get(rosters::get).put(rosters::put).delete(rosters::delete),
        )
        .route("/api/v1/bases", get(bases::list).post(bases::create))
        .route(
            "/api/v1/bases/{id}",
            get(bases::get).put(bases::put).delete(bases::delete),
        )
        .route("/api/v1/solves", get(solves::list).post(solves::create))
        .route(
            "/api/v1/solves/{id}",
            get(solves::get).delete(solves::delete),
        )
        .route("/api/v1/solves/{id}/stop", post(solves::stop))
        .fallback(error::no_route)
        .method_not_allowed_fallback(error::wrong_method)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
