//! Background solves.
//!
//! A solve is a stored document ([`SolveJob`]) plus, while it is queued or
//! running, an in-memory entry in [`Jobs`] that holds its owner, its live
//! progress and its stop flag. Every write to the document is its owner's,
//! and only its owner can stop or delete it. The document moves through these statuses:
//!
//! ```text
//! pending ──▶ running ──▶ done      (result; `result.stopped` if stopped early)
//!    │           └──────▶ failed    (error)
//!    └──▶ cancelled                 (stopped before it started)
//! ```
//!
//! At most [`Limits::max_running_solves`](crate::Limits::max_running_solves)
//! run at once; the rest wait as `pending`. Stopping a running solve ends
//! its search early and keeps what it found. Deleting a solve stops it and
//! removes the document; nothing is written back afterwards.
//!
//! When the server shuts down, running searches are told to stop and their
//! documents are left as they are, so [`resume`] queues them again on the
//! next start, from the beginning. A job interrupted [`MAX_ATTEMPTS`] times
//! is marked failed instead. A job that finishes anyway (it was re-scoring,
//! or had been stopped by request) is recorded before the server exits:
//! see [`Jobs::drain`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use uuid::Uuid;

use ak_solver::{Observer, Progress, SolveRequest, SolveResult, StopReason};
use ak_store::{Collection, Store, StoreError};

use crate::AppState;
use crate::error::ApiError;
use crate::refs::Refs;

/// Times a solve may be cut off by the server stopping before it is marked
/// failed rather than queued again.
pub const MAX_ATTEMPTS: u32 = 3;

/// Where a solve is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting for a free slot.
    Pending,
    /// Searching or re-scoring.
    Running,
    /// Finished with a result.
    Done,
    /// Finished with an error.
    Failed,
    /// Stopped before it started.
    Cancelled,
}

impl JobStatus {
    /// True for `done`, `failed` and `cancelled`.
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled
        )
    }

    /// The name used in responses.
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

/// The body of a solve document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveJob {
    /// Where it is in its life.
    pub status: JobStatus,
    /// The request as it runs: references resolved, server limits applied.
    pub request: SolveRequest,
    /// The stored base and roster the request named, if any.
    #[serde(default, skip_serializing_if = "Refs::is_empty")]
    pub refs: Refs,
    /// How many times it has started running.
    #[serde(default)]
    pub attempts: u32,
    /// The answer, once `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SolveResult>,
    /// Why it failed, once `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When it last started running (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// When it finished (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

