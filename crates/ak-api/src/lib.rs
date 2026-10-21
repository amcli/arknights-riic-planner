//! `ak-api`: the HTTP layer (Layer 7).
//!
//! ```text
//! GET    /healthz
//! POST   /api/v1/auth/register             { username, password, email? } → 201 + session cookie
//! POST   /api/v1/auth/login                { username, password } → session cookie
//! POST   /api/v1/auth/logout
//! GET    /api/v1/auth/me                   the account signed in
//! GET    /api/v1/gamedata/version          provenance + parser stats
//! GET    /api/v1/gamedata/operators        summary list
//! GET    /api/v1/gamedata/operators/{id}   full operator
//! GET    /api/v1/gamedata/skills/{id}      full skill tier
//! GET    /api/v1/gamedata/facilities       room kinds, per-level parameters
//! GET    /api/v1/gamedata/formulas         Factory formulas
//! GET    /api/v1/gamedata/layout           base slots and grid positions
//! GET    /api/v1/gamedata/constants        global tuning values
//! POST   /api/v1/evaluate                  request → Snapshot
//! POST   /api/v1/simulate                  request → SimResult
//! POST   /api/v1/rosters                   { name?, source?, roster } → 201 + metadata, import report
//! POST   /api/v1/rosters/preview           same body → the roster and report, not stored
//! GET    /api/v1/rosters                   listing
//! GET    /api/v1/rosters/{id}              the roster and its source
//! PUT    /api/v1/rosters/{id}              replace
//! DELETE /api/v1/rosters/{id}
//! GET    /api/v1/import/krooster/{username}
//!                                          a Krooster user's roster, from their public profile
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
//! GET    /api/v1/solves/{id}/candidates/{n}/simulation
//!                                          finalist n simulated as the solve scored it
//! ```
//!
//! The game data, `/healthz` and signing up, in and out are public.
//! Everything else needs a signed-in account ([`auth`]): each account sees,
//! names and changes only its own rosters, bases and solves, and another
//! account's answers `404` as if it did not exist. Those answers are
//! personal, so they are sent `Cache-Control: no-store`.
//!
//! Requests to evaluate, simulate or solve may name a stored base or
//! roster as `base_id` / `roster_id` instead of including it ([`refs`]).
//! Solves run in the background, a few at a time ([`jobs`]). A roster can
//! be fetched from Krooster by username ([`krooster`]), the one route that
//! reaches outside the server. Accounts sign in with a session cookie
//! ([`auth`]); requests from other sites that would use or start a session
//! are refused. Every error is JSON: `{ "error": message }` ([`error`]).
//!
//! Each client may make only so many requests, and fewer sign-ups and
//! sign-ins, before it is answered `429` ([`throttle`]); clients are told
//! apart by address, read from `X-Forwarded-For` only behind a trusted proxy
//! ([`client`]). Failed sign-ins to one username make its next attempt wait
//! ([`auth::backoff`]). Each account may have only so many solves queued or
//! running, and simulations computing, at once. Request bodies are capped at
//! [`MAX_BODY`], and at [`MAX_SIGN_IN_BODY`] for signing up and in.
//!
//! The binary (`src/main.rs`) loads the game data, connects to the
//! database and serves [`router`]; tests drive the same router in-process,
//! each against a fresh database.

#![forbid(unsafe_code)]

pub mod auth;
pub mod bases;
pub mod client;
pub mod error;
pub mod gamedata;
pub mod jobs;
pub mod krooster;
pub mod refs;
pub mod rosters;
pub mod simulation;
pub mod solves;
pub mod stored;
pub mod throttle;

use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, header};
use axum::middleware;
use axum::routing::{get, post};
use ipnet::IpNet;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use ak_data::stats::DataStats;
use ak_domain::GameData;
use ak_store::Store;

use auth::backoff::LoginBackoff;
use auth::password::Passwords;
pub use error::ApiError;
pub use jobs::Jobs;
use throttle::{InFlight, Rate, RateLimit};

/// Largest request body read, in bytes: ten times the largest roster export
/// a tool produces (about 100 KB).
pub const MAX_BODY: usize = 1024 * 1024;
/// Largest sign-up or sign-in body read, in bytes: room for the longest
/// password ([`auth::password::MAX_BYTES`]), a username and an email.
pub const MAX_SIGN_IN_BODY: usize = 16 * 1024;

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
    /// Solves one account may have queued or running at once.
    pub max_solves_per_account: NonZeroUsize,
    /// Simulations (evaluate, simulate, a finalist's) one account may have
    /// computing at once.
    pub max_simulations_per_account: NonZeroUsize,
    /// Requests one client may make, all routes together.
    pub requests: Rate,
    /// Sign-ups and sign-ins one client may make; each also counts toward
    /// [`requests`](Self::requests).
    pub sign_ins: Rate,
    /// Proxies trusted to name the client in `X-Forwarded-For` ([`client`]).
    pub trusted_proxies: Vec<IpNet>,
}

