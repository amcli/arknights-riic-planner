//! Turns everyone's parsed skill mechanics into numbers for one instant:
//! per-room stats and per-operator morale rates.
//!
//! Evaluation is in three passes so that counters can see what they need:
//!
//! 1. `FacilityCount` effects ("counts as one extra Power Plant"), which
//!    room-count counters read;
//! 2. every other clause whose amount does not depend on other operators'
//!    contributions;
//! 3. clauses with an `OthersStat` counter ("for every 10% productivity
//!    the others provide"), which read the pass-2 sums.
//!
//! Contributions are then combined per room:
//!
//! - **Stacking.** Upstream "(only the strongest effect of this type takes
//!   place)" is per effect *type*, not per skill: eleven different skill
//!   families share the Dormitory "whole room recovery" type, and two
//!   different Control Center families both give "+7% to all Trading
//!   Posts". Stat contributions are grouped by (receiving room, stat, room
//!   kind of the contributor); morale contributions by (recipient, room
//!   kind of the contributor, target class), where the classes are whole
//!   room, single operator, self, and so on. Only the largest in each group
//!   applies. "Distributed evenly" recovery counts as a single-operator
//!   effect when it reaches one operator and as a whole-room effect
//!   otherwise, as its description says.
//! - **Scaling.** "The productivity contributed by all other Operators in
//!   that Factory becomes 0 (excluding productivity granted based on
//!   facility count)" zeroes skill contributions from the *other operators
//!   in that room* only. Control Center bonuses, facility-count bonuses and
//!   the per-operator base bonus are untouched.
//!
//! Base rates (per-operator +1% / +5%, morale drain and recovery) are added
//! last. Operators at zero morale in a work area are *exhausted*: they still
//! occupy their slot and still count toward headcount-based morale relief
//! (PRTS 制造站), but contribute no skill effects and no base bonus, and an
//! exhausted Control Center operator no longer relieves the base. Idle
//! operators (Training Room trainees, or an assistant with no training
//! running) contribute nothing and their morale does not change.

use std::collections::BTreeMap;

use ak_domain::*;
use serde::{Deserialize, Serialize};

use crate::result::{SimWarning, Warnings};
use crate::rules;
use crate::world::{Role, World};

/// A room stat skills can contribute to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatKind {
    /// Factory productivity, percentage points.
    Productivity,
    /// Factory storage, volume units.
    Capacity,
    /// Trading Post order efficiency, percentage points.
    OrderEfficiency,
    /// Trading Post order queue limit.
    OrderLimit,
    /// Power Plant drone recovery, percentage points.
    DroneRecovery,
    /// Reception Room clue search speed, percentage points.
    ClueSpeed,
    /// Office contact speed, percentage points.
    ContactSpeed,
    /// Training Room speed, percentage points.
    TrainingSpeed,
}

impl StatKind {
    /// Every kind.
    pub const ALL: [StatKind; 8] = [
        StatKind::Productivity,
        StatKind::Capacity,
        StatKind::OrderEfficiency,
        StatKind::OrderLimit,
        StatKind::DroneRecovery,
        StatKind::ClueSpeed,
        StatKind::ContactSpeed,
        StatKind::TrainingSpeed,
    ];

    /// The room kind the stat belongs to.
    pub const fn room(self) -> RoomType {
        match self {
            StatKind::Productivity | StatKind::Capacity => RoomType::Manufacture,
            StatKind::OrderEfficiency | StatKind::OrderLimit => RoomType::Trading,
            StatKind::DroneRecovery => RoomType::Power,
            StatKind::ClueSpeed => RoomType::Meeting,
            StatKind::ContactSpeed => RoomType::Hire,
            StatKind::TrainingSpeed => RoomType::Training,
        }
    }

    fn from_stat(s: Stat) -> StatKind {
        match s {
            Stat::Productivity => StatKind::Productivity,
            Stat::OrderEfficiency => StatKind::OrderEfficiency,
            Stat::OrderLimit => StatKind::OrderLimit,
            Stat::Capacity => StatKind::Capacity,
        }
    }
}

/// One skill's contribution to one room stat, after stacking and scaling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    /// Receiving room.
    pub room: RoomId,
    /// Contributing operator.
    pub operator: OperatorId,
    /// Contributing skill tier.
    pub skill: BuffId,
    /// Which stat.
    pub stat: StatKind,
    /// Effective amount (percentage points or units).
    pub value: f64,
    /// The amount before another skill scaled it, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaled_from: Option<f64>,
}

