//! Layer 8: the roster import adapters against fixture exports.
//!
//! The fixtures in `tests/fixtures/import/` follow each tool's export
//! format as its source code defines it (see the module docs of
//! `ak_data::import::krooster` and `ak_data::import::ak_planner`); they are
//! not captured from real accounts. They deliberately include the awkward
//! cases: operators the pinned data lacks (Amiya's alternate forms), a
//! promotion a 3★ cannot reach, a level above the cap, entries marked not
//! owned, a duplicate row, an older-format ak-planner record, an inactive
//! plan, and a selected operator with no saved plan.

use std::sync::OnceLock;

use ak_data::import::{self, Format, ImportError, ImportWarning, Imported, Source};
use ak_data::{Loaded, Strictness, load_default};
use ak_domain::*;
use serde_json::{Value, json};

fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(|| load_default(Strictness::Strict).expect("pinned snapshot loads"))
}

fn data() -> &'static GameData {
    &loaded().data
}

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/import/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn run(source: Source, name: &str) -> Imported {
    import::import(source, data(), &fixture(name)).unwrap()
}

#[track_caller]
fn at(imported: &Imported, id: &str) -> (ElitePhase, u32) {
    let entry = imported
        .roster
        .get(id)
        .unwrap_or_else(|| panic!("{id} not imported"));
    assert_eq!(entry.mood, None);
    (entry.promotion.phase, entry.promotion.level)
}

fn unknown(id: &str) -> ImportWarning {
    ImportWarning::UnknownOperator {
        operator: id.into(),
    }
}

#[test]
fn operators_know_their_level_caps() {
    let op = |id: &str| data().operator(id).unwrap();
    assert_eq!(op("char_103_angel").max_levels, vec![50, 80, 90]);
    assert_eq!(op("char_281_popka").max_levels, vec![40, 55]);
    assert_eq!(op("char_285_medic2").max_levels, vec![30]);
    assert_eq!(op("char_281_popka").max_phase(), ElitePhase::E1);
    assert_eq!(op("char_281_popka").max_level(ElitePhase::E2), None);
    // Every operator reaches at least Elite 0, and caps rise with promotion.
    for o in data().operators.values() {
        assert!(!o.max_levels.is_empty(), "{}", o.id);
        assert!(o.max_levels.windows(2).all(|w| w[0] <= w[1]), "{}", o.id);
    }
}

#[test]
fn krooster_current_roster() {
    let r = run(Source::Krooster, "krooster-v3.json");
    assert_eq!(r.report.source, Source::Krooster);
    assert_eq!(r.report.format, Format::KroosterV3);
    assert_eq!(r.report.entries, 12);
    assert_eq!(r.report.not_owned, 1);
    assert_eq!(r.report.imported, 10);
    assert_eq!(r.roster.entries.len(), 10);

    assert_eq!(at(&r, "char_103_angel"), (ElitePhase::E2, 90));
    assert_eq!(at(&r, "char_002_amiya"), (ElitePhase::E2, 50));
    assert_eq!(at(&r, "char_140_whitew"), (ElitePhase::E1, 70));
    assert_eq!(at(&r, "char_190_clour"), (ElitePhase::E0, 45));
    assert_eq!(at(&r, "char_285_medic2"), (ElitePhase::E0, 30));
    assert!(r.roster.get("char_263_skadi").is_none(), "potential 0");
    assert!(r.roster.get("char_1001_amiya2").is_none());

    // A 3★ cannot reach Elite 2: taken as fully raised at Elite 1.
    assert_eq!(at(&r, "char_124_kroos"), (ElitePhase::E1, 55));
    // Level 95 is above the 6★ cap.
    assert_eq!(at(&r, "char_456_ash"), (ElitePhase::E2, 90));

    // Keyed exports are read in id order.
    assert_eq!(
        r.report.warnings,
        vec![
            unknown("char_1001_amiya2"),
            ImportWarning::PromotionClamped {
                operator: "char_124_kroos".into(),
                found: 2,
                used: ElitePhase::E1,
                level: 55,
            },
            ImportWarning::LevelClamped {
                operator: "char_456_ash".into(),
                found: 95,
                used: 90,
            },
        ]
    );
}

