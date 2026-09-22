//! Game rules the evaluator needs, with provenance for every number that
//! does not come from the upstream data files.
//!
//! Everything derivable from `building_data.json` is read from
//! [`GameConstants`] and the facility tables at run time. The rest is
//! community-documented and deliberately concentrated here so it can be
//! audited or overridden:
//!
//! - **Trading Post order table.** Upstream ships an empty `shopFormulas`,
//!   so the order kinds, base times and per-level odds come from the PRTS
//!   wiki (贸易站) and arknights.wiki.gg (Trading Post), which agree.
//! - **Battle Record EXP.** `item_table.json` is not ingested; the values
//!   below were checked against its `expItems` at the pinned commit.
//! - **Office and Training Room timings** (PRTS 办公室, 训练室).
//!
//! Unit note: upstream stores morale in "manpower" units with
//! `manpowerDisplayFactor` (360 000) units per displayed morale point. Every
//! per-second rate in the data (`manpowerCost`, `manpowerRecover`,
//! `basicCostBuff`, `…ManpowerCostByNum`, ambience ÷
//! `comfortManpowerRecoverFactor`) converts to morale per hour by
//! `× 3600 ÷ 360 000`, so 100 units/s is exactly 1 morale per hour.
//!
//! Cross-checks against published numbers:
//!
//! | Rule | Data | Published |
//! | --- | --- | --- |
//! | Work drain | `manpowerCost` 100 | 1.0/h (PRTS 制造站, 贸易站, 发电站) |
//! | 2 / 3 operators in a Factory or Trading Post | `ByNum` −5 / −10 | −0.05 / −0.10 per hour each; exhausted operators still count (PRTS 制造站) |
//! | Each Control Center operator | `basicCostBuff` −5 | −0.05/h to every working operator (wiki.gg Control Center) |
//! | Dormitory level 1..5 | `manpowerRecover` 160..200 | 1.6..2.0/h (wiki.gg Dormitory) |
//! | Ambience | ÷ 25 | +0.0004/h per point, +2.0/h at 5000 (wiki.gg Dormitory) |
//! | Per-operator base bonus | `basicSpeedBuff` | +1% Factory and Trading Post (PRTS), +5% Power Plant (PRTS 发电站), +5% Office (PRTS 办公室), +5% Training Room assistant (PRTS 训练室), +5% Reception Room (PRTS 会客室, which adds rarity and promotion bonuses not modelled here) |
//! | Production time | formula `costPoint` seconds | Pure Gold 1:12, Drill 0:45, Frontline 1:20, Tactical 3:00, Shard 1:00 (wiki.gg Factory/Production) |
//! | Factory storage | `outputCapacity` 24/36/54, formula `weight` | storage is volume; Pure Gold and Drill take 2, Frontline 3, Tactical 5, Dualchips 5 (PRTS 制造站) |
//! | Drones | `laborRecoverTime` 360 s | 6 min per drone at 100% (PRTS 发电站) |
//! | Office | none | 12 h per contact at 100%, 3 stored at most (PRTS 办公室) |
//! | Training | none | Specialisation 1/2/3 take 8/16/24 h at 100%; only the assistant's skills apply; the trainee does not drain morale; the assistant drains only while training runs (PRTS 训练室) |
//!
//! Known gap: the Reception Room also produces clues (20 h base per clue,
//! PRTS 会客室), but its speed depends on per-operator rarity and promotion,
//! on total Dormitory ambience and on room level. Those are not modelled,
//! so clue output is not simulated.

use ak_domain::{GameConstants, GameData, Room, RoomType, TradingStrategy};

/// One kind of Trading Post order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderKind {
    /// Units of the input item (Pure Gold or Originium Shards) consumed.
    pub input: f64,
    /// LMD or Orundum produced.
    pub output: f64,
    /// Base acquisition time at 100% efficiency.
    pub minutes: f64,
}

/// Pure Gold → LMD orders, low to high yield.
pub const GOLD_ORDERS: [OrderKind; 3] = [
    OrderKind {
        input: 2.0,
        output: 1000.0,
        minutes: 144.0,
    },
    OrderKind {
        input: 3.0,
        output: 1500.0,
        minutes: 210.0,
    },
    OrderKind {
        input: 4.0,
        output: 2000.0,
        minutes: 276.0,
    },
];

