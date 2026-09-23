//! ak-planner (GoodEffort/Arknights-Planner, goodeffort.github.io) exports.
//!
//! Its "Import/Export" button produces (from
//! `src/store/store-operator-functions.ts` and `src/types/plans.ts`):
//!
//! ```text
//! { "s": [operator id, …],                 selected operators
//!   "i": { item id: count, … },            inventory
//!   "p": [ { "operatorId", "active", "sort",
//!            "plans": { "currentElite", "currentLevel", "currentSkillLevels",
//!                       "currentSkillMasteries", "currentModules",
//!                       "targetElite", "targetLevel", … } }, … ] }
//! ```
//!
//! Older exports have `currentModules` as `{ x, y, d }` and no `sort`;
//! nothing read here changed. Ids are upstream `char_…` ids, including
//! CN-only operators and Amiya's alternate forms.
//!
//! The gap: ak-planner is a planner, not a roster. It lists the operators
//! you are planning upgrades for, with their current promotion, and has no
//! notion of ownership. Every operator in the plan is imported as owned at
//! its *current* promotion, inactive plans included; operators you own but
//! are not planning for are absent.

use ak_domain::GameData;
use serde_json::Value;

use super::{Builder, Entry, Format, ImportError, ImportWarning, Imported, Source};

/// Reads an ak-planner export.
pub fn import(data: &GameData, input: &Value) -> Result<Imported, ImportError> {
    let Some(map) = input.as_object() else {
        return Err(not_ak_planner(
            "expected an object with `s`, `i` and `p` lists",
        ));
    };
    let Some(records) = map.get("p").and_then(Value::as_array) else {
        let reason = if map.values().any(|v| v.get("op_id").is_some()) {
            "it looks like a Krooster roster; import it with source krooster"
        } else {
            "no `p` list of saved plans"
        };
        return Err(not_ak_planner(reason));
    };

    let mut b = Builder::new(data);
    let mut planned = Vec::new();
    for (i, record) in records.iter().enumerate() {
        let at = match record.get("operatorId").and_then(Value::as_str) {
            Some(id) => format!("p[{i}] ({id})"),
            None => format!("p[{i}]"),
        };
        let entry = Entry::new(at, record)?;
        let id = entry.text("operatorId")?;
        let plans = entry.object("plans")?;
        b.add(id, plans.elite("currentElite")?, plans.int("currentLevel")?);
        planned.push(id);
    }

    // Selected operators without a saved plan are shown at Elite 0 level 1.
    if let Some(selected) = map.get("s") {
        let Some(selected) = selected.as_array() else {
            return Err(not_ak_planner("`s` must be a list of operator ids"));
        };
        for (i, id) in selected.iter().enumerate() {
            let Some(id) = id.as_str() else {
                return Err(ImportError::BadEntry {
                    at: format!("s[{i}]"),
                    reason: format!("expected an operator id, found {id}"),
                });
            };
            if !planned.contains(&id) {
                b.warnings.push(ImportWarning::NoSavedPlan {
                    operator: id.to_owned(),
                });
                b.add(id, 0, 1);
                planned.push(id);
            }
        }
    }
    Ok(b.finish(Source::AkPlanner, Format::AkPlannerExport))
}

fn not_ak_planner(reason: &str) -> ImportError {
    ImportError::NotThisFormat {
        tool: Source::AkPlanner,
        reason: reason.to_owned(),
    }
}
