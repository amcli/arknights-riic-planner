//! Scorers the search can use: the fast steady-state proxy and the full
//! simulator. The solver is generic over this trait so that ranking and
//! reporting can use different ones.

use ak_domain::{Assignment, BaseConfig, GameData, OperatorId, Roster};
use ak_eval::{
    MoodThreshold, NoRotation, Rotation, RotationPolicy, SimConfig, SimError, SimResult,
};

use crate::objective::{Breakdown, Objective, Shifts};

/// A candidate's score and what it is made of.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    /// The objective value; higher is better.
    pub value: f64,
    /// What it is made of.
    pub breakdown: Breakdown,
}

/// Scores assignments.
pub trait Evaluator {
    /// Scores one assignment.
    fn score(&mut self, assignment: &Assignment) -> Result<Score, SimError>;

    /// How many times [`score`](Self::score) has run.
    fn evaluations(&self) -> u64;
}

/// The instantaneous evaluator extrapolated over the horizon.
pub struct SteadyState<'a> {
    data: &'a GameData,
    base: &'a BaseConfig,
    roster: &'a Roster,
    config: &'a SimConfig,
    objective: &'a Objective,
    shifts: Option<Shifts>,
    count: u64,
}

impl<'a> SteadyState<'a> {
    /// Constructs the proxy scorer. With a `mood_threshold` rotation, morale
    /// drain is priced as time spent resting (see [`Breakdown::proxy`]).
    pub fn new(
        data: &'a GameData,
        base: &'a BaseConfig,
        roster: &'a Roster,
        config: &'a SimConfig,
        objective: &'a Objective,
        rotation: &Rotation,
    ) -> Self {
        SteadyState {
            data,
            base,
            roster,
            config,
            objective,
            shifts: Shifts::for_rotation(data, base, rotation),
            count: 0,
        }
    }
}

impl Evaluator for SteadyState<'_> {
    fn score(&mut self, assignment: &Assignment) -> Result<Score, SimError> {
        self.count += 1;
        let snapshot =
            ak_eval::evaluate(self.data, self.base, assignment, self.roster, self.config)?;
        let breakdown = Breakdown::proxy(
            &snapshot,
            self.data,
            self.base,
            self.roster,
            self.config,
            self.shifts,
        );
        Ok(Score {
            value: self.objective.value(&breakdown),
            breakdown,
        })
    }

    fn evaluations(&self) -> u64 {
        self.count
    }
}

/// The full simulator.
pub struct Simulation<'a> {
    data: &'a GameData,
    base: &'a BaseConfig,
    roster: &'a Roster,
    config: &'a SimConfig,
    rotation: &'a Rotation,
    pool: &'a [OperatorId],
    objective: &'a Objective,
    count: u64,
}

impl<'a> Simulation<'a> {
    /// Constructs the simulating scorer. With a `mood_threshold` rotation,
    /// every `pool` operator a candidate leaves unassigned joins its bench.
    pub fn new(
        data: &'a GameData,
        base: &'a BaseConfig,
        roster: &'a Roster,
        config: &'a SimConfig,
        rotation: &'a Rotation,
        pool: &'a [OperatorId],
        objective: &'a Objective,
    ) -> Self {
        Simulation {
            data,
            base,
            roster,
            config,
            rotation,
            pool,
            objective,
            count: 0,
        }
    }

    fn policy(&self, assignment: &Assignment) -> Box<dyn RotationPolicy> {
        match self.rotation {
            Rotation::None => Box::new(NoRotation),
            Rotation::MoodThreshold {
                swap_out,
                swap_in,
                bench,
            } => {
                let mut bench = bench.clone();
                for op in self.pool {
                    if assignment.locate(op.as_str()).is_none() && !bench.contains(op) {
                        bench.push(op.clone());
                    }
                }
                Box::new(MoodThreshold::new(*swap_out, *swap_in, bench))
            }
        }
    }

    /// Runs the simulator on one assignment.
    pub fn run(&mut self, assignment: &Assignment) -> Result<SimResult, SimError> {
        self.count += 1;
        let mut policy = self.policy(assignment);
        ak_eval::simulate_with(
            self.data,
            self.base,
            assignment,
            self.roster,
            self.config,
            policy.as_mut(),
        )
    }

    /// Scores a finished simulation.
    pub fn score_result(&self, result: &SimResult) -> Score {
        let breakdown = Breakdown::from_result(result);
        Score {
            value: self.objective.value(&breakdown),
            breakdown,
        }
    }
}

impl Evaluator for Simulation<'_> {
    fn score(&mut self, assignment: &Assignment) -> Result<Score, SimError> {
        let result = self.run(assignment)?;
        Ok(self.score_result(&result))
    }

    fn evaluations(&self) -> u64 {
        self.count
    }
}
