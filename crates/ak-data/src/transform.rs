//! Raw → domain. Strict and exhaustive: this is the boundary where upstream
//! weirdness is turned into either a clean [`GameData`] or a precise error.

use std::collections::BTreeMap;

use ak_domain::*;

use crate::raw::building::{RawBuildingChar, RawBuildingData};
use crate::raw::{RawBundle, RawRarity, RawTeamTable};
use crate::richtext;

/// Upstream layout id we build the base model from.
pub const LAYOUT_ID: &str = "v0";

/// How to treat a per-operator failure.
///
/// Structural failures (a missing room kind, a bad constant) always error.
/// Per-operator failures can be tolerated so development keeps moving when
/// upstream adds something new; tests and the sync tool run strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    /// Any failure aborts the transform.
    Strict,
    /// Per-operator failures are logged, recorded, and the operator skipped.
    Lenient,
}

/// An operator that was skipped under [`Strictness::Lenient`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkippedOperator {
    /// Which operator.
    pub id: OperatorId,
    /// Why.
    pub reason: String,
}

/// What happened during a transform.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct TransformReport {
    /// Operators present in `building_data.chars`.
    pub operators_total: usize,
    /// Operators that made it into the model.
    pub operators_loaded: usize,
    /// Operators dropped (empty under [`Strictness::Strict`]).
    pub skipped: Vec<SkippedOperator>,
    /// Layer 2 description-parser coverage.
    pub mechanics: crate::mechanics::MechanicsReport,
}

/// A transform failure. Wrapped in [`TransformError::Context`] frames naming
/// the entity being processed.
#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    #[error(transparent)]
    UnknownVariant(#[from] UnknownVariant),
    #[error("rarity index {0} is out of range 0..=5")]
    RarityIndex(u8),
    #[error("power level {0} is not one of 0 (nation), 1 (group), 2 (team)")]
    PowerLevel(i64),
    #[error("references buff {0} which is not in building_data.buffs")]
    MissingBuff(BuffId),
    #[error("present in building_data.chars but not in character_table")]
    MissingCharacter,
    #[error("references {level} {power} which is not in handbook_team_table")]
    UnknownPower { level: PowerLevel, power: PowerId },
    #[error("room {0} is missing from building_data.rooms")]
    MissingRoom(RoomType),
    #[error("layout {0:?} is missing from building_data.layouts")]
    MissingLayout(String),
    #[error("buff map key {key:?} does not match its buffId {id:?}")]
    BuffKeyMismatch { key: String, id: String },
    #[error("{context}: {source}")]
    Context {
        context: String,
        #[source]
        source: Box<TransformError>,
    },
}

fn ctx<T>(
    context: impl Into<String>,
    result: Result<T, TransformError>,
) -> Result<T, TransformError> {
    result.map_err(|source| TransformError::Context {
        context: context.into(),
        source: Box::new(source),
    })
}

/// Transforms a raw bundle into the domain model.
pub fn transform(
    raw: &RawBundle,
    version: DataVersion,
    strictness: Strictness,
) -> Result<(GameData, TransformReport), TransformError> {
    let powers = ctx("handbook_team_table", transform_powers(&raw.teams))?;
    let constants = ctx(
        "building_data constants",
        transform_constants(&raw.building),
    )?;
    let facilities = transform_facilities(&raw.building)?;
    let mut skills = transform_skills(&raw.building)?;
    let manufacture_formulas = transform_formulas(&raw.building)?;
    let layout = transform_layout(&raw.building)?;

    let mut operators = BTreeMap::new();
    let mut report = TransformReport {
        operators_total: raw.building.chars.len(),
        ..TransformReport::default()
    };

    for (char_id, building_char) in &raw.building.chars {
        let id = OperatorId::new(char_id.as_str());
        let result = ctx(
            format!("operator {char_id}"),
            transform_operator(
                &id,
                building_char,
                raw,
                &powers,
                &skills,
                constants.manpower_display_factor,
            ),
        );
        match result {
            Ok(operator) => {
                operators.insert(id, operator);
            }
            Err(err) => match strictness {
                Strictness::Strict => return Err(err),
                Strictness::Lenient => {
                    tracing::warn!(operator = %id, error = %err, "skipping operator");
                    report.skipped.push(SkippedOperator {
                        id,
                        reason: err.to_string(),
                    });
                }
            },
        }
    }
    report.operators_loaded = operators.len();

    // Layer 2: parse descriptions now that operator names can be resolved.
    report.mechanics = crate::mechanics::attach(&mut skills, &operators);

    Ok((
        GameData {
            version,
            constants,
            powers,
            facilities,
            skills,
            operators,
            manufacture_formulas,
            layout,
        },
        report,
    ))
}