impl SolveJob {
    /// A new pending job.
    pub fn new(request: SolveRequest, refs: Refs) -> Self {
        SolveJob {
            status: JobStatus::Pending,
            request,
            refs,
            attempts: 0,
            result: None,
            error: None,
            started_at: None,
            finished_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Queued,
    Running,
    Cancelled,
}

#[derive(Debug)]
struct LiveState {
    stage: Stage,
    /// The document was deleted; never write it again.
    deleted: bool,
}

/// A queued or running job. The `state` lock is held around every write to
/// the job's document, so a stop or a delete can never be overwritten by a
/// stale write from the runner; it is an async lock because it is held
/// across those writes. `state` changes only once its write has succeeded,
/// so a failed stop or delete can simply be tried again.
#[derive(Debug)]
struct Live {
    /// The account the solve belongs to.
    owner: Uuid,
    state: tokio::sync::Mutex<LiveState>,
    stop: AtomicBool,
    progress: Mutex<Option<Progress>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What [`Jobs::stop`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// The job had not started; it is now `cancelled`.
    Cancelled,
    /// The job is running; its search will end at the next checkpoint.
    Stopping,
    /// The job is not queued or running on this server.
    NotLive,
}

/// What a poll sees of a queued or running job.
#[derive(Debug, Clone, Default)]
pub struct LiveView {
    /// The latest progress report, once the search has started.
    pub progress: Option<Progress>,
    /// Whether a stop was asked for.
    pub stop_requested: bool,
}

/// The queue and the live jobs.
#[derive(Debug)]
pub struct Jobs {
    live: Mutex<HashMap<Uuid, Arc<Live>>>,
    permits: Arc<Semaphore>,
    max_running: usize,
    shutting_down: AtomicBool,
}

impl Jobs {
    /// A queue running at most `max_running` solves at once.
    pub fn new(max_running: usize) -> Self {
        // At least one, and few enough for `drain` to count them in a u32.
        let max_running = max_running.clamp(1, u32::MAX as usize);
        Jobs {
            live: Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(max_running)),
            max_running,
            shutting_down: AtomicBool::new(false),
        }
    }

    fn get(&self, id: Uuid) -> Option<Arc<Live>> {
        lock(&self.live).get(&id).cloned()
    }

    fn forget(&self, id: Uuid) {
        lock(&self.live).remove(&id);
    }

    /// Live progress of a queued or running job.
    pub fn watch(&self, id: Uuid) -> Option<LiveView> {
        let live = self.get(id)?;
        let progress = lock(&live.progress).clone();
        Some(LiveView {
            progress,
            stop_requested: live.stop.load(Ordering::SeqCst),
        })
    }

    /// Stops one of `owner`'s jobs: a queued one is cancelled at once, a
    /// running one ends its search at the next checkpoint and reports what
    /// it found. Someone else's job is not live, as far as `owner` can tell.
    /// Run it with [`to_completion`](crate::stored::to_completion), so a
    /// dropped request cannot leave it half done.
    pub async fn stop(&self, store: &Store, owner: Uuid, id: Uuid) -> Result<Stopped, StoreError> {
        let Some(live) = self.get(id).filter(|live| live.owner == owner) else {
            return Ok(Stopped::NotLive);
        };
        let mut st = live.state.lock().await;
        match st.stage {
            Stage::Queued => {
                if !st.deleted {
                    update(store, owner, id, |job| {
                        job.status = JobStatus::Cancelled;
                        job.finished_at = Some(ak_store::now());
                    })
                    .await?;
                }
                st.stage = Stage::Cancelled;
                drop(st);
                self.forget(id);
                Ok(Stopped::Cancelled)
            }
            Stage::Running => {
                live.stop.store(true, Ordering::SeqCst);
                Ok(Stopped::Stopping)
            }
            Stage::Cancelled => Ok(Stopped::Cancelled),
        }
    }

    /// Deletes one of `owner`'s job documents, stopping the job first if it
    /// is queued or running. Returns whether there was a document. Run it
    /// with [`to_completion`](crate::stored::to_completion), so a dropped
    /// request cannot leave it half done.
    pub async fn delete(&self, store: &Store, owner: Uuid, id: Uuid) -> Result<bool, StoreError> {
        let Some(live) = self.get(id).filter(|live| live.owner == owner) else {
            return store.delete(owner, Collection::Solves, id).await;
        };
        let mut st = live.state.lock().await;
        let removed = store.delete(owner, Collection::Solves, id).await?;
        st.deleted = true;
        if st.stage == Stage::Queued {
            st.stage = Stage::Cancelled;
        }
        live.stop.store(true, Ordering::SeqCst);
        drop(st);
        self.forget(id);
        Ok(removed)
    }

    /// Tells every running search to stop and keeps queued jobs from
    /// starting. Their documents stay `pending` or `running`, so [`resume`]
    /// picks them up on the next start.
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let n = lock(&self.live).len();
        if n > 0 {
            tracing::info!(
                jobs = n,
                "stopping solves; they will run again on the next start"
            );
        }
    }

    /// After [`shutdown`](Self::shutdown), waits until every runner has
    /// written its last change to its job, so that a solve that finished
    /// (or was stopped by request) while the server was stopping is recorded
    /// before the store closes. Searches end at their next checkpoint; a
    /// re-scoring phase in progress runs to its end.
    pub async fn drain(&self) {
        let all = u32::try_from(self.max_running).unwrap_or(u32::MAX);
        // Each runner holds its permit until after its last write, and
        // queued jobs that get one now return it at once.
        let _ = self.permits.acquire_many(all).await;
    }

    /// True once [`shutdown`](Self::shutdown) has been called.
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    /// Jobs queued or running, not counting deleted ones.
    pub fn live_count(&self) -> usize {
        lock(&self.live).len()
    }

    /// Solves holding a slot right now, including a deleted one whose
    /// search has not wound down yet.
    pub fn running(&self) -> usize {
        self.max_running - self.permits.available_permits()
    }
}

/// Reads one of `owner`'s solve documents, changes its job, and writes it
/// back. A missing document is left missing.
async fn update(
    store: &Store,
    owner: Uuid,
    id: Uuid,
    f: impl FnOnce(&mut SolveJob),
) -> Result<(), StoreError> {
    let Some(mut doc) = store.get(owner, Collection::Solves, id).await? else {
        return Ok(());
    };
    let Ok(mut job) = serde_json::from_value::<SolveJob>(doc.body.clone()) else {
        tracing::error!(%id, "stored solve is unreadable; leaving it alone");
        return Ok(());
    };
    f(&mut job);
    match serde_json::to_value(&job) {
        Ok(body) => doc.body = body,
        Err(e) => {
            tracing::error!(%id, "cannot serialise solve: {e}");
            return Ok(());
        }
    }
    doc.touch();
    store.update(Collection::Solves, &doc).await?;
    Ok(())
}

/// Changes the runner's job document, unless it was deleted.
async fn record(store: &Store, id: Uuid, live: &Live, f: impl FnOnce(&mut SolveJob)) {
    let st = live.state.lock().await;
    if st.deleted {
        return;
    }
    let result = update(store, live.owner, id, f).await;
    drop(st);
    if let Err(e) = result {
        tracing::error!(%id, "cannot save solve: {e}");
    }
}

/// Feeds a running solve's progress to its live entry and passes on stop
/// requests and shutdown.
struct LiveObserver<'a> {
    live: &'a Live,
    jobs: &'a Jobs,
}

