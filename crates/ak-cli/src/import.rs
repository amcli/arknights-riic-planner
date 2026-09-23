//! `ak import <source> <export.json>`: run a Layer 8 adapter on another
//! tool's export and print what it read, or write the canonical roster.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;

use ak_data::import::{self, ImportWarning, Imported, Source};
use ak_domain::{ElitePhase, GameData};

/// Imports the export at `path`. With `out`, writes the canonical roster
/// there (the shape a request's `roster` takes); with `json`, prints the
/// roster and the report as JSON instead of a summary.
pub fn run(
    data: &GameData,
    source: Source,
    path: &Path,
    out: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let input: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let imported = import::import(source, data, &input)?;
    if let Some(out) = out {
        let roster = serde_json::to_string_pretty(&imported.roster)?;
        std::fs::write(out, roster + "\n").with_context(|| format!("writing {}", out.display()))?;
    }
    if json {
        let both = serde_json::json!({
            "roster": imported.roster,
            "import": imported.report,
        });
        println!("{}", serde_json::to_string_pretty(&both)?);
    } else {
        print_summary(data, &imported);
        if let Some(out) = out {
            println!("Roster written to {}", out.display());
        }
    }
    Ok(())
}

fn name<'a>(data: &'a GameData, id: &'a str) -> &'a str {
    data.operator(id).map_or(id, |o| o.name.as_str())
}

fn elite(phase: ElitePhase) -> &'static str {
    match phase {
        ElitePhase::E0 => "Elite 0",
        ElitePhase::E1 => "Elite 1",
        ElitePhase::E2 => "Elite 2",
    }
}

fn print_summary(data: &GameData, imported: &Imported) {
    let r = &imported.report;
    let format = serde_json::to_value(r.format).unwrap_or_default();
    println!(
        "Read {}'s export ({}): {} entries, {} imported, {} not owned, {} warnings",
        r.source.label(),
        format.as_str().unwrap_or("?"),
        r.entries,
        r.imported,
        r.not_owned,
        r.warnings.len()
    );
    let mut by_phase: BTreeMap<ElitePhase, usize> = BTreeMap::new();
    for entry in imported.roster.entries.values() {
        *by_phase.entry(entry.promotion.phase).or_default() += 1;
    }
    let phases: Vec<String> = by_phase
        .iter()
        .rev()
        .map(|(phase, n)| format!("{} {n}", elite(*phase)))
        .collect();
    if !phases.is_empty() {
        println!("By promotion: {}", phases.join(", "));
    }
    for w in &r.warnings {
        let line = match w {
            ImportWarning::UnknownOperator { operator } => {
                format!("{operator}: not in the pinned game data; skipped")
            }
            ImportWarning::PromotionClamped {
                operator,
                found,
                used,
                level,
            } => format!(
                "{}: Elite {found} is beyond its rarity; taken as {} level {level}",
                name(data, operator),
                elite(*used)
            ),
            ImportWarning::LevelClamped {
                operator,
                found,
                used,
            } => format!(
                "{}: level {found} is out of range; taken as {used}",
                name(data, operator)
            ),
            ImportWarning::Duplicate { operator } => format!(
                "{}: listed more than once; kept the last",
                name(data, operator)
            ),
            ImportWarning::NoSavedPlan { operator } => format!(
                "{}: selected without a saved plan; taken as Elite 0 level 1",
                name(data, operator)
            ),
        };
        println!("  warning: {line}");
    }
}