/// A Workshop skill's static effect; Workshops are not simulated over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkshopStat {
    /// Contributing operator.
    pub operator: OperatorId,
    /// Contributing skill tier.
    pub skill: BuffId,
    /// Which formulas it applies to.
    pub material: MaterialFilter,
    /// Base morale-cost filter, if any.
    pub base_cost: Option<CostFilter>,
    /// Byproduct rate change, percentage points.
    pub byproduct_rate_pct: Option<f64>,
    /// Morale cost change.
    pub mood_cost: Option<CostChange>,
}

/// One room's stats at one instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomStats {
    /// Room label.
    pub id: RoomId,
    /// Kind.
    pub kind: RoomType,
    /// Level.
    pub level: u8,
    /// Stationed operators.
    pub headcount: usize,
    /// Stationed operators whose skills apply (not idle, not exhausted).
    pub active: usize,
    /// 100 plus the per-operator base bonus, in percent.
    pub base_pct: f64,
    /// Skill contributions summed per stat, after stacking and scaling.
    pub bonus: BTreeMap<StatKind, f64>,
    /// Factory productivity, percent (0 for other kinds).
    pub productivity_pct: f64,
    /// Factory storage, volume units.
    pub capacity: f64,
    /// Trading Post order efficiency, percent.
    pub order_efficiency_pct: f64,
    /// Trading Post order queue limit.
    pub order_limit: f64,
    /// Power Plant drone recovery, percent.
    pub drone_recovery_pct: f64,
    /// Reception Room clue search speed, percent. Excludes the rarity,
    /// promotion, ambience and room-level bonuses, which are not modelled.
    pub clue_speed_pct: f64,
    /// Office contact speed, percent.
    pub contact_speed_pct: f64,
    /// Training Room speed, percent.
    pub training_speed_pct: f64,
    /// Workshop skill effects, if this is a Workshop.
    pub workshop: Vec<WorkshopStat>,
}

impl RoomStats {
    /// The kind's headline stat and its value.
    pub fn main_stat(&self) -> Option<(StatKind, f64)> {
        Some(match self.kind {
            RoomType::Manufacture => (StatKind::Productivity, self.productivity_pct),
            RoomType::Trading => (StatKind::OrderEfficiency, self.order_efficiency_pct),
            RoomType::Power => (StatKind::DroneRecovery, self.drone_recovery_pct),
            RoomType::Meeting => (StatKind::ClueSpeed, self.clue_speed_pct),
            RoomType::Hire => (StatKind::ContactSpeed, self.contact_speed_pct),
            RoomType::Training => (StatKind::TrainingSpeed, self.training_speed_pct),
            _ => return None,
        })
    }
}

/// One operator's morale rate at one instant, morale per hour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoodRate {
    /// Whose.
    pub operator: OperatorId,
    /// Where they are.
    pub room: RoomId,
    /// Kind of that room.
    pub kind: RoomType,
    /// Base drain (negative) or recovery (positive) before skills.
    pub base: f64,
    /// Net skill effects targeting this operator.
    pub skills: f64,
    /// `base + skills`.
    pub total: f64,
    /// True when at zero morale in a work area.
    pub exhausted: bool,
    /// True when stationed but neither working nor resting.
    pub idle: bool,
}

/// Everything the evaluator computes for one instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Per room, in base order.
    pub rooms: Vec<RoomStats>,
    /// Per stationed operator.
    pub mood: Vec<MoodRate>,
    /// Every skill contribution that survived stacking.
    pub contributions: Vec<Contribution>,
    /// Warnings, when produced by [`crate::evaluate`].
    #[serde(default)]
    pub warnings: Vec<SimWarning>,
}

/// Mutable state the evaluator reads, indexed like [`World::ops`].
#[derive(Debug, Clone, Copy)]
pub struct Dynamic<'s> {
    /// Current morale.
    pub mood: &'s [f64],
    /// Hours since the operator entered their current room.
    pub hours_in_room: &'s [f64],
}

/// Evaluates the world at one instant.
pub fn evaluate(world: &World<'_>, dynamic: &Dynamic<'_>, warnings: &mut Warnings) -> Snapshot {
    let n = world.ops.len();
    let eval = Eval {
        w: world,
        mood: dynamic.mood.to_vec(),
        hours: dynamic.hours_in_room.to_vec(),
        warn: warnings,
        exhausted: vec![false; n],
        idle: vec![false; n],
        extra_rooms: BTreeMap::new(),
        stats: Vec::new(),
        moods: Vec::new(),
        scalers: Vec::new(),
        workshop: Vec::new(),
    };
    eval.run()
}

struct StatRec<'a> {
    room: usize,
    op: usize,
    skill: &'a BaseSkill,
    stat: StatKind,
    value: f64,
    stacking: Stacking,
    facility_based: bool,
}

/// Target classes for morale stacking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MoodClass {
    WholeRoom,
    Single,
    OwnerOnly,
    Rooms(RoomType),
    WorkAreas,
    Named,
}

