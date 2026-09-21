//! Layer 2: parse skill descriptions into [`Mechanics`].
//!
//! Upstream ships no structured effect data, so the localised description is
//! the only source. It is, however, highly templated. Pipeline per tier:
//!
//! 1. [`template::templatize`]: rich text → a flat string where every markup
//!    span becomes a `{V:…}` (value), `{K:…}` (keyword), `{T:…}` (glossary
//!    term id) or `{R:…}` placeholder.
//! 2. [`prefix::parse_prefix`]: strip "When this Operator is assigned to …",
//!    extracting any predicate or Workshop material filter it carries.
//! 3. [`clause::preprocess`] + [`clause::split`]: drop parenthetical notes
//!    (after harvesting stacking / cap information) and divide the body into
//!    clauses.
//! 4. [`clause::parse_clause`]: each clause → predicate + effect via the
//!    rule tables in [`rules`].
//!
//! The parser never guesses. An unrecognised phrase fails the whole tier
//! ([`Outcome::Unparsed`]); a recognised-but-unquantifiable phrase becomes an
//! explicit `Unmodeled` node ([`Outcome::Partial`]). Both are reported.

pub mod clause;
pub mod prefix;
pub mod rules;
pub mod template;
pub mod terms;
pub mod value;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ak_domain::*;

/// Per-tier parsing context shared by the rule tables.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The room the skill applies in.
    pub room: RoomType,
    /// Workshop material filter from the prefix.
    pub material: Option<MaterialFilter>,
    /// Workshop base-cost filter from the prefix.
    pub base_cost: Option<CostFilter>,
    /// "caps at N Operators" harvested from parentheticals.
    pub max_count: Option<f64>,
    /// "(max N)" harvested from parentheticals.
    pub max_total: Option<f64>,
}

/// Result of parsing one tier.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Every part is modelled.
    Parsed(Mechanics),
    /// Parsed, but contains `Unmodeled` parts.
    Partial(Mechanics),
    /// Could not be parsed at all.
    Unparsed {
        /// The templatized description, for grouping failures.
        template: String,
        /// Why.
        reason: String,
    },
}

/// Parses one skill tier's description.
pub fn parse_skill(skill: &BaseSkill) -> Outcome {
    let template = template::templatize(&skill.description);
    match parse_template(&template, skill.room_type) {
        Ok(m) if m.is_fully_modeled() => Outcome::Parsed(m),
        Ok(m) => Outcome::Partial(m),
        Err(reason) => Outcome::Unparsed { template, reason },
    }
}

/// Parses an already-templatized description.
pub fn parse_template(template: &str, room: RoomType) -> Result<Mechanics, String> {
    let pre = prefix::parse_prefix(template, room)?;
    let (body, notes) = clause::preprocess(&pre.body);
    let ctx = Ctx {
        room,
        material: pre.material,
        base_cost: pre.base_cost,
        max_count: notes.max_count,
        max_total: notes.max_total,
    };

    let mut clauses: Vec<Clause> = Vec::new();
    match rules::match_compound(&body, &ctx)? {
        Some(found) => clauses = found,
        None => {
            for piece in clause::split(&body) {
                clause::parse_clause(&piece, &ctx, &mut clauses)?;
            }
        }
    }
    if clauses.is_empty() {
        return Err("no clauses".to_owned());
    }
    for c in &mut clauses {
        let own = std::mem::replace(&mut c.when, Predicate::Always);
        // A clause with its own morale threshold is an alternative to the
        // prefix's threshold ("above 12: X; below 12: Y instead"), not a
        // refinement of it.
        c.when = if matches!(
            own,
            Predicate::SelfMoodAbove { .. } | Predicate::SelfMoodBelow { .. }
        ) {
            own
        } else {
            pre.predicate.clone().and(own)
        };

        // Control Center skills that name factory / trading stats act on
        // every such room, not on the Control Center itself.
        if room == RoomType::Control {
            match &mut c.effect {
                Effect::Productivity { scope, .. } | Effect::Capacity { scope, .. }
                    if *scope == Scope::ThisRoom =>
                {
                    *scope = Scope::AllRooms(RoomType::Manufacture);
                }
                Effect::OrderEfficiency { scope, .. } | Effect::OrderLimit { scope, .. }
                    if *scope == Scope::ThisRoom =>
                {
                    *scope = Scope::AllRooms(RoomType::Trading);
                }
                _ => {}
            }
        }
        if notes.excluding_self
            && let Effect::Mood {
                target: target @ MoodTarget::AllInRoom,
                ..
            } = &mut c.effect
        {
            *target = MoodTarget::OthersInRoom;
        }
    }
    Ok(Mechanics {
        clauses,
        stacking: notes.stacking,
    })
}

