//! Machine-readable skill mechanics: the output of the Layer 2 description
//! parser.
//!
//! A skill is a list of [`Clause`]s. Each clause fires when its
//! [`Predicate`] holds and contributes one [`Effect`]. Effects carry an
//! [`Amount`], which may be flat, scale with a [`Counter`] ("for each
//! Blacksteel operator in this Factory"), or ramp over time.
//!
//! Anything the parser can name but not quantify is kept as an explicit
//! `Unmodeled` variant rather than silently dropped, so coverage numbers stay
//! honest and the evaluator can warn instead of guessing.

use serde::{Deserialize, Serialize};

use crate::{OperatorId, PowerId, ProductType, Profession, RoomType};

/// The parsed mechanics of one skill tier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mechanics {
    /// Conditions and effects, in description order.
    pub clauses: Vec<Clause>,
    /// How this skill combines with other copies of "the same type".
    pub stacking: Stacking,
}

impl Mechanics {
    /// True when every predicate, counter, and effect is modelled.
    pub fn is_fully_modeled(&self) -> bool {
        self.unmodeled_parts().is_empty()
    }

    /// Human-readable summaries of every unmodeled part.
    pub fn unmodeled_parts(&self) -> Vec<String> {
        let mut out = Vec::new();
        for clause in &self.clauses {
            clause.when.collect_unmodeled(&mut out);
            clause.effect.collect_unmodeled(&mut out);
        }
        out
    }
}

/// Upstream stacking note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stacking {
    /// Adds with everything else (the default).
    Additive,
    /// "Only the strongest effect of this type takes place."
    StrongestOfType,
}

/// One condition → effect pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clause {
    /// When the effect applies. The implicit "this operator is assigned to
    /// the skill's room" is *not* included; it is always required.
    pub when: Predicate,
    /// What happens.
    pub effect: Effect,
}

/// A reference to a specific operator by display name. `id` is filled in
/// when the name resolves against the loaded operator table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorRef {
    /// Display name as written in the description, e.g. `Texas`.
    pub name: String,
    /// Resolved id, if the name matched exactly one operator.
    pub id: Option<OperatorId>,
}

impl OperatorRef {
    /// An unresolved reference.
    pub fn named(name: impl Into<String>) -> Self {
        OperatorRef {
            name: name.into(),
            id: None,
        }
    }
}

/// A set of operators, used by counters and predicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Group {
    /// Nation, group, or team from `handbook_team_table`.
    Power(PowerId),
    /// An upstream glossary tag with no table behind it (e.g. `knight`,
    /// `op` for Operation Platforms, `mh` for Soubo Adventurers). Membership
    /// must be defined by a later layer.
    Tag(String),
    /// An operator class.
    Profession(Profession),
    /// A subclass by display name, e.g. `Wandering Medic`.
    Subclass(String),
}

/// Where a counter looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CountScope {
    /// The room this operator is assigned to.
    SameRoom,
    /// Anywhere in the base (upstream: "excluding Assistants and Activity
    /// Room users").
    Base,
    /// Any room of the given kind.
    Rooms(RoomType),
    /// Any room except Dormitories (upstream: "non-Dormitory facility").
    WorkAreas,
    /// The room receiving the effect. Used by effects that reach every room
    /// of a kind but scale with who is in each of those rooms ("all Knight
    /// Operators assigned to Factories gain productivity +7%" counts the
    /// Knights in each Factory separately).
    TargetRoom,
}

/// A stat other operators contribute, for "for every N X provided by all
/// other operators" counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stat {
    Productivity,
    OrderEfficiency,
    OrderLimit,
    Capacity,
}