struct MoodRec {
    recipient: usize,
    source: usize,
    value: f64,
    stacking: Stacking,
    class: MoodClass,
}

struct Scaler {
    room: usize,
    owner: usize,
    stat: StatKind,
    percent: f64,
}

struct Eval<'e, 'a> {
    w: &'e World<'a>,
    mood: Vec<f64>,
    hours: Vec<f64>,
    warn: &'e mut Warnings,
    exhausted: Vec<bool>,
    idle: Vec<bool>,
    extra_rooms: BTreeMap<RoomType, i32>,
    stats: Vec<StatRec<'a>>,
    moods: Vec<MoodRec>,
    scalers: Vec<Scaler>,
    workshop: Vec<(usize, WorkshopStat)>,
}

const EPS: f64 = 1e-9;

fn uses_others_stat(c: &Clause) -> bool {
    let in_amount = matches!(
        c.effect.amount(),
        Some(Amount::PerCount {
            counter: Counter::OthersStat { .. },
            ..
        })
    );
    let in_pred = matches!(
        c.when,
        Predicate::CountAtLeast {
            counter: Counter::OthersStat { .. },
            ..
        }
    );
    in_amount || in_pred
}

/// "Productivity granted based on facility count": amounts that scale with
/// how many rooms of a kind exist.
fn facility_based(a: &Amount) -> bool {
    matches!(
        a,
        Amount::PerCount {
            counter: Counter::RoomCount { .. } | Counter::GoldProductionLines,
            ..
        }
    )
}

impl<'e, 'a> Eval<'e, 'a> {
    fn acting(&self, o: usize) -> bool {
        !self.idle[o] && !self.exhausted[o]
    }

    fn run(mut self) -> Snapshot {
        let w = self.w;
        let n = w.ops.len();
        for o in 0..n {
            let role = w.role(o);
            self.idle[o] = role == Role::Idle;
            self.exhausted[o] = role == Role::Working && self.mood[o] <= EPS;
        }

        // Pass 1: facility counts.
        for o in 0..n {
            if !self.acting(o) {
                continue;
            }
            for s in w.skills_in_room(o) {
                let Some(m) = &s.mechanics else {
                    self.warn.push(SimWarning::UnparsedSkill {
                        skill: s.id.clone(),
                    });
                    continue;
                };
                for c in &m.clauses {
                    if let Effect::FacilityCount { room, delta } = &c.effect
                        && self.holds(&c.when, o, None, s)
                    {
                        *self.extra_rooms.entry(*room).or_default() += delta;
                    }
                }
            }
        }

        // Pass 2 and 3.
        let mut deferred = Vec::new();
        for o in 0..n {
            if !self.acting(o) {
                continue;
            }
            for s in w.skills_in_room(o) {
                let Some(m) = &s.mechanics else { continue };
                for c in &m.clauses {
                    if matches!(c.effect, Effect::FacilityCount { .. }) {
                        continue;
                    }
                    if uses_others_stat(c) {
                        deferred.push((o, s, c, m.stacking));
                    } else {
                        self.apply(o, s, c, m.stacking);
                    }
                }
            }
        }
        for (o, s, c, stacking) in deferred {
            self.apply(o, s, c, stacking);
        }

        self.aggregate()
    }

