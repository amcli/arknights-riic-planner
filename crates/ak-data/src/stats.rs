//! Summary numbers about a loaded dataset, for the README, the CLI, and the
//! `/gamedata/version` endpoint.

use std::collections::{BTreeMap, BTreeSet};

use ak_domain::{GameData, Rarity, RoomType};

/// Counts describing a [`GameData`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DataStats {
    /// Operators with base skills.
    pub operators: usize,
    /// Operators by rarity.
    pub operators_by_rarity: BTreeMap<Rarity, usize>,
    /// Skill tiers (entries in `buffs`).
    pub skill_tiers: usize,
    /// Distinct skill families (ids with the tier suffix removed).
    pub skill_families: usize,
    /// Skill tiers by the room they apply in.
    pub skill_tiers_by_room: BTreeMap<RoomType, usize>,
    /// Skill tiers whose description parsed into fully-modelled mechanics.
    pub mechanics_parsed: usize,
    /// Skill tiers that parsed but contain unmodeled parts.
    pub mechanics_partial: usize,
    /// Skill tiers the description parser rejected.
    pub mechanics_unparsed: usize,
    /// `mechanics_parsed` as a percentage of all tiers.
    pub mechanics_coverage_pct: f64,
    /// Nations, groups, and teams.
    pub powers: usize,
    /// Room kinds.
    pub facilities: usize,
    /// Factory formulas.
    pub manufacture_formulas: usize,
    /// Layout slots.
    pub layout_slots: usize,
}

/// Computes stats for a dataset.
pub fn compute(data: &GameData) -> DataStats {
    let mut operators_by_rarity = BTreeMap::new();
    for op in data.operators.values() {
        *operators_by_rarity.entry(op.rarity).or_insert(0) += 1;
    }
    let mut skill_tiers_by_room = BTreeMap::new();
    let mut families = BTreeSet::new();
    let (mut parsed, mut partial, mut unparsed) = (0usize, 0usize, 0usize);
    for skill in data.skills.values() {
        *skill_tiers_by_room.entry(skill.room_type).or_insert(0) += 1;
        families.insert(skill.family().to_owned());
        match &skill.mechanics {
            Some(m) if m.is_fully_modeled() => parsed += 1,
            Some(_) => partial += 1,
            None => unparsed += 1,
        }
    }
    let tiers = data.skills.len();
    DataStats {
        operators: data.operators.len(),
        operators_by_rarity,
        skill_tiers: tiers,
        skill_families: families.len(),
        skill_tiers_by_room,
        mechanics_parsed: parsed,
        mechanics_partial: partial,
        mechanics_unparsed: unparsed,
        mechanics_coverage_pct: if tiers == 0 {
            0.0
        } else {
            100.0 * parsed as f64 / tiers as f64
        },
        powers: data.powers.len(),
        facilities: data.facilities.len(),
        manufacture_formulas: data.manufacture_formulas.len(),
        layout_slots: data.layout.slots.len(),
    }
}
