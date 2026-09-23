//! Operators and their base-skill progression.
//!
//! This is the *static* view: what an operator can do, loaded from data and
//! never mutated. Simulation state (current mood, where they are assigned)
//! belongs to the evaluator, not here.

use serde::{Deserialize, Serialize};

use crate::{BuffId, ElitePhase, OperatorId, PowerId, Profession, Rarity, SubProfessionId};

/// The promotion state at which a skill tier unlocks. Ordered so that
/// `(E1, 1) < (E2, 1) < (E2, 30)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnlockCond {
    /// Promotion level.
    pub phase: ElitePhase,
    /// Level within that promotion (1-based).
    pub level: u32,
}

impl UnlockCond {
    /// Elite 0, level 1: the state every operator starts in.
    pub const BASE: UnlockCond = UnlockCond {
        phase: ElitePhase::E0,
        level: 1,
    };

    /// Elite 2, level 90: every tier of every skill is unlocked.
    pub const MAX: UnlockCond = UnlockCond {
        phase: ElitePhase::E2,
        level: 90,
    };

    /// Constructs a condition.
    pub const fn new(phase: ElitePhase, level: u32) -> Self {
        UnlockCond { phase, level }
    }
}

impl std::fmt::Display for UnlockCond {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} L{}", self.phase, self.level)
    }
}

/// One tier of one skill slot: which buff is active once `cond` is met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUnlock {
    /// The buff that becomes active.
    pub buff: BuffId,
    /// When it becomes active.
    pub cond: UnlockCond,
}

/// A base-skill slot. Operators currently have exactly two; each lists its
/// tiers in ascending unlock order. A slot may be empty (no skill in it).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkillSlot {
    /// Tiers, sorted ascending by [`SkillUnlock::cond`].
    pub unlocks: Vec<SkillUnlock>,
}

impl SkillSlot {
    /// The tier active at promotion state `at`: the highest unlock whose
    /// condition is satisfied, or `None` if nothing has unlocked yet.
    pub fn active_at(&self, at: UnlockCond) -> Option<&SkillUnlock> {
        self.unlocks.iter().rev().find(|u| u.cond <= at)
    }

    /// The highest tier this slot can reach.
    pub fn max(&self) -> Option<&SkillUnlock> {
        self.unlocks.last()
    }
}

/// A recruitable operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operator {
    /// Upstream id, e.g. `char_285_medic2`.
    pub id: OperatorId,
    /// Localised name, e.g. `Lancet-2`.
    pub name: String,
    /// Upstream `appellation` (romanised / alternate name); may be empty.
    pub appellation: String,
    /// Star rarity.
    pub rarity: Rarity,
    /// Class.
    pub profession: Profession,
    /// Subclass.
    pub sub_profession: SubProfessionId,
    /// Nation affiliation, if any.
    pub nation: Option<PowerId>,
    /// Group affiliation, if any.
    pub group: Option<PowerId>,
    /// Team affiliation, if any.
    pub team: Option<PowerId>,
    /// Maximum morale on the in-game 0–24 scale (`maxManpower` divided by
    /// `manpowerDisplayFactor`). Currently 24 for every operator.
    pub max_mood: f64,
    /// Highest level at each promotion this operator can reach, from
    /// upstream `phases[].maxLevel`: index 0 is Elite 0. Its length is one
    /// more than the highest promotion (a 3★ has two entries, `[40, 55]`).
    pub max_levels: Vec<u32>,
    /// Base-skill slots in upstream order.
    pub skill_slots: Vec<SkillSlot>,
}

impl Operator {
    /// The highest promotion this operator can reach.
    pub fn max_phase(&self) -> ElitePhase {
        let top = self.max_levels.len().saturating_sub(1);
        ElitePhase::ALL.get(top).copied().unwrap_or(ElitePhase::E2)
    }

    /// The highest level at a promotion, or `None` if the operator cannot
    /// reach that promotion.
    pub fn max_level(&self, phase: ElitePhase) -> Option<u32> {
        let index = ElitePhase::ALL.iter().position(|p| *p == phase)?;
        self.max_levels.get(index).copied()
    }

    /// Every affiliation this operator has (nation, group, team), in that order.
    pub fn powers(&self) -> impl Iterator<Item = &PowerId> {
        self.nation
            .iter()
            .chain(self.group.iter())
            .chain(self.team.iter())
    }

    /// Buffs active at promotion state `at`, one per slot that has unlocked.
    pub fn active_buffs(&self, at: UnlockCond) -> Vec<&BuffId> {
        self.skill_slots
            .iter()
            .filter_map(|slot| slot.active_at(at))
            .map(|u| &u.buff)
            .collect()
    }

    /// Buffs active once every tier is unlocked.
    pub fn max_buffs(&self) -> Vec<&BuffId> {
        self.skill_slots
            .iter()
            .filter_map(SkillSlot::max)
            .map(|u| &u.buff)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(tiers: &[(&str, ElitePhase, u32)]) -> SkillSlot {
        SkillSlot {
            unlocks: tiers
                .iter()
                .map(|&(b, p, l)| SkillUnlock {
                    buff: BuffId::new(b),
                    cond: UnlockCond::new(p, l),
                })
                .collect(),
        }
    }

    #[test]
    fn active_tier_is_highest_satisfied() {
        let s = slot(&[
            ("a[000]", ElitePhase::E0, 1),
            ("a[010]", ElitePhase::E1, 1),
            ("a[020]", ElitePhase::E2, 1),
        ]);
        assert_eq!(
            s.active_at(UnlockCond::BASE).unwrap().buff.as_str(),
            "a[000]"
        );
        assert_eq!(
            s.active_at(UnlockCond::new(ElitePhase::E1, 60))
                .unwrap()
                .buff
                .as_str(),
            "a[010]"
        );
        assert_eq!(
            s.active_at(UnlockCond::new(ElitePhase::E2, 1))
                .unwrap()
                .buff
                .as_str(),
            "a[020]"
        );
    }

    #[test]
    fn nothing_active_before_first_unlock() {
        let s = slot(&[("d[000]", ElitePhase::E0, 30)]);
        assert!(s.active_at(UnlockCond::BASE).is_none());
        assert!(s.active_at(UnlockCond::new(ElitePhase::E0, 30)).is_some());
    }
}
