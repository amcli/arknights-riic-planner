//! Integration tests against the pinned snapshot in `data/`.
//!
//! These run the full strict pipeline and assert facts that are true of the
//! pinned commit. When the pin is bumped, update the expectations here
//! deliberately, in the same change.

use std::sync::OnceLock;

use ak_data::manifest::{BUILDING_FILE, CHARACTER_FILE, TEAM_FILE};
use ak_data::{Loaded, Strictness, load_default, schema, stats};
use ak_domain::*;

const PINNED_SHA: &str = "57010cb5b2afea112cae57daa756b58676ba6850";

fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(|| {
        load_default(Strictness::Strict).expect("pinned snapshot must transform strictly")
    })
}

fn data() -> &'static GameData {
    &loaded().data
}

#[test]
fn pinned_snapshot_counts() {
    let d = data();
    assert_eq!(d.operators.len(), 374);
    assert_eq!(d.skills.len(), 640);
    assert_eq!(d.powers.len(), 45);
    assert_eq!(d.facilities.len(), 12);
    assert_eq!(d.manufacture_formulas.len(), 14);
    assert_eq!(d.layout.slots.len(), 51);
    assert!(loaded().report.skipped.is_empty());
    assert_eq!(loaded().report.operators_loaded, 374);
}

#[test]
fn version_matches_manifest_pin() {
    let v = &data().version;
    assert_eq!(v.sha, PINNED_SHA);
    assert_eq!(v.source, "en_US");
    assert_eq!(v.locale, "en_US");
    assert_eq!(v.repo, "Kengxxiao/ArknightsGameData_YoStar");
    assert!(v.fetched_at.is_some(), "sidecar should record fetched_at");
    assert_eq!(v.parser_version, ak_data::PARSER_VERSION);
    assert_eq!(v.short_sha(), "57010cb5b2af");
}

#[test]
fn lancet2_is_ingested_correctly() {
    let op = data().operator("char_285_medic2").expect("Lancet-2");
    assert_eq!(op.name, "Lancet-2");
    assert_eq!(op.rarity, Rarity::Tier1);
    assert_eq!(op.profession, Profession::Medic);
    assert_eq!(op.sub_profession.as_str(), "physician");
    assert_eq!(op.nation.as_ref().map(PowerId::as_str), Some("rhodes"));
    assert_eq!(op.group, None);
    assert_eq!(op.team, None);
    assert_eq!(op.max_mood, 24.0);

    assert_eq!(op.skill_slots.len(), 2);
    let s0 = &op.skill_slots[0].unlocks;
    assert_eq!(s0.len(), 1);
    assert_eq!(s0[0].buff.as_str(), "power_rec_spd[000]");
    assert_eq!(s0[0].cond, UnlockCond::new(ElitePhase::E0, 1));
    let s1 = &op.skill_slots[1].unlocks;
    assert_eq!(s1[0].buff.as_str(), "dorm_rec_single[000]");
    assert_eq!(s1[0].cond, UnlockCond::new(ElitePhase::E0, 30));

    let at_start: Vec<_> = op
        .active_buffs(UnlockCond::BASE)
        .iter()
        .map(|b| b.as_str())
        .collect();
    assert_eq!(at_start, vec!["power_rec_spd[000]"]);
    let at_30: Vec<_> = op
        .active_buffs(UnlockCond::new(ElitePhase::E0, 30))
        .iter()
        .map(|b| b.as_str())
        .collect();
    assert_eq!(at_30, vec!["power_rec_spd[000]", "dorm_rec_single[000]"]);
}

#[test]
fn factory_productivity_skill_is_ingested_correctly() {
    let s = data().skill("manu_prod_spd[000]").expect("skill");
    assert_eq!(s.room_type, RoomType::Manufacture);
    assert_eq!(s.family(), "manu_prod_spd");
    assert_eq!(s.efficiency_hint, 15);
    assert_eq!(
        s.targets,
        vec![
            EfficiencyTarget::Product(ProductType::Gold),
            EfficiencyTarget::Product(ProductType::Exp),
            EfficiencyTarget::Product(ProductType::OriginiumShard),
        ]
    );
    assert_eq!(
        s.description.plain(),
        "When this Operator is assigned to a Factory, productivity +15%"
    );
    assert_eq!(s.description.values_up(), vec!["+15%"]);
}

#[test]
fn nested_markup_is_parsed() {
    let s = data().skill("control_prod_fraction[000]").expect("skill");
    assert_eq!(s.room_type, RoomType::Control);
    assert_eq!(
        s.description.plain(),
        "When this Operator is assigned to the Control Center, all Knight Operators assigned to Factories gain productivity +7%"
    );
    assert_eq!(s.description.terms(), vec!["cc.tag.knight"]);
    assert_eq!(
        s.description.find_tagged(|t| *t == RichTag::Keyword),
        vec!["Knight"]
    );
}

#[test]
fn every_skill_reference_resolves_and_slots_are_sorted() {
    let d = data();
    for op in d.operators.values() {
        assert_eq!(op.skill_slots.len(), 2, "{} slot count", op.id);
        for slot in &op.skill_slots {
            for unlock in &slot.unlocks {
                assert!(
                    d.skills.contains_key(&unlock.buff),
                    "{} → {}",
                    op.id,
                    unlock.buff
                );
            }
            assert!(
                slot.unlocks.windows(2).all(|w| w[0].cond < w[1].cond),
                "{} slot not strictly ascending",
                op.id
            );
        }
        for power in op.powers() {
            assert!(d.powers.contains_key(power), "{} → {}", op.id, power);
        }
        assert_eq!(op.max_mood, 24.0, "{} max mood", op.id);
    }
}

