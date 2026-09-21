//! Prints Layer 2 parser coverage and every rejected / partial tier,
//! grouped so the highest-impact gaps come first.
//!
//! ```text
//! cargo run -p ak-data --example coverage
//! cargo run -p ak-data --example coverage -- --partial     # also list partials
//! ```

use std::collections::BTreeMap;

use ak_data::{Strictness, load_default};

fn main() {
    let show_partial = std::env::args().any(|a| a == "--partial");
    let loaded = load_default(Strictness::Lenient).expect("load");
    let m = &loaded.report.mechanics;
    println!(
        "tiers {}  parsed {} ({:.1}%)  partial {}  unparsed {}  accepted {:.1}%",
        m.tiers,
        m.parsed,
        m.coverage_pct(),
        m.partial,
        m.unparsed,
        m.accepted_pct()
    );
    if !m.unresolved_names.is_empty() {
        println!("unresolved operator names: {:?}", m.unresolved_names);
    }

    // Group rejects by reason (the reason already contains the offending phrase).
    let mut by_reason: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for u in &m.unparsed_skills {
        by_reason
            .entry(u.reason.as_str())
            .or_default()
            .push(u.id.as_str());
    }
    let mut groups: Vec<_> = by_reason.into_iter().collect();
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
    println!("\n== unparsed: {} distinct reasons ==", groups.len());
    for (reason, ids) in groups {
        println!("x{:<3} {}", ids.len(), reason);
        println!(
            "      e.g. {}",
            ids.iter().take(3).copied().collect::<Vec<_>>().join(", ")
        );
        let template = m
            .unparsed_skills
            .iter()
            .find(|u| u.id.as_str() == ids[0])
            .map(|u| u.template.as_str())
            .unwrap_or("");
        println!("      {template}");
    }

    if show_partial {
        println!("\n== partial: {} ==", m.partial_skills.len());
        for p in &m.partial_skills {
            println!("{}: {}", p.id, p.unmodeled.join(" | "));
        }
    }
}
