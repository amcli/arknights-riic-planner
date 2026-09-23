//! Schema drift detection on the *raw* JSON, run before typed
//! deserialisation.
//!
//! The typed [`crate::raw`] structs ignore unknown fields, so a renamed or
//! newly-required field would surface as a confusing "missing field" error
//! or, worse, silently as a default. This module inspects the untyped
//! `serde_json::Value` and reports:
//!
//! - **errors**: things the transform cannot survive (missing required keys,
//!   unknown enum vocabulary, dangling references);
//! - **warnings**: additions we do not consume yet (new top-level keys, new
//!   description tags) that someone should look at when bumping the pin.

use std::collections::BTreeSet;
use std::str::FromStr;

use ak_domain::{BuffCategory, EfficiencyTarget, ElitePhase, ProductType, Profession, RoomType};
use serde_json::Value;

/// Result of a schema check.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SchemaReport {
    /// Problems that will break the transform.
    pub errors: Vec<String>,
    /// Additions worth a look; the transform tolerates them.
    pub warnings: Vec<String>,
}

impl SchemaReport {
    /// True when there are no errors (warnings are allowed).
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    /// Appends another report's findings, prefixing each with `prefix`.
    pub fn merge(&mut self, prefix: &str, other: SchemaReport) {
        self.errors
            .extend(other.errors.into_iter().map(|e| format!("{prefix}: {e}")));
        self.warnings
            .extend(other.warnings.into_iter().map(|w| format!("{prefix}: {w}")));
    }
}

/// Top-level keys of `building_data.json` the transform reads.
pub const BUILDING_REQUIRED_KEYS: &[&str] = &[
    "controlSlotId",
    "meetingSlotId",
    "laborRecoverTime",
    "manufactInputCapacity",
    "shopCounterCapacity",
    "comfortLimit",
    "manpowerDisplayFactor",
    "basicFavorPerDay",
    "tiredApThreshold",
    "manufactManpowerCostByNum",
    "tradingManpowerCostByNum",
    "rooms",
    "layouts",
    "controlData",
    "manufactData",
    "dormData",
    "tradingData",
    "powerData",
    "meetingData",
    "hireData",
    "trainingData",
    "workshopData",
    "chars",
    "buffs",
    "manufactFormulas",
];

/// Every top-level key observed in the pinned snapshot. New keys are a
/// warning, not an error.
pub const BUILDING_KNOWN_KEYS: &[&str] = &[
    "controlSlotId",
    "meetingSlotId",
    "initMaxLabor",
    "laborRecoverTime",
    "manufactInputCapacity",
    "shopCounterCapacity",
    "comfortLimit",
    "creditInitiativeLimit",
    "creditPassiveLimit",
    "creditComfortFactor",
    "creditGuaranteed",
    "creditCeiling",
    "manufactUnlockTips",
    "shopUnlockTips",
    "manufactStationBuff",
    "comfortManpowerRecoverFactor",
    "manpowerDisplayFactor",
    "shopOutputRatio",
    "shopStackRatio",
    "basicFavorPerDay",
    "humanResourceLimit",
    "tiredApThreshold",
    "processedCountRatio",
    "tradingStrategyUnlockLevel",
    "tradingReduceTimeUnit",
    "tradingLaborCostUnit",
    "manufactReduceTimeUnit",
    "manufactLaborCostUnit",
    "laborAssistUnlockLevel",
    "apToLaborUnlockLevel",
    "apToLaborRatio",
    "socialResourceLimit",
    "socialSlotNum",
    "furniDuplicationLimit",
    "assistFavorReport",
    "manufactManpowerCostByNum",
    "tradingManpowerCostByNum",
    "trainingBonusMax",
    "betaRemoveTime",
    "furniHighlightTime",
    "canNotVisitToast",
    "musicPlayerOpenTime",
    "roomsWithoutRemoveStaff",
    "privateFavorLevelThresholds",
    "roomUnlockConds",
    "rooms",
    "layouts",
    "prefabs",
    "controlData",
    "manufactData",
    "shopData",
    "hireData",
    "dormData",
    "privateRoomData",
    "meetingData",
    "tradingData",
    "workshopData",
    "trainingData",
    "powerData",
    "chars",
    "buffs",
    "workshopBonus",
    "customData",
    "manufactFormulas",
    "shopFormulas",
    "workshopFormulas",
    "creditFormula",
    "goldItems",
    "assistantUnlock",
    "workshopRarities",
    "todoItemSortPriorityDict",
    "slotPrequeDatas",
    "dormitoryPrequeDatas",
    "workshopTargetDesDict",
    "tradingOrderDesDict",
    "stationManageConstData",
    "stationManageFilterInfos",
    "musicData",
    "emojis",
    "categoryNames",
    "buffSortData",
    // Present in zh_CN as of 2026-09 but not in the archived en_US snapshot.
    "meetingMessageBoardEmoteTime",
    "tradingRoomInfoData",
];