impl Limits {
    /// Default [`max_ticks`](Self::max_ticks): about 35 days at one-minute
    /// ticks, or several seconds of simulation.
    pub const DEFAULT_MAX_TICKS: u64 = 50_000;
    /// Default [`max_solve_ms`](Self::max_solve_ms): ten minutes.
    pub const DEFAULT_MAX_SOLVE_MS: u64 = 600_000;
    /// Default [`max_solves_per_account`](Self::max_solves_per_account).
    pub const DEFAULT_MAX_SOLVES_PER_ACCOUNT: NonZeroUsize = NonZeroUsize::new(2).unwrap();
    /// Default [`max_simulations_per_account`](Self::max_simulations_per_account).
    pub const DEFAULT_MAX_SIMULATIONS_PER_ACCOUNT: NonZeroUsize = NonZeroUsize::new(2).unwrap();
    /// Default [`requests`](Self::requests): 20 a second after 120 at once,
    /// far above what the frontend sends, polling a solve included.
    pub const DEFAULT_REQUESTS: Rate = Rate {
        per_minute: NonZeroU32::new(1200).unwrap(),
        burst: NonZeroU32::new(120).unwrap(),
    };
    /// Default [`sign_ins`](Self::sign_ins): five a minute after ten at once.
    pub const DEFAULT_SIGN_INS: Rate = Rate {
        per_minute: NonZeroU32::new(5).unwrap(),
        burst: NonZeroU32::new(10).unwrap(),
    };

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
            max_solves_per_account: Self::DEFAULT_MAX_SOLVES_PER_ACCOUNT,
            max_simulations_per_account: Self::DEFAULT_MAX_SIMULATIONS_PER_ACCOUNT,
            requests: Self::DEFAULT_REQUESTS,
            sign_ins: Self::DEFAULT_SIGN_INS,
            trusted_proxies: Vec::new(),
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
    pub store: Store,
    /// Bounds on the work one request can ask for.
    pub limits: Arc<Limits>,
    /// Queued and running solves.
    pub jobs: Arc<Jobs>,
    /// Where rosters are fetched from by Krooster username.
    pub krooster: Arc<krooster::Krooster>,
    /// Hashes and checks passwords, a few at a time.
    pub passwords: Arc<Passwords>,
    /// Each client's budget of requests.
    pub requests: Arc<RateLimit>,
    /// Each client's budget of sign-ups and sign-ins.
    pub sign_ins: Arc<RateLimit>,
    /// Failed sign-ins by username.
    pub logins: Arc<LoginBackoff>,
    /// Simulations computing, per account.
    pub simulations: Arc<InFlight>,
}

impl AppState {
    /// Builds the state. Call [`jobs::resume`] afterwards to pick up solves
    /// an earlier run left unfinished.
    pub fn new(
        data: Arc<GameData>,
        operators_skipped: usize,
        store: Store,
        limits: Limits,
    ) -> Self {
        let stats = Arc::new(ak_data::stats::compute(&data));
        let jobs = Arc::new(Jobs::new(limits.max_running_solves));
        let trusted: Arc<[IpNet]> = limits.trusted_proxies.clone().into();
        let requests = RateLimit::new("requests", limits.requests, trusted.clone());
        let sign_ins = RateLimit::new("sign-ups and sign-ins", limits.sign_ins, trusted);
        let simulations = InFlight::new("simulations", limits.max_simulations_per_account);
        AppState {
            data,
            stats,
            operators_skipped,
            store,
            limits: Arc::new(limits),
            jobs,
            krooster: Arc::new(krooster::Krooster::default()),
            passwords: Arc::new(Passwords::default()),
            requests: Arc::new(requests),
            sign_ins: Arc::new(sign_ins),
            logins: Arc::new(LoginBackoff::default()),
            simulations: Arc::new(simulations),
        }
    }
}

/// The API's routes, with JSON fallbacks, the per-client limits
/// ([`throttle`]), body limits, the cross-site guard
/// ([`auth::session::guard`]), gzip compression and request tracing. CORS
/// is left to the binary, since it depends on deployment, and so is giving
/// requests their peer address, without which no client is limited.
pub fn router(state: AppState) -> Router {
    let public = Router::new()
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
        .route("/api/v1/gamedata/layout", get(gamedata::layout))
        .route("/api/v1/gamedata/constants", get(gamedata::constants));
    // Signing up and in: also the sign-in budget, and a smaller body cap.
    let signing_in = Router::new()
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/login", post(auth::login))
        .route_layer(middleware::from_fn_with_state(
            state.sign_ins.clone(),
            throttle::limit,
        ))
        .layer(DefaultBodyLimit::max(MAX_SIGN_IN_BODY));
    // Accounts and everything they own: each handler takes the
    // `auth::CurrentUser` it acts for. Nobody else's business, caches
    // included.
    let personal = Router::new()
        .merge(signing_in)
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/evaluate", post(simulation::evaluate))
        .route("/api/v1/simulate", post(simulation::simulate))
        .route("/api/v1/rosters", get(rosters::list).post(rosters::create))
        .route("/api/v1/rosters/preview", post(rosters::preview))
        .route(
            "/api/v1/rosters/{id}",
            get(rosters::get).put(rosters::put).delete(rosters::delete),
        )
        .route("/api/v1/import/krooster/{username}", get(krooster::roster))
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
        .route(
            "/api/v1/solves/{id}/candidates/{n}/simulation",
            get(solves::candidate_simulation),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));
    Router::new()
        .merge(public)
        .merge(personal)
        .fallback(error::no_route)
        .method_not_allowed_fallback(error::wrong_method)
        .layer(middleware::from_fn(auth::session::guard))
        .layer(middleware::from_fn_with_state(
            state.requests.clone(),
            throttle::limit,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