    fn apply(&mut self, o: usize, s: &'a BaseSkill, c: &'a Clause, stacking: Stacking) {
        let r = self.w.ops[o].room;
        if let Effect::Mood { amount, target } = &c.effect {
            let Some(v) = self.amount(amount, o, s, None) else {
                return;
            };
            let recipients = self.recipients(target, o, r, s);
            let class = match target {
                MoodTarget::SelfOnly => MoodClass::OwnerOnly,
                MoodTarget::AllInRoom | MoodTarget::OthersInRoom => MoodClass::WholeRoom,
                MoodTarget::OneOtherInRoom => MoodClass::Single,
                MoodTarget::DistributedInRoom if recipients.len() == 1 => MoodClass::Single,
                MoodTarget::DistributedInRoom => MoodClass::WholeRoom,
                MoodTarget::Rooms(kind) => MoodClass::Rooms(*kind),
                MoodTarget::AllWorkAreas => MoodClass::WorkAreas,
                MoodTarget::Named(_) => MoodClass::Named,
            };
            let share = if matches!(target, MoodTarget::DistributedInRoom) && !recipients.is_empty()
            {
                v / recipients.len() as f64
            } else {
                v
            };
            for t in recipients {
                if self.holds(&c.when, o, Some(t), s) {
                    self.moods.push(MoodRec {
                        recipient: t,
                        source: o,
                        value: share,
                        stacking,
                        class,
                    });
                }
            }
            return;
        }
        if !self.holds(&c.when, o, None, s) {
            return;
        }
        match &c.effect {
            Effect::Productivity {
                amount,
                product,
                scope,
            } => {
                let rooms = self.scope_rooms(scope, r, s);
                self.stat(
                    o,
                    s,
                    stacking,
                    StatKind::Productivity,
                    amount,
                    &rooms,
                    *product,
                );
            }
            Effect::Capacity {
                amount,
                product,
                scope,
            } => {
                let rooms = self.scope_rooms(scope, r, s);
                self.stat(o, s, stacking, StatKind::Capacity, amount, &rooms, *product);
            }
            Effect::OrderEfficiency { amount, scope } => {
                let rooms = self.scope_rooms(scope, r, s);
                self.stat(
                    o,
                    s,
                    stacking,
                    StatKind::OrderEfficiency,
                    amount,
                    &rooms,
                    None,
                );
            }
            Effect::OrderLimit { amount, scope } => {
                let rooms = self.scope_rooms(scope, r, s);
                self.stat(o, s, stacking, StatKind::OrderLimit, amount, &rooms, None);
            }
            Effect::DroneRecovery { amount } => {
                let rooms = self.natural_rooms(r, RoomType::Power);
                self.stat(
                    o,
                    s,
                    stacking,
                    StatKind::DroneRecovery,
                    amount,
                    &rooms,
                    None,
                );
            }
            Effect::ClueSpeed { amount } => {
                let rooms = self.natural_rooms(r, RoomType::Meeting);
                self.stat(o, s, stacking, StatKind::ClueSpeed, amount, &rooms, None);
            }
            Effect::ContactSpeed { amount } => {
                let rooms = self.natural_rooms(r, RoomType::Hire);
                self.stat(o, s, stacking, StatKind::ContactSpeed, amount, &rooms, None);
            }
            Effect::TrainingSpeed {
                amount,
                professions,
                subclass,
                spec_level,
            } => {
                let rooms: Vec<usize> = self
                    .natural_rooms(r, RoomType::Training)
                    .into_iter()
                    .filter(|&ri| {
                        self.training_matches(ri, professions, subclass.as_deref(), *spec_level, s)
                    })
                    .collect();
                self.stat(
                    o,
                    s,
                    stacking,
                    StatKind::TrainingSpeed,
                    amount,
                    &rooms,
                    None,
                );
            }
            Effect::ByproductRate {
                amount,
                material,
                base_cost,
            } => {
                let Some(v) = self.amount(amount, o, s, None) else {
                    return;
                };
                self.workshop.push((
                    r,
                    WorkshopStat {
                        operator: self.w.ops[o].id.clone(),
                        skill: s.id.clone(),
                        material: material.clone(),
                        base_cost: *base_cost,
                        byproduct_rate_pct: Some(v),
                        mood_cost: None,
                    },
                ));
            }
            Effect::WorkshopMoodCost {
                change,
                material,
                cost,
            } => {
                self.workshop.push((
                    r,
                    WorkshopStat {
                        operator: self.w.ops[o].id.clone(),
                        skill: s.id.clone(),
                        material: material.clone(),
                        base_cost: *cost,
                        byproduct_rate_pct: None,
                        mood_cost: Some(*change),
                    },
                ));
            }
            Effect::ScaleOthersContribution { stat, percent } => {
                self.scalers.push(Scaler {
                    room: r,
                    owner: o,
                    stat: StatKind::from_stat(*stat),
                    percent: *percent,
                });
            }
            Effect::FacilityCount { .. } | Effect::Mood { .. } => {}
            Effect::ClueBias { .. } => self.warn.push(SimWarning::ClueBiasIgnored {
                skill: s.id.clone(),
            }),
            Effect::GainResource { resource, .. } => {
                self.warn.push(SimWarning::ResourceNotSimulated {
                    skill: s.id.clone(),
                    resource: resource.clone(),
                });
            }
            Effect::ConvertResource { from, .. } => {
                self.warn.push(SimWarning::ResourceNotSimulated {
                    skill: s.id.clone(),
                    resource: from.clone(),
                });
            }
            Effect::Unmodeled { summary } => self.warn.push(SimWarning::UnmodeledEffect {
                skill: s.id.clone(),
                summary: summary.clone(),
            }),
        }
    }