impl Observer for LiveObserver<'_> {
    fn progress(&mut self, p: &Progress) {
        *lock(&self.live.progress) = Some(p.clone());
    }

    fn should_stop(&self) -> bool {
        self.live.stop.load(Ordering::Relaxed) || self.jobs.is_shutting_down()
    }
}

/// Queues one of `owner`'s stored jobs; it runs when a slot is free.
pub fn enqueue(state: &AppState, owner: Uuid, id: Uuid, job: SolveJob) {
    let live = Arc::new(Live {
        owner,
        state: tokio::sync::Mutex::new(LiveState {
            stage: Stage::Queued,
            deleted: false,
        }),
        stop: AtomicBool::new(false),
        progress: Mutex::new(None),
    });
    lock(&state.jobs.live).insert(id, live.clone());
    let state = state.clone();
    tokio::spawn(async move {
        let Ok(permit) = state.jobs.permits.clone().acquire_owned().await else {
            return;
        };
        {
            let mut st = live.state.lock().await;
            if st.stage == Stage::Cancelled || st.deleted || state.jobs.is_shutting_down() {
                drop(st);
                state.jobs.forget(id);
                return;
            }
            st.stage = Stage::Running;
        }
        run(&state, id, &live, job).await;
        state.jobs.forget(id);
        drop(permit);
    });
}

/// Runs one job to the end, recording each status change. The search runs
/// on a blocking thread; the writes around it do not.
async fn run(state: &AppState, id: Uuid, live: &Arc<Live>, mut job: SolveJob) {
    job.status = JobStatus::Running;
    job.started_at = Some(ak_store::now());
    job.attempts += 1;
    let started = job.clone();
    record(&state.store, id, live, |stored| *stored = started).await;
    tracing::info!(%id, attempt = job.attempts, "solve started");

    let (data, jobs, observed) = (state.data.clone(), state.jobs.clone(), live.clone());
    let searched = tokio::task::spawn_blocking(move || {
        let mut observer = LiveObserver {
            live: &observed,
            jobs: &jobs,
        };
        let outcome = ak_solver::solve_with(&data, &job.request, &mut observer);
        (job, outcome)
    })
    .await;
    let (mut job, outcome) = match searched {
        Ok(searched) => searched,
        Err(e) => {
            tracing::error!(%id, "solve crashed: {e}");
            let message = format!("the solver crashed: {e}");
            record(&state.store, id, live, |job| {
                job.status = JobStatus::Failed;
                job.error = Some(message);
                job.finished_at = Some(ak_store::now());
            })
            .await;
            return;
        }
    };
    match outcome {
        Ok(result) => {
            let by_shutdown =
                result.stopped == Some(StopReason::Requested) && !live.stop.load(Ordering::SeqCst);
            if by_shutdown {
                // Leave the document `running`; the next start queues it
                // again.
                tracing::info!(%id, "solve interrupted by shutdown");
                return;
            }
            tracing::info!(
                %id,
                elapsed_ms = result.elapsed_ms,
                evaluations = result.evaluations,
                stopped = ?result.stopped,
                "solve done"
            );
            job.status = JobStatus::Done;
            job.result = Some(result);
        }
        Err(e) => {
            tracing::warn!(%id, "solve failed: {e}");
            job.status = JobStatus::Failed;
            job.error = Some(e.to_string());
        }
    }
    job.finished_at = Some(ak_store::now());
    record(&state.store, id, live, |stored| *stored = job).await;
}

/// Queues again every solve the store holds as `pending` or `running`: the
/// server stopped before they finished. Returns how many were queued.
pub async fn resume(state: &AppState) -> Result<usize, ApiError> {
    let store = &state.store;
    let mut queued = Vec::new();
    // Everyone's, oldest first, so they run in the order they were
    // submitted.
    for doc in store.all_documents(Collection::Solves).await? {
        let (owner, id) = (doc.owner, doc.id);
        let Ok(job) = serde_json::from_value::<SolveJob>(doc.body) else {
            tracing::warn!(%id, "stored solve is unreadable; skipping it");
            continue;
        };
        if job.status.is_finished() {
            continue;
        }
        if job.attempts >= MAX_ATTEMPTS {
            let attempts = job.attempts;
            update(store, owner, id, |job| {
                job.status = JobStatus::Failed;
                job.error = Some(format!(
                    "the server stopped during each of {attempts} attempts; giving up"
                ));
                job.finished_at = Some(ak_store::now());
            })
            .await?;
            continue;
        }
        let mut again = job;
        again.status = JobStatus::Pending;
        again.started_at = None;
        let copy = again.clone();
        update(store, owner, id, |job| *job = copy).await?;
        queued.push((owner, id, again));
    }
    let n = queued.len();
    for (owner, id, job) in queued {
        enqueue(state, owner, id, job);
    }
    Ok(n)
}