/// Fields every `buffs` entry carries.
pub const BUFF_KEYS: &[&str] = &[
    "buffId",
    "buffName",
    "buffIcon",
    "skillIcon",
    "sortId",
    "buffColor",
    "textColor",
    "buffCategory",
    "roomType",
    "description",
    "efficiency",
    "targetGroupSortId",
    "targets",
];

/// Fields every `chars` entry carries.
pub const BUILDING_CHAR_KEYS: &[&str] = &["charId", "maxManpower", "buffChar"];

/// `<@…>` tags the rich-text parser classifies. Others still parse (as
/// `RichTag::Other`) but are reported.
pub const KNOWN_DESCRIPTION_TAGS: &[&str] = &["cc.vup", "cc.vdown", "cc.kw", "cc.rem"];

/// Fields the transform reads from a `character_table` entry.
pub const CHARACTER_REQUIRED_KEYS: &[&str] = &[
    "name",
    "nationId",
    "groupId",
    "teamId",
    "rarity",
    "profession",
    "subProfessionId",
    "phases",
];

/// Professions that appear in `character_table` but never in
/// `building_data.chars`.
pub const NON_OPERATOR_PROFESSIONS: &[&str] = &["TOKEN", "TRAP"];

/// Fields the transform reads from a `handbook_team_table` entry.
pub const TEAM_REQUIRED_KEYS: &[&str] = &["powerId", "orderNum", "powerLevel", "powerName"];

/// Checks all three files together (cross-file references included).
pub fn check_bundle(building: &Value, characters: &Value, teams: &Value) -> SchemaReport {
    let mut report = SchemaReport::default();
    report.merge("building_data.json", check_building(building));
    report.merge("character_table.json", check_characters(characters));
    report.merge("handbook_team_table.json", check_teams(teams));

    // Cross-file: every building char must exist in character_table.
    if let (Some(chars), Some(table)) = (
        building.get("chars").and_then(Value::as_object),
        characters.as_object(),
    ) {
        let missing: Vec<_> = chars.keys().filter(|k| !table.contains_key(*k)).collect();
        if !missing.is_empty() {
            report.error(format!(
                "{} building_data.chars entries are absent from character_table: {}",
                missing.len(),
                sample(missing.iter().map(|s| s.as_str()))
            ));
        }
    }
    report
}

/// Checks `building_data.json`.
pub fn check_building(v: &Value) -> SchemaReport {
    let mut r = SchemaReport::default();
    let Some(root) = v.as_object() else {
        r.error("root is not an object");
        return r;
    };

    for key in BUILDING_REQUIRED_KEYS {
        if !root.contains_key(*key) {
            r.error(format!("missing required top-level key {key:?}"));
        }
    }
    let unknown_top: Vec<_> = root
        .keys()
        .filter(|k| !BUILDING_KNOWN_KEYS.contains(&k.as_str()))
        .collect();
    if !unknown_top.is_empty() {
        r.warn(format!(
            "new top-level keys (not consumed): {}",
            sample(unknown_top.iter().map(|s| s.as_str()))
        ));
    }

    check_rooms(&mut r, root.get("rooms"));
    let buff_ids = check_buffs(&mut r, root.get("buffs"));
    check_building_chars(&mut r, root.get("chars"), &buff_ids);
    check_formulas(&mut r, root.get("manufactFormulas"));

    match root.get("layouts").and_then(Value::as_object) {
        Some(layouts) if layouts.contains_key(crate::transform::LAYOUT_ID) => {}
        Some(_) => r.error(format!(
            "layouts has no {:?} entry",
            crate::transform::LAYOUT_ID
        )),
        None => {}
    }
    r
}