/// Something that can be counted to scale an effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Counter {
    /// Operators in a group within a scope. The skill owner counts when they
    /// are in the group and the scope, unless `excluding_self` is set
    /// (upstream: "for every *other* Rhine Lab Operator"). Evidence that
    /// the owner counts by default: four Team Rainbow Operators in the
    /// Control Center bring its morale drain to exactly zero, which only
    /// adds up if each of them counts themself.
    Operators {
        group: Group,
        scope: CountScope,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        excluding_self: bool,
    },
    /// Other operators in this room, any kind.
    OtherOperatorsInRoom,
    /// All operators in this room, including self.
    OperatorsInRoom,
    /// Operators assigned to rooms of a kind, anywhere in the base.
    OperatorsInRooms { room: RoomType },
    /// Number of rooms of a kind in the base.
    RoomCount { room: RoomType },
    /// Sum of levels over every room of a kind.
    RoomLevels { room: RoomType },
    /// Level of the room this operator is in.
    ThisRoomLevel,
    /// Recruitment slots in the Office beyond the default ones.
    RecruitSlots,
    /// Accumulated units of a named resource (e.g. `worldly_plight`).
    Resource { resource: String },
    /// Pure Gold Production Lines (Factories producing gold).
    GoldProductionLines,
    /// Operators in this room whose active skill belongs to a family.
    OperatorsWithSkillFamily { family: String },
    /// Points of a stat contributed by other operators in this room.
    OthersStat { stat: Stat },
    /// This operator's morale deficit below maximum.
    SelfMoodDeficit,
    /// Recognised but not modelled.
    Unmodeled { text: String },
}

/// How much an effect contributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Amount {
    /// A fixed value.
    Flat { value: f64 },
    /// `per` for every `step` units of `counter`, optionally capped.
    PerCount {
        per: f64,
        step: f64,
        counter: Counter,
        /// Maximum number of counted units that apply.
        max_count: Option<f64>,
        /// Maximum total contribution.
        max_total: Option<f64>,
    },
    /// `initial` in the first hour, then `per_hour` more each hour, up to
    /// `max`.
    Ramp {
        initial: f64,
        per_hour: f64,
        max: f64,
    },
    /// Sets the stat to a value instead of adding.
    Set { value: f64 },
}

impl Amount {
    /// A flat amount.
    pub fn flat(value: f64) -> Self {
        Amount::Flat { value }
    }
}

/// Which rooms an effect reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Scope {
    /// The room this operator is assigned to.
    ThisRoom,
    /// Every room of a kind (Control Center skills).
    AllRooms(RoomType),
    /// The room a named operator is assigned to.
    RoomOf(OperatorRef),
}

/// Who a morale effect applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MoodTarget {
    /// The skill owner.
    SelfOnly,
    /// Everyone in this room, including self.
    AllInRoom,
    /// Everyone in this room except self.
    OthersInRoom,
    /// One other operator in this room whose morale is not full.
    OneOtherInRoom,
    /// Everyone assigned to rooms of a kind, anywhere in the base.
    Rooms(RoomType),
    /// Everyone working in a non-Dormitory room.
    AllWorkAreas,
    /// A specific operator, wherever they are.
    Named(OperatorRef),
    /// Split evenly among operators in this room whose morale is not full.
    DistributedInRoom,
}

/// Which Workshop materials an effect applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MaterialFilter {
    /// Every formula.
    Any,
    /// A product family (elite materials, skill summaries, …).
    Product(ProductType),
    /// A named material family, e.g. `Device`, `Crystal`.
    Named(String),
}

/// A filter on a formula's base morale cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CostFilter {
    Exactly(u32),
    AtLeast(u32),
}

/// How a Workshop formula's morale cost changes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CostChange {
    /// Add (negative reduces).
    Delta(f64),
    /// Set to a value.
    Set(f64),
    /// Divide by a value.
    Divide(f64),
}

