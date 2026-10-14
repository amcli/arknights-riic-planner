//! Solve endpoints: submit, list, poll, stop, delete. The queue and the
//! runner live in [`crate::jobs`].

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use ak_domain::{Assignment, GameData};
use ak_solver::{Progress, SolveRequest, Space};
use ak_store::{Collection, Document, DocumentMeta};

use crate::auth::CurrentUser;
use crate::error::{ApiError, Body};
use crate::jobs::{self, JobStatus, SolveJob, Stopped};
use crate::rosters::check_roster;
use crate::simulation::check_ticks;
use crate::stored::{self, Stored, blocking, to_completion};
use crate::{AppState, Limits, refs};

/// `POST` body: an optional name and the request. The request may name a
/// stored base or roster as `base_id` or `roster_id`.
#[derive(Deserialize)]
pub struct SolveBody {
    #[serde(default)]
    name: Option<String>,
    request: Value,
}

/// A listing entry.
#[derive(Serialize)]
struct SolveSummary {
    #[serde(flatten)]
    meta: DocumentMeta,
    status: JobStatus,
}

/// A solve as a poll sees it: the document, plus live progress while it
/// runs.
#[derive(Serialize)]
struct SolveView {
    #[serde(flatten)]
    meta: DocumentMeta,
    #[serde(flatten)]
    job: SolveJob,
    /// The latest progress report, while running.
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<Progress>,
    /// Set once a stop was asked for and the search has not ended yet.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stop_requested: bool,
}

/// Applies the server's limits: the tick limit, and the search-time cap as
/// the time budget of any request without a shorter one.
fn apply_limits(limits: &Limits, req: &mut SolveRequest) -> Result<(), ApiError> {
    check_ticks(&req.config, limits)?;
    let cap = limits.max_solve_ms;
    req.solver.time_budget_ms = Some(req.solver.time_budget_ms.map_or(cap, |ms| ms.min(cap)));
    Ok(())
}

/// Rejects a request the solver would refuse, before a job is created.
fn check_solve(data: &GameData, req: &SolveRequest) -> Result<(), ApiError> {
    req.solver.validate().map_err(ApiError::bad_request)?;
    req.config.validate().map_err(ApiError::bad_request)?;
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
    .map_err(ApiError::bad_request)?;
    Ok(())
}

/// `POST /api/v1/solves`: validates, stores the job as `pending`, answers
/// `202`, and queues it. Storing and queueing finish together even if the
/// client goes away, so no stored job is left unqueued.
pub async fn create(
    State(s): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<SolveBody>,
) -> Result<Response, ApiError> {
    let (mut request, refs) = refs::resolve::<SolveRequest>(&s, user.id, body.request).await?;
    apply_limits(&s.limits, &mut request)?;
    check_solve(&s.data, &request)?;
    let job = SolveJob::new(request, refs);
    let value = serde_json::to_value(&job).map_err(ApiError::internal)?;
    let doc = Document::new(user.id, body.name, value);
    let meta = doc.meta();
    let state = s.clone();
    to_completion(async move {
        state.store.insert(Collection::Solves, &doc).await?;
        jobs::enqueue(&state, doc.owner, doc.id, job);
        Ok::<_, ApiError>(())
    })
    .await??;
    let summary = SolveSummary {
        meta,
        status: JobStatus::Pending,
    };
    Ok((StatusCode::ACCEPTED, Json(summary)).into_response())
}

/// `GET /api/v1/solves`: the account's jobs with their status, newest first.
pub async fn list(
    State(s): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Response, ApiError> {
    let summaries: Vec<SolveSummary> = s
        .store
        .documents(user.id, Collection::Solves)
        .await?
        .into_iter()
        .map(|doc| {
            let status = doc
                .body
                .get("status")
                .and_then(|status| serde_json::from_value(status.clone()).ok())
                .unwrap_or(JobStatus::Failed);
            SolveSummary {
                meta: doc.meta(),
                status,
            }
        })
        .collect();
    Ok(Json(summaries).into_response())
}

async fn current(s: &AppState, owner: Uuid, id: String) -> Result<SolveView, ApiError> {
    let doc = stored::fetch(s, owner, Collection::Solves, id.clone()).await?;
    let Stored { meta, body: job } = stored::view::<SolveJob>(doc)?;
    let live = if job.status.is_finished() {
        None
    } else {
        s.jobs.watch(meta.id)
    };
    let live = live.unwrap_or_default();
    Ok(SolveView {
        meta,
        job,
        progress: live.progress,
        stop_requested: live.stop_requested,
    })
}

/// `GET /api/v1/solves/{id}`: status, request, progress while running,
/// and the result or error once finished.
pub async fn get(
    State(s): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    Ok(Json(current(&s, user.id, id).await?).into_response())
}

/// `POST /api/v1/solves/{id}/stop`: a pending solve is cancelled (`200`);
/// a running one ends its search at the next checkpoint and finishes
/// `done` with the best found so far (`202`); a finished one is a `409`.
pub async fn stop(
    State(s): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let key = stored::parse_id(&id)?;
    let (jobs, store, owner) = (s.jobs.clone(), s.store.clone(), user.id);
    let outcome = to_completion(async move { jobs.stop(&store, owner, key).await }).await??;
    let status = match outcome {
        Stopped::Cancelled => StatusCode::OK,
        Stopped::Stopping => StatusCode::ACCEPTED,
        Stopped::NotLive => {
            let view = current(&s, user.id, id.clone()).await?;
            return Err(ApiError::conflict(format!(
                "solve {id} is {}; only a pending or running solve can be stopped",
                view.job.status.as_str()
            )));
        }
    };
    Ok((status, Json(current(&s, user.id, id).await?)).into_response())
}

/// `GET /api/v1/solves/{id}/candidates/{n}/simulation`: the full
/// simulation of finalist `n` (0 is the best), run the way the solve scored
/// it. The best one's comes from the result; the others are simulated on
/// request.
pub async fn candidate_simulation(
    State(s): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((id, n)): Path<(String, usize)>,
) -> Result<Response, ApiError> {
    let doc = stored::fetch(&s, user.id, Collection::Solves, id.clone()).await?;
    let Stored { body: job, .. } = stored::view::<SolveJob>(doc)?;
    let Some(result) = job.result else {
        return Err(ApiError::conflict(format!(
            "solve {id} is {}; it has no finalists yet",
            job.status.as_str()
        )));
    };
    let Some(candidate) = result.candidates.get(n) else {
        return Err(ApiError::not_found(format!(
            "solve {id} has {} finalists; there is no number {n}",
            result.candidates.len()
        )));
    };
    if n == 0
        && let Some(best) = result.best_simulation
    {
        return Ok(Json(best).into_response());
    }
    let (data, request, assignment) = (s.data.clone(), job.request, candidate.assignment.clone());
    let sim = blocking(move || ak_solver::simulate_candidate(&data, &request, &assignment))
        .await?
        .map_err(ApiError::bad_request)?;
    Ok(Json(sim).into_response())
}

/// `DELETE /api/v1/solves/{id}`: stops the solve if it is queued or
/// running, and removes it.
pub async fn delete(
    State(s): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let key = stored::parse_id(&id)?;
    let (jobs, store, owner) = (s.jobs.clone(), s.store.clone(), user.id);
    let removed = to_completion(async move { jobs.delete(&store, owner, key).await }).await??;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(stored::missing(Collection::Solves, &id))
    }
}