/// A tier the parser rejected.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UnparsedSkill {
    pub id: BuffId,
    pub template: String,
    pub reason: String,
}

/// A tier with unmodeled parts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PartialSkill {
    pub id: BuffId,
    pub unmodeled: Vec<String>,
}

/// Coverage of the description parser over a dataset.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct MechanicsReport {
    /// Skill tiers examined.
    pub tiers: usize,
    /// Fully modelled.
    pub parsed: usize,
    /// Parsed with `Unmodeled` parts.
    pub partial: usize,
    /// Rejected.
    pub unparsed: usize,
    /// Every rejected tier.
    pub unparsed_skills: Vec<UnparsedSkill>,
    /// Every partial tier.
    pub partial_skills: Vec<PartialSkill>,
    /// Operator names in descriptions that matched no operator (or several).
    pub unresolved_names: Vec<String>,
}

impl MechanicsReport {
    /// Fully-modelled tiers as a percentage of all tiers.
    pub fn coverage_pct(&self) -> f64 {
        if self.tiers == 0 {
            0.0
        } else {
            100.0 * self.parsed as f64 / self.tiers as f64
        }
    }

    /// Parsed-or-partial tiers as a percentage of all tiers.
    pub fn accepted_pct(&self) -> f64 {
        if self.tiers == 0 {
            0.0
        } else {
            100.0 * (self.parsed + self.partial) as f64 / self.tiers as f64
        }
    }
}

/// Parses every skill, resolves operator-name references against the
/// operator table, and stores the result on each [`BaseSkill::mechanics`].
pub fn attach(
    skills: &mut BTreeMap<BuffId, BaseSkill>,
    operators: &BTreeMap<OperatorId, Operator>,
) -> MechanicsReport {
    let mut by_name: HashMap<&str, Vec<&OperatorId>> = HashMap::new();
    for op in operators.values() {
        by_name.entry(op.name.as_str()).or_default().push(&op.id);
    }

    let mut report = MechanicsReport {
        tiers: skills.len(),
        ..MechanicsReport::default()
    };
    let mut unresolved = BTreeSet::new();

    for skill in skills.values_mut() {
        let mut resolve = |r: &mut OperatorRef| match by_name.get(r.name.as_str()) {
            Some(ids) if ids.len() == 1 => r.id = Some(ids[0].clone()),
            _ => {
                unresolved.insert(r.name.clone());
            }
        };
        match parse_skill(skill) {
            Outcome::Parsed(mut m) => {
                link(&mut m, &mut resolve);
                report.parsed += 1;
                skill.mechanics = Some(m);
            }
            Outcome::Partial(mut m) => {
                link(&mut m, &mut resolve);
                report.partial += 1;
                report.partial_skills.push(PartialSkill {
                    id: skill.id.clone(),
                    unmodeled: m.unmodeled_parts(),
                });
                skill.mechanics = Some(m);
            }
            Outcome::Unparsed { template, reason } => {
                report.unparsed += 1;
                tracing::debug!(skill = %skill.id, %reason, "unparsed skill description");
                report.unparsed_skills.push(UnparsedSkill {
                    id: skill.id.clone(),
                    template,
                    reason,
                });
                skill.mechanics = None;
            }
        }
    }
    report.unresolved_names = unresolved.into_iter().collect();
    report
}

fn link(m: &mut Mechanics, resolve: &mut dyn FnMut(&mut OperatorRef)) {
    for clause in &mut m.clauses {
        clause.when.operator_refs_mut(resolve);
        clause.effect.operator_refs_mut(resolve);
    }
}
