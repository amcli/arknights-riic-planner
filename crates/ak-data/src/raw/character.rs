//! Mirror of `character_table.json` (the subset we consume).

use std::collections::BTreeMap;

use serde::Deserialize;

/// Keyed by character id (`char_…`, `token_…`, `trap_…`).
pub type RawCharacterTable = BTreeMap<String, RawCharacter>;

/// Rarity has changed encoding upstream over time: current snapshots use
/// `"TIER_n"` strings, older ones used the integers `0..=5`. Accept both.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawRarity {
    Tier(String),
    Index(u8),
}

/// One character. The table also contains summons and traps; the transform
/// only visits entries referenced from `building_data.chars`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawCharacter {
    pub name: String,
    #[serde(default)]
    pub appellation: String,
    pub nation_id: Option<String>,
    pub group_id: Option<String>,
    pub team_id: Option<String>,
    #[serde(default)]
    pub display_number: Option<String>,
    #[serde(default)]
    pub is_not_obtainable: bool,
    pub rarity: RawRarity,
    pub profession: String,
    pub sub_profession_id: String,
}
