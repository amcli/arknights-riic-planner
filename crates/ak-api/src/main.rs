//! `ak-api`: connects to the database, loads the game data, picks up
//! unfinished solves, and serves the routes in [`ak_api::router`].
//! `ak-api migrate` applies the database migrations instead.
//!
//! Settings come from flags or the environment, and a `.env` file in the
//! working directory or above it fills in any the environment lacks.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::http::{HeaderValue, Method, header};
use clap::{Args, Parser, Subcommand};
use tower_http::cors::{AllowOrigin, CorsLayer};

use ak_api::{AppState, Jobs, Limits};
use ak_data::{Strictness, load_source};
use ak_store::{PoolConfig, Store};

#[derive(Parser)]
#[command(
    name = "ak-api",
    version,
    about,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    serve: Serve,
}

#[derive(Subcommand)]
enum Command {
    /// Apply any database migrations not yet applied, then exit.
    Migrate {
        /// Postgres URL of the database to migrate, as a role allowed to
        /// change its schema.
        #[arg(long, env = "AK_MIGRATE_DATABASE_URL", hide_env_values = true)]
        database_url: String,
    },
}

/// Serve the API (the default).
#[derive(Args)]
struct Serve {
    /// Postgres URL the server connects with, such as
    /// postgres://riic_app:PASSWORD@localhost/riic. Set it in the environment
    /// or .env rather than as a flag, which other users of the machine can
    /// see; add ?sslmode=verify-full for a database on another host.
    // Optional in the type so that `migrate` parses without it; clap still
    // requires it to serve.
    #[arg(long, env = "AK_DATABASE_URL", hide_env_values = true, required = true)]
    database_url: Option<String>,
    /// Most database connections the server holds open at once.
    #[arg(long, env = "AK_DB_MAX_CONNECTIONS", default_value_t = PoolConfig::DEFAULT_MAX_CONNECTIONS)]
    db_max_connections: u32,
    /// Longest a request waits for a free database connection before it is
    /// answered 503, in milliseconds.
    #[arg(
        long,
        env = "AK_DB_ACQUIRE_TIMEOUT_MS",
        default_value_t = PoolConfig::DEFAULT_ACQUIRE_TIMEOUT.as_millis() as u64
    )]
    db_acquire_timeout_ms: u64,
    /// Address to listen on.
    #[arg(long, env = "AK_API_ADDR", default_value = "127.0.0.1:8080")]
    addr: SocketAddr,
    /// Data root containing manifest.toml (default: <workspace>/data).
    #[arg(long, env = ak_data::paths::DATA_DIR_ENV)]
    data_root: Option<PathBuf>,
    /// Manifest source to load (default: the manifest's default_source).
    #[arg(long, env = "AK_DATA_SOURCE")]
    source: Option<String>,
    /// Browser origin allowed to call the API from another origin (CORS),
    /// such as http://localhost:5173. Repeat the flag or separate origins
    /// with commas. None by default: the frontend's dev server proxies
    /// /api, so it never needs CORS.
    #[arg(
        long = "cors-origin",
        env = "AK_API_CORS_ORIGINS",
        value_delimiter = ','
    )]
    cors_origins: Vec<String>,
    /// Most simulation ticks (horizon ÷ tick) one request may ask for.
    #[arg(long, env = "AK_API_MAX_TICKS", default_value_t = Limits::DEFAULT_MAX_TICKS)]
    max_ticks: u64,
    /// Longest a solve may search, in milliseconds.
    #[arg(long, env = "AK_API_MAX_SOLVE_MS", default_value_t = Limits::DEFAULT_MAX_SOLVE_MS)]
    max_solve_ms: u64,
    /// Solves that may run at once (default: half the CPU cores, at least
    /// one).
    #[arg(long, env = "AK_API_MAX_RUNNING_SOLVES")]
    max_running_solves: Option<usize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(e) if e.not_found() => {}
        Err(e) => return Err(e).context("reading .env"),
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Migrate { database_url }) => {
            ak_store::apply_migrations(&database_url)
                .await
                .context("migrating the database")?;
            tracing::info!("the database schema is up to date");
            Ok(())
        }
        None => serve(cli.serve).await,
    }
}

