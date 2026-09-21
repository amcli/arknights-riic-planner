//! Nations, groups, and teams ("powers" upstream), e.g. Rhodes Island,
//! Rhine Lab, Reserve Op Team A4.

use serde::{Deserialize, Serialize};

use crate::PowerId;

/// The three tiers of affiliation. Upstream encodes these as `powerLevel`
/// 0, 1, 2 and stores the id in `nationId`, `groupId`, `teamId` respectively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerLevel {
    /// e.g. Rhodes Island, Victoria.
    Nation,
    /// e.g. Rhine Lab, Penguin Logistics.
    Group,
    /// e.g. Reserve Op Team A4, Rainbow.
    Team,
}

impl PowerLevel {
    /// Maps the upstream integer encoding.
    pub const fn from_upstream(level: i64) -> Option<Self> {
        match level {
            0 => Some(PowerLevel::Nation),
            1 => Some(PowerLevel::Group),
            2 => Some(PowerLevel::Team),
            _ => None,
        }
    }

    /// Lower-case English label.
    pub const fn as_str(self) -> &'static str {
        match self {
            PowerLevel::Nation => "nation",
            PowerLevel::Group => "group",
            PowerLevel::Team => "team",
        }
    }
}

impl std::fmt::Display for PowerLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A nation, group, or team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    /// Upstream id, e.g. `rhine`.
    pub id: PowerId,
    /// Localised display name, e.g. `Rhine Lab`.
    pub name: String,
    /// Short code, e.g. `Rhine·Lab`.
    pub code: String,
    /// Which tier of affiliation this is.
    pub level: PowerLevel,
    /// Upstream display ordering.
    pub order: i32,
}