/// A mechanical effect. Percent-style stats (productivity, efficiency,
/// speeds, rates) are in percentage points; counts and morale are flat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    /// Factory productivity, percentage points, optionally for one product.
    Productivity {
        amount: Amount,
        product: Option<ProductType>,
        scope: Scope,
    },
    /// Factory storage capacity, optionally for one product.
    Capacity {
        amount: Amount,
        product: Option<ProductType>,
        scope: Scope,
    },
    /// Trading Post order acquisition efficiency, percentage points.
    OrderEfficiency { amount: Amount, scope: Scope },
    /// Trading Post order limit.
    OrderLimit { amount: Amount, scope: Scope },
    /// Power Plant drone recovery rate, percentage points.
    DroneRecovery { amount: Amount },
    /// Morale change per hour; positive restores, negative drains.
    Mood { amount: Amount, target: MoodTarget },
    /// Reception Room clue search speed, percentage points.
    ClueSpeed { amount: Amount },
    /// Office contact speed, percentage points.
    ContactSpeed { amount: Amount },
    /// Training Room specialisation speed, percentage points.
    TrainingSpeed {
        amount: Amount,
        /// Empty means every class.
        professions: Vec<Profession>,
        /// Only trainees of this subclass (display name).
        subclass: Option<String>,
        /// Only when training to this specialisation level.
        spec_level: Option<u8>,
    },
    /// Workshop byproduct rate, percentage points.
    ByproductRate {
        amount: Amount,
        material: MaterialFilter,
        base_cost: Option<CostFilter>,
    },
    /// Workshop formula morale cost change.
    WorkshopMoodCost {
        change: CostChange,
        material: MaterialFilter,
        cost: Option<CostFilter>,
    },
    /// Multiplies the contribution other operators in this room make to a
    /// stat (e.g. "productivity contributed by all other Operators -15%").
    ScaleOthersContribution { stat: Stat, percent: f64 },
    /// Counts an extra room of a kind for facility-count based effects.
    FacilityCount { room: RoomType, delta: i32 },
    /// Qualitative clue-type bias; not quantified upstream.
    ClueBias { description: String },
    /// Accumulates a named resource.
    GainResource { resource: String, amount: Amount },
    /// Converts `per` units of one resource into `amount` of another.
    ConvertResource {
        from: String,
        per: f64,
        to: String,
        amount: f64,
    },
    /// Recognised but not modelled.
    Unmodeled { summary: String },
}

impl Effect {
    /// The effect's amount, if it has one.
    pub fn amount(&self) -> Option<&Amount> {
        match self {
            Effect::Productivity { amount, .. }
            | Effect::Capacity { amount, .. }
            | Effect::OrderEfficiency { amount, .. }
            | Effect::OrderLimit { amount, .. }
            | Effect::DroneRecovery { amount }
            | Effect::Mood { amount, .. }
            | Effect::ClueSpeed { amount }
            | Effect::ContactSpeed { amount }
            | Effect::TrainingSpeed { amount, .. }
            | Effect::ByproductRate { amount, .. }
            | Effect::GainResource { amount, .. } => Some(amount),
            _ => None,
        }
    }

    /// Mutable access to the effect's amount, if it has one.
    pub fn amount_mut(&mut self) -> Option<&mut Amount> {
        match self {
            Effect::Productivity { amount, .. }
            | Effect::Capacity { amount, .. }
            | Effect::OrderEfficiency { amount, .. }
            | Effect::OrderLimit { amount, .. }
            | Effect::DroneRecovery { amount }
            | Effect::Mood { amount, .. }
            | Effect::ClueSpeed { amount }
            | Effect::ContactSpeed { amount }
            | Effect::TrainingSpeed { amount, .. }
            | Effect::ByproductRate { amount, .. }
            | Effect::GainResource { amount, .. } => Some(amount),
            _ => None,
        }
    }

    /// Short kind name, e.g. `productivity`.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Effect::Productivity { .. } => "productivity",
            Effect::Capacity { .. } => "capacity",
            Effect::OrderEfficiency { .. } => "order_efficiency",
            Effect::OrderLimit { .. } => "order_limit",
            Effect::DroneRecovery { .. } => "drone_recovery",
            Effect::Mood { .. } => "mood",
            Effect::ClueSpeed { .. } => "clue_speed",
            Effect::ContactSpeed { .. } => "contact_speed",
            Effect::TrainingSpeed { .. } => "training_speed",
            Effect::ByproductRate { .. } => "byproduct_rate",
            Effect::WorkshopMoodCost { .. } => "workshop_mood_cost",
            Effect::ScaleOthersContribution { .. } => "scale_others_contribution",
            Effect::FacilityCount { .. } => "facility_count",
            Effect::ClueBias { .. } => "clue_bias",
            Effect::GainResource { .. } => "gain_resource",
            Effect::ConvertResource { .. } => "convert_resource",
            Effect::Unmodeled { .. } => "unmodeled",
        }
    }

    fn collect_unmodeled(&self, out: &mut Vec<String>) {
        if let Effect::Unmodeled { summary } = self {
            out.push(format!("effect: {summary}"));
        }
        if let Some(Amount::PerCount { counter, .. }) = self.amount()
            && let Counter::Unmodeled { text } = counter
        {
            out.push(format!("counter: {text}"));
        }
    }
}

