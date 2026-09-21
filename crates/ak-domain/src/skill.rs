//! Base skills ("buffs" upstream).

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{
    BuffCategory, BuffId, Mechanics, ProductType, Profession, RichText, RoomType, UnknownVariant,
};

/// A single tier of a base skill, as listed in `building_data.buffs`.
///
/// Note on mechanics: upstream ships **no** structured effect data. The
/// only description of what a skill does is the localised [`description`]
/// rich text, plus the display-oriented [`efficiency_hint`] / [`targets`]
/// sort metadata. The Layer 2 parser in `ak-data` derives [`mechanics`]
/// from the description; it is `None` when the description could not be
/// parsed, and the transform report says why.
///
/// [`description`]: BaseSkill::description
/// [`efficiency_hint`]: BaseSkill::efficiency_hint
/// [`targets`]: BaseSkill::targets
/// [`mechanics`]: BaseSkill::mechanics
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseSkill {
    /// Upstream buff id, e.g. `manu_prod_spd[000]`.
    pub id: BuffId,
    /// Localised display name, e.g. `Standardization α`.
    pub name: String,
    /// The room this skill applies in.
    pub room_type: RoomType,
    /// Upstream role classification.
    pub category: BuffCategory,
    /// Localised description with inline markup preserved.
    pub description: RichText,
    /// Upstream `efficiency` sort hint, in percent. Display metadata used by
    /// the in-game "sort by efficiency" list. **Not** a mechanic; it is often
    /// zero for skills that clearly do something.
    pub efficiency_hint: i32,
    /// Upstream `targetGroupSortId`; display grouping.
    pub target_group_sort_id: i32,
    /// What the efficiency hint applies to, per upstream sort metadata.
    /// Empty when not applicable.
    pub targets: Vec<EfficiencyTarget>,
    /// Upstream `sortId`.
    pub sort_id: i32,
    /// Upstream `buffIcon` asset key.
    pub icon: String,
    /// Upstream `skillIcon` asset key.
    pub skill_icon: String,
    /// Parsed mechanics; `None` when the description parser failed.
    #[serde(default)]
    pub mechanics: Option<Mechanics>,
}

impl BaseSkill {
    /// The skill family (id without the tier suffix).
    pub fn family(&self) -> &str {
        self.id.family()
    }
}

/// What a skill's efficiency metadata targets: a product family (Factory,
/// Workshop) or an operator class (Training Room).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EfficiencyTarget {
    /// A product family such as `F_GOLD`.
    Product(ProductType),
    /// An operator class such as `WARRIOR`.
    Profession(Profession),
}

impl EfficiencyTarget {
    /// The upstream string.
    pub const fn as_str(self) -> &'static str {
        match self {
            EfficiencyTarget::Product(p) => p.as_str(),
            EfficiencyTarget::Profession(p) => p.as_str(),
        }
    }
}

impl FromStr for EfficiencyTarget {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(p) = ProductType::from_str(s) {
            return Ok(EfficiencyTarget::Product(p));
        }
        if let Ok(p) = Profession::from_str(s) {
            return Ok(EfficiencyTarget::Profession(p));
        }
        Err(UnknownVariant {
            type_name: "EfficiencyTarget",
            value: s.to_owned(),
        })
    }
}

impl std::fmt::Display for EfficiencyTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efficiency_target_parses_both_arms() {
        assert_eq!(
            "F_GOLD".parse::<EfficiencyTarget>(),
            Ok(EfficiencyTarget::Product(ProductType::Gold))
        );
        assert_eq!(
            "WARRIOR".parse::<EfficiencyTarget>(),
            Ok(EfficiencyTarget::Profession(Profession::Guard))
        );
        assert!("F_NOPE".parse::<EfficiencyTarget>().is_err());
    }
}