    /// Records a stat contribution to each matching room. The amount is
    /// evaluated per receiving room, because a `TargetRoom` counter counts
    /// the operators in that room.
    #[allow(clippy::too_many_arguments)]
    fn stat(
        &mut self,
        o: usize,
        s: &'a BaseSkill,
        stacking: Stacking,
        stat: StatKind,
        amount: &Amount,
        rooms: &[usize],
        product: Option<ProductType>,
    ) {
        for &ri in rooms {
            let room = self.w.rooms[ri].room;
            if room.kind != stat.room() {
                continue;
            }
            if let Some(p) = product {
                let producing = room
                    .settings
                    .formula
                    .as_ref()
                    .and_then(|f| self.w.data.manufacture_formulas.get(f.as_str()))
                    .is_some_and(|f| f.product == p);
                if !producing {
                    continue;
                }
            }
            let Some(v) = self.amount(amount, o, s, Some(ri)) else {
                return;
            };
            self.stats.push(StatRec {
                room: ri,
                op: o,
                skill: s,
                stat,
                value: v,
                stacking,
                facility_based: facility_based(amount),
            });
        }
    }

    fn scope_rooms(&mut self, scope: &Scope, r: usize, s: &BaseSkill) -> Vec<usize> {
        match scope {
            Scope::ThisRoom => vec![r],
            Scope::AllRooms(kind) => self.w.rooms_of(*kind),
            Scope::RoomOf(who) => self
                .find(who, s)
                .map(|t| vec![self.w.ops[t].room])
                .unwrap_or_default(),
        }
    }

    /// The owner's room if it is of `kind`, else every room of `kind`
    /// (Control Center skills that name another room's stat).
    fn natural_rooms(&self, r: usize, kind: RoomType) -> Vec<usize> {
        if self.w.rooms[r].room.kind == kind {
            vec![r]
        } else {
            self.w.rooms_of(kind)
        }
    }

    fn find(&mut self, who: &OperatorRef, s: &BaseSkill) -> Option<usize> {
        let found = self.w.find_op(who);
        if found.is_none() && who.id.is_none() {
            self.warn.push(SimWarning::UnresolvedOperator {
                skill: s.id.clone(),
                name: who.name.clone(),
            });
        }
        found
    }

    fn training_matches(
        &mut self,
        ri: usize,
        professions: &[Profession],
        subclass: Option<&str>,
        spec_level: Option<u8>,
        s: &BaseSkill,
    ) -> bool {
        if !self.w.training_active[ri] {
            return false;
        }
        let Some(job) = &self.w.rooms[ri].room.settings.training else {
            return false;
        };
        if !professions.is_empty() && !professions.contains(&job.profession) {
            return false;
        }
        if let Some(name) = subclass {
            match self.w.config.memberships.subclass_id(name) {
                Some(id) => {
                    if job.subclass.as_ref().is_none_or(|j| j.as_str() != id) {
                        return false;
                    }
                }
                None => {
                    self.warn.push(SimWarning::UnknownSubclassName {
                        skill: s.id.clone(),
                        name: name.to_owned(),
                    });
                    return false;
                }
            }
        }
        if let Some(l) = spec_level
            && l != job.spec_level
        {
            return false;
        }
        true
    }

    fn not_full(&self, t: usize) -> bool {
        self.mood[t] < self.w.max_mood(t) - EPS
    }

    fn recipients(&mut self, target: &MoodTarget, o: usize, r: usize, s: &BaseSkill) -> Vec<usize> {
        let occupants = &self.w.rooms[r].occupants;
        match target {
            MoodTarget::SelfOnly => vec![o],
            MoodTarget::AllInRoom => occupants.clone(),
            MoodTarget::OthersInRoom => occupants.iter().copied().filter(|&t| t != o).collect(),
            MoodTarget::OneOtherInRoom => occupants
                .iter()
                .copied()
                .filter(|&t| t != o && self.not_full(t))
                .min_by(|&a, &b| self.mood[a].total_cmp(&self.mood[b]))
                .into_iter()
                .collect(),
            MoodTarget::DistributedInRoom => occupants
                .iter()
                .copied()
                .filter(|&t| t != o && self.not_full(t))
                .collect(),
            MoodTarget::Rooms(kind) => self.w.ops_in_rooms_of(*kind),
            MoodTarget::AllWorkAreas => self.w.ops_in_work_areas(),
            MoodTarget::Named(who) => self.find(who, s).into_iter().collect(),
        }
    }

