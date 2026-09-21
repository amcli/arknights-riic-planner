//! The fully-loaded, validated game data bundle.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    BaseLayout, BaseSkill, BuffId, Facility, FormulaId, GameConstants, ManufactureFormula,
    Operator, OperatorId, Power, PowerId, RoomType,
};

/// Provenance of a loaded dataset. Every solve result should carry this so
/// numbers can be traced back to the exact upstream commit and parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataVersion {
    /// Manifest source name, e.g. `en_US`.
    pub source: String,
    /// Upstream GitHub repository, e.g. `Kengxxiao/ArknightsGameData_YoStar`.
    pub repo: String,
    /// Pinned upstream commit.
    pub sha: String,
    /// Locale directory, e.g. `en_US`.
    pub locale: String,
    /// When the files were fetched (RFC 3339), if known.
    pub fetched_at: Option<String>,
    /// Version of the `ak-data` crate that produced this model.
    pub parser_version: String,
}

impl DataVersion {
    /// First 12 characters of the commit id.
    pub fn short_sha(&self) -> &str {
        let end = self.sha.len().min(12);
        &self.sha[..end]
    }
}

/// Everything the evaluator and solver need, in clean typed form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameData {
    /// Where this came from.
    pub version: DataVersion,
    /// Global tuning constants.
    pub constants: GameConstants,
    /// Nations, groups, and teams.
    pub powers: BTreeMap<PowerId, Power>,
    /// One entry per [`RoomType`].
    pub facilities: BTreeMap<RoomType, Facility>,
    /// Every base-skill tier.
    pub skills: BTreeMap<BuffId, BaseSkill>,
    /// Every operator that has base skills.
    pub operators: BTreeMap<OperatorId, Operator>,
    /// Factory formulas.
    pub manufacture_formulas: BTreeMap<FormulaId, ManufactureFormula>,
    /// Physical layout.
    pub layout: BaseLayout,
}

impl GameData {
    /// Looks up an operator by upstream id.
    pub fn operator(&self, id: &str) -> Option<&Operator> {
        self.operators.get(id)
    }

    /// Looks up a skill tier by upstream buff id.
    pub fn skill(&self, id: &str) -> Option<&BaseSkill> {
        self.skills.get(id)
    }

    /// Looks up a room kind. Every [`RoomType`] is present after a
    /// successful transform.
    pub fn facility(&self, room: RoomType) -> Option<&Facility> {
        self.facilities.get(&room)
    }

    /// Looks up a nation/group/team.
    pub fn power(&self, id: &str) -> Option<&Power> {
        self.powers.get(id)
    }
}
