//! The fixed-tick simulation loop.
//!
//! Per tick: resolve the world, evaluate every skill into rates, apply
//! production for `dt` hours, integrate morale, collect if due, ask the
//! rotation policy, sample the trajectory. Rates are held constant within
//! a tick, so morale-threshold predicates take effect at the next tick
//! boundary at the latest.
//!
//! Production is a deterministic fluid model:
//!
//! - a Factory makes `count × 3600 / cost_point × productivity` units per
//!   hour, limited by its input stock (when one is configured) and, under
//!   periodic collection, by storage measured in volume (`weight` per unit);
//! - a Trading Post takes orders at `efficiency × 60 / E[minutes]` per hour
//!   up to its order limit and fulfils them from the depot at the expected
//!   input and output per order (see [`crate::rules`]); Pure Gold and
//!   Originium Shards flow from Factories through the depot;
//! - a Power Plant charges drones, an Office gains contacts, and a Training
//!   Room accumulates specialisation progress until the level completes.

use std::collections::{BTreeMap, BTreeSet};

use ak_domain::*;
use serde::{Deserialize, Serialize};

use crate::config::{CollectionPolicy, MoodPolicy, SimConfig};
use crate::effects::{self, Dynamic, RoomStats, Snapshot};
use crate::result::{
    EventKind, MoodSample, OperatorReport, RoomReport, SimError, SimEvent, SimResult, SimWarning,
    Totals, Warnings,
};
use crate::rotation::{RotationPolicy, SimAction, SimView};
use crate::rules;
use crate::world::World;

/// Mutable per-operator state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpState {
    /// Current morale.
    pub mood: f64,
    /// Maximum morale.
    pub max_mood: f64,
    /// Morale at the start.
    pub initial_mood: f64,
    /// Lowest morale so far.
    pub min_mood: f64,
    /// Hours since entering the current room (ramps key on this).
    pub hours_in_room: f64,
    /// Hours worked above zero morale.
    pub hours_working: f64,
    /// Hours in a Dormitory.
    pub hours_resting: f64,
    /// Hours stationed at zero morale.
    pub hours_exhausted: f64,
    /// Hours stationed but idle.
    pub hours_idle: f64,
    /// Hours off the base.
    pub hours_benched: f64,
}

impl OpState {
    fn new(mood: f64, max_mood: f64) -> Self {
        OpState {
            mood,
            max_mood,
            initial_mood: mood,
            min_mood: mood,
            hours_in_room: 0.0,
            hours_working: 0.0,
            hours_resting: 0.0,
            hours_exhausted: 0.0,
            hours_idle: 0.0,
            hours_benched: 0.0,
        }
    }
}

/// Starting morale under the config's policy.
pub(crate) fn initial_mood(op: &Operator, roster: &Roster, config: &SimConfig) -> f64 {
    match config.initial_mood {
        MoodPolicy::Full => op.max_mood,
        MoodPolicy::Roster => roster
            .get(op.id.as_str())
            .and_then(|e| e.mood)
            .map_or(op.max_mood, |m| m.clamp(0.0, op.max_mood)),
    }
}

const EPS: f64 = 1e-9;

#[derive(Default)]
struct RoomRun {
    /// Factory units in storage.
    stored: f64,
    /// Trading Post orders waiting.
    pending: f64,
    produced: BTreeMap<ItemId, f64>,
    lmd: f64,
    orundum: f64,
    orders: f64,
    drones: f64,
    contacts: f64,
    contacts_stored: f64,
    training_progress: f64,
    training_done_at: Option<f64>,
    stat_hours: f64,
    initial_stat: Option<f64>,
    blocked_hours: f64,
    storage_full: bool,
    queue_full: bool,
    starved: bool,
    input_starved: bool,
}

struct Sim<'a> {
    data: &'a GameData,
    base: &'a BaseConfig,
    roster: &'a Roster,
    config: &'a SimConfig,
    assignment: Assignment,
    states: BTreeMap<OperatorId, OpState>,
    runs: Vec<RoomRun>,
    training_done: Vec<bool>,
    gold: f64,
    shards: f64,
    stock: Option<BTreeMap<ItemId, f64>>,
    totals: Totals,
    events: Vec<SimEvent>,
    trajectory: Vec<MoodSample>,
    warnings: Warnings,
    hour: f64,
}

