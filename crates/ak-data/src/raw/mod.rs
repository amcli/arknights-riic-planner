//! Permissive mirrors of the upstream JSON.
//!
//! These structs declare only the fields we consume and tolerate everything
//! else. Field names follow upstream (`camelCase` via serde) so a reader can
//! diff them against the JSON directly. Nothing here is validated beyond
//! JSON type shape; that is [`crate::transform`]'s job.

pub mod building;
pub mod character;
pub mod team;

pub use building::RawBuildingData;
pub use character::{RawCharacter, RawCharacterTable, RawRarity};
pub use team::{RawTeam, RawTeamTable};

/// The three upstream files, deserialised but not yet validated.
#[derive(Debug, Clone)]
pub struct RawBundle {
    /// `building_data.json`.
    pub building: RawBuildingData,
    /// `character_table.json`.
    pub characters: RawCharacterTable,
    /// `handbook_team_table.json`.
    pub teams: RawTeamTable,
}
