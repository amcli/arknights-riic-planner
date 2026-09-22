//! Global tuning constants and per-room-kind parameter tables.
//!
//! These are the numbers the evaluator needs and nothing more. Upstream
//! `building_data.json` has ~80 top-level keys; the ones omitted here are
//! UI, furniture, or unlock-tutorial data.

use serde::{Deserialize, Serialize};

use crate::SlotId;

/// Control Center parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlData {
    /// Upstream `basicCostBuff`.
    pub basic_cost_buff: i32,
}

/// Factory parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManufacturePhase {
    /// Base production speed multiplier.
    pub speed: f64,
    /// Output storage capacity.
    pub output_capacity: u32,
}

/// Factory parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManufactureData {
    /// Upstream `basicSpeedBuff`.
    pub basic_speed_buff: f64,
    /// Per level; index 0 is level 1.
    pub phases: Vec<ManufacturePhase>,
}

/// Trading Post parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradingPhase {
    /// Base order acquisition speed multiplier.
    pub order_speed: f64,
    /// Maximum queued orders.
    pub order_limit: u32,
    /// Highest order rarity obtainable.
    pub order_rarity: u32,
}

/// Trading Post parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradingData {
    /// Upstream `basicSpeedBuff`.
    pub basic_speed_buff: f64,
    /// Per level; index 0 is level 1.
    pub phases: Vec<TradingPhase>,
}

/// Dormitory parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DormPhase {
    /// Upstream `manpowerRecover` (morale recovery, in upstream units).
    pub manpower_recover: u32,
    /// Maximum ambience (decoration) points.
    pub decoration_limit: u32,
}

/// Dormitory parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DormData {
    /// Per level; index 0 is level 1.
    pub phases: Vec<DormPhase>,
}

/// Power Plant parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerData {
    /// Upstream `basicSpeedBuff`.
    pub basic_speed_buff: f64,
}

/// Reception Room parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingPhase {
    /// Upstream `friendSlotInc`.
    pub friend_slot_inc: u32,
    /// Upstream `maxVisitorNum`.
    pub max_visitor_num: u32,
    /// Upstream `gatheringSpeed` (clue search speed, percent).
    pub gathering_speed: u32,
}

/// Reception Room parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingData {
    /// Upstream `basicSpeedBuff`.
    pub basic_speed_buff: f64,
    /// Per level; index 0 is level 1.
    pub phases: Vec<MeetingPhase>,
}

/// Office parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HirePhase {
    /// Upstream `economizeRate`.
    pub economize_rate: f64,
    /// Upstream `resSpeed` (contact search speed, percent).
    pub res_speed: u32,
    /// Upstream `refreshTimes`.
    pub refresh_times: u32,
}

/// Office parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HireData {
    /// Upstream `basicSpeedBuff`.
    pub basic_speed_buff: f64,
    /// Per level; index 0 is level 1.
    pub phases: Vec<HirePhase>,
}

/// Training Room parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingPhase {
    /// Highest specialisation level trainable.
    pub spec_skill_lvl_limit: u32,
}

/// Training Room parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingData {
    /// Upstream `basicSpeedBuff`.
    pub basic_speed_buff: f64,
    /// Per level; index 0 is level 1.
    pub phases: Vec<TrainingPhase>,
}

/// Workshop parameters at one level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkshopPhase {
    /// Upstream `manpowerFactor`.
    pub manpower_factor: f64,
}

/// Workshop parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkshopData {
    /// Per level; index 0 is level 1.
    pub phases: Vec<WorkshopPhase>,
}

/// Everything global.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameConstants {
    /// The layout slot that holds the Control Center.
    pub control_slot: SlotId,
    /// The layout slot that holds the Reception Room.
    pub meeting_slot: SlotId,
    /// Divisor from upstream `maxManpower` units to the 0–24 morale scale.
    pub manpower_display_factor: u32,
    /// Upstream `laborRecoverTime` (seconds per drone).
    pub labor_recover_time: u32,
    /// Upstream `comfortLimit`.
    pub comfort_limit: u32,
    /// Upstream `comfortManpowerRecoverFactor`: ambience points per morale unit
    /// (`manpower` units) of Dormitory recovery per second. At the pinned
    /// value of 25, 5000 ambience adds 200 units/s = 2.0 morale per hour.
    pub comfort_manpower_recover_factor: u32,
    /// Upstream `basicFavorPerDay` (trust per day, upstream units).
    pub basic_favor_per_day: u32,
    /// Upstream `tiredApThreshold`.
    pub tired_ap_threshold: u32,
    /// Factory input storage.
    pub manufact_input_capacity: u32,
    /// Trading Post counter capacity.
    pub shop_counter_capacity: u32,
    /// Upstream `manufactManpowerCostByNum`: morale-cost modifier by number
    /// of stationed operators (index = count).
    pub manufact_manpower_cost_by_num: Vec<i32>,
    /// Upstream `tradingManpowerCostByNum`.
    pub trading_manpower_cost_by_num: Vec<i32>,
    /// Control Center parameters.
    pub control: ControlData,
    /// Factory parameters.
    pub manufacture: ManufactureData,
    /// Trading Post parameters.
    pub trading: TradingData,
    /// Dormitory parameters.
    pub dormitory: DormData,
    /// Power Plant parameters.
    pub power: PowerData,
    /// Reception Room parameters.
    pub meeting: MeetingData,
    /// Office parameters.
    pub hire: HireData,
    /// Training Room parameters.
    pub training: TrainingData,
    /// Workshop parameters.
    pub workshop: WorkshopData,
}
