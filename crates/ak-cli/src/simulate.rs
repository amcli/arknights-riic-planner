//! `ak simulate <request.json>`: run the Layer 4 evaluator on a request
//! file and print a readable report (or the raw JSON with `--json`).

use std::path::Path;

use anyhow::Context;

use ak_domain::{GameData, OperatorId};
use ak_eval::{SimRequest, SimResult, SimWarning, Snapshot, evaluate, simulate};

/// Runs or evaluates the request at `path`.
pub fn run(data: &GameData, path: &Path, json: bool, evaluate_only: bool) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let request: SimRequest =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if evaluate_only {
        let snap = evaluate(
            data,
            &request.base,
            &request.assignment,
            &request.roster,
            &request.config,
        )?;
        if json {
            println!("{}", serde_json::to_string_pretty(&snap)?);
        } else {
            print_snapshot(data, &snap);
        }
        return Ok(());
    }
    let result = simulate(data, &request)?;
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

fn print_snapshot(data: &GameData, snap: &Snapshot) {
    println!("Rooms (instantaneous, at starting morale):");
    for r in &snap.rooms {
        let stat = r
            .main_stat()
            .map(|(k, v)| format!("{k:?} {v:.1}%"))
            .unwrap_or_default();
        println!(
            "  {:<8} {:<12} L{}  {} op  {}",
            r.id, r.kind, r.level, r.headcount, stat
        );
        for (k, v) in &r.bonus {
            println!("           {k:?} from skills: {v:+.2}");
        }
        for w in &r.workshop {
            println!("           workshop: {w:?}");
        }
    }
    println!("Morale rates (per hour):");
    for m in &snap.mood {
        println!(
            "  {:<24} {:<8} base {:+.3}  skills {:+.3}  = {:+.3}{}",
            name(data, &m.operator),
            m.room,
            m.base,
            m.skills,
            m.total,
            if m.exhausted {
                "  (exhausted)"
            } else if m.idle {
                "  (idle)"
            } else {
                ""
            }
        );
    }
    if !snap.contributions.is_empty() {
        println!("Contributions:");
        for c in &snap.contributions {
            println!(
                "  {:<8} {:<24} {:<32} {:?} {:+.2}{}",
                c.room,
                name(data, &c.operator),
                c.skill,
                c.stat,
                c.value,
                c.scaled_from
                    .map(|v| format!(" (scaled from {v:+.2})"))
                    .unwrap_or_default()
            );
        }
    }
    print_warnings(&snap.warnings);
}

fn print_result(data: &GameData, r: &SimResult) {
    println!(
        "Simulated {:.0} h at {}-minute ticks on {} {}",
        r.horizon_hours,
        r.tick_minutes,
        r.data.source,
        r.data.short_sha()
    );
    let t = &r.totals;
    println!(
        "Totals: LMD {:.0}  Orundum {:.0}  EXP {:.0}  drones {:.1}  orders {:.2}  contacts {:.2}",
        t.lmd, t.orundum, t.exp, t.drones, t.orders_completed, t.contacts
    );
    if t.lmd_spent > 0.0 {
        println!("        LMD spent on Factory inputs {:.0}", t.lmd_spent);
    }
    println!(
        "        gold produced {:.2}  consumed {:.2}  in depot {:.2}",
        t.gold_produced, t.gold_consumed, t.gold_in_depot
    );
    for (item, n) in &t.items {
        println!("        item {item}: {n:.2}");
    }
    println!("Rooms:");
    for room in &r.rooms {
        let stat = room
            .average_stat_pct
            .map(|v| format!("{v:6.1}% avg"))
            .unwrap_or_else(|| " ".repeat(10));
        let who: Vec<&str> = room.operators.iter().map(|o| name(data, o)).collect();
        println!(
            "  {:<8} {:<12} L{}  {}  {}",
            room.id,
            room.kind,
            room.level,
            stat,
            who.join(", ")
        );
        if room.lmd > 0.0 || room.orundum > 0.0 {
            println!(
                "           LMD {:.0}  Orundum {:.0}  orders {:.2}  pending {:.2}",
                room.lmd, room.orundum, room.orders_completed, room.pending_orders
            );
        }
        for (item, n) in &room.produced {
            println!("           item {item}: {n:.2}");
        }
        if room.drones > 0.0 {
            println!("           drones {:.1}", room.drones);
        }
        if room.contacts > 0.0 {
            println!("           contacts {:.2}", room.contacts);
        }
        if room.training_progress_hours > 0.0 {
            let done = room
                .training_completed_hour
                .map(|h| format!(", completed at {h:.2} h"))
                .unwrap_or_default();
            println!(
                "           training progress {:.2} base hours{done}",
                room.training_progress_hours
            );
        }
        if room.hours_blocked > 0.0 || room.in_storage > 0.0 {
            println!(
                "           blocked {:.1} h, {:.2} left in storage",
                room.hours_blocked, room.in_storage
            );
        }
    }
    println!("Operators:");
    for o in &r.operators {
        println!(
            "  {:<24} mood {:>4.1} → {:>4.1} (min {:>4.1})  work {:>5.1} h  rest {:>5.1} h  exhausted {:>5.1} h  idle {:>5.1} h  benched {:>5.1} h",
            o.name,
            o.initial_mood,
            o.final_mood,
            o.min_mood,
            o.hours_working,
            o.hours_resting,
            o.hours_exhausted,
            o.hours_idle,
            o.hours_benched
        );
    }
    if !r.events.is_empty() {
        const MAX: usize = 60;
        println!("Events ({}):", r.events.len());
        for e in r.events.iter().take(MAX) {
            println!("  {:>7.2} h  {:?}", e.hour, e.kind);
        }
        if r.events.len() > MAX {
            println!("  … {} more", r.events.len() - MAX);
        }
    }
    print_warnings(&r.warnings);
}

fn print_warnings(warnings: &[SimWarning]) {
    if warnings.is_empty() {
        println!("Warnings: none");
        return;
    }
    println!("Warnings ({}):", warnings.len());
    for w in warnings {
        println!("  {w:?}");
    }
}