fn transform_powers(teams: &RawTeamTable) -> Result<BTreeMap<PowerId, Power>, TransformError> {
    teams
        .iter()
        .map(|(key, team)| {
            let level = PowerLevel::from_upstream(team.power_level)
                .ok_or(TransformError::PowerLevel(team.power_level))?;
            let id = PowerId::new(key.as_str());
            Ok((
                id.clone(),
                Power {
                    id,
                    name: team.power_name.clone(),
                    code: team.power_code.clone(),
                    level,
                    order: team.order_num,
                },
            ))
        })
        .collect()
}

fn transform_constants(b: &RawBuildingData) -> Result<GameConstants, TransformError> {
    Ok(GameConstants {
        control_slot: SlotId::new(b.control_slot_id.as_str()),
        meeting_slot: SlotId::new(b.meeting_slot_id.as_str()),
        manpower_display_factor: b.manpower_display_factor,
        labor_recover_time: b.labor_recover_time,
        comfort_limit: b.comfort_limit,
        basic_favor_per_day: b.basic_favor_per_day,
        tired_ap_threshold: b.tired_ap_threshold,
        manufact_input_capacity: b.manufact_input_capacity,
        shop_counter_capacity: b.shop_counter_capacity,
        manufact_manpower_cost_by_num: b.manufact_manpower_cost_by_num.clone(),
        trading_manpower_cost_by_num: b.trading_manpower_cost_by_num.clone(),
        control: ControlData {
            basic_cost_buff: b.control_data.basic_cost_buff,
        },
        manufacture: ManufactureData {
            basic_speed_buff: b.manufact_data.basic_speed_buff,
            phases: b
                .manufact_data
                .phases
                .iter()
                .map(|p| ManufacturePhase {
                    speed: p.speed,
                    output_capacity: p.output_capacity,
                })
                .collect(),
        },
        trading: TradingData {
            basic_speed_buff: b.trading_data.basic_speed_buff,
            phases: b
                .trading_data
                .phases
                .iter()
                .map(|p| TradingPhase {
                    order_speed: p.order_speed,
                    order_limit: p.order_limit,
                    order_rarity: p.order_rarity,
                })
                .collect(),
        },
        dormitory: DormData {
            phases: b
                .dorm_data
                .phases
                .iter()
                .map(|p| DormPhase {
                    manpower_recover: p.manpower_recover,
                    decoration_limit: p.decoration_limit,
                })
                .collect(),
        },
        power: PowerData {
            basic_speed_buff: b.power_data.basic_speed_buff,
        },
        meeting: MeetingData {
            basic_speed_buff: b.meeting_data.basic_speed_buff,
            phases: b
                .meeting_data
                .phases
                .iter()
                .map(|p| MeetingPhase {
                    friend_slot_inc: p.friend_slot_inc,
                    max_visitor_num: p.max_visitor_num,
                    gathering_speed: p.gathering_speed,
                })
                .collect(),
        },
        hire: HireData {
            basic_speed_buff: b.hire_data.basic_speed_buff,
            phases: b
                .hire_data
                .phases
                .iter()
                .map(|p| HirePhase {
                    economize_rate: p.economize_rate,
                    res_speed: p.res_speed,
                    refresh_times: p.refresh_times,
                })
                .collect(),
        },
        training: TrainingData {
            basic_speed_buff: b.training_data.basic_speed_buff,
            phases: b
                .training_data
                .phases
                .iter()
                .map(|p| TrainingPhase {
                    spec_skill_lvl_limit: p.spec_skill_lvl_limit,
                })
                .collect(),
        },
        workshop: WorkshopData {
            phases: b
                .workshop_data
                .phases
                .iter()
                .map(|p| WorkshopPhase {
                    manpower_factor: p.manpower_factor,
                })
                .collect(),
        },
    })
}