async fn serve(cli: Serve) -> anyhow::Result<()> {
    let cors = cors_layer(&cli.cors_origins)?;
    let pool = PoolConfig {
        max_connections: cli.db_max_connections,
        acquire_timeout: Duration::from_millis(cli.db_acquire_timeout_ms),
    };
    let url = cli
        .database_url
        .context("--database-url or AK_DATABASE_URL is required")?;
    let store = Store::connect(&url, &pool)
        .await
        .context("connecting to the database")?;
    store
        .check_schema()
        .await
        .context("checking the database schema")?;
    tracing::info!(?pool, "database connected");

    let root = cli
        .data_root
        .unwrap_or_else(ak_data::paths::default_data_root);
    let loaded = load_source(&root, cli.source.as_deref(), Strictness::Lenient)
        .context("loading game data")?;
    tracing::info!(
        sha = loaded.data.version.short_sha(),
        operators = loaded.data.operators.len(),
        skipped = loaded.report.skipped.len(),
        "game data loaded"
    );

    let limits = Limits {
        max_ticks: cli.max_ticks,
        max_solve_ms: cli.max_solve_ms,
        max_running_solves: cli
            .max_running_solves
            .unwrap_or_else(Limits::default_max_running_solves),
    };
    tracing::info!(?limits, "limits");
    let state = AppState::new(
        Arc::new(loaded.data),
        loaded.report.skipped.len(),
        store,
        limits,
    );
    tokio::spawn(ak_api::auth::session::sweep(state.store.clone()));
    let resumed = ak_api::jobs::resume(&state)
        .await
        .context("resuming unfinished solves")?;
    if resumed > 0 {
        tracing::info!(solves = resumed, "queued unfinished solves again");
    }

    let mut app = ak_api::router(state.clone());
    if let Some(cors) = cors {
        app = app.layer(cors);
    }
    let listener = tokio::net::TcpListener::bind(cli.addr)
        .await
        .with_context(|| format!("binding {}", cli.addr))?;
    tracing::info!("listening on http://{}", cli.addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state.jobs.clone()))
        .await?;
    state.jobs.drain().await;
    state.store.close().await;
    Ok(())
}

/// A CORS layer for the given origins, or none when the list is empty.
fn cors_layer(origins: &[String]) -> anyhow::Result<Option<CorsLayer>> {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .map(|o| o.trim())
        .filter(|o| !o.is_empty())
        .map(|o| HeaderValue::from_str(o).with_context(|| format!("bad CORS origin {o:?}")))
        .collect::<Result<_, _>>()?;
    if origins.is_empty() {
        return Ok(None);
    }
    tracing::info!(?origins, "CORS enabled");
    Ok(Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::CONTENT_TYPE, header::ACCEPT]),
    ))
}

/// Waits for Ctrl-C, then stops running solves so the process can exit;
/// they are queued again on the next start.
async fn shutdown_signal(jobs: Arc<Jobs>) {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
    jobs.shutdown();
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    async fn preflight(app: &Router, origin: &str) -> Option<String> {
        let req = Request::builder()
            .method(Method::OPTIONS)
            .uri("/x")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .map(|v| v.to_str().unwrap().to_owned())
    }

    #[test]
    fn migrate_needs_only_its_own_url() {
        let cli = Cli::try_parse_from(["ak-api", "migrate", "--database-url", "postgres://x"])
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            cli.command,
            Some(Command::Migrate { database_url }) if database_url == "postgres://x"
        ));
    }

    #[tokio::test]
    async fn cors_is_opt_in_and_limited_to_the_listed_origins() {
        assert!(cors_layer(&[]).unwrap().is_none());
        assert!(cors_layer(&[" ".into()]).unwrap().is_none());
        assert!(cors_layer(&["bad\norigin".into()]).is_err());

        let cors = cors_layer(&["http://localhost:5173".into()])
            .unwrap()
            .unwrap();
        let app = Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(cors);
        assert_eq!(
            preflight(&app, "http://localhost:5173").await.as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(preflight(&app, "https://evil.example").await, None);
    }
}
