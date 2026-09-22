//! What a run produces: totals, per-room and per-operator reports, the
//! morale trajectory, events, and every warning about something the model
//! could not honour.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ak_domain::{
    AssignmentError, BaseError, BuffId, DataVersion, FormulaId, ItemId, OperatorId, RoomId,
    RoomType, Slot,
};
use serde::{Deserialize, Serialize};

/// Something the simulator could not model faithfully. Each variant is
/// emitted at most once per run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SimWarning {
    /// The Layer 2 parser rejected this skill's description entirely.
    UnparsedSkill { skill: BuffId },
    /// A recognised-but-unquantified effect; applied nothing.
    UnmodeledEffect { skill: BuffId, summary: String },
    /// A recognised-but-unquantified condition; treated as false.
    UnmodeledPredicate { skill: BuffId, text: String },
    /// A recognised-but-unquantified counter; counted zero.
    UnmodeledCounter { skill: BuffId, text: String },
    /// Resource accumulators (Worldly Plight, Perception Information, …)
    /// are not simulated yet; the effect applied nothing.
    ResourceNotSimulated { skill: BuffId, resource: String },
    /// A glossary tag with no membership table; counted nobody.
    UnknownTag { skill: BuffId, tag: String },
    /// A subclass display name with no id mapping; matched nobody.
    UnknownSubclassName { skill: BuffId, name: String },
    /// A skill-type glossary id with no family mapping; counted nobody.
    UnknownSkillType { skill: BuffId, family: String },
    /// An operator named in a description that the loader could not
    /// resolve; the reference never matches.
    UnresolvedOperator { skill: BuffId, name: String },
    /// "Becomes N" amounts are not supported yet.
    SetAmountUnsupported { skill: BuffId },
    /// Qualitative clue-type biases have no numeric model.
    ClueBiasIgnored { skill: BuffId },
    /// A stationed operator the roster does not list; assumed maxed.
    OperatorNotInRoster { operator: OperatorId },
    /// A rotation policy named an operator the game data lacks.
    UnknownOperator { operator: OperatorId },
    /// A rotation action could not be applied.
    RotationRefused {
        operator: OperatorId,
        reason: String,
    },
    /// A Factory without a formula or a Training Room without a job.
    IdleRoom { room: RoomId, room_kind: RoomType },
    /// A formula bounded by its inputs (Dualchips) ran without an input
    /// stock, so it produced nothing.
    InputBoundFormula { room: RoomId, formula: FormulaId },
}

/// A de-duplicating warning sink.
#[derive(Debug, Default)]
pub struct Warnings(BTreeSet<SimWarning>);

impl Warnings {
    /// Records a warning (idempotent).
    pub fn push(&mut self, w: SimWarning) {
        self.0.insert(w);
    }

    /// True when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Sorted, unique warnings.
    pub fn into_vec(self) -> Vec<SimWarning> {
        self.0.into_iter().collect()
    }
}

/// Something that happened at a point in simulated time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// Morale hit zero while working; the operator's skills stopped.
    Exhausted { operator: OperatorId, room: RoomId },
    /// A rotation moved an operator.
    Moved {
        operator: OperatorId,
        from: Option<Slot>,
        to: Slot,
    },
    /// A rotation took an operator off the base.
    Benched { operator: OperatorId, from: Slot },
    /// Factory storage filled; production paused until collection.
    StorageFull { room: RoomId },
    /// Trading Post order queue filled; no new orders until collection.
    OrderQueueFull { room: RoomId },
    /// Trading Post has orders but no input stock to fulfil them.
    InputStarved { room: RoomId, item: ItemId },
    /// A Training Room finished its specialisation.
    TrainingCompleted { room: RoomId },
    /// A periodic collection happened.
    Collected,
}

/// An [`EventKind`] with its time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimEvent {
    /// Simulated hours since the start.
    pub hour: f64,
    /// What happened.
    pub kind: EventKind,
}

/// One point of an operator's morale trajectory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoodSample {
    /// Simulated hours since the start.
    pub hour: f64,
    /// Whose morale.
    pub operator: OperatorId,
    /// Morale on the 0–24 scale.
    pub mood: f64,
}

