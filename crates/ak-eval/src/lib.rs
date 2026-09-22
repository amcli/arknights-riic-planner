//! Evaluator and mood-aware base simulator (Layer 4).
//!
//! Pure: no async, no I/O, no framework. The entry points are plain
//! functions over [`ak_domain`] types:
//!
//! - [`simulate`] runs a [`SimRequest`] (base, assignment, roster, config,
//!   rotation) over a horizon with a fixed tick and returns a [`SimResult`]
//!   with totals, per-room and per-operator reports, the morale trajectory,
//!   events, and every warning about something the model could not honour;
//! - [`simulate_with`] is the same with a caller-supplied
//!   [`RotationPolicy`];
//! - [`evaluate`] is the instantaneous, no-time evaluator: room stats and
//!   morale rates for an assignment at its starting morale. The solver
//!   ranks with this and reports with [`simulate`].
//!
//! Every number is either read from the game data or documented with its
//! source in [`rules`]. Skill parts the model cannot honour are reported as
//! [`SimWarning`]s rather than silently dropped.
//!
//! Dependency direction: `ak-domain ← ak-eval ← ak-solver ← ak-api / ak-cli`.

#![forbid(unsafe_code)]

pub mod config;
pub mod effects;
pub mod result;
pub mod rotation;
pub mod rules;
mod sim;
pub mod world;

pub use config::{
    CollectionPolicy, Memberships, MissingRoster, MoodPolicy, SimConfig, SimRequest,
    builtin_subclass_id,
};
pub use effects::{Contribution, Dynamic, MoodRate, RoomStats, Snapshot, StatKind, WorkshopStat};
pub use result::{
    EventKind, MoodSample, OperatorReport, RoomReport, SimError, SimEvent, SimResult, SimWarning,
    Totals, Warnings,
};
pub use rotation::{MoodThreshold, NoRotation, Rotation, RotationPolicy, SimAction, SimView};
pub use sim::OpState;
pub use world::World;

use ak_domain::{Assignment, BaseConfig, GameData, Roster};

/// Runs a request with the rotation policy it names.
pub fn simulate(data: &GameData, request: &SimRequest) -> Result<SimResult, SimError> {
    let mut policy = request.rotation.policy();
    simulate_with(
        data,
        &request.base,
        &request.assignment,
        &request.roster,
        &request.config,
        policy.as_mut(),
    )
}

/// Runs a simulation with a caller-supplied rotation policy.
pub fn simulate_with(
    data: &GameData,
    base: &BaseConfig,
    assignment: &Assignment,
    roster: &Roster,
    config: &SimConfig,
    policy: &mut dyn RotationPolicy,
) -> Result<SimResult, SimError> {
    sim::run(data, base, assignment, roster, config, policy)
}

/// Evaluates an assignment at its starting morale, without time.
pub fn evaluate(
    data: &GameData,
    base: &BaseConfig,
    assignment: &Assignment,
    roster: &Roster,
    config: &SimConfig,
) -> Result<Snapshot, SimError> {
    config.validate()?;
    base.validate(data)?;
    assignment.check(base, data)?;
    let mut warnings = Warnings::default();
    let world = World::build(data, base, assignment, roster, config, &mut warnings)?;
    let mood: Vec<f64> = world
        .ops
        .iter()
        .map(|o| sim::initial_mood(o.op, roster, config))
        .collect();
    let hours = vec![0.0; world.ops.len()];
    let mut snapshot = effects::evaluate(
        &world,
        &Dynamic {
            mood: &mood,
            hours_in_room: &hours,
        },
        &mut warnings,
    );
    snapshot.warnings = warnings.into_vec();
    Ok(snapshot)
}
