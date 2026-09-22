//! Simulation configuration and the self-contained request type the API and
//! CLI accept.

use std::collections::{BTreeMap, BTreeSet};

use ak_domain::{Assignment, BaseConfig, ItemId, OperatorId, Roster, SubProfessionId};
use serde::{Deserialize, Serialize};

use crate::result::SimError;
use crate::rotation::Rotation;

/// Where each operator's morale starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoodPolicy {
    /// Everyone starts at maximum morale.
    #[default]
    Full,
    /// Use the roster's recorded morale; operators without one start full.
    Roster,
}

/// When Factory output and Trading Post orders are collected.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CollectionPolicy {
    /// Collected the instant it is produced: storage and order limits never
    /// bind. Matches how most calculators quote "per day" numbers.
    #[default]
    Continuous,
    /// Collected every `hours`. Factories stop when their storage is full
    /// and Trading Posts stop taking orders when their queue is full, as in
    /// the game. Whatever is in storage at the end of the horizon is
    /// reported separately, not counted.
    EveryHours { hours: f64 },
}

/// What to do about a stationed operator the roster does not list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingRoster {
    /// Assume every skill tier is unlocked, and warn.
    #[default]
    AssumeMaxed,
    /// Refuse to simulate.
    Error,
}

/// Membership tables for groups the game data does not define.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Memberships {
    /// Upstream glossary tags with no table behind them (`knight`, `op`,
    /// `mh`, `durin`, `alternate`, `a1`, `attack`, `defence`) → operator
    /// ids. A tag with no entry makes every counter over it zero, with a
    /// warning.
    pub tags: BTreeMap<String, BTreeSet<OperatorId>>,
    /// Skill-type glossary ids (`manu1`, `manu2`, `manu4`) → the skill
    /// family prefixes (e.g. `manu_prod_spd`) that belong to the type.
    pub skill_types: BTreeMap<String, Vec<String>>,
    /// Subclass display names → upstream subclass ids, overriding or
    /// extending [`builtin_subclass_id`].
    pub subclass_names: BTreeMap<String, SubProfessionId>,
}

impl Memberships {
    /// Resolves a subclass display name to its upstream id.
    pub fn subclass_id(&self, name: &str) -> Option<&str> {
        self.subclass_names
            .get(name)
            .map(SubProfessionId::as_str)
            .or_else(|| builtin_subclass_id(name))
    }
}

/// Subclass display names that appear in skill descriptions, mapped to the
/// upstream `subProfessionId` (verified against the pinned character table).
pub fn builtin_subclass_id(name: &str) -> Option<&'static str> {
    Some(match name {
        "Arts Protector" => "artsprotector",
        "Besieger" => "siegesniper",
        "Chain Medic" => "chainhealer",
        "Fighter" => "fighter",
        "Lord" => "lord",
        "Marksman" => "fastshot",
        "Wandering Medic" => "wandermedic",
        _ => return None,
    })
}

/// Everything about a run that is not the base, roster or assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimConfig {
    /// Length of the run in hours. 24 or 168 are typical.
    pub horizon_hours: f64,
    /// Fixed time step. Morale is linear within a step; predicates that
    /// depend on morale thresholds are re-evaluated at step boundaries.
    pub tick_minutes: u32,
    /// Where morale starts.
    pub initial_mood: MoodPolicy,
    /// When output is collected.
    pub collection: CollectionPolicy,
    /// Pure Gold in the depot at the start. Trading Posts consume it; gold
    /// Factories add to it. Zero means the posts live off the Factories.
    pub initial_gold: f64,
    /// Originium Shards in the depot at the start.
    pub initial_shards: f64,
    /// Factory input stock by item id. `None` assumes every input is
    /// available and only records what was consumed, except that formulas
    /// faster than one batch a minute (Dualchips) produce nothing and warn,
    /// because their output is set by their inputs rather than by time.
    /// LMD inputs are never limited; they are reported as `lmd_spent`.
    pub input_stock: Option<BTreeMap<ItemId, f64>>,
    /// Handling of stationed operators the roster lacks.
    pub missing_roster: MissingRoster,
    /// Whether the Reception Room is in a clue exchange (some skills key
    /// on it).
    pub in_clue_exchange: bool,
    /// Office recruitment slots beyond the default, for "per recruitment
    /// slot" counters.
    pub extra_recruit_slots: u32,
    /// Group memberships the game data does not define.
    pub memberships: Memberships,
    /// Morale trajectory sampling interval.
    pub trajectory_every_minutes: u32,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            horizon_hours: 24.0,
            tick_minutes: 5,
            initial_mood: MoodPolicy::Full,
            collection: CollectionPolicy::Continuous,
            initial_gold: 0.0,
            initial_shards: 0.0,
            input_stock: None,
            missing_roster: MissingRoster::AssumeMaxed,
            in_clue_exchange: false,
            extra_recruit_slots: 0,
            memberships: Memberships::default(),
            trajectory_every_minutes: 60,
        }
    }
}

impl SimConfig {
    /// Rejects nonsensical values.
    pub fn validate(&self) -> Result<(), SimError> {
        if self.horizon_hours.is_nan()
            || self.horizon_hours.is_infinite()
            || self.horizon_hours <= 0.0
        {
            return Err(SimError::Config("horizon_hours must be positive".into()));
        }
        if self.tick_minutes == 0 {
            return Err(SimError::Config("tick_minutes must be at least 1".into()));
        }
        if let CollectionPolicy::EveryHours { hours } = self.collection
            && (hours.is_nan() || hours <= 0.0)
        {
            return Err(SimError::Config("collection.hours must be positive".into()));
        }
        if self.initial_gold < 0.0 || self.initial_shards < 0.0 {
            return Err(SimError::Config("initial stock cannot be negative".into()));
        }
        if let Some(stock) = &self.input_stock
            && stock.values().any(|v| v.is_nan() || *v < 0.0)
        {
            return Err(SimError::Config("input_stock cannot be negative".into()));
        }
        Ok(())
    }
}

/// A complete, serialisable description of one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimRequest {
    /// The rooms.
    pub base: BaseConfig,
    /// Who is where at the start.
    pub assignment: Assignment,
    /// Owned operators and their promotion state.
    #[serde(default)]
    pub roster: Roster,
    /// Run settings.
    #[serde(default)]
    pub config: SimConfig,
    /// Shift policy.
    #[serde(default)]
    pub rotation: Rotation,
}
