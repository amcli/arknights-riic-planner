//! Request and result types for a solve.

use std::fmt;

use ak_domain::{Assignment, BaseConfig, DataVersion, OperatorId, Roster, Slot};
use ak_eval::{Rotation, SimConfig, SimError, SimResult};
use serde::{Deserialize, Serialize};

use crate::objective::{Breakdown, Objective};

/// Which search to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Exhaustive when the space is at most `exhaustive_limit` assignments,
    /// else annealing.
    #[default]
    Auto,
    /// Enumerate every assignment of the pool to the variable slots.
    Exhaustive,
    /// Simulated annealing with restarts.
    Annealing,
}

/// What scores candidates inside the search loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InnerEvaluator {
    /// The instantaneous evaluator extrapolated over the horizon: fast, and
    /// good enough to rank. The finalists are re-scored with the simulator.
    #[default]
    SteadyState,
    /// The full simulator on every candidate: slow, exact with respect to
    /// the model.
    Simulation,
}

/// Search settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SolverConfig {
    /// Seed; the same seed and request give the same result.
    pub seed: u64,
    /// Which search to run.
    pub strategy: Strategy,
    /// What scores candidates inside the search.
    pub inner: InnerEvaluator,
    /// Annealing steps per restart.
    pub iterations: u32,
    /// Independent annealing runs; their finalists are pooled.
    pub restarts: u32,
    /// How many distinct finalists to return.
    pub top_k: usize,
    /// Starting temperature in objective units. `None` picks it from the
    /// typical size of a single move's effect.
    pub initial_temperature: Option<f64>,
    /// Final temperature as a fraction of the initial one (geometric
    /// cooling).
    pub final_temperature_ratio: f64,
    /// Largest space `Strategy::Auto` enumerates exhaustively.
    pub exhaustive_limit: f64,
    /// Stop annealing after this long; finalists so far are still re-scored.
    pub time_budget_ms: Option<u64>,
    /// Re-score the finalists with the full simulator when the inner
    /// evaluator is the steady state. The best is always simulated once for
    /// the report.
    pub rescore_with_simulation: bool,
}

impl Default for SolverConfig {
    fn default() -> Self {
        SolverConfig {
            seed: 1,
            strategy: Strategy::Auto,
            inner: InnerEvaluator::SteadyState,
            iterations: 3000,
            restarts: 4,
            top_k: 5,
            initial_temperature: None,
            final_temperature_ratio: 1e-3,
            exhaustive_limit: 20_000.0,
            time_budget_ms: None,
            rescore_with_simulation: true,
        }
    }
}

impl SolverConfig {
    /// Rejects nonsensical values.
    pub fn validate(&self) -> Result<(), SolveError> {
        if self.iterations == 0 {
            return Err(SolveError::Config("iterations must be at least 1".into()));
        }
        if self.restarts == 0 {
            return Err(SolveError::Config("restarts must be at least 1".into()));
        }
        if self.top_k == 0 {
            return Err(SolveError::Config("top_k must be at least 1".into()));
        }
        if !(self.final_temperature_ratio > 0.0 && self.final_temperature_ratio < 1.0) {
            return Err(SolveError::Config(
                "final_temperature_ratio must be between 0 and 1".into(),
            ));
        }
        if let Some(t) = self.initial_temperature
            && (t.is_nan() || t <= 0.0)
        {
            return Err(SolveError::Config(
                "initial_temperature must be positive".into(),
            ));
        }
        if self.exhaustive_limit.is_nan() || self.exhaustive_limit < 0.0 {
            return Err(SolveError::Config(
                "exhaustive_limit cannot be negative".into(),
            ));
        }
        Ok(())
    }
}

/// A complete, serialisable description of one solve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveRequest {
    /// The rooms.
    pub base: BaseConfig,
    /// Owned operators; by default every one of them is a candidate.
    pub roster: Roster,
    /// Restrict the candidates to these operators.
    #[serde(default)]
    pub pool: Option<Vec<OperatorId>>,
    /// Starting assignment. Empty by default. Operators it places in
    /// variable slots join the pool.
    #[serde(default)]
    pub initial: Option<Assignment>,
    /// Slots the solver must leave exactly as `initial` has them. Training
    /// Room trainee slots are always locked.
    #[serde(default)]
    pub locked: Vec<Slot>,
    /// Simulation settings used for scoring.
    #[serde(default)]
    pub config: SimConfig,
    /// Shift policy used when simulating. With `mood_threshold`, every pool
    /// operator the candidate leaves unassigned joins the bench.
    #[serde(default)]
    pub rotation: Rotation,
    /// What "better" means.
    #[serde(default)]
    pub objective: Objective,
    /// Search settings.
    #[serde(default)]
    pub solver: SolverConfig,
}

/// One finalist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Who is where.
    pub assignment: Assignment,
    /// Score from the inner evaluator (the steady-state proxy or the
    /// simulator, per `SolverConfig::inner`).
    pub inner_score: f64,
    /// Final score: from the simulator when the candidate was simulated,
    /// else the inner score.
    pub score: f64,
    /// What the score is made of.
    pub breakdown: Breakdown,
    /// True when `score` and `breakdown` come from the full simulator.
    pub simulated: bool,
}

/// The size of the search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceSummary {
    /// Slots the solver may change.
    pub variable_slots: usize,
    /// Slots held fixed.
    pub locked_slots: usize,
    /// Candidate operators.
    pub pool: usize,
    /// Distinct assignments (symmetric slots within a room counted once).
    pub estimated_size: f64,
}

/// The result of one solve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveResult {
    /// Which data produced these numbers.
    pub data: DataVersion,
    /// The search that ran.
    pub strategy: Strategy,
    /// What scored candidates inside the search.
    pub inner: InnerEvaluator,
    /// The size of the search.
    pub space: SpaceSummary,
    /// The starting assignment, scored the same way as the finalists.
    pub initial: Candidate,
    /// Finalists, best first.
    pub candidates: Vec<Candidate>,
    /// The full simulation of the best finalist.
    pub best_simulation: Option<SimResult>,
    /// Inner-evaluator calls.
    pub evaluations: u64,
    /// Full simulations run.
    pub simulations: u64,
    /// Wall-clock time.
    pub elapsed_ms: u64,
}

/// Why a solve could not run.
#[derive(Debug, Clone, PartialEq)]
pub enum SolveError {
    /// The simulator rejected the request.
    Sim(SimError),
    /// A solver setting is out of range.
    Config(String),
    /// A pool or locked operator is not in the game data.
    UnknownOperator(OperatorId),
    /// A locked slot names a room or index the base lacks.
    BadLockedSlot(Slot),
    /// Nothing to decide: no variable slots.
    EmptySpace,
    /// Exhaustive search was asked for on a space above the limit.
    TooLarge { estimated: f64, limit: f64 },
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveError::Sim(e) => write!(f, "{e}"),
            SolveError::Config(msg) => write!(f, "invalid solver config: {msg}"),
            SolveError::UnknownOperator(op) => write!(f, "unknown operator {op}"),
            SolveError::BadLockedSlot(s) => write!(f, "locked slot {s} does not exist"),
            SolveError::EmptySpace => write!(f, "no slots for the solver to fill"),
            SolveError::TooLarge { estimated, limit } => write!(
                f,
                "{estimated:.0} assignments is above the exhaustive limit of {limit:.0}"
            ),
        }
    }
}

impl std::error::Error for SolveError {}

impl From<SimError> for SolveError {
    fn from(e: SimError) -> Self {
        SolveError::Sim(e)
    }
}
