//! `ak solve <request.json>`: run the Layer 5 solver on a request file and
//! print the finalists (or the raw JSON with `--json`).

use std::path::Path;

use anyhow::Context;

use ak_domain::{GameData, OperatorId};
use ak_solver::{SolveRequest, SolveResult, solve};

/// Solves the request at `path`.
pub fn run(data: &GameData, path: &Path, json: bool) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let request: SolveRequest =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let result = solve(data, &request)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_result(data, &result);
    }
    Ok(())
}

fn name<'a>(data: &'a GameData, id: &'a OperatorId) -> &'a str {
    data.operators
        .get(id.as_str())
        .map_or(id.as_str(), |o| o.name.as_str())
}

fn print_result(data: &GameData, r: &SolveResult) {
    println!(
        "Solved in {} ms: {:?} search scored by {:?}; {} evaluations, {} simulations",
        r.elapsed_ms, r.strategy, r.inner, r.evaluations, r.simulations
    );
    println!(
        "Space: {} variable slots, {} locked, {} candidate operators, about {:.0} distinct assignments",
        r.space.variable_slots, r.space.locked_slots, r.space.pool, r.space.estimated_size
    );
    println!("Starting assignment: score {:.0}", r.initial.score);
    for (i, c) in r.candidates.iter().enumerate() {
        let b = &c.breakdown;
        println!(
            "#{} score {:.0} (inner {:.0}){}",
            i + 1,
            c.score,
            c.inner_score,
            if c.simulated { "" } else { "  not simulated" }
        );
        println!(
            "   LMD {:.0}  EXP {:.0}  Orundum {:.0}  drones {:.0}  contacts {:.2}  training {:.1} h  gold net {:+.2}  exhausted {:.1} h",
            b.lmd,
            b.exp,
            b.orundum,
            b.drones,
            b.contacts,
            b.training_hours,
            b.gold_net,
            b.exhausted_hours
        );
        for room in c.assignment.rooms() {
            let who: Vec<&str> = c
                .assignment
                .occupants(room.as_str())
                .map(|o| name(data, o))
                .collect();
            if !who.is_empty() {
                println!("   {:<10} {}", room, who.join(", "));
            }
        }
    }
    if let Some(sim) = &r.best_simulation {
        println!();
        println!("Best candidate, simulated:");
        crate::simulate::print_result(data, sim);
    }
}