    fn holds(&mut self, p: &Predicate, o: usize, target: Option<usize>, s: &BaseSkill) -> bool {
        let r = self.w.ops[o].room;
        match p {
            Predicate::Always => true,
            Predicate::CoworkerIs { who } => self
                .find(who, s)
                .is_some_and(|t| t != o && self.w.ops[t].room == r),
            Predicate::OperatorInRoom { who, room } => self
                .find(who, s)
                .is_some_and(|t| self.w.room_of(t).room.kind == *room),
            Predicate::OperatorInWorkArea { who } => self
                .find(who, s)
                .is_some_and(|t| self.w.room_of(t).room.kind.is_work_area()),
            Predicate::OperatorInBase { who } => self.find(who, s).is_some(),
            Predicate::CoworkerIn { group } => {
                let pool = self.w.rooms[r].occupants.clone();
                pool.into_iter()
                    .any(|t| t != o && self.w.in_group(t, group, &s.id, self.warn) == Some(true))
            }
            Predicate::GroupInRoom { group, room } => {
                let pool = self.w.ops_in_rooms_of(*room);
                pool.into_iter()
                    .any(|t| t != o && self.w.in_group(t, group, &s.id, self.warn) == Some(true))
            }
            Predicate::AloneInRoom => self.w.rooms[r].occupants.len() == 1,
            Predicate::SelfMoodFull => !self.not_full(o),
            Predicate::SelfMoodAbove { value } => self.mood[o] > *value,
            Predicate::SelfMoodBelow { value } => self.mood[o] < *value,
            Predicate::SelfMoodDeficitAbove { value } => self.w.max_mood(o) - self.mood[o] > *value,
            Predicate::InClueExchange => self.w.config.in_clue_exchange,
            Predicate::CountAtLeast { counter, n } => {
                self.count(counter, o, s, None) >= f64::from(*n)
            }
            Predicate::TargetIsOperator { who } => {
                let found = self.find(who, s);
                target.is_some() && found == target
            }
            Predicate::TargetIn { group } => {
                target.is_some_and(|t| self.w.in_group(t, group, &s.id, self.warn) == Some(true))
            }
            Predicate::TrainingProfession {
                profession,
                spec_level,
            } => {
                let room = self.w.rooms[r].room;
                room.kind == RoomType::Training
                    && self.w.training_active[r]
                    && room.settings.training.as_ref().is_some_and(|job| {
                        job.profession == *profession
                            && spec_level.is_none_or(|l| l == job.spec_level)
                    })
            }
            Predicate::Producing { product } => {
                let room = self.w.rooms[r].room;
                room.kind == RoomType::Manufacture
                    && room
                        .settings
                        .formula
                        .as_ref()
                        .and_then(|f| self.w.data.manufacture_formulas.get(f.as_str()))
                        .is_some_and(|f| f.product == *product)
            }
            Predicate::Not { inner } => !self.holds(inner, o, target, s),
            Predicate::And { all } => all.iter().all(|q| self.holds(q, o, target, s)),
            Predicate::Or { any } => any.iter().any(|q| self.holds(q, o, target, s)),
            Predicate::Unmodeled { text } => {
                self.warn.push(SimWarning::UnmodeledPredicate {
                    skill: s.id.clone(),
                    text: text.clone(),
                });
                false
            }
        }
    }

