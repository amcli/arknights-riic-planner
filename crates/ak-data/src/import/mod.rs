//! Roster import (Layer 8): other tools' exports → the canonical
//! [`Roster`].
//!
//! Each adapter reads one tool's format and nothing else, so a change to
//! one tool's export breaks one adapter only:
//!
//! - [`krooster`]: Krooster's roster, current and legacy shapes;
//! - [`ak_planner`]: ak-planner's (GoodEffort/Arknights-Planner) export.
//!
//! The adapters share the checks in `Builder`: an operator the pinned game
//! data lacks is skipped with a warning; a promotion the operator's rarity
//! cannot reach, or a level outside what the promotion allows, is clamped
//! with a warning. A file that is not the tool's format, or an entry whose
//! fields have the wrong type, is an [`ImportError`]: nothing is guessed.
//!
//! What no export records: current morale. Every imported entry has
//! `mood: None`, so the simulator's initial-mood policy decides it.

pub mod ak_planner;
pub mod krooster;

use std::collections::BTreeSet;
use std::fmt;

use ak_domain::{ElitePhase, GameData, Roster, RosterEntry, UnlockCond};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A tool whose export can be imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Krooster (krooster.com).
    Krooster,
    /// ak-planner (GoodEffort/Arknights-Planner).
    AkPlanner,
}

impl Source {
    /// Every source.
    pub const ALL: [Source; 2] = [Source::Krooster, Source::AkPlanner];

    /// The name used in requests and on the command line.
    pub const fn as_str(self) -> &'static str {
        match self {
            Source::Krooster => "krooster",
            Source::AkPlanner => "ak-planner",
        }
    }

    /// The tool's name as people write it.
    pub const fn label(self) -> &'static str {
        match self {
            Source::Krooster => "Krooster",
            Source::AkPlanner => "ak-planner",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Source {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Source::ALL
            .into_iter()
            .find(|source| source.as_str() == s)
            .ok_or_else(|| format!("unknown roster source {s:?}; expected krooster or ak-planner"))
    }
}

/// Which shape of a tool's export was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Krooster's current roster (localStorage `v3_roster`): operators keyed
    /// by id.
    KroosterV3,
    /// Krooster's current roster as database rows: a list of operators.
    KroosterV3Rows,
    /// Krooster's earlier roster (localStorage `operators`), with
    /// `promotion` and `owned`.
    KroosterLegacy,
    /// ak-planner's export, `{ s, i, p }`.
    AkPlannerExport,
}

/// Something about an import worth telling the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportWarning {
    /// Not in the pinned game data: released after the snapshot, only on
    /// the CN server, an alternate form (Guard Amiya), or without base
    /// skills. Skipped.
    UnknownOperator { operator: String },
    /// A promotion the operator's rarity cannot reach. Taken as fully
    /// raised at the highest promotion it can reach.
    PromotionClamped {
        operator: String,
        found: u8,
        used: ElitePhase,
        level: u32,
    },
    /// A level outside `1..=max` for the promotion; clamped into range.
    LevelClamped {
        operator: String,
        found: i64,
        used: u32,
    },
    /// The same operator more than once; the last entry was kept.
    Duplicate { operator: String },
    /// Selected in ak-planner without a saved plan. Imported at Elite 0
    /// level 1, which is how ak-planner shows it.
    NoSavedPlan { operator: String },
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    /// The tool the export came from.
    pub source: Source,
    /// Which of its shapes was read.
    pub format: Format,
    /// Operator entries the export listed.
    pub entries: usize,
    /// Operators in the imported roster.
    pub imported: usize,
    /// Entries the export marks as not owned; skipped without a warning
    /// each.
    pub not_owned: usize,
    /// Everything skipped or changed, in the order the entries were read
    /// (keyed exports are read in id order).
    pub warnings: Vec<ImportWarning>,
}

/// A roster and how it was read.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The canonical roster.
    pub roster: Roster,
    /// What happened on the way.
    pub report: ImportReport,
}

/// Why an export could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// The input is not this tool's export at all.
    #[error("this does not look like {}'s export: {reason}", tool.label())]
    NotThisFormat { tool: Source, reason: String },
    /// One entry is malformed.
    #[error("{at}: {reason}")]
    BadEntry { at: String, reason: String },
}

/// Imports an export from `source`.
pub fn import(source: Source, data: &GameData, input: &Value) -> Result<Imported, ImportError> {
    match source {
        Source::Krooster => krooster::import(data, input),
        Source::AkPlanner => ak_planner::import(data, input),
    }
}

/// The checks every adapter shares.
struct Builder<'a> {
    data: &'a GameData,
    roster: Roster,
    warnings: Vec<ImportWarning>,
    seen: BTreeSet<String>,
    entries: usize,
    not_owned: usize,
}

