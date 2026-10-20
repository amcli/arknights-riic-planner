//! Layer 7 tests: the router driven in-process against the pinned snapshot
//! and a fresh database per test, which `#[sqlx::test]` creates (from
//! `DATABASE_URL`), migrates, and drops after a pass.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use ak_api::auth::backoff::{FIRST_WAIT, FREE_FAILURES};
use ak_api::auth::session::Token;
use ak_api::jobs::{JobStatus, SolveJob};
use ak_api::krooster::{Krooster, MAX_FETCHES};
use ak_api::refs::Refs;
use ak_api::throttle::Rate;
use ak_api::{AppState, Limits, MAX_BODY, MAX_SIGN_IN_BODY};
use ak_data::{Strictness, load_default};
use ak_domain::{BaseConfig, GameData, Roster};
use ak_eval::SimRequest;
use ak_solver::SolveRequest;
use ak_store::accounts::{Session, User};
use ak_store::{Collection, Document, Store};

fn data() -> Arc<GameData> {
    static DATA: OnceLock<Arc<GameData>> = OnceLock::new();
    DATA.get_or_init(|| {
        Arc::new(
            load_default(Strictness::Strict)
                .expect("pinned snapshot loads")
                .data,
        )
    })
    .clone()
}

fn example(name: &str) -> Value {
    let path = format!(
        "{}/../../examples/requests/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A roster export fixture from `ak-data`.
fn export(name: &str) -> Value {
    let path = format!(
        "{}/../ak-data/tests/fixtures/import/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A solve request over the example base that searches for as long as it
/// is allowed to.
fn endless_solve() -> Value {
    let sim = example("243-base.json");
    json!({
        "base": sim["base"],
        "roster": sim["roster"],
        "initial": sim["assignment"],
        "config": sim["config"],
        "solver": { "strategy": "annealing", "iterations": 10_000_000, "restarts": 1, "top_k": 2 },
    })
}

/// A solve request over the example base that finishes quickly.
fn quick_solve() -> Value {
    let mut req = endless_solve();
    req["solver"] = json!({ "strategy": "annealing", "iterations": 60, "restarts": 1, "top_k": 2 });
    req
}

struct Harness {
    app: Router,
    state: AppState,
    /// The session cookie sent with every request that sends none of its
    /// own.
    cookie: Option<String>,
}

impl Harness {
    /// A server over the test's database, not signed in.
    fn new(pool: PgPool, limits: Limits) -> Self {
        let state = AppState::new(data(), 0, Store::from_pool(pool), limits);
        Harness {
            app: ak_api::router(state.clone()),
            state,
            cookie: None,
        }
    }

    /// A server over the test's database, signed in as a new account,
    /// `doctor`.
    async fn signed_in(pool: PgPool, limits: Limits) -> Self {
        let mut h = Self::new(pool, limits);
        h.cookie = Some(h.register("doctor").await);
        h
    }

    /// The account signed in.
    async fn user_id(&self) -> Uuid {
        let (status, user) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{user}");
        user["id"].as_str().unwrap().parse().unwrap()
    }

    /// Fetches Krooster rosters from `base` instead of Krooster itself.
    fn with_krooster(mut self, base: &str) -> Self {
        self.state.krooster = Arc::new(Krooster::at(base));
        self.app = ak_api::router(self.state.clone());
        self
    }

    async fn raw(&self, method: Method, uri: &str, body: Option<&str>) -> Response<Body> {
        self.raw_with(method, uri, body, &[]).await
    }

    async fn raw_with(
        &self,
        method: Method,
        uri: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Response<Body> {
        self.raw_as(self.cookie.as_deref(), None, method, uri, body, headers)
            .await
    }

    /// A request carrying `cookie`, unless `headers` carry one of their own,
    /// and coming from `peer` as if through a socket, if given.
    async fn raw_as(
        &self,
        cookie: Option<&str>,
        peer: Option<SocketAddr>,
        method: Method,
        uri: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Response<Body> {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(addr) = peer {
            req = req.extension(ConnectInfo(addr));
        }
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let own_cookie = headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("cookie"));
        if let (Some(cookie), false) = (cookie, own_cookie) {
            req = req.header(header::COOKIE, cookie);
        }
        let body = match body {
            Some(text) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(text.to_owned())
            }
            None => Body::empty(),
        };
        self.app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let text = body.map(|b| b.to_string());
        let resp = self.raw(method, uri, text.as_deref()).await;
        read(resp).await
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.call(Method::GET, uri, None).await
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::POST, uri, Some(body)).await
    }

    async fn put(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.call(Method::PUT, uri, Some(body)).await
    }

    async fn delete(&self, uri: &str) -> (StatusCode, Value) {
        self.call(Method::DELETE, uri, None).await
    }

    /// Stores a document and returns its id.
    async fn create(&self, collection: &str, body: Value) -> String {
        let (status, meta) = self.post(&format!("/api/v1/{collection}"), body).await;
        assert!(
            status == StatusCode::CREATED || status == StatusCode::ACCEPTED,
            "{status} {meta}"
        );
        meta["id"].as_str().unwrap().to_owned()
    }

    /// Polls a solve until `pred` holds.
    async fn wait_for(&self, id: &str, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let (status, job) = self.get(&format!("/api/v1/solves/{id}")).await;
            assert_eq!(status, StatusCode::OK, "{job}");
            if pred(&job) {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; status {}",
                job["status"]
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Waits until no solve is queued or running, and every runner, even a
    /// deleted solve's, has finished.
    async fn wait_idle(&self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.state.jobs.live_count() > 0 || self.state.jobs.running() > 0 {
            assert!(Instant::now() < deadline, "solves never finished");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Let any search still running end quickly, so the test runtime can
        // shut down.
        self.state.jobs.shutdown();
    }
}

async fn read(resp: Response<Body>) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, value)
}

/// `value` as the server stores it: read as `T` and written back, so
/// defaults are filled in.
fn canonical<T: serde::de::DeserializeOwned + serde::Serialize>(value: &Value) -> Value {
    serde_json::to_value(serde_json::from_value::<T>(value.clone()).unwrap()).unwrap()
}

/// `value` as a client reads it off the wire.
fn wire(value: &impl serde::Serialize) -> Value {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn error_of(body: &Value) -> &str {
    body["error"]
        .as_str()
        .unwrap_or_else(|| panic!("not an error body: {body}"))
}

fn is(status: &str) -> impl Fn(&Value) -> bool + '_ {
    move |job: &Value| job["status"] == status
}

// ---- game data -------------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn game_data_endpoints(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let data = data();

    let (status, body) = h.get("/healthz").await;
    assert_eq!((status, body), (StatusCode::OK, json!("ok")));

    let (status, v) = h.get("/api/v1/gamedata/version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["sha"], data.version.sha.as_str());
    assert_eq!(v["stats"]["operators"], data.operators.len());
    assert_eq!(
        v["stats"]["mechanics_unparsed"],
        ak_data::stats::compute(&data).mechanics_unparsed
    );
    assert_eq!(v["operators_skipped"], 0);

    let (status, ops) = h.get("/api/v1/gamedata/operators").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ops.as_array().unwrap().len(), data.operators.len());

    let (status, op) = h.get("/api/v1/gamedata/operators/char_285_medic2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(op["id"], "char_285_medic2");
    let (status, err) = h.get("/api/v1/gamedata/operators/char_nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(error_of(&err).contains("no operator"));

    let (status, skill) = h.get("/api/v1/gamedata/skills/control_bd_spd[000]").await;
    assert_eq!(status, StatusCode::OK, "{skill}");
    assert!(skill["mechanics"].is_object());

    let (status, rooms) = h.get("/api/v1/gamedata/facilities").await;
    assert_eq!(status, StatusCode::OK);
    let rooms = rooms.as_array().unwrap();
    assert_eq!(rooms.len(), data.facilities.len());
    let factory = rooms
        .iter()
        .find(|r| r["room_type"] == "MANUFACTURE")
        .unwrap();
    assert_eq!(factory["phases"][2]["max_stationed"], 3);

    let (status, formulas) = h.get("/api/v1/gamedata/formulas").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        formulas.as_array().unwrap().len(),
        data.manufacture_formulas.len()
    );
}

// ---- errors ----------------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn every_error_is_json(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;

    let (status, err) = h.get("/api/v1/nothing").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&err), "no route GET /api/v1/nothing");

    let resp = h.raw(Method::DELETE, "/api/v1/simulate", None).await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(resp.headers()[header::ALLOW], "POST");
    let (_, err) = read(resp).await;
    assert!(error_of(&err).contains("does not accept DELETE"), "{err}");

    let resp = h.raw(Method::POST, "/api/v1/rosters", Some("{")).await;
    let (status, err) = read(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    error_of(&err);

    let req = Request::post("/api/v1/rosters")
        .header(header::COOKIE, h.cookie.as_deref().unwrap())
        .body(Body::from("{}"))
        .unwrap();
    let (status, err) = read(h.app.clone().oneshot(req).await.unwrap()).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    error_of(&err);

    // Wrong shapes name the path, whether the body is read by axum or
    // after resolving references.
    let bad_base = json!({ "rooms": [{ "id": "x", "kind": "NOPE", "level": 1 }] });
    let (status, err) = h.post("/api/v1/bases", json!({ "base": bad_base })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("base.rooms[0].kind"), "{err}");
    let (status, err) = h
        .post(
            "/api/v1/simulate",
            json!({ "base": bad_base, "assignment": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("base.rooms[0].kind"), "{err}");

    let (status, err) = h.get("/api/v1/rosters/not%20an%20id").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("invalid document id"), "{err}");
    let (status, err) = h.delete("/api/v1/solves/42").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("invalid document id"), "{err}");
    // Only the spelling the store issues names a document.
    let id = h
        .create("bases", json!({ "base": example("243-base.json")["base"] }))
        .await;
    let (status, _) = h.get(&format!("/api/v1/bases/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, err) = h.get(&format!("/api/v1/bases/{}", id.to_uppercase())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("invalid document id"), "{err}");
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn text_postgres_cannot_store_is_a_400(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let nul = "names and bodies may not contain NUL (\\u0000) characters";
    // In a name (a text column)…
    let base = example("243-base.json")["base"].clone();
    let (status, err) = h
        .post("/api/v1/bases", json!({ "name": "a\u{0}b", "base": base }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&err), nul);
    // …and in a body (jsonb): an unknown operator's id is kept in the
    // import report.
    let roster = json!({ "char_x\u{0}": { "op_id": "char_x\u{0}", "elite": 0, "level": 1, "potential": 1 } });
    let (status, err) = h
        .post(
            "/api/v1/rosters",
            json!({ "source": "krooster", "roster": roster }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(error_of(&err), nul);
    let (_, listed) = h.get("/api/v1/rosters").await;
    assert_eq!(listed, json!([]));
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn an_unreachable_database_is_a_503(pool: PgPool) {
    let h = Harness::signed_in(pool.clone(), Limits::default()).await;
    pool.close().await;
    let (status, err) = h.get("/api/v1/rosters").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error_of(&err),
        "the database is unavailable; try again shortly"
    );
}

// ---- simulation ------------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn simulate_matches_the_library(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let req = example("243-base.json");

    let (status, result) = h.post("/api/v1/simulate", req.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let parsed: SimRequest = serde_json::from_value(req.clone()).unwrap();
    let direct = ak_eval::simulate(&data(), &parsed).unwrap();
    assert_eq!(result, wire(&direct));
    // The reference numbers for this base (f2 and f4 are unstaffed).
    let lmd = result["totals"]["lmd"].as_f64().unwrap();
    assert!((10_000.0..10_200.0).contains(&lmd), "{lmd}");

    let (status, snapshot) = h.post("/api/v1/evaluate", req.clone()).await;
    assert_eq!(status, StatusCode::OK, "{snapshot}");
    assert!(!snapshot["rooms"].as_array().unwrap().is_empty());

    let mut unpowered = req.clone();
    unpowered["base"]["rooms"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["kind"] != "POWER");
    unpowered["assignment"]
        .as_object_mut()
        .unwrap()
        .retain(|room, _| !room.starts_with('p'));
    let (status, err) = h.post("/api/v1/simulate", unpowered).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).starts_with("invalid base"), "{err}");
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn long_runs_are_refused(pool: PgPool) {
    let h = Harness::signed_in(
        pool,
        Limits {
            max_ticks: 100,
            ..Limits::default()
        },
    )
    .await;
    // 24 hours at 5 minutes is 288 ticks.
    let (status, err) = h.post("/api/v1/simulate", example("243-base.json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("limit of 100"), "{err}");
    let (status, err) = h
        .post("/api/v1/solves", json!({ "request": quick_solve() }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("limit of 100"), "{err}");
    // Evaluating one instant has no ticks.
    let (status, _) = h.post("/api/v1/evaluate", example("243-base.json")).await;
    assert_eq!(status, StatusCode::OK);
}

// ---- stored documents ------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn rosters_round_trip(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let roster = example("243-base.json")["roster"].clone();

    let id = h
        .create("rosters", json!({ "name": "mine", "roster": roster }))
        .await;
    let (status, doc) = h.get(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["name"], "mine");
    assert_eq!(doc["source"], "manual");
    assert_eq!(doc["roster"], canonical::<Roster>(&roster));
    assert_eq!(doc["schema_version"], 1);

    let (_, list) = h.get("/api/v1/rosters").await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (status, _) = h
        .put(&format!("/api/v1/rosters/{id}"), json!({ "roster": {} }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, doc) = h.get(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(doc["roster"], json!({}));
    assert_eq!(doc["name"], "mine");

    let (status, err) = h
        .post(
            "/api/v1/rosters",
            json!({ "roster": { "char_nope": { "promotion": { "phase": "PHASE_2", "level": 1 } } } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&err), "unknown operators: char_nope");

    let (status, err) = h
        .post(
            "/api/v1/rosters",
            json!({ "source": "krooster", "roster": [1] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&err), "row 0: expected an object");
    let (status, _) = h
        .post(
            "/api/v1/rosters",
            json!({ "source": "spreadsheet", "roster": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = h.delete(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, err) = h.get(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&err), format!("no roster with id {id}"));
    let (status, _) = h.delete(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn bases_are_validated(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let base = example("243-base.json")["base"].clone();
    let id = h.create("bases", json!({ "base": base })).await;
    let (_, doc) = h.get(&format!("/api/v1/bases/{id}")).await;
    assert_eq!(doc["base"], canonical::<BaseConfig>(&base));

    let mut unpowered = base.clone();
    unpowered["rooms"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["kind"] != "POWER");
    let (status, err) = h.post("/api/v1/bases", json!({ "base": unpowered })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("power"), "{err}");
    let (status, _) = h
        .put(&format!("/api/v1/bases/{id}"), json!({ "base": unpowered }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn requests_can_name_stored_documents(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let req = example("243-base.json");
    let base_id = h.create("bases", json!({ "base": req["base"] })).await;
    let roster_id = h
        .create("rosters", json!({ "roster": req["roster"] }))
        .await;

    let by_ref = json!({
        "base_id": base_id,
        "roster_id": roster_id,
        "assignment": req["assignment"],
        "config": req["config"],
    });
    let (status, a) = h.post("/api/v1/simulate", by_ref.clone()).await;
    assert_eq!(status, StatusCode::OK, "{a}");
    let (_, b) = h.post("/api/v1/simulate", req.clone()).await;
    assert_eq!(a, b);
    let (status, _) = h.post("/api/v1/evaluate", by_ref.clone()).await;
    assert_eq!(status, StatusCode::OK);

    let mut both = by_ref.clone();
    both["base"] = req["base"].clone();
    let (status, err) = h.post("/api/v1/simulate", both).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&err), "give `base` or `base_id`, not both");

    let mut missing = by_ref.clone();
    missing["roster_id"] = json!("00000000-0000-0000-0000-000000000000");
    let (status, err) = h.post("/api/v1/simulate", missing).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).starts_with("no stored roster"), "{err}");

    let mut not_text = by_ref.clone();
    not_text["base_id"] = json!(5);
    let (status, err) = h.post("/api/v1/simulate", not_text).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&err), "`base_id` must be a string");

    let (status, err) = h.post("/api/v1/simulate", json!([1, 2])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    error_of(&err);
}

// ---- solves ----------------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn a_solve_runs_in_the_background(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let req = quick_solve();
    let base_id = h.create("bases", json!({ "base": req["base"] })).await;
    let roster_id = h
        .create("rosters", json!({ "roster": req["roster"] }))
        .await;
    let mut by_ref = req.clone();
    let fields = by_ref.as_object_mut().unwrap();
    fields.remove("base");
    fields.remove("roster");
    fields.insert("base_id".into(), json!(base_id));
    fields.insert("roster_id".into(), json!(roster_id));

    let (status, summary) = h
        .post(
            "/api/v1/solves",
            json!({ "name": "quick", "request": by_ref }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{summary}");
    assert_eq!(summary["status"], "pending");
    assert_eq!(summary["name"], "quick");
    let id = summary["id"].as_str().unwrap().to_owned();

    let job = h.wait_for(&id, "done", is("done")).await;
    assert_eq!(
        job["refs"],
        json!({ "base_id": base_id, "roster_id": roster_id })
    );
    // The stored request is self-contained, with the server's time cap.
    assert_eq!(
        job["request"]["base"],
        canonical::<BaseConfig>(&req["base"])
    );
    assert_eq!(
        job["request"]["roster"],
        canonical::<Roster>(&req["roster"])
    );
    assert_eq!(
        job["request"]["solver"]["time_budget_ms"],
        Limits::DEFAULT_MAX_SOLVE_MS
    );
    assert_eq!(job["attempts"], 1);
    assert!(job["started_at"].is_string() && job["finished_at"].is_string());
    assert!(job.get("progress").is_none());
    let result = &job["result"];
    assert!(result.get("stopped").is_none(), "{}", result["stopped"]);
    let best = result["candidates"][0]["score"].as_f64().unwrap();
    let start = result["initial"]["score"].as_f64().unwrap();
    assert!(best >= start - 1e-6, "best {best} < start {start}");

    let (_, list) = h.get("/api/v1/solves").await;
    assert_eq!(list[0]["id"], id.as_str());
    assert_eq!(list[0]["status"], "done");

    let (status, err) = h
        .post(&format!("/api/v1/solves/{id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(error_of(&err).contains("is done"), "{err}");

    let (status, _) = h.delete(&format!("/api/v1/solves/{id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = h.get(&format!("/api/v1/solves/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn a_bad_solve_is_refused_before_it_is_queued(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let mut req = quick_solve();
    req["solver"]["restarts"] = json!(0);
    let (status, err) = h.post("/api/v1/solves", json!({ "request": req })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("restarts"), "{err}");
    let mut req = quick_solve();
    req["locked"] = json!([{ "room": "nowhere", "index": 0 }]);
    let (status, err) = h.post("/api/v1/solves", json!({ "request": req })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("nowhere"), "{err}");
    let (_, list) = h.get("/api/v1/solves").await;
    assert_eq!(list, json!([]));
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn a_running_solve_reports_progress_and_can_be_stopped(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let id = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    let job = h
        .wait_for(&id, "progress", |job| {
            job["status"] == "running" && job["progress"]["evaluations"].as_u64() > Some(40)
        })
        .await;
    let progress = &job["progress"];
    assert_eq!(progress["phase"], "searching");
    assert_eq!(progress["strategy"], "annealing");
    assert_eq!(progress["total"], 10_000_000.0);
    assert!(progress["best_score"].as_f64() >= progress["initial_score"].as_f64());

    let (status, view) = h
        .post(&format!("/api/v1/solves/{id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{}", view["status"]);
    assert!(view["status"] == "running" || view["status"] == "done");

    let job = h.wait_for(&id, "done", is("done")).await;
    let result = &job["result"];
    assert_eq!(result["stopped"], "requested");
    assert!(result["evaluations"].as_u64().unwrap() < 10_000_000);
    let best = result["candidates"][0]["score"].as_f64().unwrap();
    let start = result["initial"]["score"].as_f64().unwrap();
    assert!(best >= start - 1e-6, "best {best} < start {start}");
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn solves_queue_and_can_be_cancelled_or_deleted(pool: PgPool) {
    let h = Harness::signed_in(
        pool,
        Limits {
            max_running_solves: 1,
            ..Limits::default()
        },
    )
    .await;
    let first = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    let second = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    h.wait_for(&first, "running", is("running")).await;
    let (_, queued) = h.get(&format!("/api/v1/solves/{second}")).await;
    assert_eq!(queued["status"], "pending");

    let (status, view) = h
        .post(&format!("/api/v1/solves/{second}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["status"], "cancelled");
    assert!(view["finished_at"].is_string());
    let (status, err) = h
        .post(&format!("/api/v1/solves/{second}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(error_of(&err).contains("is cancelled"), "{err}");

    // Deleting a running solve stops it, and it is never written back.
    let (status, _) = h.delete(&format!("/api/v1/solves/{first}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    h.wait_idle().await;
    let (status, _) = h.get(&format!("/api/v1/solves/{first}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = h.get("/api/v1/solves").await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![second.as_str()]);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn the_server_caps_search_time(pool: PgPool) {
    let h = Harness::signed_in(
        pool,
        Limits {
            max_solve_ms: 300,
            ..Limits::default()
        },
    )
    .await;
    let id = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    let job = h.wait_for(&id, "done", is("done")).await;
    assert_eq!(job["request"]["solver"]["time_budget_ms"], 300);
    assert_eq!(job["result"]["stopped"], "time_budget");

    // A shorter budget of the request's own is kept.
    let mut req = endless_solve();
    req["solver"]["time_budget_ms"] = json!(100);
    let id = h.create("solves", json!({ "request": req })).await;
    let job = h.wait_for(&id, "done", is("done")).await;
    assert_eq!(job["request"]["solver"]["time_budget_ms"], 100);
}

/// Writes one of `owner`'s solve documents straight into a store.
async fn stored_job(
    store: &Store,
    owner: Uuid,
    request: &Value,
    status: JobStatus,
    attempts: u32,
) -> String {
    let request: SolveRequest = serde_json::from_value(request.clone()).unwrap();
    let mut job = SolveJob::new(request, Refs::default());
    job.status = status;
    job.attempts = attempts;
    let doc = Document::new(owner, None, serde_json::to_value(&job).unwrap());
    store.insert(Collection::Solves, &doc).await.unwrap();
    doc.id.to_string()
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn unfinished_solves_resume_after_a_restart(pool: PgPool) {
    // Solves picked up after a restart were admitted before it, so the
    // account's allowance does not hold them back: two resume here.
    let h = Harness::signed_in(
        pool,
        Limits {
            max_solves_per_account: NonZeroUsize::MIN,
            ..Limits::default()
        },
    )
    .await;
    let (store, owner, quick) = (&h.state.store, h.user_id().await, quick_solve());
    let interrupted = stored_job(store, owner, &quick, JobStatus::Running, 1).await;
    let waiting = stored_job(store, owner, &quick, JobStatus::Pending, 0).await;
    let attempts = ak_api::jobs::MAX_ATTEMPTS;
    let doomed = stored_job(store, owner, &quick, JobStatus::Running, attempts).await;
    let cancelled = stored_job(store, owner, &quick, JobStatus::Cancelled, 0).await;
    // Someone else's unfinished solve is picked up too, and stays theirs.
    let other = User::new("kaltsit".into(), None);
    assert!(store.create_user(&other, "hash").await.unwrap());
    let theirs = stored_job(store, other.id, &quick, JobStatus::Pending, 0).await;
    assert_eq!(ak_api::jobs::resume(&h.state).await.unwrap(), 3);

    let job = h.wait_for(&interrupted, "done", is("done")).await;
    assert_eq!(job["attempts"], 2);
    let job = h.wait_for(&waiting, "done", is("done")).await;
    assert_eq!(job["attempts"], 1);
    let (_, job) = h.get(&format!("/api/v1/solves/{doomed}")).await;
    assert_eq!(job["status"], "failed");
    assert!(
        job["error"].as_str().unwrap().contains("giving up"),
        "{}",
        job["error"]
    );
    let (_, job) = h.get(&format!("/api/v1/solves/{cancelled}")).await;
    assert_eq!(job["status"], "cancelled");
    assert!(job.get("started_at").is_none());
    let (status, _) = h.get(&format!("/api/v1/solves/{theirs}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    h.wait_idle().await;
    let theirs = theirs.parse().unwrap();
    let doc = store
        .get(other.id, Collection::Solves, theirs)
        .await
        .unwrap();
    assert_eq!(doc.unwrap().body["status"], "done");
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn shutdown_leaves_running_solves_for_the_next_start(pool: PgPool) {
    let (id, cookie) = {
        let h = Harness::signed_in(pool.clone(), Limits::default()).await;
        let id = h
            .create("solves", json!({ "request": endless_solve() }))
            .await;
        h.wait_for(&id, "running", is("running")).await;
        h.state.jobs.shutdown();
        h.wait_idle().await;
        let (_, job) = h.get(&format!("/api/v1/solves/{id}")).await;
        assert_eq!(job["status"], "running");
        assert!(job.get("result").is_none());
        (id, h.cookie.clone())
    };
    // The next start, with the same account still signed in.
    let mut h = Harness::new(pool, Limits::default());
    h.cookie = cookie;
    assert_eq!(ak_api::jobs::resume(&h.state).await.unwrap(), 1);
    let job = h
        .wait_for(&id, "running again", |job| job["attempts"] == 2)
        .await;
    assert_eq!(job["status"], "running");
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn shutdown_waits_for_a_stopped_solve_to_be_recorded(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let id = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    h.wait_for(&id, "running", is("running")).await;
    let (status, _) = h
        .post(&format!("/api/v1/solves/{id}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    // The server stops while the stopped search is still winding down; it
    // must not exit before the result is written.
    h.state.jobs.shutdown();
    h.state.jobs.drain().await;
    let (_, job) = h.get(&format!("/api/v1/solves/{id}")).await;
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["result"]["stopped"], "requested");
}

// ---- accounts --------------------------------------------------------------

const PASSWORD: &str = "correct horse";

/// The `Set-Cookie` line for the session cookie, attributes and all.
fn set_cookie_line(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("__Host-session="))
        .map(str::to_owned)
}

/// The session cookie a response sets, as a request sends it back.
fn session_of(headers: &HeaderMap) -> Option<String> {
    let line = set_cookie_line(headers)?;
    Some(line.split(';').next().unwrap().to_owned())
}

impl Harness {
    /// A request with extra headers; answers status, headers and body.
    async fn send(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Value) {
        let text = body.map(|b| b.to_string());
        let resp = self.raw_with(method, uri, text.as_deref(), headers).await;
        let headers = resp.headers().clone();
        let (status, body) = read(resp).await;
        (status, headers, body)
    }

    /// Registers `username` and returns its session cookie. Sent as a
    /// stranger: signing up ends the session a request carries, and the
    /// harness's own must survive.
    async fn register(&self, username: &str) -> String {
        let body = json!({ "username": username, "password": PASSWORD }).to_string();
        let resp = self
            .raw_as(
                None,
                None,
                Method::POST,
                "/api/v1/auth/register",
                Some(&body),
                &[],
            )
            .await;
        let headers = resp.headers().clone();
        let (status, user) = read(resp).await;
        assert_eq!(status, StatusCode::CREATED, "{user}");
        session_of(&headers).unwrap()
    }

    /// `GET /auth/me` with `cookie`.
    async fn me(&self, cookie: &str) -> (StatusCode, Value) {
        let (status, _, body) = self
            .send(Method::GET, "/api/v1/auth/me", None, &[("cookie", cookie)])
            .await;
        (status, body)
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn accounts_register_sign_in_and_out(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let body =
        json!({ "username": "Doctor", "password": PASSWORD, "email": " doc@rhodes.example " });
    let (status, headers, user) = h
        .send(Method::POST, "/api/v1/auth/register", Some(body), &[])
        .await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    assert_eq!(user["username"], "Doctor");
    assert_eq!(user["email"], "doc@rhodes.example");
    let fields: Vec<&str> = user
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(fields, ["created_at", "email", "id", "username"]);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let set = set_cookie_line(&headers).unwrap();
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=2592000",
    ] {
        assert!(set.contains(attribute), "{set}");
    }
    let first = session_of(&headers).unwrap();

    let (status, me) = h.me(&first).await;
    assert_eq!((status, &me), (StatusCode::OK, &user));
    let (status, err) = h.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_of(&err), "not signed in");
    let (status, _) = h.me("__Host-session=0123").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Usernames are unique regardless of case.
    let again = json!({ "username": "doctor", "password": "another password" });
    let (status, err) = h.post("/api/v1/auth/register", again).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(error_of(&err).contains("taken"), "{err}");

    // A wrong password, an unknown user and an impossible username all get
    // the same answer, as does a password too long to be one (but not too
    // long to send).
    let huge = "x".repeat(8 * 1024);
    for (username, password) in [
        ("Doctor", "wrong horse"),
        ("nobody", PASSWORD),
        ("no one", PASSWORD),
        ("Doctor", huge.as_str()),
    ] {
        let attempt = json!({ "username": username, "password": password });
        let (status, err) = h.post("/api/v1/auth/login", attempt).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{username}");
        assert_eq!(error_of(&err), "wrong username or password");
    }

    // Signing in, in any case, gets a fresh token and ends the session the
    // request carried.
    let attempt = json!({ "username": "DOCTOR", "password": PASSWORD });
    let (status, headers, me) = h
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(attempt),
            &[("cookie", &first)],
        )
        .await;
    assert_eq!((status, &me), (StatusCode::OK, &user));
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let second = session_of(&headers).unwrap();
    assert_ne!(second, first);
    assert_eq!(h.me(&first).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(h.me(&second).await.0, StatusCode::OK);

    // Signing out ends the session and clears the cookie.
    let (status, headers, _) = h
        .send(
            Method::POST,
            "/api/v1/auth/logout",
            None,
            &[("cookie", &second)],
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(set_cookie_line(&headers).unwrap().contains("Max-Age=0"));
    assert_eq!(h.me(&second).await.0, StatusCode::UNAUTHORIZED);
    let (status, _, _) = h.send(Method::POST, "/api/v1/auth/logout", None, &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn registration_checks_its_fields(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    for (body, expected) in [
        (
            json!({ "username": "ab", "password": PASSWORD }),
            "a username is",
        ),
        (
            json!({ "username": "dot.ted", "password": PASSWORD }),
            "a username is",
        ),
        (
            json!({ "username": "kaltsit", "password": "short" }),
            "8 to 128",
        ),
        (
            json!({ "username": "kaltsit", "password": "x".repeat(129) }),
            "8 to 128",
        ),
        (
            json!({ "username": "kaltsit", "password": "x".repeat(8 * 1024) }),
            "8 to 128",
        ),
        (
            json!({ "username": "kaltsit1", "password": "KALTSIT1" }),
            "not be the username",
        ),
        (
            json!({ "username": "kaltsit", "password": PASSWORD, "email": "kaltsit" }),
            "email",
        ),
        (json!({ "username": "kaltsit" }), "password"),
    ] {
        let (status, err) = h.post("/api/v1/auth/register", body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(error_of(&err).contains(expected), "{body}: {err}");
    }
    // Nothing was stored.
    let attempt = json!({ "username": "kaltsit", "password": PASSWORD });
    let (status, _) = h.post("/api/v1/auth/login", attempt).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // No email is fine, and passwords are compared normalised: "ﬁ" is "fi".
    let body = json!({ "username": "kaltsit", "password": "ﬁrewall" });
    let (status, user) = h.post("/api/v1/auth/register", body).await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    assert_eq!(user["email"], Value::Null);
    let attempt = json!({ "username": "kaltsit", "password": "firewall" });
    let (status, _) = h.post("/api/v1/auth/login", attempt).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn expired_sessions_are_refused(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let cookie = h.register("closure").await;
    let (_, user) = h.me(&cookie).await;
    let token = Token::generate().unwrap();
    let now = chrono::Utc::now();
    h.state
        .store
        .create_session(&Session {
            digest: token.digest(),
            user_id: user["id"].as_str().unwrap().parse().unwrap(),
            created_at: now - chrono::TimeDelta::days(31),
            expires_at: now - chrono::TimeDelta::days(1),
        })
        .await
        .unwrap();
    let set = token.set_cookie();
    let stale = set.to_str().unwrap().split(';').next().unwrap();
    assert_eq!(h.me(stale).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn a_planted_look_alike_cookie_is_not_the_session(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let victim = h.register("amiya").await;
    let attacker = h.register("w-the-sarkaz").await;
    // A cookie whose name only percent-decodes to the session cookie's,
    // which another site could have planted, is ignored whichever way round
    // the browser sends them.
    let planted = attacker.replacen("__Host-session", "%5F%5FHost-session", 1);
    for cookies in [
        format!("{victim}; {planted}"),
        format!("{planted}; {victim}"),
    ] {
        let (status, me) = h.me(&cookies).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["username"], "amiya", "{cookies}");
    }
    assert_eq!(h.me(&planted).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn old_password_hashes_are_upgraded_at_sign_in(pool: PgPool) {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use argon2::{Algorithm, Argon2, Params, Version};

    let h = Harness::new(pool, Limits::default());
    let weak = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(8 * 1024, 1, 1, None).unwrap(),
    );
    let salt = SaltString::encode_b64(&[9; 16]).unwrap();
    let old = weak
        .hash_password(PASSWORD.as_bytes(), &salt)
        .unwrap()
        .to_string();
    let user = User::new("warfarin".into(), None);
    assert!(h.state.store.create_user(&user, &old).await.unwrap());

    let attempt = json!({ "username": "warfarin", "password": PASSWORD });
    let (status, _) = h.post("/api/v1/auth/login", attempt.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let (_, stored) = h
        .state
        .store
        .credentials("warfarin")
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{stored}"
    );
    let (status, _) = h.post("/api/v1/auth/login", attempt).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn personal_routes_need_an_account(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let id = "00000000-0000-0000-0000-000000000000";
    let personal = [
        (Method::GET, "/api/v1/auth/me".to_owned()),
        (Method::POST, "/api/v1/evaluate".to_owned()),
        (Method::POST, "/api/v1/simulate".to_owned()),
        (Method::GET, "/api/v1/rosters".to_owned()),
        (Method::POST, "/api/v1/rosters".to_owned()),
        (Method::POST, "/api/v1/rosters/preview".to_owned()),
        (Method::GET, format!("/api/v1/rosters/{id}")),
        (Method::PUT, format!("/api/v1/rosters/{id}")),
        (Method::DELETE, format!("/api/v1/rosters/{id}")),
        (Method::GET, "/api/v1/import/krooster/someone".to_owned()),
        (Method::GET, "/api/v1/bases".to_owned()),
        (Method::POST, "/api/v1/bases".to_owned()),
        (Method::GET, format!("/api/v1/bases/{id}")),
        (Method::PUT, format!("/api/v1/bases/{id}")),
        (Method::DELETE, format!("/api/v1/bases/{id}")),
        (Method::GET, "/api/v1/solves".to_owned()),
        (Method::POST, "/api/v1/solves".to_owned()),
        (Method::GET, format!("/api/v1/solves/{id}")),
        (Method::DELETE, format!("/api/v1/solves/{id}")),
        (Method::POST, format!("/api/v1/solves/{id}/stop")),
        (
            Method::GET,
            format!("/api/v1/solves/{id}/candidates/0/simulation"),
        ),
    ];
    for (method, uri) in personal {
        // A body that would be wrong is still refused for want of an
        // account first.
        let body = (method == Method::POST || method == Method::PUT).then(|| json!({}));
        let (status, headers, err) = h.send(method.clone(), &uri, body, &[]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}: {err}");
        assert_eq!(error_of(&err), "not signed in");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{method} {uri}");
    }
    for uri in [
        "/healthz",
        "/api/v1/gamedata/version",
        "/api/v1/gamedata/layout",
    ] {
        let (status, headers, _) = h.send(Method::GET, uri, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(headers.get(header::CACHE_CONTROL).is_none(), "{uri}");
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn accounts_see_only_their_own_documents(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let sim = example("243-base.json");
    let roster = h
        .create(
            "rosters",
            json!({ "name": "mine", "roster": sim["roster"] }),
        )
        .await;
    let base = h
        .create("bases", json!({ "name": "mine", "base": sim["base"] }))
        .await;
    let solve = h
        .create("solves", json!({ "request": quick_solve() }))
        .await;
    h.wait_for(&solve, "done", is("done")).await;

    let other = h.register("kaltsit").await;
    let as_other = |method: Method, uri: String, body: Option<Value>| {
        let other = other.clone();
        let h = &h;
        async move {
            let (status, _, body) = h.send(method, &uri, body, &[("cookie", &other)]).await;
            (status, body)
        }
    };

    // Someone else's documents are not there: not listed, not readable,
    // not changeable, not deletable.
    for (collection, id, body) in [
        ("rosters", &roster, json!({ "roster": sim["roster"] })),
        ("bases", &base, json!({ "base": sim["base"] })),
    ] {
        let (status, listed) = as_other(Method::GET, format!("/api/v1/{collection}"), None).await;
        assert_eq!((status, listed), (StatusCode::OK, json!([])));
        let uri = format!("/api/v1/{collection}/{id}");
        let (status, _) = as_other(Method::GET, uri.clone(), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        let (status, _) = as_other(Method::PUT, uri.clone(), Some(body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        let (status, _) = as_other(Method::DELETE, uri.clone(), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    let (status, listed) = as_other(Method::GET, "/api/v1/solves".into(), None).await;
    assert_eq!((status, listed), (StatusCode::OK, json!([])));
    for (method, uri) in [
        (Method::GET, format!("/api/v1/solves/{solve}")),
        (Method::POST, format!("/api/v1/solves/{solve}/stop")),
        (
            Method::GET,
            format!("/api/v1/solves/{solve}/candidates/0/simulation"),
        ),
        (Method::DELETE, format!("/api/v1/solves/{solve}")),
    ] {
        let body = (method == Method::POST).then(|| json!({}));
        let (status, _) = as_other(method.clone(), uri.clone(), body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
    }

    // Nor can a request name them.
    let mut by_ref = sim.clone();
    by_ref.as_object_mut().unwrap().remove("base");
    by_ref["base_id"] = json!(base);
    let (status, err) = as_other(
        Method::POST,
        "/api/v1/simulate".into(),
        Some(by_ref.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).starts_with("no stored base"), "{err}");
    let (status, _) = h.post("/api/v1/simulate", by_ref).await;
    assert_eq!(status, StatusCode::OK);

    // The owner still has everything.
    for uri in [
        format!("/api/v1/rosters/{roster}"),
        format!("/api/v1/bases/{base}"),
        format!("/api/v1/solves/{solve}"),
    ] {
        let (status, _) = h.get(&uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
    for collection in ["rosters", "bases", "solves"] {
        let (_, listed) = h.get(&format!("/api/v1/{collection}")).await;
        assert_eq!(listed.as_array().unwrap().len(), 1, "{collection}");
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn other_accounts_cannot_stop_or_delete_a_live_solve(pool: PgPool) {
    let one_at_a_time = Limits {
        max_running_solves: 1,
        ..Limits::default()
    };
    let h = Harness::signed_in(pool, one_at_a_time).await;
    let running = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    h.wait_for(&running, "running", is("running")).await;
    let pending = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;

    let other = h.register("kaltsit").await;
    for id in [&running, &pending] {
        let stop = format!("/api/v1/solves/{id}/stop");
        let (status, _, _) = h
            .send(Method::POST, &stop, Some(json!({})), &[("cookie", &other)])
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{stop}");
        let uri = format!("/api/v1/solves/{id}");
        let (status, _, _) = h
            .send(Method::DELETE, &uri, None, &[("cookie", &other)])
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }

    // Untouched: still running with no stop asked for, and still queued.
    let (_, job) = h.get(&format!("/api/v1/solves/{running}")).await;
    assert_eq!(job["status"], "running");
    assert!(job.get("stop_requested").is_none(), "{job}");
    let (_, job) = h.get(&format!("/api/v1/solves/{pending}")).await;
    assert_eq!(job["status"], "pending");

    // Their owner still can: the queued one starts once the first ends.
    let (status, _) = h
        .post(&format!("/api/v1/solves/{running}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    h.wait_for(&pending, "running", is("running")).await;
    let (status, _) = h
        .post(&format!("/api/v1/solves/{pending}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    h.wait_idle().await;
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn other_sites_cannot_use_or_start_a_session(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let cookie = h.register("mostima").await;
    let refused = "refused a request from another site";
    let attempt = json!({ "username": "mostima", "password": PASSWORD });

    for site in ["cross-site", "same-site"] {
        // Ending the session, or changing anything while carrying it…
        let (status, _, err) = h
            .send(
                Method::POST,
                "/api/v1/auth/logout",
                None,
                &[("cookie", &cookie), ("sec-fetch-site", site)],
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{site}");
        assert_eq!(error_of(&err), refused);
        let base = json!({ "base": example("243-base.json")["base"] });
        let (status, _, _) = h
            .send(
                Method::POST,
                "/api/v1/bases",
                Some(base),
                &[("cookie", &cookie), ("sec-fetch-site", site)],
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{site}");
        // …or signing in, even without a session.
        let (status, _, _) = h
            .send(
                Method::POST,
                "/api/v1/auth/login",
                Some(attempt.clone()),
                &[("sec-fetch-site", site)],
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{site}");
    }
    assert_eq!(h.me(&cookie).await.0, StatusCode::OK);

    // Reading is fine from anywhere, and so is a request with no session.
    let (status, _, _) = h
        .send(
            Method::GET,
            "/api/v1/auth/me",
            None,
            &[("cookie", &cookie), ("sec-fetch-site", "cross-site")],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, err) = h
        .send(
            Method::POST,
            "/api/v1/simulate",
            Some(example("243-base.json")),
            &[("sec-fetch-site", "cross-site")],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_of(&err), "not signed in");

    // The site's own pages, and requests the user typed, may.
    for site in ["same-origin", "none"] {
        let (status, _, _) = h
            .send(
                Method::POST,
                "/api/v1/auth/login",
                Some(attempt.clone()),
                &[("sec-fetch-site", site)],
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{site}");
    }
}

// ---- roster import ---------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn rosters_import_from_other_tools(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let krooster = export("krooster-v3.json");

    // A preview reads the export without storing it.
    let (status, preview) = h
        .post(
            "/api/v1/rosters/preview",
            json!({ "source": "krooster", "roster": krooster }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["source"], "krooster");
    assert_eq!(preview["roster"].as_object().unwrap().len(), 10);
    assert_eq!(
        preview["roster"]["char_124_kroos"]["promotion"],
        json!({ "phase": "PHASE_1", "level": 55 })
    );
    let report = &preview["import"];
    assert_eq!(report["format"], "krooster_v3");
    assert_eq!(report["imported"], 10);
    assert_eq!(report["not_owned"], 1);
    assert_eq!(report["warnings"][0]["kind"], "unknown_operator");
    let (_, list) = h.get("/api/v1/rosters").await;
    assert_eq!(list, json!([]));

    // Storing it answers with the same report and keeps it.
    let (status, saved) = h
        .post(
            "/api/v1/rosters",
            json!({ "name": "from krooster", "source": "krooster", "roster": krooster }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["import"], *report);
    let id = saved["id"].as_str().unwrap().to_owned();
    let (_, doc) = h.get(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(doc["source"], "krooster");
    assert_eq!(doc["roster"], preview["roster"]);
    assert_eq!(doc["import"], *report);

    let (status, planner) = h
        .post(
            "/api/v1/rosters",
            json!({ "source": "ak-planner", "roster": export("ak-planner.json") }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{planner}");
    assert_eq!(planner["import"]["format"], "ak_planner_export");
    assert_eq!(planner["import"]["imported"], 4);

    // The wrong tool is named in the error.
    let (status, err) = h
        .post(
            "/api/v1/rosters/preview",
            json!({ "source": "ak-planner", "roster": krooster }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&err).contains("Krooster roster"), "{err}");

    // A canonical roster from a stored document is a manual one again.
    let (status, _) = h
        .put(
            &format!("/api/v1/rosters/{id}"),
            json!({ "roster": doc["roster"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, doc) = h.get(&format!("/api/v1/rosters/{id}")).await;
    assert_eq!(doc["source"], "manual");
    assert!(doc.get("import").is_none());

    // The imported roster drives a solve by reference.
    let base = example("243-base.json")["base"].clone();
    let request = json!({
        "base": base,
        "roster_id": id,
        "solver": { "strategy": "annealing", "iterations": 60, "restarts": 1, "top_k": 2 },
    });
    let solve = h.create("solves", json!({ "request": request })).await;
    let job = h.wait_for(&solve, "done", is("done")).await;
    assert_eq!(job["refs"]["roster_id"], id.as_str());
    assert_eq!(job["result"]["space"]["pool"], 10);
    let best = &job["result"]["candidates"][0];
    assert!(best["score"].as_f64().unwrap() > 0.0, "{}", best["score"]);
}

/// A stand-in Krooster on a local port.
struct FakeKrooster {
    base: String,
    /// The paths asked for, in order.
    asked: Arc<Mutex<Vec<String>>>,
    /// The most requests it was answering at once.
    most_at_once: Arc<AtomicUsize>,
}

impl FakeKrooster {
    /// How many times `path` was asked for.
    fn asks(&self, path: &str) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == path)
            .count()
    }
}

/// A Krooster that knows `example-doctor` and, answering after 300 ms,
/// every `slow-…`; fails for `down`; answers `garbled` with a web page;
/// and knows nobody else. Each connection is answered on its own thread.
fn fake_krooster() -> FakeKrooster {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let most_at_once = Arc::new(AtomicUsize::new(0));
    let (log, most) = (asked.clone(), most_at_once.clone());
    let profile: Arc<str> = export("krooster-profile.json").to_string().into();
    let in_hand = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (log, most, profile, in_hand) =
                (log.clone(), most.clone(), profile.clone(), in_hand.clone());
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut buf = [0; 1024];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&head);
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                log.lock().unwrap().push(path.clone());
                most.fetch_max(in_hand.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                let (status, kind, body) = match path.as_str() {
                    "/api/u/example-doctor" => ("200 OK", "application/json", &*profile),
                    slow if slow.starts_with("/api/u/slow-") => {
                        std::thread::sleep(Duration::from_millis(300));
                        ("200 OK", "application/json", &*profile)
                    }
                    "/api/u/garbled" => ("200 OK", "text/html", "<!DOCTYPE html><p>Krooster</p>"),
                    "/api/u/down" => (
                        "500 Internal Server Error",
                        "text/plain",
                        "Internal server error.",
                    ),
                    _ => ("404 Not Found", "text/plain", "User not found"),
                };
                in_hand.fetch_sub(1, Ordering::SeqCst);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    FakeKrooster {
        base,
        asked,
        most_at_once,
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn rosters_fetched_from_krooster(pool: PgPool) {
    let krooster = fake_krooster();
    let h = Harness::signed_in(pool, Limits::default())
        .await
        .with_krooster(&krooster.base);
    let asked = &krooster.asked;

    // The profile's roster, without the account details and supports.
    let (status, fetched) = h.get("/api/v1/import/krooster/Example-Doctor").await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    let profile = export("krooster-profile.json");
    assert_eq!(
        fetched,
        json!({ "data": { "roster": profile["data"]["roster"] } })
    );
    // Krooster looks usernames up in lower case.
    assert_eq!(*asked.lock().unwrap(), ["/api/u/example-doctor"]);

    // It stores like any Krooster roster, read as a profile.
    let (status, saved) = h
        .post(
            "/api/v1/rosters",
            json!({ "name": "example-doctor", "source": "krooster", "roster": fetched }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["import"]["format"], "krooster_profile");
    assert_eq!(saved["import"]["imported"], 10);

    // What went wrong is told apart.
    let (status, err) = h.get("/api/v1/import/krooster/nobody").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&err), "no Krooster user named \"nobody\"");
    let (status, err) = h.get("/api/v1/import/krooster/down").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        error_of(&err),
        "Krooster answered 500 when asked for \"down\""
    );
    let (status, err) = h.get("/api/v1/import/krooster/garbled").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        error_of(&err),
        "Krooster's answer for \"garbled\" is not a Krooster profile"
    );

    // A name Krooster could not have is refused without asking it, so it
    // cannot steer the request anywhere else.
    let before = asked.lock().unwrap().len();
    let long = "x".repeat(33);
    for bad in ["no.dots", "a%2Fb", "..%2F..%2Fapi", "%20", long.as_str()] {
        let (status, err) = h.get(&format!("/api/v1/import/krooster/{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {err}");
        assert!(error_of(&err).contains("not a Krooster username"), "{err}");
    }
    assert_eq!(asked.lock().unwrap().len(), before);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn an_unreachable_krooster_is_a_bad_gateway(pool: PgPool) {
    // A port nothing listens on any more.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let h = Harness::signed_in(pool, Limits::default())
        .await
        .with_krooster(&format!("http://127.0.0.1:{port}"));
    let (status, err) = h.get("/api/v1/import/krooster/example-doctor").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        error_of(&err).starts_with("could not reach Krooster"),
        "{err}"
    );
}

// ---- frontend support ------------------------------------------------------

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn layout_and_constants_for_drawing_a_base(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    let (status, layout) = h.get("/api/v1/gamedata/layout").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(layout["id"], "v0");
    let slots = layout["slots"].as_array().unwrap();
    assert_eq!(slots.len(), data().layout.slots.len());
    let output = slots.iter().filter(|s| s["category"] == "OUTPUT").count();
    assert_eq!(output, 9);
    let cc = slots.iter().find(|s| s["category"] == "SPECIAL").unwrap();
    assert_eq!(cc["size"], json!({ "rows": 4, "cols": 8 }));

    let (status, constants) = h.get("/api/v1/gamedata/constants").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(constants["comfort_limit"], data().constants.comfort_limit);

    let (_, op) = h.get("/api/v1/gamedata/operators").await;
    let exusiai = op
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "char_103_angel")
        .unwrap();
    assert_eq!(exusiai["max_levels"], json!([50, 80, 90]));
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn every_finalist_can_be_simulated(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    let mut req = quick_solve();
    req["solver"]["top_k"] = json!(3);
    let id = h.create("solves", json!({ "request": req })).await;

    // Not before it has finished.
    let (status, _) = h
        .get(&format!("/api/v1/solves/{id}/candidates/0/simulation"))
        .await;
    assert!(
        status == StatusCode::CONFLICT || status == StatusCode::OK,
        "{status}"
    );

    let job = h.wait_for(&id, "done", is("done")).await;
    let result = &job["result"];
    let n = result["candidates"].as_array().unwrap().len();
    assert!(n >= 2, "{n} finalists");

    let (status, best) = h
        .get(&format!("/api/v1/solves/{id}/candidates/0/simulation"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(best, result["best_simulation"]);

    let (status, other) = h
        .get(&format!("/api/v1/solves/{id}/candidates/1/simulation"))
        .await;
    assert_eq!(status, StatusCode::OK, "{other}");
    assert_eq!(
        other["horizon_hours"],
        result["best_simulation"]["horizon_hours"]
    );
    let lmd = other["totals"]["lmd"].as_f64().unwrap();
    let expected = result["candidates"][1]["breakdown"]["lmd"]
        .as_f64()
        .unwrap();
    assert!((lmd - expected).abs() < 1e-6, "{lmd} vs {expected}");

    let (status, err) = h
        .get(&format!("/api/v1/solves/{id}/candidates/{n}/simulation"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(error_of(&err).contains("finalists"), "{err}");
}

// ---- limits ----------------------------------------------------------------

/// A budget of `burst` requests refilling at one a minute: in a test, a
/// fixed number of requests.
fn budget(burst: u32) -> Rate {
    Rate {
        per_minute: NonZeroU32::new(1).unwrap(),
        burst: NonZeroU32::new(burst).unwrap(),
    }
}

impl Harness {
    /// A request from `peer`, as if it had come in through a socket, with
    /// the harness's session cookie.
    async fn send_from(
        &self,
        peer: &str,
        method: Method,
        uri: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Value) {
        let text = body.map(|b| b.to_string());
        let peer = Some(peer.parse().unwrap());
        let resp = self
            .raw_as(
                self.cookie.as_deref(),
                peer,
                method,
                uri,
                text.as_deref(),
                headers,
            )
            .await;
        let headers = resp.headers().clone();
        let (status, body) = read(resp).await;
        (status, headers, body)
    }
}

/// Asserts a `429` with a JSON error and a `Retry-After` of at least a
/// second, and returns the error.
fn too_many(status: StatusCode, headers: &HeaderMap, body: &Value) -> String {
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    let retry: u64 = headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry >= 1, "{retry}");
    error_of(body).to_owned()
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn clients_are_limited_by_address(pool: PgPool) {
    let h = Harness::new(
        pool,
        Limits {
            requests: budget(3),
            ..Limits::default()
        },
    );
    let get = |peer: &'static str| {
        let h = &h;
        async move {
            h.send_from(peer, Method::GET, "/api/v1/gamedata/version", None, &[])
                .await
        }
    };
    for _ in 0..3 {
        assert_eq!(get("198.51.100.1:4000").await.0, StatusCode::OK);
    }
    let (status, headers, body) = get("198.51.100.1:4001").await;
    let message = too_many(status, &headers, &body);
    assert!(message.contains("too many requests"), "{message}");
    // Every route draws on the same budget, unknown ones included.
    let (status, _, _) = h
        .send_from("198.51.100.1:4002", Method::GET, "/nowhere", None, &[])
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // Another address has its own budget; a request with no address (made
    // in-process) is not counted.
    assert_eq!(get("198.51.100.2:4000").await.0, StatusCode::OK);
    for _ in 0..5 {
        assert_eq!(h.get("/api/v1/gamedata/version").await.0, StatusCode::OK);
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn x_forwarded_for_counts_only_from_a_trusted_proxy(pool: PgPool) {
    let limits = |trusted: &[&str]| Limits {
        requests: budget(1),
        trusted_proxies: trusted
            .iter()
            .map(|t| ak_api::client::parse_trusted(t).unwrap())
            .collect(),
        ..Limits::default()
    };
    let version = "/api/v1/gamedata/version";
    let via = |client: &'static str| [("x-forwarded-for", client)];

    // Behind a trusted proxy, each client named by the proxy is counted
    // apart, whatever it claims in front of that.
    let h = Harness::new(pool.clone(), limits(&["127.0.0.1"]));
    for (client, expected) in [
        ("203.0.113.1", StatusCode::OK),
        ("203.0.113.2", StatusCode::OK),
        ("203.0.113.1", StatusCode::TOO_MANY_REQUESTS),
        ("192.0.2.50, 203.0.113.1", StatusCode::TOO_MANY_REQUESTS),
    ] {
        let (status, _, body) = h
            .send_from("127.0.0.1:9000", Method::GET, version, None, &via(client))
            .await;
        assert_eq!(status, expected, "{client}: {body}");
    }

    // Without one, the header is ignored: the address connecting is the
    // client.
    let h = Harness::new(pool, limits(&[]));
    let (status, _, _) = h
        .send_from(
            "127.0.0.1:9000",
            Method::GET,
            version,
            None,
            &via("203.0.113.1"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = h
        .send_from(
            "127.0.0.1:9000",
            Method::GET,
            version,
            None,
            &via("203.0.113.2"),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn signing_up_and_in_have_a_smaller_budget(pool: PgPool) {
    let h = Harness::new(
        pool,
        Limits {
            sign_ins: budget(2),
            ..Limits::default()
        },
    );
    let peer = "198.51.100.7:5000";
    let sign_up = json!({ "username": "doctor", "password": PASSWORD });
    let (status, _, body) = h
        .send_from(
            peer,
            Method::POST,
            "/api/v1/auth/register",
            Some(sign_up),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let wrong = json!({ "username": "doctor", "password": "wrong horse" });
    let (status, _, _) = h
        .send_from(
            peer,
            Method::POST,
            "/api/v1/auth/login",
            Some(wrong.clone()),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, headers, body) = h
        .send_from(peer, Method::POST, "/api/v1/auth/login", Some(wrong), &[])
        .await;
    let message = too_many(status, &headers, &body);
    assert!(message.contains("sign-ups and sign-ins"), "{message}");
    // The rest of the API, signing out and asking who is signed in included,
    // is not counted against it.
    for uri in ["/api/v1/gamedata/version", "/api/v1/auth/me"] {
        let (status, _, _) = h.send_from(peer, Method::GET, uri, None, &[]).await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "{uri}");
    }
    let (status, _, _) = h
        .send_from(peer, Method::POST, "/api/v1/auth/logout", None, &[])
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn failed_sign_ins_make_the_next_attempt_wait(pool: PgPool) {
    let h = Harness::new(pool, Limits::default());
    h.register("doctor").await;
    let attempt = |username: &str, password: &str| {
        let body = json!({ "username": username, "password": password });
        let h = &h;
        async move {
            h.send(Method::POST, "/api/v1/auth/login", Some(body), &[])
                .await
        }
    };
    // An account, and a username no account has, are treated alike.
    for username in ["Doctor", "nobody"] {
        for _ in 0..=FREE_FAILURES {
            let (status, _, body) = attempt(username, "wrong horse").await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{username}: {body}");
        }
        // Now even the right password must wait.
        let (status, headers, body) = attempt(&username.to_uppercase(), PASSWORD).await;
        let message = too_many(status, &headers, &body);
        assert!(message.contains("failed sign-ins"), "{message}");
    }
    // Another username is not held up.
    let (status, _, _) = attempt("amiya", "wrong horse").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // After the wait, the right password signs in and clears the count:
    // the whole allowance is back.
    tokio::time::sleep(FIRST_WAIT + Duration::from_millis(100)).await;
    let (status, _, body) = attempt("doctor", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for _ in 0..=FREE_FAILURES {
        let (status, _, body) = attempt("doctor", "wrong horse").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn an_account_may_have_only_so_many_solves(pool: PgPool) {
    let h = Harness::signed_in(
        pool,
        Limits {
            max_solves_per_account: NonZeroUsize::MIN,
            max_running_solves: 1,
            ..Limits::default()
        },
    )
    .await;
    let first = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    let (status, err) = h
        .post("/api/v1/solves", json!({ "request": quick_solve() }))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{err}");
    assert!(
        error_of(&err).contains("1 solves queued or running"),
        "{err}"
    );
    // Nothing was stored for the refused one.
    let (_, list) = h.get("/api/v1/solves").await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");

    // Another account has its own allowance, even while the first queues.
    let cookie = h.register("amiya").await;
    let (status, _, body) = h
        .send(
            Method::POST,
            "/api/v1/solves",
            Some(json!({ "request": quick_solve() })),
            &[("cookie", &cookie)],
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    // Once the first has finished, there is room again.
    let (status, _) = h
        .post(&format!("/api/v1/solves/{first}/stop"), json!({}))
        .await;
    assert!(status.is_success(), "{status}");
    h.wait_for(&first, "the stopped solve to finish", is("done"))
        .await;
    h.wait_idle().await;
    let next = h
        .create("solves", json!({ "request": quick_solve() }))
        .await;
    h.wait_for(&next, "the next solve", is("done")).await;
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn an_account_may_have_only_so_many_simulations(pool: PgPool) {
    let h = Harness::signed_in(
        pool,
        Limits {
            max_simulations_per_account: NonZeroUsize::MIN,
            ..Limits::default()
        },
    )
    .await;
    let id = h
        .create("solves", json!({ "request": quick_solve() }))
        .await;
    h.wait_for(&id, "the solve", is("done")).await;

    // While the account's one slot is taken, every kind of simulation waits
    // its turn, but the best finalist's, already simulated, is served.
    let held = h.state.simulations.take(h.user_id().await).unwrap();
    for (uri, body) in [
        ("/api/v1/simulate", Some(example("243-base.json"))),
        ("/api/v1/evaluate", Some(example("243-base.json"))),
        (
            &*format!("/api/v1/solves/{id}/candidates/1/simulation"),
            None,
        ),
    ] {
        let method = if body.is_some() {
            Method::POST
        } else {
            Method::GET
        };
        let (status, _, err) = h.send(method, uri, body, &[]).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{uri}: {err}");
        assert!(error_of(&err).contains("1 simulations running"), "{err}");
    }
    let (status, _) = h
        .get(&format!("/api/v1/solves/{id}/candidates/0/simulation"))
        .await;
    assert_eq!(status, StatusCode::OK);

    // Another account is not held up; freeing the slot lets the first go on.
    let cookie = h.register("amiya").await;
    let (status, _, _) = h
        .send(
            Method::POST,
            "/api/v1/evaluate",
            Some(example("243-base.json")),
            &[("cookie", &cookie)],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    drop(held);
    let (status, _) = h.post("/api/v1/simulate", example("243-base.json")).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn request_bodies_are_capped(pool: PgPool) {
    let h = Harness::signed_in(pool, Limits::default()).await;
    // Signing up and in: room for the longest password, not much more.
    let padding = "x".repeat(MAX_SIGN_IN_BODY);
    let body = json!({ "username": "doctor", "password": padding });
    for uri in ["/api/v1/auth/login", "/api/v1/auth/register"] {
        let (status, err) = h.post(uri, body.clone()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{uri}");
        assert!(err.get("error").is_some(), "{err}");
    }
    // Everything else: rosters and requests far larger than any tool
    // exports, but no larger than that.
    let mut roster = json!({ "roster": {}, "name": "x".repeat(MAX_BODY) });
    let (status, err) = h.post("/api/v1/rosters", roster.clone()).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{err}");
    assert!(err.get("error").is_some(), "{err}");
    roster["name"] = json!("x".repeat(MAX_BODY / 2));
    let (status, err) = h.post("/api/v1/rosters", roster).await;
    assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE, "{err}");
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn stopped_or_deleted_solves_leave_the_queue_at_once(pool: PgPool) {
    // While the one slot is taken, solves wait in line. Stopping or deleting
    // one lets its runner go at once, and gives back its place.
    let h = Harness::signed_in(
        pool,
        Limits {
            max_running_solves: 1,
            max_solves_per_account: NonZeroUsize::new(3).unwrap(),
            ..Limits::default()
        },
    )
    .await;
    let running = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    h.wait_for(&running, "the first to run", is("running"))
        .await;
    let stopped = h
        .create("solves", json!({ "request": quick_solve() }))
        .await;
    let deleted = h
        .create("solves", json!({ "request": quick_solve() }))
        .await;
    let waiting = |n: usize| {
        let deadline = Instant::now() + Duration::from_secs(10);
        let jobs = h.state.jobs.clone();
        async move {
            while jobs.waiting() != n {
                assert!(
                    Instant::now() < deadline,
                    "{} waiting, not {n}",
                    jobs.waiting()
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };
    waiting(2).await;
    let (status, _) = h
        .post(&format!("/api/v1/solves/{stopped}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    waiting(1).await;
    let (status, _) = h.delete(&format!("/api/v1/solves/{deleted}")).await;
    assert!(status.is_success(), "{status}");
    waiting(0).await;
    // Both places are free again.
    for _ in 0..2 {
        h.create("solves", json!({ "request": quick_solve() }))
            .await;
    }
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn a_deleted_solve_keeps_its_place_until_it_has_wound_down(pool: PgPool) {
    // Deleting a running solve stops its search, but it holds its slot until
    // the search has wound down, and its place among the account's solves
    // is held as long.
    let h = Harness::signed_in(
        pool,
        Limits {
            max_solves_per_account: NonZeroUsize::MIN,
            ..Limits::default()
        },
    )
    .await;
    let running = h
        .create("solves", json!({ "request": endless_solve() }))
        .await;
    h.wait_for(&running, "it to run", is("running")).await;
    let (status, _) = h.delete(&format!("/api/v1/solves/{running}")).await;
    assert!(status.is_success(), "{status}");
    let deadline = Instant::now() + Duration::from_secs(120);
    while h.state.jobs.running() > 0 {
        assert_eq!(
            h.state.jobs.live_count(),
            1,
            "a slot is held with no place taken"
        );
        assert!(Instant::now() < deadline, "the search never wound down");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    h.wait_idle().await;
    h.create("solves", json!({ "request": quick_solve() }))
        .await;
}

/// GETs every one of `uris` at once, signed in as the harness; answers the
/// statuses.
async fn all_at_once(h: &Harness, uris: impl IntoIterator<Item = String>) -> Vec<StatusCode> {
    let mut requests = tokio::task::JoinSet::new();
    for uri in uris {
        let (app, cookie) = (h.app.clone(), h.cookie.clone().unwrap());
        requests.spawn(async move {
            let req = Request::get(uri)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap();
            app.oneshot(req).await.unwrap().status()
        });
    }
    requests.join_all().await
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn krooster_answers_are_kept_for_a_while(pool: PgPool) {
    let krooster = fake_krooster();
    let h = Harness::signed_in(pool, Limits::default())
        .await
        .with_krooster(&krooster.base);
    let get = |name: &str| {
        let h = &h;
        let uri = format!("/api/v1/import/krooster/{name}");
        async move { h.send(Method::GET, &uri, None, &[]).await }
    };
    let age =
        |headers: &HeaderMap| -> u64 { headers[header::AGE].to_str().unwrap().parse().unwrap() };

    // A roster, asked for in any case, is fetched once, then served from
    // memory with its age.
    let (status, headers, first) = get("example-doctor").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert!(age(&headers) < 5);
    let (status, headers, again) = get("Example-Doctor").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again, first);
    assert!(age(&headers) < 5);
    assert_eq!(krooster.asks("/api/u/example-doctor"), 1);

    // So is the news that there is no such user, each time named as asked.
    for name in ["nobody", "NoBody"] {
        let (status, _, err) = get(name).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error_of(&err), format!("no Krooster user named {name:?}"));
    }
    assert_eq!(krooster.asks("/api/u/nobody"), 1);

    // Krooster's failures, and answers that are not a profile, are not.
    for _ in 0..2 {
        assert_eq!(get("down").await.0, StatusCode::BAD_GATEWAY);
        assert_eq!(get("garbled").await.0, StatusCode::BAD_GATEWAY);
    }
    assert_eq!(krooster.asks("/api/u/down"), 2);
    assert_eq!(krooster.asks("/api/u/garbled"), 2);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn krooster_is_asked_once_per_name_and_a_few_names_at_a_time(pool: PgPool) {
    let krooster = fake_krooster();
    let h = Harness::signed_in(pool, Limits::default())
        .await
        .with_krooster(&krooster.base);

    // Eight requests for one name at once share a single fetch.
    let uri = "/api/v1/import/krooster/slow-one".to_owned();
    let statuses = all_at_once(&h, std::iter::repeat_n(uri, 8)).await;
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "{statuses:?}"
    );
    assert_eq!(krooster.asks("/api/u/slow-one"), 1);

    // Eight names at once: Krooster is asked about a few at a time.
    let uris = (0..8).map(|i| format!("/api/v1/import/krooster/slow-{i}"));
    let statuses = all_at_once(&h, uris).await;
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "{statuses:?}"
    );
    assert_eq!(krooster.most_at_once.load(Ordering::SeqCst), MAX_FETCHES);
}

#[sqlx::test(migrator = "ak_store::MIGRATOR")]
async fn a_krooster_fetch_carries_on_when_its_request_is_dropped(pool: PgPool) {
    let krooster = fake_krooster();
    let h = Harness::signed_in(pool, Limits::default())
        .await
        .with_krooster(&krooster.base);
    let names: Vec<String> = (0..MAX_FETCHES).map(|i| format!("slow-gone-{i}")).collect();
    let uri = |name: &str| format!("/api/v1/import/krooster/{name}");

    // Requests that start as many fetches as may run at once, then go away
    // while Krooster is still answering.
    let mut gone = tokio::task::JoinSet::new();
    for name in &names {
        let (app, cookie) = (h.app.clone(), h.cookie.clone().unwrap());
        let req = Request::get(uri(name))
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        gone.spawn(async move { app.oneshot(req).await });
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while names
        .iter()
        .any(|n| krooster.asks(&format!("/api/u/{n}")) == 0)
    {
        assert!(Instant::now() < deadline, "Krooster was never asked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(gone);

    // Their fetches carried on: asked again, those names get the answers
    // already on their way, and other names wait for a place.
    let others = (0..MAX_FETCHES).map(|i| uri(&format!("slow-next-{i}")));
    let statuses = all_at_once(&h, names.iter().map(|n| uri(n)).chain(others)).await;
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "{statuses:?}"
    );
    for name in &names {
        assert_eq!(krooster.asks(&format!("/api/u/{name}")), 1, "{name}");
    }
    assert!(krooster.most_at_once.load(Ordering::SeqCst) <= MAX_FETCHES);
}