/// Probability of each [`GOLD_ORDERS`] kind by Trading Post level (index
/// `level − 1`). Skills that "increase the chance of higher-yield orders"
/// are parsed as `Unmodeled` and reported, not applied.
pub const GOLD_ORDER_WEIGHTS: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.6, 0.4, 0.0], [0.3, 0.5, 0.2]];

/// Originium Shards → Orundum. One kind at every level.
pub const SHARD_ORDER: OrderKind = OrderKind {
    input: 2.0,
    output: 20.0,
    minutes: 120.0,
};

/// Expected values of one order under a strategy at a level. The order
/// stream is a renewal process, so long-run throughput at 100% efficiency
/// is `output / minutes` per minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderMix {
    /// Expected input units per order.
    pub input: f64,
    /// Expected output per order.
    pub output: f64,
    /// Expected base minutes per order.
    pub minutes: f64,
}

/// The order mix for a strategy at a Trading Post level (clamped to 1..=3).
pub fn order_mix(strategy: TradingStrategy, level: u8) -> OrderMix {
    match strategy {
        TradingStrategy::Gold => {
            let weights = GOLD_ORDER_WEIGHTS[usize::from(level.clamp(1, 3) - 1)];
            let mut mix = OrderMix {
                input: 0.0,
                output: 0.0,
                minutes: 0.0,
            };
            for (kind, p) in GOLD_ORDERS.iter().zip(weights) {
                mix.input += p * kind.input;
                mix.output += p * kind.output;
                mix.minutes += p * kind.minutes;
            }
            mix
        }
        TradingStrategy::OriginiumShard => OrderMix {
            input: SHARD_ORDER.input,
            output: SHARD_ORDER.output,
            minutes: SHARD_ORDER.minutes,
        },
    }
}

/// Upstream item id of Pure Gold.
pub const GOLD_ITEM: &str = "3003";
/// Upstream item id of Originium Shards.
pub const SHARD_ITEM: &str = "3141";
/// Upstream item id of LMD. As a Factory input it is never limited; it is
/// reported as LMD spent.
pub const LMD_ITEM: &str = "4001";

/// Formulas faster than this many seconds per batch (Dualchips take one
/// second) are bounded by their inputs, not by time.
pub const INPUT_BOUND_SECONDS: u32 = 60;

/// Base hours per Office contact at 100% speed.
pub const CONTACT_BASE_HOURS: f64 = 12.0;

/// Contacts the Office stores before it stops accumulating.
pub const CONTACT_CAP: f64 = 3.0;

/// Base hours to train a skill to specialisation `level` at 100% speed.
pub fn spec_base_hours(level: u8) -> f64 {
    match level {
        1 => 8.0,
        2 => 16.0,
        _ => 24.0,
    }
}

/// EXP granted by one Battle Record, by item id.
pub fn exp_value(item: &str) -> Option<f64> {
    match item {
        "2001" => Some(200.0),
        "2002" => Some(400.0),
        "2003" => Some(1000.0),
        "2004" => Some(2000.0),
        _ => None,
    }
}

/// Converts an upstream per-second manpower rate to morale per hour.
pub fn units_to_mood_per_hour(units_per_sec: f64, c: &GameConstants) -> f64 {
    units_per_sec * 3600.0 / f64::from(c.manpower_display_factor.max(1))
}

/// Per-operator base efficiency bonus of a room kind (fraction, e.g. 0.01).
pub fn basic_speed_buff(c: &GameConstants, kind: RoomType) -> f64 {
    match kind {
        RoomType::Manufacture => c.manufacture.basic_speed_buff,
        RoomType::Trading => c.trading.basic_speed_buff,
        RoomType::Power => c.power.basic_speed_buff,
        RoomType::Meeting => c.meeting.basic_speed_buff,
        RoomType::Hire => c.hire.basic_speed_buff,
        RoomType::Training => c.training.basic_speed_buff,
        _ => 0.0,
    }
}

