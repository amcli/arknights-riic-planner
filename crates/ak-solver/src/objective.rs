//! What "better" means: a weighted sum of what the base produces, and how
//! to estimate it quickly.
//!
//! Two scorers share one [`Breakdown`]:
//!
//! - [`Breakdown::from_result`] reads a finished simulation;
//! - [`Breakdown::proxy`] extrapolates an instantaneous [`Snapshot`] over
//!   the horizon. It uses the same production rules as the simulator (see
//!   `ak_eval::rules`), a shared depot with the trading posts limited to
//!   the gold and shards the factories make, and, without rotation, each
//!   operator's contribution stopping when their morale would hit zero. It
//!   ignores the order queue, storage limits and input stock, so it is a
//!   ranking device, not a report: the finalists are re-scored by the
//!   simulator.

use std::collections::HashMap;

use ak_domain::{BaseConfig, GameData, OperatorId, RoomType, Roster, TradingStrategy};
use ak_eval::rules;
use ak_eval::{MoodPolicy, Rotation, SimConfig, SimResult, Snapshot};
use serde::{Deserialize, Serialize};

/// Weights per unit of each output. Defaults are cost-based where a cost
/// exists:
///
/// - LMD and EXP at 1 each (a Pure Gold order pays 500 LMD per gold, and
///   leveling spends the two roughly in step);
/// - Orundum at 200: 20 Orundum cost two Originium Shards, and each Shard
///   costs 1600 LMD plus an hour of Factory time that could have made 0.83
///   Pure Gold (417 LMD), so about 4000 LMD per 20;
/// - drones at 20: one drone shortens a Factory's current batch by three
///   minutes (`manufactReduceTimeUnit`), a twenty-fourth of a Pure Gold;
/// - contacts, training and unsold gold at 0, because they have no LMD
///   price. Give them one if you want the solver to care.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Objective {
    /// Per LMD earned.
    pub lmd: f64,
    /// Per EXP produced.
    pub exp: f64,
    /// Per Orundum earned.
    pub orundum: f64,
    /// Per drone charged.
    pub drones: f64,
    /// Per Office contact gained.
    pub contacts: f64,
    /// Per base hour of specialisation training completed.
    pub training_hours: f64,
    /// Per Pure Gold produced and not sold within the horizon.
    pub gold: f64,
    /// Penalty per operator-hour spent stationed at zero morale.
    pub exhausted_hour: f64,
}

impl Default for Objective {
    fn default() -> Self {
        Objective {
            lmd: 1.0,
            exp: 1.0,
            orundum: 200.0,
            drones: 20.0,
            contacts: 0.0,
            training_hours: 0.0,
            gold: 0.0,
            exhausted_hour: 0.0,
        }
    }
}

/// The quantities an objective weighs, over the horizon.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Breakdown {
    /// LMD earned.
    pub lmd: f64,
    /// EXP produced.
    pub exp: f64,
    /// Orundum earned.
    pub orundum: f64,
    /// Drones charged.
    pub drones: f64,
    /// Office contacts gained.
    pub contacts: f64,
    /// Base hours of training completed.
    pub training_hours: f64,
    /// Pure Gold produced minus Pure Gold sold.
    pub gold_net: f64,
    /// Operator-hours at zero morale.
    pub exhausted_hours: f64,
}

impl Objective {
    /// The weighted sum.
    pub fn value(&self, b: &Breakdown) -> f64 {
        self.lmd * b.lmd
            + self.exp * b.exp
            + self.orundum * b.orundum
            + self.drones * b.drones
            + self.contacts * b.contacts
            + self.training_hours * b.training_hours
            + self.gold * b.gold_net
            - self.exhausted_hour * b.exhausted_hours
    }
}

impl Breakdown {
    /// From a finished simulation.
    pub fn from_result(r: &SimResult) -> Self {
        Breakdown {
            lmd: r.totals.lmd,
            exp: r.totals.exp,
            orundum: r.totals.orundum,
            drones: r.totals.drones,
            contacts: r.totals.contacts,
            training_hours: r.rooms.iter().map(|x| x.training_progress_hours).sum(),
            gold_net: r.totals.gold_produced - r.totals.gold_consumed,
            exhausted_hours: r.operators.iter().map(|o| o.hours_exhausted).sum(),
        }
    }