#[test]
fn imported_promotions_pick_the_skill_tiers() {
    let r = run(Source::Krooster, "krooster-v3.json");
    let active = |id: &str| -> Vec<String> {
        let op = data().operator(id).unwrap();
        op.active_buffs(r.roster.get(id).unwrap().promotion)
            .into_iter()
            .map(|b| b.as_str().to_owned())
            .collect()
    };
    // Vermeil at Elite 0: her second slot unlocks at Elite 1.
    assert_eq!(active("char_190_clour"), ["manu_prod_limit&cost[0000]"]);
    // Popukar at Elite 1 has both.
    assert_eq!(
        active("char_281_popka"),
        [
            "manu_prod_spd&limit&cost[010]",
            "dorm_rec_single&oneself[020]"
        ]
    );
}

#[test]
fn krooster_rows_keep_the_last_duplicate() {
    let r = run(Source::Krooster, "krooster-v3-rows.json");
    assert_eq!(r.report.format, Format::KroosterV3Rows);
    assert_eq!(r.report.entries, 4);
    assert_eq!(r.report.imported, 3);
    assert_eq!(at(&r, "char_102_texas"), (ElitePhase::E2, 80));
    assert_eq!(
        r.report.warnings,
        vec![ImportWarning::Duplicate {
            operator: "char_102_texas".into()
        }]
    );
    // The same operators as the keyed export, where they overlap.
    let keyed = run(Source::Krooster, "krooster-v3.json");
    for id in ["char_103_angel", "char_253_greyy"] {
        assert_eq!(r.roster.get(id), keyed.roster.get(id));
    }
}

#[test]
fn krooster_legacy_roster() {
    let r = run(Source::Krooster, "krooster-legacy.json");
    assert_eq!(r.report.format, Format::KroosterLegacy);
    assert_eq!(r.report.entries, 5);
    // Blaze has potential 0; Ch'en is marked not owned.
    assert_eq!(r.report.not_owned, 2);
    assert_eq!(r.report.imported, 3);
    assert!(r.report.warnings.is_empty(), "{:?}", r.report.warnings);
    assert_eq!(at(&r, "char_103_angel"), (ElitePhase::E2, 90));
    assert_eq!(at(&r, "char_002_amiya"), (ElitePhase::E2, 50));
    assert_eq!(at(&r, "char_285_medic2"), (ElitePhase::E0, 30));
}

#[test]
fn ak_planner_export() {
    let r = run(Source::AkPlanner, "ak-planner.json");
    assert_eq!(r.report.source, Source::AkPlanner);
    assert_eq!(r.report.format, Format::AkPlannerExport);
    // Four saved plans and one selected operator without one.
    assert_eq!(r.report.entries, 5);
    assert_eq!(r.report.not_owned, 0);
    assert_eq!(r.report.imported, 4);
    assert_eq!(at(&r, "char_103_angel"), (ElitePhase::E2, 90));
    // An inactive plan still says where the operator is now.
    assert_eq!(at(&r, "char_002_amiya"), (ElitePhase::E1, 70));
    // An older-format record (modules as { x, y, d }).
    assert_eq!(at(&r, "char_311_mudrok"), (ElitePhase::E0, 40));
    // Selected without a saved plan: Elite 0 level 1, as ak-planner shows.
    assert_eq!(at(&r, "char_120_hibisc"), (ElitePhase::E0, 1));
    assert_eq!(
        r.report.warnings,
        vec![
            unknown("char_1037_amiya3"),
            ImportWarning::NoSavedPlan {
                operator: "char_120_hibisc".into()
            },
        ]
    );
}