fn check_rooms(r: &mut SchemaReport, rooms: Option<&Value>) {
    let Some(rooms) = rooms.and_then(Value::as_object) else {
        return;
    };
    for key in rooms.keys() {
        if RoomType::from_str(key).is_err() {
            r.error(format!(
                "rooms has unknown room type {key:?}; add it to RoomType"
            ));
        }
    }
    for rt in RoomType::ALL {
        if !rooms.contains_key(rt.as_str()) {
            r.error(format!("rooms is missing {}", rt.as_str()));
        }
    }
}

fn check_buffs(r: &mut SchemaReport, buffs: Option<&Value>) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let Some(buffs) = buffs.and_then(Value::as_object) else {
        return ids;
    };
    let mut missing_fields = BTreeSet::new();
    let mut extra_fields = BTreeSet::new();
    let mut unknown_rooms = BTreeSet::new();
    let mut unknown_categories = BTreeSet::new();
    let mut unknown_targets = BTreeSet::new();
    let mut unknown_tags = BTreeSet::new();
    let mut key_mismatches = 0usize;

    for (key, buff) in buffs {
        ids.insert(key.clone());
        let Some(obj) = buff.as_object() else {
            r.error(format!("buff {key:?} is not an object"));
            continue;
        };
        for field in BUFF_KEYS {
            if !obj.contains_key(*field) {
                missing_fields.insert(*field);
            }
        }
        for field in obj.keys() {
            if !BUFF_KEYS.contains(&field.as_str()) {
                extra_fields.insert(field.clone());
            }
        }
        if obj.get("buffId").and_then(Value::as_str) != Some(key.as_str()) {
            key_mismatches += 1;
        }
        if let Some(room) = obj.get("roomType").and_then(Value::as_str)
            && RoomType::from_str(room).is_err()
        {
            unknown_rooms.insert(room.to_owned());
        }
        if let Some(cat) = obj.get("buffCategory").and_then(Value::as_str)
            && BuffCategory::from_str(cat).is_err()
        {
            unknown_categories.insert(cat.to_owned());
        }
        if let Some(targets) = obj.get("targets").and_then(Value::as_array) {
            for t in targets.iter().filter_map(Value::as_str) {
                if EfficiencyTarget::from_str(t).is_err() {
                    unknown_targets.insert(t.to_owned());
                }
            }
        }
        if let Some(desc) = obj.get("description").and_then(Value::as_str) {
            for tag in at_tags(desc) {
                if !KNOWN_DESCRIPTION_TAGS.contains(&tag) {
                    unknown_tags.insert(tag.to_owned());
                }
            }
        }
    }

    if !missing_fields.is_empty() {
        r.error(format!(
            "buffs entries are missing fields: {missing_fields:?}"
        ));
    }
    if !extra_fields.is_empty() {
        r.warn(format!("buffs entries have new fields: {extra_fields:?}"));
    }
    if key_mismatches > 0 {
        r.error(format!(
            "{key_mismatches} buffs entries have buffId != map key"
        ));
    }
    if !unknown_rooms.is_empty() {
        r.error(format!(
            "buffs use unknown roomType values: {unknown_rooms:?}"
        ));
    }
    if !unknown_categories.is_empty() {
        r.error(format!(
            "buffs use unknown buffCategory values: {unknown_categories:?}"
        ));
    }
    if !unknown_targets.is_empty() {
        r.error(format!(
            "buffs use unknown targets values: {unknown_targets:?}"
        ));
    }
    if !unknown_tags.is_empty() {
        r.warn(format!(
            "descriptions use unclassified <@…> tags: {unknown_tags:?}"
        ));
    }
    ids
}