/// When a clause applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    /// Unconditional.
    Always,
    /// A named operator is in the same room.
    CoworkerIs { who: OperatorRef },
    /// A named operator is assigned to any room of a kind.
    OperatorInRoom { who: OperatorRef, room: RoomType },
    /// A named operator is assigned to any non-Dormitory room.
    OperatorInWorkArea { who: OperatorRef },
    /// A named operator is assigned anywhere in the base.
    OperatorInBase { who: OperatorRef },
    /// Another operator from a group is in the same room.
    CoworkerIn { group: Group },
    /// Another operator from a group is assigned to any room of a kind.
    GroupInRoom { group: Group, room: RoomType },
    /// No other operator is working in this room.
    AloneInRoom,
    /// This operator's morale is at maximum.
    SelfMoodFull,
    /// This operator's morale is above a threshold.
    SelfMoodAbove { value: f64 },
    /// This operator's morale is below a threshold.
    SelfMoodBelow { value: f64 },
    /// This operator's morale deficit (max − current) exceeds a threshold.
    SelfMoodDeficitAbove { value: f64 },
    /// The Reception Room is in a clue exchange.
    InClueExchange,
    /// At least `n` of something exist.
    CountAtLeast { counter: Counter, n: u32 },
    /// For per-target morale effects: the recipient is a named operator.
    TargetIsOperator { who: OperatorRef },
    /// For per-target morale effects: the recipient is in a group.
    TargetIn { group: Group },
    /// Training a trainee of a class (optionally to a level).
    TrainingProfession {
        profession: Profession,
        spec_level: Option<u8>,
    },
    /// The Factory is producing a given product family.
    Producing { product: ProductType },
    /// Logical negation.
    Not { inner: Box<Predicate> },
    /// All must hold.
    And { all: Vec<Predicate> },
    /// Any must hold.
    Or { any: Vec<Predicate> },
    /// Recognised but not modelled.
    Unmodeled { text: String },
}

impl Predicate {
    /// Combines two predicates, flattening `Always`.
    pub fn and(self, other: Predicate) -> Predicate {
        match (self, other) {
            (Predicate::Always, p) | (p, Predicate::Always) => p,
            (Predicate::And { mut all }, p) => {
                all.push(p);
                Predicate::And { all }
            }
            (a, b) => Predicate::And { all: vec![a, b] },
        }
    }

    fn collect_unmodeled(&self, out: &mut Vec<String>) {
        match self {
            Predicate::Unmodeled { text } => out.push(format!("predicate: {text}")),
            Predicate::CountAtLeast {
                counter: Counter::Unmodeled { text },
                ..
            } => out.push(format!("counter: {text}")),
            Predicate::Not { inner } => inner.collect_unmodeled(out),
            Predicate::And { all: ps } | Predicate::Or { any: ps } => {
                for p in ps {
                    p.collect_unmodeled(out);
                }
            }
            _ => {}
        }
    }

    /// Every operator reference in this predicate, for name resolution.
    pub fn operator_refs_mut(&mut self, f: &mut dyn FnMut(&mut OperatorRef)) {
        match self {
            Predicate::CoworkerIs { who }
            | Predicate::OperatorInRoom { who, .. }
            | Predicate::OperatorInWorkArea { who }
            | Predicate::OperatorInBase { who }
            | Predicate::TargetIsOperator { who } => f(who),
            Predicate::Not { inner } => inner.operator_refs_mut(f),
            Predicate::And { all: ps } | Predicate::Or { any: ps } => {
                for p in ps {
                    p.operator_refs_mut(f);
                }
            }
            _ => {}
        }
    }
}

impl Effect {
    /// Every operator reference in this effect, for name resolution.
    pub fn operator_refs_mut(&mut self, f: &mut dyn FnMut(&mut OperatorRef)) {
        match self {
            Effect::Productivity {
                scope: Scope::RoomOf(who),
                ..
            }
            | Effect::Capacity {
                scope: Scope::RoomOf(who),
                ..
            }
            | Effect::OrderEfficiency {
                scope: Scope::RoomOf(who),
                ..
            }
            | Effect::OrderLimit {
                scope: Scope::RoomOf(who),
                ..
            }
            | Effect::Mood {
                target: MoodTarget::Named(who),
                ..
            } => f(who),
            _ => {}
        }
    }
}
