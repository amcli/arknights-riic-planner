//! Dev tool: every skill tier's parsed mechanics as one JSON document, for
//! grepping. `cargo run -p ak-data --example dump_mechanics > mechanics.json`.

use std::collections::BTreeMap;

use ak_data::{Strictness, load_default};

fn main() {
    let loaded = load_default(Strictness::Strict).expect("pinned snapshot loads");
    let all: BTreeMap<_, _> = loaded
        .data
        .skills
        .iter()
        .map(|(id, s)| (id.clone(), (s.room_type, &s.mechanics)))
        .collect();
    println!("{}", serde_json::to_string_pretty(&all).unwrap());
}
