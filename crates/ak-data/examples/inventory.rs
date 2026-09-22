//! Dev tool: what the evaluator has to consume. Prints the global constants,
//! the Factory formulas, and a histogram of every AST node kind across all
//! parsed skill tiers. `cargo run -p ak-data --example inventory`.

use std::collections::BTreeMap;

use ak_data::{Strictness, load_default};
use ak_domain::*;

fn bump(map: &mut BTreeMap<String, usize>, key: impl Into<String>) {
    *map.entry(key.into()).or_default() += 1;
}

fn counter_name(c: &Counter) -> String {
    match c {
        Counter::Operators {
            group,
            scope,
            excluding_self,
        } => format!(
            "operators({:?},{:?}{})",
            group_name(group),
            scope,
            if *excluding_self { ",others" } else { "" }
        ),
        Counter::Unmodeled { text } => format!("unmodeled: {text}"),
        other => format!("{other:?}")
            .split(' ')
            .next()
            .unwrap_or("?")
            .trim_end_matches('{')
            .to_owned(),
    }
}

fn group_name(g: &Group) -> String {
    match g {
        Group::Power(p) => format!("power:{p}"),
        Group::Tag(t) => format!("tag:{t}"),
        Group::Profession(p) => format!("prof:{p}"),
        Group::Subclass(s) => format!("sub:{s}"),
    }
}

fn pred_leaves<'a>(p: &'a Predicate, out: &mut Vec<&'a Predicate>) {
    match p {
        Predicate::Not { inner } => pred_leaves(inner, out),
        Predicate::And { all: ps } | Predicate::Or { any: ps } => {
            for q in ps {
                pred_leaves(q, out);
            }
        }
        leaf => out.push(leaf),
    }
}

fn pred_name(p: &Predicate) -> String {
    let dbg = format!("{p:?}");
    let head = dbg.split([' ', '{']).next().unwrap_or("?").to_owned();
    match p {
        Predicate::CoworkerIn { group } | Predicate::TargetIn { group } => {
            format!("{head}({})", group_name(group))
        }
        Predicate::GroupInRoom { group, room } => format!("{head}({},{room})", group_name(group)),
        Predicate::CountAtLeast { counter, .. } => format!("{head}({})", counter_name(counter)),
        Predicate::Unmodeled { text } => format!("unmodeled: {text}"),
        _ => head,
    }
}

fn main() {
    let loaded = load_default(Strictness::Strict).expect("pinned snapshot loads");
    let data = &loaded.data;

    println!("== constants ==");
    println!("{}", serde_json::to_string_pretty(&data.constants).unwrap());

    println!("\n== formulas ==");
    for f in data.manufacture_formulas.values() {
        let costs: Vec<String> = f
            .costs
            .iter()
            .map(|c| format!("{}x{}", c.count, c.item))
            .collect();
        println!(
            "{:>4} {:<10} item={} x{} cost_point={} weight={} costs=[{}] req={:?}",
            f.id,
            f.product,
            f.item,
            f.count,
            f.cost_point,
            f.weight,
            costs.join(","),
            f.require_rooms
                .iter()
                .map(|r| format!("{}L{}x{}", r.room_type, r.level, r.count))
                .collect::<Vec<_>>()
        );
    }

    let mut effects = BTreeMap::new();
    let mut effects_by_room = BTreeMap::new();
    let mut counters = BTreeMap::new();
    let mut preds = BTreeMap::new();
    let mut amounts = BTreeMap::new();
    let mut scopes = BTreeMap::new();
    let mut targets = BTreeMap::new();
    let mut groups = BTreeMap::new();
    let mut products = BTreeMap::new();
    let mut stacking = BTreeMap::new();
    for s in data.skills.values() {
        let Some(m) = &s.mechanics else { continue };
        bump(&mut stacking, format!("{:?}", m.stacking));
        for c in &m.clauses {
            bump(&mut effects, c.effect.kind_name());
            bump(
                &mut effects_by_room,
                format!("{} / {}", s.room_type, c.effect.kind_name()),
            );
            let mut leaves = Vec::new();
            pred_leaves(&c.when, &mut leaves);
            for l in leaves {
                bump(&mut preds, pred_name(l));
            }
            if let Some(a) = c.effect.amount() {
                let head = format!("{a:?}")
                    .split([' ', '{'])
                    .next()
                    .unwrap_or("?")
                    .to_owned();
                bump(&mut amounts, head);
                if let Amount::PerCount { counter, .. } = a {
                    bump(&mut counters, counter_name(counter));
                    if let Counter::Operators { group, .. } = counter {
                        bump(&mut groups, group_name(group));
                    }
                }
            }
            match &c.effect {
                Effect::Productivity { scope, product, .. }
                | Effect::Capacity { scope, product, .. } => {
                    bump(
                        &mut scopes,
                        format!("{scope:?}")
                            .split([' ', '('])
                            .next()
                            .unwrap_or("?")
                            .to_owned(),
                    );
                    bump(&mut products, format!("{product:?}"));
                }
                Effect::OrderEfficiency { scope, .. } | Effect::OrderLimit { scope, .. } => {
                    bump(
                        &mut scopes,
                        format!("{scope:?}")
                            .split([' ', '('])
                            .next()
                            .unwrap_or("?")
                            .to_owned(),
                    );
                }
                Effect::Mood { target, .. } => bump(
                    &mut targets,
                    format!("{target:?}")
                        .split([' ', '('])
                        .next()
                        .unwrap_or("?")
                        .to_owned(),
                ),
                Effect::ScaleOthersContribution { stat, percent } => {
                    bump(&mut effects, format!("  scale_others {stat:?} {percent}"))
                }
                Effect::FacilityCount { room, delta } => {
                    bump(&mut effects, format!("  facility_count {room} {delta}"))
                }
                Effect::GainResource { resource, .. } => {
                    bump(&mut effects, format!("  gain {resource}"))
                }
                Effect::ConvertResource { from, to, .. } => {
                    bump(&mut effects, format!("  convert {from}->{to}"))
                }
                Effect::Unmodeled { summary } => {
                    bump(&mut effects, format!("  unmodeled: {summary}"))
                }
                _ => {}
            }
        }
    }
    for (title, map) in [
        ("effects", &effects),
        ("effects by room", &effects_by_room),
        ("amounts", &amounts),
        ("counters", &counters),
        ("counter groups", &groups),
        ("predicates", &preds),
        ("scopes", &scopes),
        ("mood targets", &targets),
        ("products", &products),
        ("stacking", &stacking),
    ] {
        println!("\n== {title} ==");
        for (k, v) in map {
            println!("{v:>5}  {k}");
        }
    }
}
