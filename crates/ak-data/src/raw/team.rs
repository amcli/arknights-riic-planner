//! Mirror of `handbook_team_table.json`.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Keyed by power id (`rhodes`, `rhine`, `action4`, …).
pub type RawTeamTable = BTreeMap<String, RawTeam>;

/// A nation, group, or team.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTeam {
    pub power_id: String,
    pub order_num: i32,
    /// 0 = nation, 1 = group, 2 = team.
    pub power_level: i64,
    pub power_name: String,
    #[serde(default)]
    pub power_code: String,
}