/// Base-wide totals over the horizon.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Totals {
    /// LMD from Trading Post orders.
    pub lmd: f64,
    /// Orundum from Trading Post orders.
    pub orundum: f64,
    /// EXP value of Battle Records produced.
    pub exp: f64,
    /// Drones charged by Power Plants.
    pub drones: f64,
    /// LMD consumed as a Factory input (Originium Shard formulas).
    pub lmd_spent: f64,
    /// Office contacts (recruitment refreshes) gained.
    pub contacts: f64,
    /// Pure Gold produced by Factories.
    pub gold_produced: f64,
    /// Pure Gold consumed by Trading Posts.
    pub gold_consumed: f64,
    /// Pure Gold in the depot at the end.
    pub gold_in_depot: f64,
    /// Originium Shards in the depot at the end.
    pub shards_in_depot: f64,
    /// Trading Post orders fulfilled.
    pub orders_completed: f64,
    /// Every item produced by Factories, by upstream item id.
    pub items: BTreeMap<ItemId, f64>,
    /// Every item consumed as Factory input, by upstream item id. Inputs
    /// other than Pure Gold and Originium Shards are assumed stocked.
    pub consumed: BTreeMap<ItemId, f64>,
}

/// One room over the horizon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomReport {
    /// Room label.
    pub id: RoomId,
    /// Kind.
    pub kind: RoomType,
    /// Level.
    pub level: u8,
    /// Occupants at the end of the run.
    pub operators: Vec<OperatorId>,
    /// The kind's headline stat (productivity, order efficiency, drone
    /// recovery, clue / contact / training speed) at the first tick, in
    /// percent. `None` for kinds without one.
    pub initial_stat_pct: Option<f64>,
    /// The same stat averaged over the horizon.
    pub average_stat_pct: Option<f64>,
    /// Items produced here.
    pub produced: BTreeMap<ItemId, f64>,
    /// LMD earned here.
    pub lmd: f64,
    /// Orundum earned here.
    pub orundum: f64,
    /// Orders fulfilled here.
    pub orders_completed: f64,
    /// Drones charged here.
    pub drones: f64,
    /// Hours during which output was lost to a full store or queue.
    pub hours_blocked: f64,
    /// Uncollected output left at the end: Factory units, or Office
    /// contacts.
    pub in_storage: f64,
    /// Office contacts gained here.
    pub contacts: f64,
    /// Training Room: base hours of specialisation progress (8, 16 or 24
    /// base hours complete a level).
    pub training_progress_hours: f64,
    /// Training Room: when the specialisation finished, if it did.
    pub training_completed_hour: Option<f64>,
    /// Orders waiting in the queue at the end.
    pub pending_orders: f64,
}

/// One operator over the horizon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperatorReport {
    /// Upstream id.
    pub id: OperatorId,
    /// Display name.
    pub name: String,
    /// Morale at the start.
    pub initial_mood: f64,
    /// Morale at the end.
    pub final_mood: f64,
    /// Lowest morale reached.
    pub min_mood: f64,
    /// Hours spent working with morale above zero.
    pub hours_working: f64,
    /// Hours spent in a Dormitory.
    pub hours_resting: f64,
    /// Hours spent stationed at zero morale.
    pub hours_exhausted: f64,
    /// Hours spent off the base.
    pub hours_benched: f64,
    /// Hours stationed but idle (a Training Room trainee, or an assistant
    /// with no training running).
    pub hours_idle: f64,
}

/// The result of one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimResult {
    /// Which data produced these numbers.
    pub data: DataVersion,
    /// Horizon in hours.
    pub horizon_hours: f64,
    /// Tick length in minutes.
    pub tick_minutes: u32,
    /// Base-wide totals.
    pub totals: Totals,
    /// Per room.
    pub rooms: Vec<RoomReport>,
    /// Per operator, including rotation bench.
    pub operators: Vec<OperatorReport>,
    /// Morale samples.
    pub trajectory: Vec<MoodSample>,
    /// Events in time order.
    pub events: Vec<SimEvent>,
    /// Everything the model could not honour.
    pub warnings: Vec<SimWarning>,
}

/// Why a run could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimError {
    /// The base config is invalid.
    Base(BaseError),
    /// The assignment does not fit the base.
    Assignment(AssignmentError),
    /// An operator id is not in the game data.
    UnknownOperator(OperatorId),
    /// The roster lacks an operator and the config says that is fatal.
    MissingRoster(OperatorId),
    /// A configuration value is out of range.
    Config(String),
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::Base(e) => write!(f, "invalid base: {e}"),
            SimError::Assignment(e) => write!(f, "invalid assignment: {e}"),
            SimError::UnknownOperator(op) => write!(f, "unknown operator {op}"),
            SimError::MissingRoster(op) => write!(f, "operator {op} is not in the roster"),
            SimError::Config(msg) => write!(f, "invalid config: {msg}"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<BaseError> for SimError {
    fn from(e: BaseError) -> Self {
        SimError::Base(e)
    }
}

impl From<AssignmentError> for SimError {
    fn from(e: AssignmentError) -> Self {
        SimError::Assignment(e)
    }
}
