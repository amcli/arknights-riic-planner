//! `ak`: developer CLI for poking at the loaded game data.
//!
//! ```text
//! ak stats                     # counts for the README / sanity checks
//! ak op char_285_medic2        # one operator with resolved skills
//! ak skill "manu_prod_spd[000]"  # one skill tier with parsed mechanics
//! ak find lancet               # search operators by name / id
//! ak skipped                   # operators the lenient transform dropped
//! ak mechanics                 # Layer 2 coverage: rejected / partial tiers
//! ak simulate request.json     # Layer 4: run the mood-aware simulator
//! ak simulate --evaluate r.json # Layer 4: instantaneous room stats only
//! ak solve request.json        # Layer 5: search for better assignments
//! ak import krooster export.json  # Layer 8: read another tool's roster
//! ```

use std::path::PathBuf;

mod import;
mod simulate;
mod solve;

use anyhow::Context;
use clap::{Parser, Subcommand};

use ak_data::{Loaded, Strictness, load_source, stats};
use ak_domain::{Amount, BaseSkill, Counter, GameData, Operator, Predicate};

#[derive(Parser)]
#[command(name = "ak", version, about)]
struct Cli {
    /// Data root containing manifest.toml (default: <workspace>/data).
    #[arg(long, env = ak_data::paths::DATA_DIR_ENV)]
    data_root: Option<PathBuf>,
    /// Manifest source to load (default: the manifest's default_source).
    #[arg(long)]
    source: Option<String>,
    /// Fail on any per-operator transform problem instead of skipping.
    #[arg(long)]
    strict: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Summary counts as JSON.
    Stats,
    /// Show one operator with its skill progression.
    Op {
        /// Upstream id, e.g. char_285_medic2.
        id: String,
        /// Emit the raw domain struct as JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
    /// Show one skill tier and its parsed mechanics.
    Skill {
        /// Upstream buff id, e.g. "manu_prod_spd[000]".
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Search operators by name or id (case-insensitive substring).
    Find { query: String },
    /// List operators the lenient transform skipped, with reasons.
    Skipped,
    /// Layer 2 description-parser coverage: rejected and partial tiers.
    Mechanics {
        /// Also list tiers that parsed with unmodeled parts.
        #[arg(long)]
        partial: bool,
    },
    /// Layer 4: simulate a request file (base, assignment, roster, config,
    /// rotation) and print totals, per-room and per-operator reports.
    Simulate {
        /// Path to a JSON `SimRequest` (see examples/requests/).
        request: PathBuf,
        /// Emit the full `SimResult` (or `Snapshot`) as JSON.
        #[arg(long)]
        json: bool,
        /// Only evaluate the starting instant: room stats and morale rates.
        #[arg(long)]
        evaluate: bool,
    },
    /// Layer 5: solve a request file (base, roster, objective, solver
    /// settings) and print the best assignments found.
    Solve {
        /// Path to a JSON `SolveRequest` (see examples/requests/).
        request: PathBuf,
        /// Emit the full `SolveResult` as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Layer 8: read another tool's roster export and report what was
    /// imported, skipped or clamped.
    Import {
        /// The tool that made the export: krooster or ak-planner.
        source: ak_data::import::Source,
        /// Path to the export (JSON).
        export: PathBuf,
        /// Write the canonical roster here, ready to use as a request's
        /// `roster`.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Print the roster and the report as JSON.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let root = cli
        .data_root
        .unwrap_or_else(ak_data::paths::default_data_root);
    let strictness = if cli.strict {
        Strictness::Strict
    } else {
        Strictness::Lenient
    };
    let loaded =
        load_source(&root, cli.source.as_deref(), strictness).context("loading game data")?;

    match cli.cmd {
        Cmd::Stats => print_json(&stats::compute(&loaded.data))?,
        Cmd::Op { id, json } => {
            let op = loaded
                .data
                .operator(&id)
                .with_context(|| format!("no operator {id:?} (try `ak find`)"))?;
            if json {
                print_json(op)?;
            } else {
                print_operator(&loaded.data, op);
            }
        }
        Cmd::Skill { id, json } => {
            let skill = loaded
                .data
                .skill(&id)
                .with_context(|| format!("no skill {id:?}"))?;
            if json {
                print_json(skill)?;
            } else {
                print_skill(skill);
            }
        }
        Cmd::Find { query } => {
            let q = query.to_lowercase();
            let mut hits: Vec<&Operator> = loaded
                .data
                .operators
                .values()
                .filter(|op| {
                    op.name.to_lowercase().contains(&q)
                        || op.appellation.to_lowercase().contains(&q)
                        || op.id.as_str().contains(&q)
                })
                .collect();
            hits.sort_by(|a, b| b.rarity.cmp(&a.rarity).then_with(|| a.name.cmp(&b.name)));
            for op in hits {
                println!(
                    "{:<24} {:<20} {}★ {:<10} {}",
                    op.id,
                    op.name,
                    op.rarity.stars(),
                    op.profession,
                    op.sub_profession
                );
            }
        }
        Cmd::Skipped => print_skipped(&loaded),
        Cmd::Mechanics { partial } => print_mechanics(&loaded, partial),
        Cmd::Simulate {
            request,
            json,
            evaluate,
        } => simulate::run(&loaded.data, &request, json, evaluate)?,
        Cmd::Solve { request, json } => solve::run(&loaded.data, &request, json)?,
        Cmd::Import {
            source,
            export,
            out,
            json,
        } => import::run(&loaded.data, source, &export, out.as_deref(), json)?,
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_operator(data: &GameData, op: &Operator) {
    println!(
        "{} — {} ({}★ {} / {})",
        op.id,
        op.name,
        op.rarity.stars(),
        op.profession,
        op.sub_profession
    );
    let affiliations: Vec<String> = op
        .powers()
        .map(|p| match data.power(p.as_str()) {
            Some(power) => format!("{} ({})", power.name, power.level),
            None => p.to_string(),
        })
        .collect();
    if !affiliations.is_empty() {
        println!("  affiliations: {}", affiliations.join(", "));
    }
    println!("  max mood: {}", op.max_mood);
    for (i, slot) in op.skill_slots.iter().enumerate() {
        println!("  slot {}:", i + 1);
        if slot.unlocks.is_empty() {
            println!("    (empty)");
        }
        for unlock in &slot.unlocks {
            match data.skill(unlock.buff.as_str()) {
                Some(skill) => {
                    println!(
                        "    {:<8} {} — {} [{}]",
                        unlock.cond.to_string(),
                        unlock.buff,
                        skill.name,
                        skill.room_type
                    );
                    println!("             {}", skill.description.plain());
                }
                None => println!(
                    "    {:<8} {} (unresolved)",
                    unlock.cond.to_string(),
                    unlock.buff
                ),
            }
        }
    }
}

fn print_skill(skill: &BaseSkill) {
    println!(
        "{} — {} [{}, {}]",
        skill.id, skill.name, skill.room_type, skill.category
    );
    println!("  {}", skill.description.plain());
    if skill.efficiency_hint != 0 || !skill.targets.is_empty() {
        let targets: Vec<_> = skill.targets.iter().map(ToString::to_string).collect();
        println!(
            "  efficiency hint: {}%  targets: {}",
            skill.efficiency_hint,
            targets.join(", ")
        );
    }
    match &skill.mechanics {
        None => println!("  mechanics: (unparsed)"),
        Some(m) => {
            println!(
                "  mechanics: {} clause(s), stacking {:?}{}",
                m.clauses.len(),
                m.stacking,
                if m.is_fully_modeled() {
                    ""
                } else {
                    ", PARTIAL"
                }
            );
            for c in &m.clauses {
                let when = match &c.when {
                    Predicate::Always => String::new(),
                    other => format!(" when {}", short(&format!("{other:?}"))),
                };
                let amount = c.effect.amount().map(describe_amount).unwrap_or_default();
                println!(
                    "    - {}{} {}{}",
                    c.effect.kind_name(),
                    amount,
                    short(&format!("{:?}", c.effect)),
                    when
                );
            }
        }
    }
}

fn describe_amount(a: &Amount) -> String {
    match a {
        Amount::Flat { value } => format!(" {value:+}"),
        Amount::Set { value } => format!(" ={value}"),
        Amount::Ramp {
            initial,
            per_hour,
            max,
        } => format!(" ramp {initial:+} then {per_hour:+}/h up to {max}"),
        Amount::PerCount {
            per,
            step,
            counter,
            max_count,
            max_total,
        } => {
            let counter = match counter {
                Counter::Unmodeled { text } => format!("UNMODELED({text})"),
                other => short(&format!("{other:?}")),
            };
            let mut s = format!(" {per:+} per {step} × {counter}");
            if let Some(m) = max_count {
                s.push_str(&format!(" (max {m} counted)"));
            }
            if let Some(m) = max_total {
                s.push_str(&format!(" (max total {m})"));
            }
            s
        }
    }
}

fn short(s: &str) -> String {
    const MAX: usize = 110;
    if s.chars().count() > MAX {
        let cut: String = s.chars().take(MAX).collect();
        format!("{cut}…")
    } else {
        s.to_owned()
    }
}

fn print_skipped(loaded: &Loaded) {
    let r = &loaded.report;
    println!(
        "{} of {} operators loaded; {} skipped",
        r.operators_loaded,
        r.operators_total,
        r.skipped.len()
    );
    for s in &r.skipped {
        println!("  {}: {}", s.id, s.reason);
    }
}

fn print_mechanics(loaded: &Loaded, partial: bool) {
    let m = &loaded.report.mechanics;
    println!(
        "{} tiers: {} fully modelled ({:.1}%), {} partial, {} rejected",
        m.tiers,
        m.parsed,
        m.coverage_pct(),
        m.partial,
        m.unparsed
    );
    if !m.unresolved_names.is_empty() {
        println!(
            "unresolved operator names: {}",
            m.unresolved_names.join(", ")
        );
    }
    for u in &m.unparsed_skills {
        println!("REJECTED {}: {}", u.id, u.reason);
        println!("  {}", u.template);
    }
    if partial {
        for p in &m.partial_skills {
            println!("PARTIAL {}: {}", p.id, p.unmodeled.join(" | "));
        }
    }
}