pub(crate) fn run(
    data: &GameData,
    base: &BaseConfig,
    assignment: &Assignment,
    roster: &Roster,
    config: &SimConfig,
    policy: &mut dyn RotationPolicy,
) -> Result<SimResult, SimError> {
    config.validate()?;
    base.validate(data)?;
    assignment.check(base, data)?;

    let mut sim = Sim {
        data,
        base,
        roster,
        config,
        assignment: assignment.clone(),
        states: BTreeMap::new(),
        runs: base.rooms.iter().map(|_| RoomRun::default()).collect(),
        training_done: vec![false; base.rooms.len()],
        gold: config.initial_gold,
        shards: config.initial_shards,
        stock: config.input_stock.clone(),
        totals: Totals::default(),
        events: Vec::new(),
        trajectory: Vec::new(),
        warnings: Warnings::default(),
        hour: 0.0,
    };
    for id in assignment.operators() {
        sim.ensure_state(id)?;
    }
    for id in policy.extra_operators() {
        if sim.ensure_state(&id).is_err() {
            sim.warnings
                .push(SimWarning::UnknownOperator { operator: id });
        }
    }

    let dt_h = f64::from(config.tick_minutes) / 60.0;
    let horizon = config.horizon_hours;
    let collect_every = match config.collection {
        CollectionPolicy::EveryHours { hours } => hours,
        CollectionPolicy::Continuous => f64::INFINITY,
    };
    let mut next_collection = collect_every;
    let sample_every = u64::from(config.trajectory_every_minutes.max(1));
    let mut tick: u64 = 0;

    sim.sample();
    while sim.hour < horizon - EPS {
        let dt = dt_h.min(horizon - sim.hour);
        let snap = sim.snapshot()?;
        for (ri, rs) in snap.rooms.iter().enumerate() {
            sim.produce(ri, rs, dt);
        }
        if matches!(config.collection, CollectionPolicy::Continuous) {
            sim.fulfil_all();
        }
        sim.advance_mood(&snap, dt);
        sim.hour += dt;
        tick += 1;

        if sim.hour + EPS >= next_collection {
            sim.collect_all();
            sim.events.push(SimEvent {
                hour: sim.hour,
                kind: EventKind::Collected,
            });
            next_collection += collect_every;
        }

        let actions = {
            let view = SimView {
                hour: sim.hour,
                data,
                base,
                assignment: &sim.assignment,
                states: &sim.states,
            };
            policy.next_actions(&view)
        };
        if !actions.is_empty() {
            sim.apply(actions);
        }

        if (tick * u64::from(config.tick_minutes)).is_multiple_of(sample_every) {
            sim.sample();
        }
    }
    Ok(sim.finish())
}