    /// Counts for a counter. `target_room` is the room receiving the effect,
    /// for `TargetRoom` counters; it defaults to the owner's room.
    fn count(&mut self, c: &Counter, o: usize, s: &BaseSkill, target_room: Option<usize>) -> f64 {
        let r = self.w.ops[o].room;
        match c {
            Counter::Operators {
                group,
                scope,
                excluding_self,
            } => {
                let pool: Vec<usize> = match scope {
                    CountScope::SameRoom => self.w.rooms[r].occupants.clone(),
                    CountScope::TargetRoom => {
                        self.w.rooms[target_room.unwrap_or(r)].occupants.clone()
                    }
                    CountScope::Base => (0..self.w.ops.len()).collect(),
                    CountScope::Rooms(kind) => self.w.ops_in_rooms_of(*kind),
                    CountScope::WorkAreas => self.w.ops_in_work_areas(),
                };
                pool.into_iter()
                    .filter(|&t| !(*excluding_self && t == o))
                    .filter(|&t| self.w.in_group(t, group, &s.id, self.warn) == Some(true))
                    .count() as f64
            }
            Counter::OtherOperatorsInRoom => (self.w.rooms[r].occupants.len() - 1) as f64,
            Counter::OperatorsInRoom => self.w.rooms[r].occupants.len() as f64,
            Counter::OperatorsInRooms { room } => self.w.ops_in_rooms_of(*room).len() as f64,
            Counter::RoomCount { room } => {
                self.w.base.count_of(*room) as f64
                    + f64::from(self.extra_rooms.get(room).copied().unwrap_or(0))
            }
            Counter::RoomLevels { room } => self
                .w
                .base
                .rooms_of(*room)
                .map(|x| f64::from(x.level))
                .sum(),
            Counter::ThisRoomLevel => f64::from(self.w.rooms[r].room.level),
            Counter::RecruitSlots => f64::from(self.w.config.extra_recruit_slots),
            Counter::Resource { resource } => {
                self.warn.push(SimWarning::ResourceNotSimulated {
                    skill: s.id.clone(),
                    resource: resource.clone(),
                });
                0.0
            }
            Counter::GoldProductionLines => self
                .w
                .base
                .rooms_of(RoomType::Manufacture)
                .filter(|x| {
                    x.settings
                        .formula
                        .as_ref()
                        .and_then(|f| self.w.data.manufacture_formulas.get(f.as_str()))
                        .is_some_and(|f| f.product == ProductType::Gold)
                })
                .count() as f64,
            Counter::OperatorsWithSkillFamily { family } => {
                match self.w.config.memberships.skill_types.get(family) {
                    Some(prefixes) => {
                        let kind = self.w.rooms[r].room.kind;
                        self.w.rooms[r]
                            .occupants
                            .iter()
                            .filter(|&&t| {
                                self.w.ops[t].skills.iter().any(|sk| {
                                    sk.room_type == kind
                                        && prefixes.iter().any(|p| sk.family() == p)
                                })
                            })
                            .count() as f64
                    }
                    None => {
                        self.warn.push(SimWarning::UnknownSkillType {
                            skill: s.id.clone(),
                            family: family.clone(),
                        });
                        0.0
                    }
                }
            }
            Counter::OthersStat { stat } => {
                // "provided by all other operators assigned to that
                // factory": other occupants only, not the Control Center.
                let kind = StatKind::from_stat(*stat);
                self.stats
                    .iter()
                    .filter(|rec| {
                        rec.room == r
                            && rec.op != o
                            && self.w.ops[rec.op].room == r
                            && rec.stat == kind
                    })
                    .map(|rec| rec.value)
                    .sum()
            }
            Counter::SelfMoodDeficit => self.w.max_mood(o) - self.mood[o],
            Counter::Unmodeled { text } => {
                self.warn.push(SimWarning::UnmodeledCounter {
                    skill: s.id.clone(),
                    text: text.clone(),
                });
                0.0
            }
        }
    }

    fn amount(
        &mut self,
        a: &Amount,
        o: usize,
        s: &BaseSkill,
        target_room: Option<usize>,
    ) -> Option<f64> {
        match a {
            Amount::Flat { value } => Some(*value),
            Amount::PerCount {
                per,
                step,
                counter,
                max_count,
                max_total,
            } => {
                let mut n = self.count(counter, o, s, target_room);
                if let Some(m) = max_count {
                    n = n.min(*m);
                }
                let units = if *step > 0.0 {
                    (n / step + EPS).floor()
                } else {
                    0.0
                };
                let mut v = per * units;
                if let Some(m) = max_total {
                    v = if *per >= 0.0 { v.min(*m) } else { v.max(-m) };
                }
                Some(v)
            }
            Amount::Ramp {
                initial,
                per_hour,
                max,
            } => {
                let v = initial + per_hour * self.hours[o].floor();
                Some(if *per_hour >= 0.0 {
                    v.min(*max)
                } else {
                    v.max(*max)
                })
            }
            Amount::Set { .. } => {
                self.warn.push(SimWarning::SetAmountUnsupported {
                    skill: s.id.clone(),
                });
                None
            }
        }
    }

