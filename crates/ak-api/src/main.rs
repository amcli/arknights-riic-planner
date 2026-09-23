//! `ak-api`: loads the game data, opens the store, picks up unfinished
//! solves, and serves the routes in [`ak_api::router`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::http::{HeaderValue, Method, header};
use clap::Parser;
use tower_http::cors::{AllowOrigin, CorsLayer};

use ak_api::{AppState, Jobs, Limits};
use ak_data::{Strictness, load_source};
use ak_store::FileStore;

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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let cors = cors_layer(&cli.cors_origins)?;
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

    let store_dir = cli.store_dir.unwrap_or_else(|| {
        root.parent()
            .map_or_else(|| PathBuf::from("store"), |p| p.join("store"))
    });
    let store = FileStore::open(&store_dir)
        .with_context(|| format!("opening store at {}", store_dir.display()))?;
    tracing::info!(dir = %store_dir.display(), "store opened");

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
        Arc::new(store),
        limits,
    );
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