fn check_building_chars(r: &mut SchemaReport, chars: Option<&Value>, buff_ids: &BTreeSet<String>) {
    let Some(chars) = chars.and_then(Value::as_object) else {
        return;
    };
    let mut missing_fields = BTreeSet::new();
    let mut unknown_phases = BTreeSet::new();
    let mut dangling = BTreeSet::new();

    for (key, c) in chars {
        let Some(obj) = c.as_object() else {
            r.error(format!("chars {key:?} is not an object"));
            continue;
        };
        for field in BUILDING_CHAR_KEYS {
            if !obj.contains_key(*field) {
                missing_fields.insert(*field);
            }
        }
        let slots = obj
            .get("buffChar")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for slot in slots {
            let data = slot
                .get("buffData")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for bd in data {
                if let Some(id) = bd.get("buffId").and_then(Value::as_str)
                    && !buff_ids.contains(id)
                {
                    dangling.insert(id.to_owned());
                }
                if let Some(phase) = bd
                    .get("cond")
                    .and_then(|c| c.get("phase"))
                    .and_then(Value::as_str)
                    && ElitePhase::from_str(phase).is_err()
                {
                    unknown_phases.insert(phase.to_owned());
                }
            }
        }
    }

    if !missing_fields.is_empty() {
        r.error(format!(
            "chars entries are missing fields: {missing_fields:?}"
        ));
    }
    if !unknown_phases.is_empty() {
        r.error(format!(
            "chars use unknown cond.phase values: {unknown_phases:?}"
        ));
    }
    if !dangling.is_empty() {
        r.error(format!(
            "chars reference {} buffIds absent from buffs: {}",
            dangling.len(),
            sample(dangling.iter().map(String::as_str))
        ));
    }
}

fn check_formulas(r: &mut SchemaReport, formulas: Option<&Value>) {
    let Some(formulas) = formulas.and_then(Value::as_object) else {
        return;
    };
    let mut unknown_types = BTreeSet::new();
    let mut unknown_rooms = BTreeSet::new();
    for f in formulas.values() {
        if let Some(t) = f.get("formulaType").and_then(Value::as_str)
            && ProductType::from_str(t).is_err()
        {
            unknown_types.insert(t.to_owned());
        }
        let rooms = f
            .get("requireRooms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for room in rooms {
            if let Some(id) = room.get("roomId").and_then(Value::as_str)
                && RoomType::from_str(id).is_err()
            {
                unknown_rooms.insert(id.to_owned());
            }
        }
    }
    if !unknown_types.is_empty() {
        r.error(format!(
            "manufactFormulas use unknown formulaType values: {unknown_types:?}"
        ));
    }
    if !unknown_rooms.is_empty() {
        r.error(format!(
            "manufactFormulas require unknown roomId values: {unknown_rooms:?}"
        ));
    }
}

/// Checks `character_table.json`.
pub fn check_characters(v: &Value) -> SchemaReport {
    let mut r = SchemaReport::default();
    let Some(table) = v.as_object() else {
        r.error("root is not an object");
        return r;
    };
    let mut missing_fields = BTreeSet::new();
    let mut bad_rarity = BTreeSet::new();
    let mut unknown_professions = BTreeSet::new();

    for (key, c) in table {
        if !key.starts_with("char_") {
            continue;
        }
        let Some(obj) = c.as_object() else {
            r.error(format!("{key:?} is not an object"));
            continue;
        };
        for field in CHARACTER_REQUIRED_KEYS {
            if !obj.contains_key(*field) {
                missing_fields.insert(*field);
            }
        }
        match obj.get("rarity") {
            Some(Value::String(s)) if s.starts_with("TIER_") => {}
            Some(Value::Number(n)) if n.as_u64().is_some_and(|n| n <= 5) => {}
            Some(other) => {
                bad_rarity.insert(other.to_string());
            }
            None => {}
        }
        if let Some(p) = obj.get("profession").and_then(Value::as_str)
            && Profession::from_str(p).is_err()
            && !NON_OPERATOR_PROFESSIONS.contains(&p)
        {
            unknown_professions.insert(p.to_owned());
        }
    }

    if !missing_fields.is_empty() {
        r.error(format!(
            "char_ entries are missing fields: {missing_fields:?}"
        ));
    }
    if !bad_rarity.is_empty() {
        r.error(format!(
            "char_ entries have unrecognised rarity encodings: {bad_rarity:?}"
        ));
    }
    if !unknown_professions.is_empty() {
        r.warn(format!(
            "char_ entries use professions not in Profession: {unknown_professions:?}"
        ));
    }
    r
}