impl Sim<'_> {
    fn ensure_state(&mut self, id: &OperatorId) -> Result<(), SimError> {
        if self.states.contains_key(id) {
            return Ok(());
        }
        let op = self
            .data
            .operators
            .get(id.as_str())
            .ok_or_else(|| SimError::UnknownOperator(id.clone()))?;
        let mood = initial_mood(op, self.roster, self.config);
        self.states
            .insert(id.clone(), OpState::new(mood, op.max_mood));
        Ok(())
    }

    fn snapshot(&mut self) -> Result<Snapshot, SimError> {
        let mut world = World::build(
            self.data,
            self.base,
            &self.assignment,
            self.roster,
            self.config,
            &mut self.warnings,
        )?;
        for (active, done) in world.training_active.iter_mut().zip(&self.training_done) {
            *active &= !done;
        }
        let mood: Vec<f64> = world.ops.iter().map(|o| self.states[o.id].mood).collect();
        let hours: Vec<f64> = world
            .ops
            .iter()
            .map(|o| self.states[o.id].hours_in_room)
            .collect();
        Ok(effects::evaluate(
            &world,
            &Dynamic {
                mood: &mood,
                hours_in_room: &hours,
            },
            &mut self.warnings,
        ))
    }

    fn event(&mut self, kind: EventKind) {
        self.events.push(SimEvent {
            hour: self.hour,
            kind,
        });
    }

    fn produce(&mut self, ri: usize, rs: &RoomStats, dt: f64) {
        let base = self.base;
        let room = &base.rooms[ri];
        if let Some((_, v)) = rs.main_stat() {
            let run = &mut self.runs[ri];
            run.stat_hours += v * dt;
            if run.initial_stat.is_none() {
                run.initial_stat = Some(v);
            }
        }
        match room.kind {
            RoomType::Manufacture => self.produce_factory(ri, rs, dt),
            RoomType::Trading => self.produce_trading(ri, rs, dt),
            RoomType::Power => {
                let drones = rules::drones_per_hour(&self.data.constants) * rs.drone_recovery_pct
                    / 100.0
                    * dt;
                self.runs[ri].drones += drones;
                self.totals.drones += drones;
            }
            RoomType::Hire => self.produce_office(ri, rs, dt),
            RoomType::Training => self.produce_training(ri, rs, dt),
            _ => {}
        }
    }

    fn produce_factory(&mut self, ri: usize, rs: &RoomStats, dt: f64) {
        enum Limit {
            None,
            Input(ItemId),
            Storage,
        }

        let data = self.data;
        let room = &self.base.rooms[ri];
        let Some(formula) = room
            .settings
            .formula
            .as_ref()
            .and_then(|id| data.manufacture_formulas.get(id.as_str()))
        else {
            self.warnings.push(SimWarning::IdleRoom {
                room: room.id.clone(),
                room_kind: room.kind,
            });
            return;
        };
        let time_batches =
            3600.0 / f64::from(formula.cost_point.max(1)) * rs.productivity_pct / 100.0 * dt;
        if time_batches <= 0.0 {
            return;
        }
        if formula.cost_point < rules::INPUT_BOUND_SECONDS && self.stock.is_none() {
            self.warnings.push(SimWarning::InputBoundFormula {
                room: room.id.clone(),
                formula: formula.id.clone(),
            });
            return;
        }

        let mut batches = time_batches;
        let mut limit = Limit::None;
        if let Some(stock) = &self.stock {
            for cost in &formula.costs {
                if cost.item.as_str() == rules::LMD_ITEM {
                    continue;
                }
                let have = stock.get(cost.item.as_str()).copied().unwrap_or(0.0);
                let can = have / f64::from(cost.count.max(1));
                if can < batches {
                    batches = can;
                    limit = Limit::Input(cost.item.clone());
                }
            }
        }
        let count = f64::from(formula.count.max(1));
        let volume = f64::from(formula.weight.max(1));
        if matches!(self.config.collection, CollectionPolicy::EveryHours { .. }) {
            let space_units = ((rs.capacity - self.runs[ri].stored * volume) / volume).max(0.0);
            let can = space_units / count;
            if can < batches {
                batches = can;
                limit = Limit::Storage;
            }
        }
        let batches = batches.max(0.0);

        for cost in &formula.costs {
            let used = batches * f64::from(cost.count);
            *self.totals.consumed.entry(cost.item.clone()).or_default() += used;
            if cost.item.as_str() == rules::LMD_ITEM {
                self.totals.lmd_spent += used;
            } else if let Some(stock) = &mut self.stock {
                let left = stock.entry(cost.item.clone()).or_default();
                *left = (*left - used).max(0.0);
            }
        }
        let units = batches * count;
        match self.config.collection {
            CollectionPolicy::Continuous => self.deliver(ri, &formula.item, units),
            CollectionPolicy::EveryHours { .. } => self.runs[ri].stored += units,
        }

        let lost = 1.0 - batches / time_batches;
        let run = &mut self.runs[ri];
        if lost > 1e-9 {
            run.blocked_hours += dt * lost;
        }
        let storage_now = matches!(limit, Limit::Storage);
        let newly_full = storage_now && !run.storage_full;
        let newly_starved = matches!(limit, Limit::Input(_)) && !run.input_starved;
        run.storage_full |= storage_now;
        run.input_starved = matches!(limit, Limit::Input(_));
        if newly_full {
            self.event(EventKind::StorageFull {
                room: room.id.clone(),
            });
        }
        if newly_starved && let Limit::Input(item) = limit {
            self.event(EventKind::InputStarved {
                room: room.id.clone(),
                item,
            });
        }
    }

    fn produce_trading(&mut self, ri: usize, rs: &RoomStats, dt: f64) {
        let room = &self.base.rooms[ri];
        let mix = rules::order_mix(room.strategy(), room.level);
        let per_hour = rs.order_efficiency_pct / 100.0 * 60.0 / mix.minutes.max(1e-9);
        let new = per_hour * dt;
        let run = &mut self.runs[ri];
        let space = (rs.order_limit - run.pending).max(0.0);
        let added = new.min(space);
        run.pending += added;
        let mut newly_full = false;
        if new > 0.0 && added < new - 1e-12 {
            run.blocked_hours += dt * (1.0 - added / new);
            if !run.queue_full {
                run.queue_full = true;
                newly_full = true;
            }
        }
        if newly_full {
            self.event(EventKind::OrderQueueFull {
                room: room.id.clone(),
            });
        }
    }

    fn produce_office(&mut self, ri: usize, rs: &RoomStats, dt: f64) {
        let new = rs.contact_speed_pct / 100.0 / rules::CONTACT_BASE_HOURS * dt;
        match self.config.collection {
            CollectionPolicy::Continuous => {
                self.runs[ri].contacts += new;
                self.totals.contacts += new;
            }
            CollectionPolicy::EveryHours { .. } => {
                let run = &mut self.runs[ri];
                let put = new.min((rules::CONTACT_CAP - run.contacts_stored).max(0.0));
                run.contacts_stored += put;
                let mut newly_full = false;
                if new > 0.0 && put < new - 1e-12 {
                    run.blocked_hours += dt * (1.0 - put / new);
                    if !run.storage_full {
                        run.storage_full = true;
                        newly_full = true;
                    }
                }
                if newly_full {
                    let id = self.base.rooms[ri].id.clone();
                    self.event(EventKind::StorageFull { room: id });
                }
            }
        }
    }

    fn produce_training(&mut self, ri: usize, rs: &RoomStats, dt: f64) {
        let room = &self.base.rooms[ri];
        let Some(job) = &room.settings.training else {
            self.warnings.push(SimWarning::IdleRoom {
                room: room.id.clone(),
                room_kind: room.kind,
            });
            return;
        };
        if self.training_done[ri] {
            return;
        }
        let target = rules::spec_base_hours(job.spec_level);
        let rate = rs.training_speed_pct / 100.0;
        let run = &mut self.runs[ri];
        let need = target - run.training_progress;
        if rate * dt + EPS >= need {
            let when = self.hour + if rate > 0.0 { need / rate } else { dt };
            run.training_progress = target;
            run.training_done_at = Some(when);
            self.training_done[ri] = true;
            self.events.push(SimEvent {
                hour: when,
                kind: EventKind::TrainingCompleted {
                    room: room.id.clone(),
                },
            });
        } else {
            run.training_progress += rate * dt;
        }
    }

    /// Fulfils pending orders from the depot. When stock is short it is
    /// shared among Trading Posts in proportion to what their pending
    /// orders need, not handed out in room order.
    fn fulfil_all(&mut self) {
        for strategy in [TradingStrategy::Gold, TradingStrategy::OriginiumShard] {
            let posts: Vec<(usize, rules::OrderMix)> = self
                .base
                .rooms
                .iter()
                .enumerate()
                .filter(|(ri, r)| {
                    r.kind == RoomType::Trading
                        && r.strategy() == strategy
                        && self.runs[*ri].pending > 1e-12
                })
                .map(|(ri, r)| (ri, rules::order_mix(strategy, r.level)))
                .collect();
            if posts.is_empty() {
                continue;
            }
            let (stock, item) = match strategy {
                TradingStrategy::Gold => (self.gold, rules::GOLD_ITEM),
                TradingStrategy::OriginiumShard => (self.shards, rules::SHARD_ITEM),
            };
            let demand: f64 = posts
                .iter()
                .map(|(ri, mix)| self.runs[*ri].pending * mix.input)
                .sum();
            let share = if demand <= stock + EPS {
                1.0
            } else {
                stock / demand
            };
            for (ri, mix) in posts {
                let run = &mut self.runs[ri];
                let done = run.pending * share;
                run.pending -= done;
                run.orders += done;
                let consumed = done * mix.input;
                let output = done * mix.output;
                match strategy {
                    TradingStrategy::Gold => {
                        run.lmd += output;
                        self.gold = (self.gold - consumed).max(0.0);
                        self.totals.lmd += output;
                        self.totals.gold_consumed += consumed;
                    }
                    TradingStrategy::OriginiumShard => {
                        run.orundum += output;
                        self.shards = (self.shards - consumed).max(0.0);
                        self.totals.orundum += output;
                    }
                }
                self.totals.orders_completed += done;
                let starved = share < 1.0 - EPS;
                let was_starved = std::mem::replace(&mut run.starved, starved);
                if starved && !was_starved {
                    let room = self.base.rooms[ri].id.clone();
                    self.event(EventKind::InputStarved {
                        room,
                        item: ItemId::new(item),
                    });
                }
            }
        }
    }

    /// Books Factory output into the depot and totals.
    fn deliver(&mut self, ri: usize, item: &ItemId, units: f64) {
        if units <= 0.0 {
            return;
        }
        *self.runs[ri].produced.entry(item.clone()).or_default() += units;
        *self.totals.items.entry(item.clone()).or_default() += units;
        match item.as_str() {
            rules::GOLD_ITEM => {
                self.gold += units;
                self.totals.gold_produced += units;
            }
            rules::SHARD_ITEM => self.shards += units,
            other => {
                if let Some(v) = rules::exp_value(other) {
                    self.totals.exp += units * v;
                }
            }
        }
    }

    fn collect_all(&mut self) {
        let base = self.base;
        let data = self.data;
        for (ri, room) in base.rooms.iter().enumerate() {
            match room.kind {
                RoomType::Manufacture => {
                    let stored = std::mem::take(&mut self.runs[ri].stored);
                    self.runs[ri].storage_full = false;
                    if let Some(f) = room
                        .settings
                        .formula
                        .as_ref()
                        .and_then(|id| data.manufacture_formulas.get(id.as_str()))
                    {
                        self.deliver(ri, &f.item, stored);
                    }
                }
                RoomType::Hire => {
                    let run = &mut self.runs[ri];
                    let got = std::mem::take(&mut run.contacts_stored);
                    run.contacts += got;
                    run.storage_full = false;
                    self.totals.contacts += got;
                }
                _ => {}
            }
        }
        self.fulfil_all();
        for (ri, room) in base.rooms.iter().enumerate() {
            if room.kind == RoomType::Trading {
                self.runs[ri].queue_full = false;
            }
        }
    }

    fn advance_mood(&mut self, snap: &Snapshot, dt: f64) {
        let mut seen: BTreeSet<&OperatorId> = BTreeSet::new();
        for m in &snap.mood {
            seen.insert(&m.operator);
            let Some(st) = self.states.get_mut(&m.operator) else {
                continue;
            };
            let before = st.mood;
            let after = (before + m.total * dt).clamp(0.0, st.max_mood);
            st.mood = after;
            st.min_mood = st.min_mood.min(after);
            st.hours_in_room += dt;
            if m.idle {
                st.hours_idle += dt;
            } else if m.kind.is_work_area() {
                if m.exhausted {
                    st.hours_exhausted += dt;
                } else {
                    st.hours_working += dt;
                }
            } else if m.kind == RoomType::Dormitory {
                st.hours_resting += dt;
            }
            if !m.idle && m.kind.is_work_area() && before > EPS && after <= EPS {
                self.events.push(SimEvent {
                    hour: self.hour + dt,
                    kind: EventKind::Exhausted {
                        operator: m.operator.clone(),
                        room: m.room.clone(),
                    },
                });
            }
        }
        for (id, st) in &mut self.states {
            if !seen.contains(id) {
                st.hours_benched += dt;
            }
        }
    }

    fn apply(&mut self, actions: Vec<SimAction>) {
        for action in actions {
            match action {
                SimAction::Move { operator, to } => {
                    if self.ensure_state(&operator).is_err() {
                        self.warnings.push(SimWarning::UnknownOperator { operator });
                        continue;
                    }
                    let from = self.assignment.locate(operator.as_str());
                    match self.assignment.move_to(&operator, &to) {
                        Ok(displaced) => {
                            if let Some(st) = self.states.get_mut(&operator) {
                                st.hours_in_room = 0.0;
                            }
                            if let Some(d) = displaced {
                                if let Some(st) = self.states.get_mut(&d) {
                                    st.hours_in_room = 0.0;
                                }
                                self.event(EventKind::Benched {
                                    operator: d,
                                    from: to.clone(),
                                });
                            }
                            self.event(EventKind::Moved { operator, from, to });
                        }
                        Err(e) => self.warnings.push(SimWarning::RotationRefused {
                            operator,
                            reason: e.to_string(),
                        }),
                    }
                }
                SimAction::Bench { operator } => {
                    if let Some(from) = self.assignment.locate(operator.as_str()) {
                        self.assignment.remove(&from);
                        if let Some(st) = self.states.get_mut(&operator) {
                            st.hours_in_room = 0.0;
                        }
                        self.event(EventKind::Benched { operator, from });
                    }
                }
            }
        }
    }

    fn sample(&mut self) {
        for (id, st) in &self.states {
            self.trajectory.push(MoodSample {
                hour: self.hour,
                operator: id.clone(),
                mood: st.mood,
            });
        }
    }

    fn finish(self) -> SimResult {
        let horizon = self.config.horizon_hours;
        let mut totals = self.totals;
        totals.gold_in_depot = self.gold;
        totals.shards_in_depot = self.shards;
        let assignment = &self.assignment;
        let rooms = self
            .base
            .rooms
            .iter()
            .zip(self.runs)
            .map(|(room, run)| RoomReport {
                id: room.id.clone(),
                kind: room.kind,
                level: room.level,
                operators: assignment.occupants(room.id.as_str()).cloned().collect(),
                initial_stat_pct: run.initial_stat,
                average_stat_pct: run.initial_stat.map(|_| run.stat_hours / horizon),
                produced: run.produced,
                lmd: run.lmd,
                orundum: run.orundum,
                orders_completed: run.orders,
                drones: run.drones,
                hours_blocked: run.blocked_hours,
                in_storage: if room.kind == RoomType::Hire {
                    run.contacts_stored
                } else {
                    run.stored
                },
                pending_orders: run.pending,
                contacts: run.contacts,
                training_progress_hours: run.training_progress,
                training_completed_hour: run.training_done_at,
            })
            .collect();
        let operators = self
            .states
            .iter()
            .map(|(id, st)| OperatorReport {
                id: id.clone(),
                name: self
                    .data
                    .operators
                    .get(id.as_str())
                    .map(|o| o.name.clone())
                    .unwrap_or_default(),
                initial_mood: st.initial_mood,
                final_mood: st.mood,
                min_mood: st.min_mood,
                hours_working: st.hours_working,
                hours_resting: st.hours_resting,
                hours_exhausted: st.hours_exhausted,
                hours_benched: st.hours_benched,
                hours_idle: st.hours_idle,
            })
            .collect();
        SimResult {
            data: self.data.version.clone(),
            horizon_hours: horizon,
            tick_minutes: self.config.tick_minutes,
            totals,
            rooms,
            operators,
            trajectory: self.trajectory,
            events: self.events,
            warnings: self.warnings.into_vec(),
        }
    }
}