impl<'a> Builder<'a> {
    fn new(data: &'a GameData) -> Self {
        Builder {
            data,
            roster: Roster::new(),
            warnings: Vec::new(),
            seen: BTreeSet::new(),
            entries: 0,
            not_owned: 0,
        }
    }

    /// Counts an entry the export marks as not owned.
    fn not_owned(&mut self) {
        self.entries += 1;
        self.not_owned += 1;
    }

    /// Adds an owned operator at the promotion (0, 1 or 2) and level the
    /// export gives.
    fn add(&mut self, id: &str, elite: u8, level: i64) {
        self.entries += 1;
        let Some(op) = self.data.operator(id) else {
            self.warnings.push(ImportWarning::UnknownOperator {
                operator: id.to_owned(),
            });
            return;
        };
        if !self.seen.insert(id.to_owned()) {
            self.warnings.push(ImportWarning::Duplicate {
                operator: id.to_owned(),
            });
        }
        let found = ElitePhase::ALL[usize::from(elite)];
        let max_phase = op.max_phase();
        let (phase, level) = if found > max_phase {
            let cap = op.max_level(max_phase).unwrap_or(1);
            self.warnings.push(ImportWarning::PromotionClamped {
                operator: id.to_owned(),
                found: elite,
                used: max_phase,
                level: cap,
            });
            (max_phase, cap)
        } else {
            let cap = op.max_level(found).unwrap_or(1);
            let used = u32::try_from(level.clamp(1, i64::from(cap))).unwrap_or(1);
            if i64::from(used) != level {
                self.warnings.push(ImportWarning::LevelClamped {
                    operator: id.to_owned(),
                    found: level,
                    used,
                });
            }
            (found, used)
        };
        self.roster
            .insert(id, RosterEntry::at(UnlockCond::new(phase, level)));
    }

    fn finish(self, source: Source, format: Format) -> Imported {
        let imported = self.roster.entries.len();
        Imported {
            roster: self.roster,
            report: ImportReport {
                source,
                format,
                entries: self.entries,
                imported,
                not_owned: self.not_owned,
                warnings: self.warnings,
            },
        }
    }
}

/// Reads fields of one export entry, naming the entry in every error.
struct Entry<'a> {
    at: String,
    fields: &'a Map<String, Value>,
}

impl<'a> Entry<'a> {
    fn new(at: String, value: &'a Value) -> Result<Self, ImportError> {
        match value.as_object() {
            Some(fields) => Ok(Entry { at, fields }),
            None => Err(ImportError::BadEntry {
                at,
                reason: "expected an object".into(),
            }),
        }
    }

    fn bad(&self, reason: impl Into<String>) -> ImportError {
        ImportError::BadEntry {
            at: self.at.clone(),
            reason: reason.into(),
        }
    }

    fn has(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    fn get(&self, field: &str) -> Option<&'a Value> {
        self.fields.get(field).filter(|v| !v.is_null())
    }

    fn text(&self, field: &str) -> Result<&'a str, ImportError> {
        match self.get(field) {
            Some(Value::String(s)) if !s.is_empty() => Ok(s),
            Some(other) => Err(self.bad(format!("`{field}` must be text, found {other}"))),
            None => Err(self.bad(format!("`{field}` is missing"))),
        }
    }

    /// A whole number. JSON has one number type, so `2.0` counts.
    fn int(&self, field: &str) -> Result<i64, ImportError> {
        let value = self
            .get(field)
            .ok_or_else(|| self.bad(format!("`{field}` is missing")))?;
        let whole = value.as_i64().or_else(|| {
            value
                .as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() < 1e15)
                .map(|f| f as i64)
        });
        whole.ok_or_else(|| self.bad(format!("`{field}` must be a whole number, found {value}")))
    }

    /// A whole number, or `None` when the field is absent.
    fn opt_int(&self, field: &str) -> Result<Option<i64>, ImportError> {
        if self.get(field).is_none() {
            Ok(None)
        } else {
            self.int(field).map(Some)
        }
    }

    /// A promotion: 0, 1 or 2.
    fn elite(&self, field: &str) -> Result<u8, ImportError> {
        let elite = self.int(field)?;
        match u8::try_from(elite) {
            Ok(e) if usize::from(e) < ElitePhase::ALL.len() => Ok(e),
            _ => Err(self.bad(format!("`{field}` must be 0, 1 or 2, found {elite}"))),
        }
    }

    fn object(&self, field: &str) -> Result<Entry<'a>, ImportError> {
        match self.get(field) {
            Some(v) => Entry::new(format!("{} {field}", self.at), v),
            None => Err(self.bad(format!("`{field}` is missing"))),
        }
    }
}
