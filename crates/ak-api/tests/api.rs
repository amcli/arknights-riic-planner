//! Layer 7 tests: the router driven in-process against the pinned snapshot
//! and a temporary store.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use ak_api::jobs::{JobStatus, SolveJob};
use ak_api::refs::Refs;
use ak_api::{AppState, Limits};
use ak_data::{Strictness, load_default};
use ak_domain::{BaseConfig, GameData, Roster};
use ak_eval::SimRequest;
use ak_solver::SolveRequest;
use ak_store::{Collection, Document, FileStore, Store};

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

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ak-api-test-{}", uuid::Uuid::new_v4()))
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
    dir: PathBuf,
    /// Remove the store directory when dropped.
    cleanup: bool,
}

impl Harness {
    fn new(limits: Limits) -> Self {
        Self::at(temp_dir(), limits)
    }

    fn at(dir: PathBuf, limits: Limits) -> Self {
        let store = FileStore::open(&dir).unwrap();
        let state = AppState::new(data(), 0, Arc::new(store), limits);
        Harness {
            app: ak_api::router(state.clone()),
            state,
            dir,
            cleanup: true,
        }
    }

    async fn raw(&self, method: Method, uri: &str, body: Option<&str>) -> Response<Body> {
        let mut req = Request::builder().method(method).uri(uri);
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
        if self.cleanup {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
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

#[tokio::test]
async fn game_data_endpoints() {
    let h = Harness::new(Limits::default());
    let data = data();

    let (status, body) = h.get("/healthz").await;
    assert_eq!((status, body), (StatusCode::OK, json!("ok")));

    let (status, v) = h.get("/api/v1/gamedata/version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["sha"], data.version.sha.as_str());
    assert_eq!(v["stats"]["operators"], data.operators.len());
    assert_eq!(v["stats"]["mechanics_unparsed"], 0);
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

#[tokio::test]
async fn every_error_is_json() {
    let h = Harness::new(Limits::default());

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
}

// ---- simulation ------------------------------------------------------------

#[tokio::test]
async fn simulate_matches_the_library() {
    let h = Harness::new(Limits::default());
    let req = example("243-base.json");

    let (status, result) = h.post("/api/v1/simulate", req.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let parsed: SimRequest = serde_json::from_value(req.clone()).unwrap();
    let direct = ak_eval::simulate(&data(), &parsed).unwrap();
    assert_eq!(result, wire(&direct));
    // The reference numbers for this base.
    let lmd = result["totals"]["lmd"].as_f64().unwrap();
    assert!((20_000.0..20_200.0).contains(&lmd), "{lmd}");

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

#[tokio::test]
async fn long_runs_are_refused() {
    let h = Harness::new(Limits {
        max_ticks: 100,
        ..Limits::default()
    });
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

#[tokio::test]
async fn rosters_round_trip() {
    let h = Harness::new(Limits::default());
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
            json!({ "source": "krooster", "roster": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(error_of(&err).contains("krooster"), "{err}");
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

#[tokio::test]
async fn bases_are_validated() {
    let h = Harness::new(Limits::default());
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

#[tokio::test]
async fn requests_can_name_stored_documents() {
    let h = Harness::new(Limits::default());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_solve_runs_in_the_background() {
    let h = Harness::new(Limits::default());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_solve_is_refused_before_it_is_queued() {
    let h = Harness::new(Limits::default());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_solve_reports_progress_and_can_be_stopped() {
    let h = Harness::new(Limits::default());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn solves_queue_and_can_be_cancelled_or_deleted() {
    let h = Harness::new(Limits {
        max_running_solves: 1,
        ..Limits::default()
    });
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_caps_search_time() {
    let h = Harness::new(Limits {
        max_solve_ms: 300,
        ..Limits::default()
    });
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

/// Writes a solve document straight into a store.
fn stored_job(store: &FileStore, request: &Value, status: JobStatus, attempts: u32) -> String {
    let request: SolveRequest = serde_json::from_value(request.clone()).unwrap();
    let mut job = SolveJob::new(request, Refs::default());
    job.status = status;
    job.attempts = attempts;
    let doc = Document::new(None, serde_json::to_value(&job).unwrap());
    store.put(Collection::Solves, &doc).unwrap();
    doc.id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unfinished_solves_resume_after_a_restart() {
    let dir = temp_dir();
    let (interrupted, waiting, doomed, cancelled) = {
        let store = FileStore::open(&dir).unwrap();
        let quick = quick_solve();
        (
            stored_job(&store, &quick, JobStatus::Running, 1),
            stored_job(&store, &quick, JobStatus::Pending, 0),
            stored_job(
                &store,
                &quick,
                JobStatus::Running,
                ak_api::jobs::MAX_ATTEMPTS,
            ),
            stored_job(&store, &quick, JobStatus::Cancelled, 0),
        )
    };
    let h = Harness::at(dir, Limits::default());
    assert_eq!(ak_api::jobs::resume(&h.state).await.unwrap(), 2);

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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_leaves_running_solves_for_the_next_start() {
    let dir = temp_dir();
    let id = {
        let mut h = Harness::at(dir.clone(), Limits::default());
        h.cleanup = false;
        let id = h
            .create("solves", json!({ "request": endless_solve() }))
            .await;
        h.wait_for(&id, "running", is("running")).await;
        h.state.jobs.shutdown();
        h.wait_idle().await;
        let (_, job) = h.get(&format!("/api/v1/solves/{id}")).await;
        assert_eq!(job["status"], "running");
        assert!(job.get("result").is_none());
        id
    };
    let h = Harness::at(dir, Limits::default());
    assert_eq!(ak_api::jobs::resume(&h.state).await.unwrap(), 1);
    let job = h
        .wait_for(&id, "running again", |job| job["attempts"] == 2)
        .await;
    assert_eq!(job["status"], "running");
}
