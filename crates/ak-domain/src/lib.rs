//! Core domain types for the Arknights RIIC (base) planner.
//!
//! This crate is the *clean* model: newtyped IDs, closed enums, and plain data
//! structs. It depends only on `serde` and performs no I/O. The messy upstream
//! JSON never appears here; `ak-data` owns the raw mirrors and the transform
//! into these types, so "upstream weirdness" ends at that boundary.
//!
//! Rules this crate holds to:
//! - every identifier is a newtype (see [`ids`]);
//! - every fixed vocabulary is a closed enum that fails to parse on unknown
//!   input (see [`enums`]);
//! - static capabilities (what an operator *can* do) live here, mutable
//!   simulation state (mood, hours worked) does not.

#[macro_use]
mod macros;

pub mod constants;
pub mod enums;
pub mod facility;
pub mod formula;
pub mod game_data;
pub mod ids;
pub mod layout;
pub mod mechanics;
pub mod operator;
pub mod power;
pub mod richtext;
pub mod skill;

pub use constants::*;
pub use enums::*;
pub use facility::{Facility, FacilityPhase};
pub use formula::{FormulaCost, ManufactureFormula, RoomRequirement};
pub use game_data::{DataVersion, GameData};
pub use ids::*;
pub use layout::{BaseLayout, GridPos, GridSize, LayoutSlot};
pub use macros::UnknownVariant;
pub use mechanics::{
    Amount, Clause, CostChange, CostFilter, CountScope, Counter, Effect, Group, MaterialFilter,
    Mechanics, MoodTarget, OperatorRef, Predicate, Scope, Stacking, Stat,
};
pub use operator::{Operator, SkillSlot, SkillUnlock, UnlockCond};
pub use power::{Power, PowerLevel};
pub use richtext::{RichNode, RichTag, RichText};
pub use skill::{BaseSkill, EfficiencyTarget};