    fn aggregate(mut self) -> Snapshot {
        let w = self.w;
        let n = w.ops.len();
        let owner_kind = |o: usize| w.room_of(o).room.kind;

        // Stacking: strongest per (receiving room, stat, contributor's room
        // kind) among "strongest of this type" contributions.
        let mut kept: Vec<StatRec<'a>> = Vec::with_capacity(self.stats.len());
        let mut strongest: BTreeMap<(usize, StatKind, RoomType), usize> = BTreeMap::new();
        for rec in std::mem::take(&mut self.stats) {
            match rec.stacking {
                Stacking::Additive => kept.push(rec),
                Stacking::StrongestOfType => {
                    let key = (rec.room, rec.stat, owner_kind(rec.op));
                    if let Some(&i) = strongest.get(&key) {
                        if rec.value.abs() > kept[i].value.abs() {
                            kept[i] = rec;
                        }
                    } else {
                        strongest.insert(key, kept.len());
                        kept.push(rec);
                    }
                }
            }
        }

        // Scaling: each record from an operator in the receiving room is
        // scaled by the strongest scaler another operator there holds.
        let mut sums: BTreeMap<(usize, StatKind), f64> = BTreeMap::new();
        let mut contributions = Vec::with_capacity(kept.len());
        for rec in &kept {
            let factor = if rec.facility_based || w.ops[rec.op].room != rec.room {
                1.0
            } else {
                self.scalers
                    .iter()
                    .filter(|sc| sc.room == rec.room && sc.stat == rec.stat && sc.owner != rec.op)
                    .map(|sc| sc.percent)
                    .min_by(f64::total_cmp)
                    .map_or(1.0, |p| (1.0 + p / 100.0).max(0.0))
            };
            let value = rec.value * factor;
            *sums.entry((rec.room, rec.stat)).or_default() += value;
            contributions.push(Contribution {
                room: w.rooms[rec.room].room.id.clone(),
                operator: w.ops[rec.op].id.clone(),
                skill: rec.skill.id.clone(),
                stat: rec.stat,
                value,
                scaled_from: (factor != 1.0).then_some(rec.value),
            });
        }

        let c = &w.data.constants;
        let active_cc = w
            .ops_in_rooms_of(RoomType::Control)
            .into_iter()
            .filter(|&o| self.acting(o))
            .count();

        let mut rooms = Vec::with_capacity(w.rooms.len());
        for (ri, rv) in w.rooms.iter().enumerate() {
            let room = rv.room;
            let headcount = rv.occupants.len();
            let active = rv.occupants.iter().filter(|&&o| self.acting(o)).count();
            let base_pct = 100.0 + rules::basic_speed_buff(c, room.kind) * 100.0 * active as f64;
            let get = |k: StatKind| sums.get(&(ri, k)).copied().unwrap_or(0.0);
            let bonus: BTreeMap<StatKind, f64> = StatKind::ALL
                .iter()
                .filter_map(|&k| sums.get(&(ri, k)).map(|v| (k, *v)))
                .collect();
            let pct = |kind: RoomType, k: StatKind| {
                if room.kind == kind {
                    (base_pct + get(k)).max(0.0)
                } else {
                    0.0
                }
            };
            rooms.push(RoomStats {
                id: room.id.clone(),
                kind: room.kind,
                level: room.level,
                headcount,
                active,
                base_pct,
                bonus,
                productivity_pct: pct(RoomType::Manufacture, StatKind::Productivity),
                capacity: if room.kind == RoomType::Manufacture {
                    (rules::output_capacity(c, room.level) + get(StatKind::Capacity)).max(0.0)
                } else {
                    0.0
                },
                order_efficiency_pct: pct(RoomType::Trading, StatKind::OrderEfficiency),
                order_limit: if room.kind == RoomType::Trading {
                    (rules::order_limit(c, room.level) + get(StatKind::OrderLimit)).max(0.0)
                } else {
                    0.0
                },
                drone_recovery_pct: pct(RoomType::Power, StatKind::DroneRecovery),
                clue_speed_pct: pct(RoomType::Meeting, StatKind::ClueSpeed),
                contact_speed_pct: pct(RoomType::Hire, StatKind::ContactSpeed),
                training_speed_pct: pct(RoomType::Training, StatKind::TrainingSpeed),
                workshop: self
                    .workshop
                    .iter()
                    .filter(|(r, _)| *r == ri)
                    .map(|(_, s)| s.clone())
                    .collect(),
            });
        }

        // Morale stacking: strongest per (recipient, contributor's room
        // kind, target class).
        let mut kept_moods: Vec<MoodRec> = Vec::with_capacity(self.moods.len());
        let mut strongest: BTreeMap<(usize, RoomType, MoodClass), usize> = BTreeMap::new();
        for rec in std::mem::take(&mut self.moods) {
            match rec.stacking {
                Stacking::Additive => kept_moods.push(rec),
                Stacking::StrongestOfType => {
                    let key = (rec.recipient, owner_kind(rec.source), rec.class);
                    if let Some(&i) = strongest.get(&key) {
                        if rec.value.abs() > kept_moods[i].value.abs() {
                            kept_moods[i] = rec;
                        }
                    } else {
                        strongest.insert(key, kept_moods.len());
                        kept_moods.push(rec);
                    }
                }
            }
        }
        let mut skill_sum = vec![0.0; n];
        for rec in &kept_moods {
            skill_sum[rec.recipient] += rec.value;
        }
        let mood = (0..n)
            .map(|o| {
                let rv = w.room_of(o);
                let idle = self.idle[o];
                let base = if idle || self.exhausted[o] {
                    0.0
                } else {
                    rules::base_mood_rate(w.data, rv.room, rv.occupants.len(), active_cc)
                };
                let skills = if idle { 0.0 } else { skill_sum[o] };
                MoodRate {
                    operator: w.ops[o].id.clone(),
                    room: rv.room.id.clone(),
                    kind: rv.room.kind,
                    base,
                    skills,
                    total: base + skills,
                    exhausted: self.exhausted[o],
                    idle,
                }
            })
            .collect();

        Snapshot {
            rooms,
            mood,
            contributions,
            warnings: Vec::new(),
        }
    }
}