/// Checks `handbook_team_table.json`.
pub fn check_teams(v: &Value) -> SchemaReport {
    let mut r = SchemaReport::default();
    let Some(table) = v.as_object() else {
        r.error("root is not an object");
        return r;
    };
    let mut missing_fields = BTreeSet::new();
    let mut bad_levels = BTreeSet::new();
    for (key, t) in table {
        let Some(obj) = t.as_object() else {
            r.error(format!("{key:?} is not an object"));
            continue;
        };
        for field in TEAM_REQUIRED_KEYS {
            if !obj.contains_key(*field) {
                missing_fields.insert(*field);
            }
        }
        match obj.get("powerLevel").and_then(Value::as_i64) {
            Some(0..=2) | None => {}
            Some(other) => {
                bad_levels.insert(other);
            }
        }
    }
    if !missing_fields.is_empty() {
        r.error(format!("entries are missing fields: {missing_fields:?}"));
    }
    if !bad_levels.is_empty() {
        r.error(format!(
            "entries use unknown powerLevel values: {bad_levels:?}"
        ));
    }
    r
}

/// Names of every `<@name>` tag in a description.
fn at_tags(desc: &str) -> impl Iterator<Item = &str> {
    desc.match_indices("<@").filter_map(move |(i, _)| {
        let rest = &desc[i + 2..];
        let end = rest.find('>')?;
        let name = &rest[..end];
        (!name.is_empty() && !name.contains('<')).then_some(name)
    })
}

/// Up to five items, then "… (+N more)".
fn sample<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let items: Vec<&str> = items.collect();
    let shown: Vec<&str> = items.iter().take(5).copied().collect();
    if items.len() > 5 {
        format!("{} (+{} more)", shown.join(", "), items.len() - 5)
    } else {
        shown.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn at_tags_extracts_names() {
        let tags: Vec<_> =
            at_tags("a <@cc.vup>b</> <$cc.x><@cc.kw>c</></> <<@typo>x</> <@unterminated").collect();
        // `<<@typo>` still yields a tag (the parser treats the first `<` as
        // literal); an unterminated `<@…` is not a tag.
        assert_eq!(tags, vec!["cc.vup", "cc.kw", "typo"]);
    }

    #[test]
    fn flags_unknown_room_type_and_missing_keys() {
        let v = json!({
            "rooms": { "CONTROL": {}, "GARAGE": {} },
            "buffs": {},
            "chars": {},
            "layouts": {}
        });
        let r = check_building(&v);
        assert!(!r.is_ok());
        assert!(r.errors.iter().any(|e| e.contains("GARAGE")));
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("missing required top-level key"))
        );
        assert!(r.errors.iter().any(|e| e.contains("layouts has no")));
    }

    #[test]
    fn flags_dangling_buff_reference() {
        let mut root = serde_json::Map::new();
        for k in BUILDING_REQUIRED_KEYS {
            root.insert((*k).to_owned(), json!(0));
        }
        root.insert(
            "rooms".into(),
            Value::Object(
                RoomType::ALL
                    .iter()
                    .map(|rt| (rt.as_str().to_owned(), json!({})))
                    .collect(),
            ),
        );
        root.insert("layouts".into(), json!({ "v0": {} }));
        root.insert("buffs".into(), json!({}));
        root.insert(
            "chars".into(),
            json!({ "char_1": { "charId": "char_1", "maxManpower": 1,
                "buffChar": [ { "buffData": [ { "buffId": "nope[000]", "cond": { "phase": "PHASE_9", "level": 1 } } ] } ] } }),
        );
        root.insert("manufactFormulas".into(), json!({}));
        let r = check_building(&Value::Object(root));
        assert!(r.errors.iter().any(|e| e.contains("nope[000]")));
        assert!(r.errors.iter().any(|e| e.contains("PHASE_9")));
    }

    #[test]
    fn teams_power_level_range() {
        let r = check_teams(
            &json!({ "x": { "powerId": "x", "orderNum": 1, "powerLevel": 7, "powerName": "X" } }),
        );
        assert!(r.errors.iter().any(|e| e.contains("powerLevel")));
    }
}
