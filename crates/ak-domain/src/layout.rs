//! The physical base layout: the grid of slots rooms are built into.

use serde::{Deserialize, Serialize};

use crate::{SlotCategory, SlotId};

/// Footprint in grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridSize {
    /// Height in cells.
    pub rows: u8,
    /// Width in cells.
    pub cols: u8,
}

/// Position in grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridPos {
    /// Row offset.
    pub row: i16,
    /// Column offset.
    pub col: i16,
}

/// One buildable slot in the layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSlot {
    /// Slot id, e.g. `slot_34`.
    pub id: SlotId,
    /// What may be built here.
    pub category: SlotCategory,
    /// Footprint.
    pub size: GridSize,
    /// Position.
    pub offset: GridPos,
    /// Floor label, e.g. `B2`. Empty for a few utility slots upstream.
    pub storey: String,
    /// Upstream `cleanCostId`.
    pub clean_cost_id: String,
    /// Drones to clear the slot.
    pub cost_labor: i32,
    /// Drone capacity the slot provides once cleared.
    pub provide_labor: i32,
}

/// The whole layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseLayout {
    /// Upstream layout id (currently only `v0`).
    pub id: String,
    /// Slots in ascending id order.
    pub slots: Vec<LayoutSlot>,
}

impl BaseLayout {
    /// Looks up a slot by id.
    pub fn slot(&self, id: &str) -> Option<&LayoutSlot> {
        self.slots.iter().find(|s| s.id.as_str() == id)
    }

    /// Slots of a given category.
    pub fn slots_of(&self, category: SlotCategory) -> impl Iterator<Item = &LayoutSlot> {
        self.slots.iter().filter(move |s| s.category == category)
    }
}