    /// Extrapolates an instantaneous snapshot over the horizon (see the
    /// module docs).
    ///
    /// Morale is priced as time, not as a weight. Without rotation, an
    /// operator whose morale would reach zero stops contributing from then
    /// on. With rotation (`shifts`), they work until `swap_out`, then
    /// alternate work and rest; each point of morale spent must be
    /// recovered at the Dormitory rate `recovery`, so after the first stint
    /// the operator is at work for a fraction `recovery / (recovery +
    /// drain)` of the time. The substitute who covers the slot meanwhile is
    /// assumed to add nothing beyond the base bonus; the simulator scores
    /// the finalists with the real substitutes.
    pub fn proxy(
        snapshot: &Snapshot,
        data: &GameData,
        base: &BaseConfig,
        roster: &Roster,
        config: &SimConfig,
        shifts: Option<Shifts>,
    ) -> Self {
        let h = config.horizon_hours;
        let c = &data.constants;

        // Hours each operator keeps contributing.
        let mut active_hours: HashMap<&str, f64> = HashMap::new();
        let mut exhausted_hours = 0.0;
        for m in &snapshot.mood {
            let working = m.kind.is_work_area() && !m.idle;
            let hours = if m.idle || m.exhausted {
                0.0
            } else if !working || m.total >= -1e-12 {
                h
            } else {
                let drain = -m.total;
                let start = initial_mood(data, roster, config, &m.operator);
                match shifts {
                    None => (start / drain).min(h),
                    Some(s) => {
                        let first = ((start - s.swap_out).max(0.0) / drain).min(h);
                        let duty = s.recovery / (s.recovery + drain);
                        first + (h - first) * duty
                    }
                }
            };
            if working && shifts.is_none() {
                exhausted_hours += h - hours;
            }
            active_hours.insert(m.operator.as_str(), hours);
        }

        let mut b = Breakdown::default();
        let mut gold_supply = 0.0;
        let mut shard_supply = 0.0;
        let (mut gold_in, mut gold_out) = (0.0, 0.0);
        let (mut shard_in, mut shard_out) = (0.0, 0.0);

        for (room, rs) in base.rooms.iter().zip(&snapshot.rooms) {
            let Some((stat, _)) = rs.main_stat() else {
                continue;
            };
            let bonus = rules::basic_speed_buff(c, rs.kind) * 100.0;
            let mut stat_hours = 100.0 * h;
            for m in &snapshot.mood {
                if m.room == rs.id && !m.idle && !m.exhausted {
                    stat_hours +=
                        bonus * active_hours.get(m.operator.as_str()).copied().unwrap_or(h);
                }
            }
            for ct in &snapshot.contributions {
                if ct.room == rs.id && ct.stat == stat {
                    stat_hours +=
                        ct.value * active_hours.get(ct.operator.as_str()).copied().unwrap_or(h);
                }
            }
            let stat_hours = stat_hours.max(0.0);
            match rs.kind {
                RoomType::Manufacture => {
                    let Some(formula) = room
                        .settings
                        .formula
                        .as_ref()
                        .and_then(|id| data.manufacture_formulas.get(id.as_str()))
                    else {
                        continue;
                    };
                    if formula.cost_point < rules::INPUT_BOUND_SECONDS {
                        continue;
                    }
                    let units = f64::from(formula.count) * 3600.0
                        / f64::from(formula.cost_point.max(1))
                        * stat_hours
                        / 100.0;
                    match formula.item.as_str() {
                        rules::GOLD_ITEM => gold_supply += units,
                        rules::SHARD_ITEM => shard_supply += units,
                        other => {
                            if let Some(v) = rules::exp_value(other) {
                                b.exp += units * v;
                            }
                        }
                    }
                }
                RoomType::Trading => {
                    let mix = rules::order_mix(room.strategy(), room.level);
                    let orders = 60.0 / mix.minutes.max(1e-9) * stat_hours / 100.0;
                    match room.strategy() {
                        TradingStrategy::Gold => {
                            gold_in += orders * mix.input;
                            gold_out += orders * mix.output;
                        }
                        TradingStrategy::OriginiumShard => {
                            shard_in += orders * mix.input;
                            shard_out += orders * mix.output;
                        }
                    }
                }
                RoomType::Power => b.drones += rules::drones_per_hour(c) * stat_hours / 100.0,
                RoomType::Hire => b.contacts += stat_hours / 100.0 / rules::CONTACT_BASE_HOURS,
                RoomType::Training => {
                    if let Some(job) = &room.settings.training {
                        b.training_hours +=
                            (stat_hours / 100.0).min(rules::spec_base_hours(job.spec_level));
                    }
                }
                _ => {}
            }
        }

        let sold_gold = (gold_supply + config.initial_gold).min(gold_in);
        if gold_in > 0.0 {
            b.lmd = sold_gold * gold_out / gold_in;
        }
        let sold_shards = (shard_supply + config.initial_shards).min(shard_in);
        if shard_in > 0.0 {
            b.orundum = sold_shards * shard_out / shard_in;
        }
        b.gold_net = gold_supply - sold_gold;
        b.exhausted_hours = exhausted_hours;
        b
    }
}

/// How the proxy prices morale when a shift policy is in force.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shifts {
    /// Morale per hour the best Dormitory in the base restores before
    /// skills.
    pub recovery: f64,
    /// Morale at which a worker is sent to rest.
    pub swap_out: f64,
}

impl Shifts {
    /// The shift model for a rotation, or `None` when there is no rotation
    /// or no Dormitory to rotate through.
    pub fn for_rotation(data: &GameData, base: &BaseConfig, rotation: &Rotation) -> Option<Self> {
        let Rotation::MoodThreshold { swap_out, .. } = rotation else {
            return None;
        };
        let recovery = base
            .rooms_of(RoomType::Dormitory)
            .map(|room| rules::base_mood_rate(data, room, 0, 0))
            .fold(f64::NEG_INFINITY, f64::max);
        (recovery > 0.0).then_some(Shifts {
            recovery,
            swap_out: *swap_out,
        })
    }
}

/// Starting morale under the config's policy (mirrors the simulator).
fn initial_mood(data: &GameData, roster: &Roster, config: &SimConfig, op: &OperatorId) -> f64 {
    let max = data.operators.get(op.as_str()).map_or(24.0, |o| o.max_mood);
    match config.initial_mood {
        MoodPolicy::Full => max,
        MoodPolicy::Roster => roster
            .get(op.as_str())
            .and_then(|e| e.mood)
            .map_or(max, |m| m.clamp(0.0, max)),
    }
}
