//! Watching a solve and stopping it early.
//!
//! A caller that wants progress reports or a way to stop a long search
//! passes an [`Observer`] to [`solve_with`](crate::solve_with). The search
//! consults it every few dozen evaluations. Stopping ends the search, not
//! the solve: the finalists found so far are still re-scored and returned,
//! and [`SolveResult::stopped`](crate::SolveResult::stopped) says why the
//! search ended early. This is the same path the time budget takes.
//!
//! Nothing here does I/O or needs a runtime; an observer is a plain trait
//! object, so an HTTP server can back it with an atomic flag and a mutex.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::result::Strategy;

/// Why a search ended before it had covered its whole plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// `SolverConfig::time_budget_ms` ran out.
    TimeBudget,
    /// The [`Observer`] asked to stop.
    Requested,
}

/// Which part of a solve is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Exploring assignments with the inner evaluator.
    Searching,
    /// Re-scoring the finalists with the full simulator.
    Rescoring,
}

/// A snapshot of a running solve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Which part is running.
    pub phase: Phase,
    /// The search that is running (never `auto`).
    pub strategy: Strategy,
    /// Units of work finished: assignments enumerated (exhaustive),
    /// annealing steps across all restarts, or finalists re-scored.
    pub done: f64,
    /// Units of work planned for this phase. A search that stops early
    /// finishes below it.
    pub total: f64,
    /// Inner-evaluator calls so far.
    pub evaluations: u64,
    /// Inner score of the starting assignment.
    pub initial_score: f64,
    /// Best inner score seen so far.
    pub best_score: f64,
    /// Wall-clock time since the solve started.
    pub elapsed_ms: u64,
}

/// Receives progress from a running solve and may stop its search.
pub trait Observer {
    /// Called when a phase starts, every few dozen evaluations, and when a
    /// phase ends.
    fn progress(&mut self, _progress: &Progress) {}

    /// Polled as often as [`progress`](Self::progress) is called during
    /// the search. Returning true ends the search early; the finalists are
    /// still re-scored and returned.
    fn should_stop(&self) -> bool {
        false
    }
}

/// An observer that ignores everything and never stops.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unobserved;

impl Observer for Unobserved {}

/// Evaluations between two consultations of the observer and the clock.
pub(crate) const CHECK_EVERY: u64 = 32;

/// The search's side of an [`Observer`]: the clock, the budget and the
/// running totals that go into [`Progress`].
pub struct Tracker<'a> {
    observer: &'a mut dyn Observer,
    started: Instant,
    deadline: Option<Instant>,
    strategy: Strategy,
    phase: Phase,
    total: f64,
    initial_score: f64,
    best_score: f64,
    stopped: Option<StopReason>,
}

impl<'a> Tracker<'a> {
    /// Starts tracking a search of `total` units. `budget` is measured from
    /// `started`.
    pub fn new(
        observer: &'a mut dyn Observer,
        started: Instant,
        budget: Option<Duration>,
        strategy: Strategy,
        total: f64,
    ) -> Self {
        Tracker {
            observer,
            started,
            deadline: budget.map(|b| started + b),
            strategy,
            phase: Phase::Searching,
            total,
            initial_score: 0.0,
            best_score: 0.0,
            stopped: None,
        }
    }

    /// Records the starting assignment's score, which is also the best so
    /// far.
    pub fn set_initial(&mut self, score: f64) {
        self.initial_score = score;
        self.best_score = score;
    }

    /// Records a score the search has seen.
    pub fn saw(&mut self, score: f64) {
        if score > self.best_score {
            self.best_score = score;
        }
    }

    /// Reports progress and decides whether the search must end now. Once
    /// it has said stop, it keeps saying stop.
    pub fn checkpoint(&mut self, done: f64, evaluations: u64) -> bool {
        if self.stopped.is_none() {
            if self.observer.should_stop() {
                self.stopped = Some(StopReason::Requested);
            } else if self.deadline.is_some_and(|d| Instant::now() >= d) {
                self.stopped = Some(StopReason::TimeBudget);
            }
        }
        self.report(done, evaluations);
        self.stopped.is_some()
    }

    /// Why the search ended early, if it did.
    pub fn stopped(&self) -> Option<StopReason> {
        self.stopped
    }

    /// Switches to re-scoring `finalists` candidates.
    pub fn start_rescoring(&mut self, finalists: usize, evaluations: u64) {
        self.phase = Phase::Rescoring;
        self.total = finalists as f64;
        self.report(0.0, evaluations);
    }

    /// Reports progress without consulting the observer's stop request.
    pub fn report(&mut self, done: f64, evaluations: u64) {
        let progress = Progress {
            phase: self.phase,
            strategy: self.strategy,
            done,
            total: self.total,
            evaluations,
            initial_score: self.initial_score,
            best_score: self.best_score,
            elapsed_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        self.observer.progress(&progress);
    }
}
