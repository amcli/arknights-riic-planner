//! Assignment solver (Layer 5).
//!
//! Given a base, a roster and an objective, finds assignments that score
//! well and returns the top few with their trade-offs:
//!
//! 1. the search space is every slot the request leaves free, filled from
//!    the roster (or a narrower pool), and every move goes through
//!    [`Assignment`]'s checked mutators, so no candidate is ever invalid;
//! 2. small spaces are enumerated exhaustively; larger ones are searched by
//!    simulated annealing with restarts and geometric cooling;
//! 3. inside the search, candidates are scored by a fast steady-state
//!    extrapolation of the instantaneous evaluator ([`Breakdown::proxy`]);
//! 4. the finalists are re-scored by the full mood-aware simulator and
//!    ranked by that, and the best one's simulation is returned in full.
//!
//! Everything is deterministic: the same request and seed give the same
//! result. `ak-solver` depends only on `ak-domain` and `ak-eval`.

#![forbid(unsafe_code)]

pub mod anneal;
pub mod evaluator;
pub mod exhaustive;
pub mod objective;
pub mod result;
pub mod rng;
pub mod space;

use std::time::{Duration, Instant};

use ak_domain::{Assignment, GameData};
pub use anneal::{AnnealReport, Entry, TopK};
pub use evaluator::{Evaluator, Score, Simulation, SteadyState};
pub use objective::{Breakdown, Objective, Shifts};
pub use result::{
    Candidate, InnerEvaluator, SolveError, SolveRequest, SolveResult, SolverConfig, SpaceSummary,
    Strategy,
};
pub use space::Space;

/// Runs a solve.
pub fn solve(data: &GameData, req: &SolveRequest) -> Result<SolveResult, SolveError> {
    let started = Instant::now();
    req.solver.validate()?;
    req.config.validate()?;

    let initial = req
        .initial
        .clone()
        .unwrap_or_else(|| Assignment::empty(&req.base, data));
    let space = Space::new(
        data,
        &req.base,
        &req.roster,
        req.pool.as_deref(),
        &initial,
        &req.locked,
    )?;
    let estimated = space.size_estimate();
    let strategy = match req.solver.strategy {
        Strategy::Auto => {
            if estimated <= req.solver.exhaustive_limit {
                Strategy::Exhaustive
            } else {
                Strategy::Annealing
            }
        }
        s => s,
    };
    let mut steady = SteadyState::new(
        data,
        &req.base,
        &req.roster,
        &req.config,
        &req.objective,
        &req.rotation,
    );
    let mut sim = Simulation::new(
        data,
        &req.base,
        &req.roster,
        &req.config,
        &req.rotation,
        &space.pool,
        &req.objective,
    );
    let inner = req.solver.inner;
    let deadline = req
        .solver
        .time_budget_ms
        .map(|ms| started + Duration::from_millis(ms));
    let mut top = TopK::new(req.solver.top_k);

    let (initial_inner, evaluations) = {
        let evaluator: &mut dyn Evaluator = match inner {
            InnerEvaluator::SteadyState => &mut steady,
            InnerEvaluator::Simulation => &mut sim,
        };
        let initial_inner = evaluator.score(&initial)?;
        top.insert(
            initial_inner.clone(),
            space.canonical(&initial),
            initial.clone(),
        );
        match strategy {
            Strategy::Exhaustive => {
                if estimated > req.solver.exhaustive_limit {
                    return Err(SolveError::TooLarge {
                        estimated,
                        limit: req.solver.exhaustive_limit,
                    });
                }
                let mut start = initial.clone();
                space.clear_variable(&mut start);
                exhaustive::enumerate(&space, &start, evaluator, &mut top)?;
            }
            Strategy::Annealing => {
                for restart in 0..req.solver.restarts {
                    if let Some(d) = deadline
                        && Instant::now() >= d
                    {
                        break;
                    }
                    anneal::anneal(
                        &space,
                        evaluator,
                        &initial,
                        &req.solver,
                        &mut top,
                        u64::from(restart),
                        deadline,
                    )?;
                }
            }
            Strategy::Auto => unreachable!("resolved above"),
        };
        (initial_inner, evaluator.evaluations())
    };

    // Finalists: re-score with the simulator when the inner evaluator was
    // the proxy; the best is always simulated for the report.
    let rescore = inner == InnerEvaluator::SteadyState && req.solver.rescore_with_simulation;
    let mut candidates = Vec::new();
    let mut best_simulation = None;
    let mut entries = top.into_entries();
    // The starting assignment is always a finalist, so the answer can never
    // be worse than what the caller already had.
    let initial_key = space.canonical(&initial);
    if !entries.iter().any(|e| e.key == initial_key) {
        entries.push(anneal::Entry {
            score: initial_inner.clone(),
            key: initial_key,
            assignment: initial.clone(),
        });
    }
    if rescore {
        let mut simulated = Vec::with_capacity(entries.len());
        for e in entries {
            let result = sim.run(&e.assignment)?;
            let score = sim.score_result(&result);
            simulated.push((e, score, result));
        }
        simulated.sort_by(|a, b| {
            b.1.value
                .total_cmp(&a.1.value)
                .then_with(|| a.0.key.cmp(&b.0.key))
        });
        for (i, (e, score, result)) in simulated.into_iter().enumerate() {
            if i == 0 {
                best_simulation = Some(result);
            }
            candidates.push(Candidate {
                assignment: e.assignment,
                inner_score: e.score.value,
                score: score.value,
                breakdown: score.breakdown,
                simulated: true,
            });
        }
    } else {
        for (i, e) in entries.into_iter().enumerate() {
            if i == 0 {
                best_simulation = Some(sim.run(&e.assignment)?);
            }
            candidates.push(Candidate {
                assignment: e.assignment,
                inner_score: e.score.value,
                score: e.score.value,
                breakdown: e.score.breakdown,
                simulated: inner == InnerEvaluator::Simulation,
            });
        }
    }

    let initial_candidate = if rescore {
        let result = sim.run(&initial)?;
        let score = sim.score_result(&result);
        Candidate {
            assignment: initial,
            inner_score: initial_inner.value,
            score: score.value,
            breakdown: score.breakdown,
            simulated: true,
        }
    } else {
        Candidate {
            assignment: initial,
            inner_score: initial_inner.value,
            score: initial_inner.value,
            breakdown: initial_inner.breakdown,
            simulated: inner == InnerEvaluator::Simulation,
        }
    };

    Ok(SolveResult {
        data: data.version.clone(),
        strategy,
        inner,
        space: SpaceSummary {
            variable_slots: space.slots.len(),
            locked_slots: space.locked,
            pool: space.pool.len(),
            estimated_size: estimated,
        },
        initial: initial_candidate,
        candidates,
        best_simulation,
        evaluations,
        simulations: sim.evaluations(),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}