#[test]
fn only_staffable_rooms_have_skills() {
    let d = data();
    for skill in d.skills.values() {
        assert!(
            skill.room_type.has_base_skills(),
            "{} targets {}",
            skill.id,
            skill.room_type
        );
    }
    for rt in RoomType::ALL {
        let f = d.facility(*rt).expect("facility");
        if rt.has_base_skills() {
            assert!(
                f.is_staffable(),
                "{rt} has skills but no stationing capacity"
            );
        }
    }
}

#[test]
fn facility_levels_match_game() {
    let d = data();
    let factory = d.facility(RoomType::Manufacture).unwrap();
    assert_eq!(factory.max_level(), 3);
    assert_eq!(factory.phase(3).unwrap().max_stationed, 3);
    assert_eq!(factory.phase(1).unwrap().electricity, -10);
    assert_eq!(factory.category, RoomCategory::Output);

    let control = d.facility(RoomType::Control).unwrap();
    assert_eq!(control.max_level(), 5);

    let dorm = d.facility(RoomType::Dormitory).unwrap();
    assert_eq!(dorm.phase(1).unwrap().max_stationed, 5);
    assert_eq!(dorm.max_level(), 5);

    let trading = d.facility(RoomType::Trading).unwrap();
    assert_eq!(trading.name, "Trading Post");
    assert_eq!(trading.size, GridSize { rows: 2, cols: 4 });
}

#[test]
fn constants_match_game() {
    let c = &data().constants;
    assert_eq!(c.manpower_display_factor, 360_000);
    assert_eq!(c.control_slot.as_str(), "slot_34");
    assert_eq!(c.meeting_slot.as_str(), "slot_36");
    let caps: Vec<_> = c
        .manufacture
        .phases
        .iter()
        .map(|p| p.output_capacity)
        .collect();
    assert_eq!(caps, vec![24, 36, 54]);
    let limits: Vec<_> = c.trading.phases.iter().map(|p| p.order_limit).collect();
    assert_eq!(limits, vec![6, 8, 10]);
    let recover: Vec<_> = c
        .dormitory
        .phases
        .iter()
        .map(|p| p.manpower_recover)
        .collect();
    assert_eq!(recover, vec![160, 170, 180, 190, 200]);
    assert_eq!(c.manufact_manpower_cost_by_num, vec![0, 0, -5, -10]);
}

#[test]
fn formulas_and_layout() {
    let d = data();
    let exp = d.manufacture_formulas.get("1").expect("formula 1");
    assert_eq!(exp.product, ProductType::Exp);
    assert_eq!(exp.item.as_str(), "2001");
    assert_eq!(exp.cost_point, 2700);
    assert_eq!(exp.require_rooms[0].room_type, RoomType::Manufacture);

    let layout = &d.layout;
    assert_eq!(layout.id, "v0");
    assert_eq!(layout.slots[0].id.as_str(), "slot_1");
    assert_eq!(layout.slots[50].id.as_str(), "slot_51");
    assert_eq!(layout.slots_of(SlotCategory::Output).count(), 9);
    assert!(layout.slot("slot_34").is_some());
}

#[test]
fn powers_have_all_three_levels() {
    let d = data();
    assert_eq!(d.power("rhodes").unwrap().level, PowerLevel::Nation);
    assert_eq!(d.power("rhine").unwrap().level, PowerLevel::Group);
    assert_eq!(d.power("rhine").unwrap().name, "Rhine Lab");
    assert_eq!(d.power("action4").unwrap().level, PowerLevel::Team);
}

#[test]
fn schema_check_passes_on_pinned_snapshot() {
    let root = ak_data::paths::default_data_root().join("en_US");
    let building: serde_json::Value =
        ak_data::loader::read_json(&root.join(BUILDING_FILE)).unwrap();
    let characters: serde_json::Value =
        ak_data::loader::read_json(&root.join(CHARACTER_FILE)).unwrap();
    let teams: serde_json::Value = ak_data::loader::read_json(&root.join(TEAM_FILE)).unwrap();
    let report = schema::check_bundle(&building, &characters, &teams);
    assert!(report.is_ok(), "schema errors: {:#?}", report.errors);
    assert!(
        report.warnings.is_empty(),
        "schema warnings: {:#?}",
        report.warnings
    );
}

#[test]
fn stats_summary() {
    let s = stats::compute(data());
    assert_eq!(s.operators, 374);
    assert_eq!(s.skill_tiers, 640);
    assert_eq!(s.skill_tiers_by_room[&RoomType::Manufacture], 99);
    assert_eq!(s.skill_tiers_by_room[&RoomType::Trading], 82);
    assert_eq!(s.operators_by_rarity.values().sum::<usize>(), 374);
    assert!(s.skill_families > 100);
}

#[test]
fn game_data_round_trips_through_json() {
    let d = data();
    let json = serde_json::to_string(d).expect("serialise");
    let back: GameData = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(&back, d);
}
