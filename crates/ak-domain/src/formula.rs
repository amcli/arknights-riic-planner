//! Factory production formulas.

use serde::{Deserialize, Serialize};

use crate::{FormulaId, ItemId, ProductType, RoomType};

/// An input consumed by a formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaCost {
    /// Item consumed.
    pub item: ItemId,
    /// Quantity per batch.
    pub count: u32,
}

/// A room the base must have for a formula to be available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRequirement {
    /// Which room.
    pub room_type: RoomType,
    /// Minimum level.
    pub level: u8,
    /// Minimum count.
    pub count: u32,
}

/// Something a Factory can produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManufactureFormula {
    /// Upstream formula id.
    pub id: FormulaId,
    /// Item produced.
    pub item: ItemId,
    /// Quantity produced per batch.
    pub count: u32,
    /// Upstream `weight` (display / capacity weighting).
    pub weight: u32,
    /// Upstream `costPoint`. For Gold, EXP, and Originium Shard formulas
    /// this is the production time per batch in seconds (e.g. 2700 for a
    /// Drill Battle Record).
    pub cost_point: u32,
    /// Product family.
    pub product: ProductType,
    /// Inputs consumed per batch.
    pub costs: Vec<FormulaCost>,
    /// Rooms required to unlock this formula.
    pub require_rooms: Vec<RoomRequirement>,
}
