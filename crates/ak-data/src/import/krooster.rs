//! Krooster (krooster.com, `neeia/ak-roster`) rosters.
//!
//! Krooster has no roster export button. Its roster lives in the browser's
//! local storage and in its database, in one of these shapes (from
//! `src/types/operators/operator.ts` and `src/util/hooks/useOperators.ts`):
//!
//! - **current** (`Format::KroosterV3`), local storage key `v3_roster`: an
//!   object keyed by operator id, each value
//!   `{ op_id, elite, level, potential, skill_level, masteries, modules,
//!   favorite, skin }`. Krooster deletes an operator's entry when its
//!   potential is set to 0, so an entry means owned.
//! - **rows** (`Format::KroosterV3Rows`): the same operators as a list, as
//!   its `operators` table returns them (with a `user_id`).
//! - **legacy** (`Format::KroosterLegacy`), local storage key `operators`:
//!   `{ id, name, rarity, potential, promotion, owned, level, skillLevel,
//!   … }` keyed by id. Krooster itself migrates an entry only when its
//!   potential is non-zero; this adapter also skips entries marked
//!   `owned: false`.
//!
//! To copy the current roster: open krooster.com, open the browser console,
//! and run `copy(localStorage.getItem("v3_roster"))`.
//!
//! Only `elite` (or `promotion`) and `level` matter to the base; potential,
//! skills, masteries, modules and skins are read past.

use ak_domain::GameData;
use serde_json::Value;

use super::{Builder, Entry, Format, ImportError, Imported, Source};

/// Reads a Krooster roster in any of its shapes.
pub fn import(data: &GameData, input: &Value) -> Result<Imported, ImportError> {
    let mut b = Builder::new(data);
    let format = match input {
        Value::Array(rows) => {
            for (i, row) in rows.iter().enumerate() {
                let at = match row.get("op_id").and_then(Value::as_str) {
                    Some(id) => format!("row {i} ({id})"),
                    None => format!("row {i}"),
                };
                current(&mut b, &Entry::new(at, row)?)?;
            }
            Format::KroosterV3Rows
        }
        Value::Object(map) => {
            if map.contains_key("p") && map.contains_key("s") {
                return Err(not_krooster(
                    "it looks like an ak-planner export; import it with source ak-planner",
                ));
            }
            let legacy = match map.values().next() {
                None => false,
                Some(first) if first.get("op_id").is_some() => false,
                Some(first) if first.get("promotion").is_some() => true,
                Some(_) => {
                    return Err(not_krooster(
                        "expected operators keyed by id, each with `op_id` and `elite` \
                         (or, in the older format, `id` and `promotion`)",
                    ));
                }
            };
            for (key, value) in map {
                let entry = Entry::new(key.clone(), value)?;
                if legacy {
                    older(&mut b, &entry)?;
                } else {
                    current(&mut b, &entry)?;
                }
            }
            if legacy {
                Format::KroosterLegacy
            } else {
                Format::KroosterV3
            }
        }
        _ => {
            return Err(not_krooster(
                "expected an object of operators keyed by id, or a list of operator rows",
            ));
        }
    };
    Ok(b.finish(Source::Krooster, format))
}

fn not_krooster(reason: &str) -> ImportError {
    ImportError::NotThisFormat {
        tool: Source::Krooster,
        reason: reason.to_owned(),
    }
}

/// One entry of the current shape.
fn current(b: &mut Builder<'_>, e: &Entry<'_>) -> Result<(), ImportError> {
    if !e.has("op_id") && e.has("promotion") {
        return Err(e.bad("an older-format entry among current-format ones"));
    }
    let id = e.text("op_id")?;
    if e.int("potential")? < 1 {
        b.not_owned();
        return Ok(());
    }
    b.add(id, e.elite("elite")?, e.int("level")?);
    Ok(())
}

/// One entry of the legacy shape.
fn older(b: &mut Builder<'_>, e: &Entry<'_>) -> Result<(), ImportError> {
    if e.has("op_id") {
        return Err(e.bad("a current-format entry among older-format ones"));
    }
    let id = e.text("id")?;
    let owned = !matches!(e.get("owned"), Some(Value::Bool(false)));
    if !owned || e.opt_int("potential")?.unwrap_or(0) < 1 {
        b.not_owned();
        return Ok(());
    }
    b.add(id, e.elite("promotion")?, e.int("level")?);
    Ok(())
}
