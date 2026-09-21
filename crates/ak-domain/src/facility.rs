//! Room kinds and their per-level parameters.

use serde::{Deserialize, Serialize};

use crate::{GridSize, RoomCategory, RoomType};

/// One upgrade level of a room kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacilityPhase {
    /// 1-based level.
    pub level: u8,
    /// Power draw (negative) or supply (positive) at this level.
    pub electricity: i32,
    /// How many operators can be stationed.
    pub max_stationed: u8,
    /// Upstream `manpowerCost`.
    pub manpower_cost: i32,
    /// Drones required to build this level.
    pub build_labor: i32,
}

/// A kind of room, with its parameters at every level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facility {
    /// Which room this describes.
    pub room_type: RoomType,
    /// Localised display name.
    pub name: String,
    /// Localised flavour description.
    pub description: String,
    /// Upstream grouping.
    pub category: RoomCategory,
    /// Maximum number of this room a base may contain; `None` means
    /// unlimited (elevators and corridors).
    pub max_count: Option<u32>,
    /// Footprint in the layout grid.
    pub size: GridSize,
    /// Per-level parameters; index 0 is level 1.
    pub phases: Vec<FacilityPhase>,
}

impl Facility {
    /// Highest upgrade level.
    pub fn max_level(&self) -> u8 {
        u8::try_from(self.phases.len()).unwrap_or(u8::MAX)
    }

    /// Parameters at a 1-based level.
    pub fn phase(&self, level: u8) -> Option<&FacilityPhase> {
        level
            .checked_sub(1)
            .and_then(|i| self.phases.get(usize::from(i)))
    }

    /// Whether operators can be stationed here at any level.
    pub fn is_staffable(&self) -> bool {
        self.phases.iter().any(|p| p.max_stationed > 0)
    }
}
