//! Mirror of `building_data.json` (the subset we consume).

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top level of `building_data.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawBuildingData {
    pub control_slot_id: String,
    pub meeting_slot_id: String,
    pub labor_recover_time: u32,
    pub manufact_input_capacity: u32,
    pub shop_counter_capacity: u32,
    pub comfort_limit: u32,
    pub manpower_display_factor: u32,
    pub basic_favor_per_day: u32,
    pub tired_ap_threshold: u32,
    pub manufact_manpower_cost_by_num: Vec<i32>,
    pub trading_manpower_cost_by_num: Vec<i32>,
    pub rooms: BTreeMap<String, RawRoom>,
    pub layouts: BTreeMap<String, RawLayout>,
    pub control_data: RawControlData,
    pub manufact_data: RawManufactData,
    pub dorm_data: RawDormData,
    pub trading_data: RawTradingData,
    pub power_data: RawPowerData,
    pub meeting_data: RawMeetingData,
    pub hire_data: RawHireData,
    pub training_data: RawTrainingData,
    pub workshop_data: RawWorkshopData,
    pub chars: BTreeMap<String, RawBuildingChar>,
    pub buffs: BTreeMap<String, RawBuff>,
    pub manufact_formulas: BTreeMap<String, RawManufactFormula>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawGridSize {
    pub row: u8,
    pub col: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawGridPos {
    pub row: i16,
    pub col: i16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawRoom {
    pub id: String,
    pub name: String,
    /// `null` upstream for ELEVATOR and CORRIDOR.
    #[serde(default)]
    pub description: Option<String>,
    /// `-1` upstream means unlimited (ELEVATOR, CORRIDOR).
    pub max_count: i32,
    pub category: String,
    pub size: RawGridSize,
    pub phases: Vec<RawRoomPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawRoomPhase {
    pub unlock_cond_id: String,
    pub build_cost: RawBuildCost,
    pub electricity: i32,
    pub max_stationed_num: u8,
    pub manpower_cost: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawBuildCost {
    pub labor: i32,
    pub time: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawLayout {
    pub id: String,
    pub slots: BTreeMap<String, RawLayoutSlot>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawLayoutSlot {
    pub id: String,
    pub clean_cost_id: String,
    pub cost_labor: i32,
    pub provide_labor: i32,
    pub size: RawGridSize,
    pub offset: RawGridPos,
    pub category: String,
    pub storey_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawControlData {
    pub basic_cost_buff: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawManufactData {
    pub basic_speed_buff: f64,
    pub phases: Vec<RawManufactPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawManufactPhase {
    pub speed: f64,
    pub output_capacity: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDormData {
    pub phases: Vec<RawDormPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDormPhase {
    pub manpower_recover: u32,
    pub decoration_limit: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTradingData {
    pub basic_speed_buff: f64,
    pub phases: Vec<RawTradingPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTradingPhase {
    pub order_speed: f64,
    pub order_limit: u32,
    pub order_rarity: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPowerData {
    pub basic_speed_buff: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMeetingData {
    pub basic_speed_buff: f64,
    pub phases: Vec<RawMeetingPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMeetingPhase {
    pub friend_slot_inc: u32,
    pub max_visitor_num: u32,
    pub gathering_speed: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHireData {
    pub basic_speed_buff: f64,
    pub phases: Vec<RawHirePhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHirePhase {
    pub economize_rate: f64,
    pub res_speed: u32,
    pub refresh_times: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTrainingData {
    pub basic_speed_buff: f64,
    pub phases: Vec<RawTrainingPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTrainingPhase {
    pub spec_skill_lvl_limit: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawWorkshopData {
    pub phases: Vec<RawWorkshopPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawWorkshopPhase {
    pub manpower_factor: f64,
}

/// One entry of `building_data.chars`: an operator's skill progression.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawBuildingChar {
    pub char_id: String,
    pub max_manpower: u64,
    pub buff_char: Vec<RawBuffCharSlot>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawBuffCharSlot {
    pub buff_data: Vec<RawBuffData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawBuffData {
    pub buff_id: String,
    pub cond: RawUnlockCond,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUnlockCond {
    pub phase: String,
    pub level: u32,
}

/// One entry of `building_data.buffs`: a base-skill tier.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawBuff {
    pub buff_id: String,
    pub buff_name: String,
    pub buff_icon: String,
    pub skill_icon: String,
    pub sort_id: i32,
    pub buff_category: String,
    pub room_type: String,
    pub description: String,
    pub efficiency: i32,
    pub target_group_sort_id: i32,
    #[serde(default)]
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawManufactFormula {
    pub formula_id: String,
    pub item_id: String,
    pub count: u32,
    pub weight: u32,
    pub cost_point: u32,
    pub formula_type: String,
    #[serde(default)]
    pub costs: Vec<RawItemCost>,
    #[serde(default)]
    pub require_rooms: Vec<RawRoomRequirement>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawItemCost {
    pub id: String,
    pub count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawRoomRequirement {
    pub room_id: String,
    pub room_level: u8,
    pub room_count: u32,
}