fn transform_facilities(
    b: &RawBuildingData,
) -> Result<BTreeMap<RoomType, Facility>, TransformError> {
    RoomType::ALL
        .iter()
        .map(|&room_type| {
            let room = b
                .rooms
                .get(room_type.as_str())
                .ok_or(TransformError::MissingRoom(room_type))?;
            let category = ctx(
                format!("room {room_type}"),
                room.category
                    .parse::<RoomCategory>()
                    .map_err(TransformError::from),
            )?;
            let phases = room
                .phases
                .iter()
                .enumerate()
                .map(|(i, p)| FacilityPhase {
                    level: u8::try_from(i + 1).unwrap_or(u8::MAX),
                    electricity: p.electricity,
                    max_stationed: p.max_stationed_num,
                    manpower_cost: p.manpower_cost,
                    build_labor: p.build_cost.labor,
                })
                .collect();
            Ok((
                room_type,
                Facility {
                    room_type,
                    name: room.name.clone(),
                    description: room.description.clone().unwrap_or_default(),
                    category,
                    max_count: u32::try_from(room.max_count).ok(),
                    size: GridSize {
                        rows: room.size.row,
                        cols: room.size.col,
                    },
                    phases,
                },
            ))
        })
        .collect()
}

fn transform_skills(b: &RawBuildingData) -> Result<BTreeMap<BuffId, BaseSkill>, TransformError> {
    b.buffs
        .iter()
        .map(|(key, buff)| {
            let built = (|| -> Result<BaseSkill, TransformError> {
                if key != &buff.buff_id {
                    return Err(TransformError::BuffKeyMismatch {
                        key: key.clone(),
                        id: buff.buff_id.clone(),
                    });
                }
                let room_type = buff.room_type.parse::<RoomType>()?;
                let category = buff.buff_category.parse::<BuffCategory>()?;
                let targets = buff
                    .targets
                    .iter()
                    .map(|t| t.parse::<EfficiencyTarget>())
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(BaseSkill {
                    id: BuffId::new(buff.buff_id.as_str()),
                    name: buff.buff_name.clone(),
                    room_type,
                    category,
                    description: richtext::parse(&buff.description),
                    efficiency_hint: buff.efficiency,
                    target_group_sort_id: buff.target_group_sort_id,
                    targets,
                    sort_id: buff.sort_id,
                    icon: buff.buff_icon.clone(),
                    skill_icon: buff.skill_icon.clone(),
                    mechanics: None,
                })
            })();
            ctx(format!("buff {key}"), built).map(|skill| (skill.id.clone(), skill))
        })
        .collect()
}

fn transform_formulas(
    b: &RawBuildingData,
) -> Result<BTreeMap<FormulaId, ManufactureFormula>, TransformError> {
    b.manufact_formulas
        .iter()
        .map(|(key, f)| {
            let built = (|| -> Result<ManufactureFormula, TransformError> {
                let product = f.formula_type.parse::<ProductType>()?;
                let require_rooms = f
                    .require_rooms
                    .iter()
                    .map(|r| {
                        Ok(RoomRequirement {
                            room_type: r.room_id.parse::<RoomType>()?,
                            level: r.room_level,
                            count: r.room_count,
                        })
                    })
                    .collect::<Result<Vec<_>, TransformError>>()?;
                Ok(ManufactureFormula {
                    id: FormulaId::new(f.formula_id.as_str()),
                    item: ItemId::new(f.item_id.as_str()),
                    count: f.count,
                    weight: f.weight,
                    cost_point: f.cost_point,
                    product,
                    costs: f
                        .costs
                        .iter()
                        .map(|c| FormulaCost {
                            item: ItemId::new(c.id.as_str()),
                            count: c.count,
                        })
                        .collect(),
                    require_rooms,
                })
            })();
            ctx(format!("manufactFormula {key}"), built)
                .map(|formula| (formula.id.clone(), formula))
        })
        .collect()
}