/// Upstream `manpowerCost` of a room kind at a level, in units per second.
pub fn manpower_cost_units(data: &GameData, kind: RoomType, level: u8) -> f64 {
    data.facility(kind)
        .and_then(|f| f.phase(level))
        .map_or(0.0, |p| f64::from(p.manpower_cost))
}

/// Upstream `…ManpowerCostByNum` relief for a headcount (negative units).
pub fn headcount_relief_units(c: &GameConstants, kind: RoomType, headcount: usize) -> f64 {
    let table = match kind {
        RoomType::Manufacture => &c.manufact_manpower_cost_by_num,
        RoomType::Trading => &c.trading_manpower_cost_by_num,
        _ => return 0.0,
    };
    match table.last() {
        None => 0.0,
        Some(_) => f64::from(table[headcount.min(table.len() - 1)]),
    }
}

/// Upstream `basicCostBuff` × active Control Center operators (negative
/// units), applied to every working operator in the base.
pub fn control_relief_units(c: &GameConstants, active_cc: usize) -> f64 {
    f64::from(c.control.basic_cost_buff) * active_cc as f64
}

/// Dormitory recovery in units per second: level base plus ambience.
pub fn dorm_recovery_units(c: &GameConstants, level: u8, ambience: u32) -> f64 {
    let base = level
        .checked_sub(1)
        .and_then(|i| c.dormitory.phases.get(usize::from(i)))
        .map_or(0, |p| p.manpower_recover);
    f64::from(base) + f64::from(ambience) / f64::from(c.comfort_manpower_recover_factor.max(1))
}

/// Base morale rate of a working or resting operator in a room, in morale
/// per hour, before any skill: negative while working, positive while
/// resting, zero in a Workshop (which has no `manpowerCost`). The caller
/// decides whether the operator is working at all (see
/// [`crate::world::Role`]).
pub fn base_mood_rate(data: &GameData, room: &Room, headcount: usize, active_cc: usize) -> f64 {
    let c = &data.constants;
    match room.kind {
        RoomType::Dormitory => units_to_mood_per_hour(
            dorm_recovery_units(c, room.level, room.settings.ambience),
            c,
        ),
        kind if kind.is_work_area() => {
            let cost = manpower_cost_units(data, kind, room.level);
            if cost <= 0.0 {
                return 0.0;
            }
            let units = cost
                + headcount_relief_units(c, kind, headcount)
                + control_relief_units(c, active_cc);
            -units_to_mood_per_hour(units, c)
        }
        _ => 0.0,
    }
}

/// Factory output storage at a level, in volume units.
pub fn output_capacity(c: &GameConstants, level: u8) -> f64 {
    level
        .checked_sub(1)
        .and_then(|i| c.manufacture.phases.get(usize::from(i)))
        .map_or(0.0, |p| f64::from(p.output_capacity))
}

/// Trading Post order queue limit at a level.
pub fn order_limit(c: &GameConstants, level: u8) -> f64 {
    level
        .checked_sub(1)
        .and_then(|i| c.trading.phases.get(usize::from(i)))
        .map_or(0.0, |p| f64::from(p.order_limit))
}

/// Drones charged per hour by one Power Plant at 100% (upstream
/// `laborRecoverTime` seconds per drone).
pub fn drones_per_hour(c: &GameConstants) -> f64 {
    3600.0 / f64::from(c.labor_recover_time.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_three_gold_mix_matches_published_odds() {
        let m = order_mix(TradingStrategy::Gold, 3);
        assert!((m.input - 2.9).abs() < 1e-9);
        assert!((m.output - 1450.0).abs() < 1e-9);
        assert!((m.minutes - 203.4).abs() < 1e-9);
        let l1 = order_mix(TradingStrategy::Gold, 1);
        assert_eq!(l1.output, 1000.0);
        assert_eq!(l1.minutes, 144.0);
    }

    #[test]
    fn weights_sum_to_one() {
        for w in GOLD_ORDER_WEIGHTS {
            assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn specialisation_times() {
        assert_eq!(spec_base_hours(1), 8.0);
        assert_eq!(spec_base_hours(2), 16.0);
        assert_eq!(spec_base_hours(3), 24.0);
    }
}
