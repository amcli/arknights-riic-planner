//! The player's roster: which operators they own and how far each is
//! raised. This decides which skill tier is active. Layer 8 (roster import)
//! will populate it; until then it is built by hand or assumed maxed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{GameData, OperatorId, UnlockCond};

/// One owned operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    /// Promotion and level, which select the active tier of each skill slot.
    pub promotion: UnlockCond,
    /// Current morale on the 0–24 scale, if known. `None` means "use the
    /// simulation's initial-mood policy".
    #[serde(default)]
    pub mood: Option<f64>,
}

impl RosterEntry {
    /// An entry at the given promotion with unknown morale.
    pub fn at(promotion: UnlockCond) -> Self {
        RosterEntry {
            promotion,
            mood: None,
        }
    }

    /// Fully raised, morale unknown.
    pub fn maxed() -> Self {
        Self::at(UnlockCond::MAX)
    }
}

/// Owned operators keyed by id.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Roster {
    /// Entries keyed by operator id.
    pub entries: BTreeMap<OperatorId, RosterEntry>,
}

impl Roster {
    /// An empty roster.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every operator in the game data, fully raised. Handy for "what could
    /// this base do" questions; wrong for "what will my base do".
    pub fn everyone_maxed(data: &GameData) -> Self {
        Roster {
            entries: data
                .operators
                .keys()
                .map(|id| (id.clone(), RosterEntry::maxed()))
                .collect(),
        }
    }

    /// Adds or replaces an entry.
    pub fn insert(&mut self, op: impl Into<OperatorId>, entry: RosterEntry) -> &mut Self {
        self.entries.insert(op.into(), entry);
        self
    }

    /// Looks up an operator.
    pub fn get(&self, op: &str) -> Option<&RosterEntry> {
        self.entries.get(op)
    }

    /// True when the operator is owned.
    pub fn contains(&self, op: &str) -> bool {
        self.entries.contains_key(op)
    }
}