fn transform_layout(b: &RawBuildingData) -> Result<BaseLayout, TransformError> {
    let layout = b
        .layouts
        .get(LAYOUT_ID)
        .ok_or_else(|| TransformError::MissingLayout(LAYOUT_ID.to_owned()))?;
    let mut slots = layout
        .slots
        .values()
        .map(|s| {
            let category = ctx(
                format!("layout slot {}", s.id),
                s.category
                    .parse::<SlotCategory>()
                    .map_err(TransformError::from),
            )?;
            Ok(LayoutSlot {
                id: SlotId::new(s.id.as_str()),
                category,
                size: GridSize {
                    rows: s.size.row,
                    cols: s.size.col,
                },
                offset: GridPos {
                    row: s.offset.row,
                    col: s.offset.col,
                },
                storey: s.storey_id.clone(),
                clean_cost_id: s.clean_cost_id.clone(),
                cost_labor: s.cost_labor,
                provide_labor: s.provide_labor,
            })
        })
        .collect::<Result<Vec<_>, TransformError>>()?;
    slots.sort_by_key(|s| slot_ordinal(s.id.as_str()));
    Ok(BaseLayout {
        id: layout.id.clone(),
        slots,
    })
}

/// `slot_12` → `(12, "")`; anything else sorts after, lexicographically.
fn slot_ordinal(id: &str) -> (u32, String) {
    id.rsplit_once('_')
        .and_then(|(_, n)| n.parse::<u32>().ok())
        .map_or_else(|| (u32::MAX, id.to_owned()), |n| (n, String::new()))
}

fn transform_operator(
    id: &OperatorId,
    building_char: &RawBuildingChar,
    raw: &RawBundle,
    powers: &BTreeMap<PowerId, Power>,
    skills: &BTreeMap<BuffId, BaseSkill>,
    manpower_display_factor: u32,
) -> Result<Operator, TransformError> {
    let character = raw
        .characters
        .get(id.as_str())
        .ok_or(TransformError::MissingCharacter)?;

    let rarity = match &character.rarity {
        RawRarity::Tier(text) => text.parse::<Rarity>()?,
        RawRarity::Index(index) => {
            Rarity::from_index(*index).ok_or(TransformError::RarityIndex(*index))?
        }
    };
    let profession = character.profession.parse::<Profession>()?;

    let power =
        |level: PowerLevel, value: &Option<String>| -> Result<Option<PowerId>, TransformError> {
            match value {
                None => Ok(None),
                Some(v) => {
                    let power = PowerId::new(v.as_str());
                    if powers.contains_key(&power) {
                        Ok(Some(power))
                    } else {
                        Err(TransformError::UnknownPower { level, power })
                    }
                }
            }
        };
    let nation = power(PowerLevel::Nation, &character.nation_id)?;
    let group = power(PowerLevel::Group, &character.group_id)?;
    let team = power(PowerLevel::Team, &character.team_id)?;

    let skill_slots = building_char
        .buff_char
        .iter()
        .enumerate()
        .map(|(slot_index, slot)| {
            let unlocks = slot
                .buff_data
                .iter()
                .map(|bd| {
                    let buff = BuffId::new(bd.buff_id.as_str());
                    if !skills.contains_key(&buff) {
                        return Err(TransformError::MissingBuff(buff));
                    }
                    let phase = bd.cond.phase.parse::<ElitePhase>()?;
                    Ok(SkillUnlock {
                        buff,
                        cond: UnlockCond::new(phase, bd.cond.level),
                    })
                })
                .collect::<Result<Vec<_>, TransformError>>();
            let mut unlocks = ctx(format!("skill slot {slot_index}"), unlocks)?;
            unlocks.sort_by_key(|u| u.cond);
            Ok(SkillSlot { unlocks })
        })
        .collect::<Result<Vec<_>, TransformError>>()?;

    Ok(Operator {
        id: id.clone(),
        name: character.name.clone(),
        appellation: character.appellation.trim().to_owned(),
        rarity,
        profession,
        sub_profession: SubProfessionId::new(character.sub_profession_id.as_str()),
        nation,
        group,
        team,
        max_mood: building_char.max_manpower as f64 / f64::from(manpower_display_factor),
        skill_slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_ordinal_sorts_numerically() {
        let mut ids = vec!["slot_10", "slot_2", "slot_1", "weird"];
        ids.sort_by_key(|s| slot_ordinal(s));
        assert_eq!(ids, vec!["slot_1", "slot_2", "slot_10", "weird"]);
    }
}