#[test]
fn an_empty_roster_is_fine() {
    let r = import::import(Source::Krooster, data(), &json!({})).unwrap();
    assert_eq!(r.report.format, Format::KroosterV3);
    assert!(r.roster.entries.is_empty());
    let r = import::import(
        Source::AkPlanner,
        data(),
        &json!({ "s": [], "i": {}, "p": [] }),
    )
    .unwrap();
    assert!(r.roster.entries.is_empty());
}

#[test]
fn the_wrong_tool_is_named() {
    let err = import::import(Source::Krooster, data(), &fixture("ak-planner.json")).unwrap_err();
    assert!(
        matches!(&err, ImportError::NotThisFormat { tool: Source::Krooster, reason } if reason.contains("ak-planner")),
        "{err}"
    );
    let err = import::import(Source::AkPlanner, data(), &fixture("krooster-v3.json")).unwrap_err();
    assert!(err.to_string().contains("Krooster roster"), "{err}");
    let err = import::import(Source::Krooster, data(), &json!("text")).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("this does not look like Krooster's export"),
        "{err}"
    );
    let err = import::import(Source::Krooster, data(), &json!({ "x": { "a": 1 } })).unwrap_err();
    assert!(matches!(err, ImportError::NotThisFormat { .. }), "{err}");
}

#[test]
fn malformed_entries_are_refused_by_name() {
    let mut v3 = fixture("krooster-v3.json");
    v3["char_102_texas"]["elite"] = json!("two");
    let err = import::import(Source::Krooster, data(), &v3).unwrap_err();
    assert_eq!(
        err.to_string(),
        "char_102_texas: `elite` must be a whole number, found \"two\""
    );

    let mut v3 = fixture("krooster-v3.json");
    v3["char_102_texas"]["elite"] = json!(3);
    let err = import::import(Source::Krooster, data(), &v3).unwrap_err();
    assert_eq!(
        err.to_string(),
        "char_102_texas: `elite` must be 0, 1 or 2, found 3"
    );

    let mut v3 = fixture("krooster-v3.json");
    v3["char_102_texas"]
        .as_object_mut()
        .unwrap()
        .remove("level");
    let err = import::import(Source::Krooster, data(), &v3).unwrap_err();
    assert_eq!(err.to_string(), "char_102_texas: `level` is missing");

    // JSON's single number type: 2.0 is a whole number.
    let mut v3 = fixture("krooster-v3.json");
    v3["char_102_texas"]["level"] = json!(60.0);
    assert!(import::import(Source::Krooster, data(), &v3).is_ok());

    let mut rows = fixture("krooster-v3-rows.json");
    rows[1]["op_id"] = json!(7);
    let err = import::import(Source::Krooster, data(), &rows).unwrap_err();
    assert_eq!(err.to_string(), "row 1: `op_id` must be text, found 7");

    let mut plan = fixture("ak-planner.json");
    plan["p"][1]["plans"]
        .as_object_mut()
        .unwrap()
        .remove("currentElite");
    let err = import::import(Source::AkPlanner, data(), &plan).unwrap_err();
    assert_eq!(
        err.to_string(),
        "p[1] (char_002_amiya) plans: `currentElite` is missing"
    );
}

#[test]
fn a_low_level_is_raised_to_one() {
    let input = json!({
        "char_102_texas": { "op_id": "char_102_texas", "potential": 1, "elite": 0, "level": 0 }
    });
    let r = import::import(Source::Krooster, data(), &input).unwrap();
    assert_eq!(at(&r, "char_102_texas"), (ElitePhase::E0, 1));
    assert_eq!(
        r.report.warnings,
        vec![ImportWarning::LevelClamped {
            operator: "char_102_texas".into(),
            found: 0,
            used: 1,
        }]
    );
}

#[test]
fn sources_round_trip_through_their_names() {
    for source in Source::ALL {
        assert_eq!(source.as_str().parse::<Source>(), Ok(source));
        assert_eq!(
            serde_json::to_value(source).unwrap(),
            json!(source.as_str())
        );
    }
    assert!("manual".parse::<Source>().is_err());
}
