//! Layer 2 (description parser) tests against the pinned snapshot.
//!
//! Three kinds of check:
//! 1. exact parses for representative skills of every room and construct;
//! 2. a coverage ratchet: the numbers may only go up, and a bump must be a
//!    deliberate edit here;
//! 3. a cross-check of parsed magnitudes against upstream's display-only
//!    `efficiency` hint, which is an independent source of the same number.

use std::sync::OnceLock;

use ak_data::{Loaded, Strictness, load_default};
use ak_domain::*;

fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(|| load_default(Strictness::Strict).expect("pinned snapshot loads"))
}

fn mechanics(id: &str) -> &'static Mechanics {
    loaded()
        .data
        .skill(id)
        .unwrap_or_else(|| panic!("skill {id}"))
        .mechanics
        .as_ref()
        .unwrap_or_else(|| panic!("{id} should parse"))
}

fn effects(id: &str) -> Vec<&'static Effect> {
    mechanics(id).clauses.iter().map(|c| &c.effect).collect()
}

fn flat(v: f64) -> Amount {
    Amount::flat(v)
}

// ---- coverage ratchet ------------------------------------------------------

/// Pinned coverage. Raise these when the parser improves; never lower them
/// without a note in the commit explaining what regressed and why.
const MIN_FULLY_MODELED: usize = 589;
const MAX_PARTIAL: usize = 51;
const MAX_REJECTED: usize = 0;

#[test]
fn coverage_ratchet() {
    let m = &loaded().report.mechanics;
    assert_eq!(m.tiers, 640);
    assert!(
        m.parsed >= MIN_FULLY_MODELED,
        "fully modelled dropped to {} (< {MIN_FULLY_MODELED})",
        m.parsed
    );
    assert!(m.partial <= MAX_PARTIAL, "partial rose to {}", m.partial);
    assert_eq!(
        m.unparsed, MAX_REJECTED,
        "rejected tiers: {:#?}",
        m.unparsed_skills
    );
    assert!(
        m.unresolved_names.is_empty(),
        "unresolved operator names: {:?}",
        m.unresolved_names
    );
    // Every skill has mechanics attached unless it was rejected.
    let attached = loaded()
        .data
        .skills
        .values()
        .filter(|s| s.mechanics.is_some())
        .count();
    assert_eq!(attached, m.parsed + m.partial);
}

// ---- representative parses -------------------------------------------------

#[test]
fn factory_flat_productivity() {
    let m = mechanics("manu_prod_spd[000]");
    assert_eq!(m.stacking, Stacking::Additive);
    assert_eq!(
        m.clauses,
        vec![Clause {
            when: Predicate::Always,
            effect: Effect::Productivity {
                amount: flat(15.0),
                product: None,
                scope: Scope::ThisRoom,
            },
        }]
    );
}

#[test]
fn factory_product_specific_and_capacity() {
    assert_eq!(
        effects("manu_formula_spd[100]"),
        vec![&Effect::Productivity {
            amount: flat(30.0),
            product: Some(ProductType::Gold),
            scope: Scope::ThisRoom,
        }]
    );
    assert_eq!(
        effects("manu_formula_limit[010]"),
        vec![&Effect::Capacity {
            amount: flat(15.0),
            product: Some(ProductType::Exp),
            scope: Scope::ThisRoom,
        }]
    );
    // "capacity limit +N and Morale consumed per hour -M" → two clauses.
    let e = effects("manu_prod_limit&cost[000]");
    assert_eq!(e.len(), 2);
    assert!(matches!(e[0], Effect::Capacity { .. }));
    assert!(matches!(
        e[1],
        Effect::Mood {
            target: MoodTarget::SelfOnly,
            ..
        }
    ));
}

#[test]
fn factory_ramp() {
    assert_eq!(
        effects("manu_prod_spd_addition[030]"),
        vec![&Effect::Productivity {
            amount: Amount::Ramp {
                initial: 20.0,
                per_hour: 1.0,
                max: 25.0
            },
            product: None,
            scope: Scope::ThisRoom,
        }]
    );
}

#[test]
fn factory_scale_others_and_room_count() {
    let e = effects("manu_prod_spd&power[000]");
    assert_eq!(
        e[0],
        &Effect::ScaleOthersContribution {
            stat: Stat::Productivity,
            percent: -100.0
        }
    );
    assert_eq!(
        e[1],
        &Effect::Productivity {
            amount: Amount::PerCount {
                per: 5.0,
                step: 1.0,
                counter: Counter::RoomCount {
                    room: RoomType::Power
                },
                max_count: None,
                max_total: None,
            },
            product: None,
            scope: Scope::ThisRoom,
        }
    );
}

#[test]
fn control_center_group_bonus_targets_all_factories() {
    assert_eq!(
        effects("control_prod_fraction[000]"),
        vec![&Effect::Productivity {
            amount: Amount::PerCount {
                per: 7.0,
                step: 1.0,
                // Counted per receiving Factory, not in the Control Center.
                counter: Counter::Operators {
                    group: Group::Tag("knight".into()),
                    scope: CountScope::TargetRoom,
                    excluding_self: false,
                },
                max_count: None,
                max_total: None,
            },
            product: None,
            scope: Scope::AllRooms(RoomType::Manufacture),
        }]
    );
    // A Control Center skill naming "productivity" acts on every Factory.
    let e = effects("control_bd_spd[000]");
    assert!(matches!(
        e[1],
        Effect::Productivity {
            scope: Scope::AllRooms(RoomType::Manufacture),
            ..
        }
    ));
}

#[test]
fn control_center_conditional_named_operators_resolve() {
    let m = mechanics("control_meeting&ord[000]");
    assert_eq!(m.clauses.len(), 2);
    match &m.clauses[0].when {
        Predicate::OperatorInRoom { who, room } => {
            assert_eq!(who.name, "Ines");
            assert_eq!(
                who.id.as_ref().map(OperatorId::as_str),
                Some("char_4087_ines")
            );
            assert_eq!(*room, RoomType::Meeting);
        }
        other => panic!("{other:?}"),
    }
    match &m.clauses[1].effect {
        Effect::OrderLimit {
            amount,
            scope: Scope::RoomOf(who),
        } => {
            assert_eq!(*amount, flat(1.0));
            assert_eq!(who.name, "Hoederer");
            assert!(who.id.is_some());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn trading_efficiency_limit_and_capped_counter() {
    assert_eq!(
        effects("trade_ord_spd&limit[000]"),
        vec![
            &Effect::OrderEfficiency {
                amount: flat(10.0),
                scope: Scope::ThisRoom
            },
            &Effect::OrderLimit {
                amount: flat(2.0),
                scope: Scope::ThisRoom
            },
        ]
    );
    let e = effects("trade_ord_spd&meet[000]");
    assert_eq!(
        e[1],
        &Effect::OrderEfficiency {
            amount: Amount::PerCount {
                per: 5.0,
                step: 1.0,
                counter: Counter::RoomLevels {
                    room: RoomType::Meeting
                },
                max_count: None,
                max_total: Some(40.0),
            },
            scope: Scope::ThisRoom,
        }
    );
}

#[test]
fn power_plant_and_conditional() {
    assert_eq!(
        effects("power_rec_spd[000]"),
        vec![&Effect::DroneRecovery { amount: flat(10.0) }]
    );
    let m = mechanics("power_rec_spd_P[000]");
    assert_eq!(
        m.clauses[0].when,
        Predicate::OperatorInRoom {
            who: OperatorRef {
                name: "Kal'tsit".into(),
                id: Some(OperatorId::new("char_003_kalts")),
            },
            room: RoomType::Control,
        }
    );
}

#[test]
fn dormitory_targets_and_stacking() {
    let m = mechanics("dorm_rec_all[000]");
    assert_eq!(m.stacking, Stacking::StrongestOfType);
    assert_eq!(
        m.clauses[0].effect,
        Effect::Mood {
            amount: flat(0.1),
            target: MoodTarget::AllInRoom
        }
    );
    assert_eq!(
        effects("dorm_rec_single&oneself[000]"),
        vec![
            &Effect::Mood {
                amount: flat(0.2),
                target: MoodTarget::OneOtherInRoom
            },
            &Effect::Mood {
                amount: flat(0.4),
                target: MoodTarget::SelfOnly
            },
        ]
    );
    // "self Morale recovered per hour -0.1, but restores +0.2 to all".
    assert_eq!(
        effects("dorm_rec_all&oneself[000]"),
        vec![
            &Effect::Mood {
                amount: flat(-0.1),
                target: MoodTarget::SelfOnly
            },
            &Effect::Mood {
                amount: flat(0.2),
                target: MoodTarget::AllInRoom
            },
        ]
    );
}

#[test]
fn morale_sign_conventions() {
    // "Morale consumed per hour -0.25" restores morale.
    let e = effects("hire_spd_cost[100]");
    assert_eq!(e[0], &Effect::ContactSpeed { amount: flat(10.0) });
    assert_eq!(
        e[1],
        &Effect::Mood {
            amount: flat(0.25),
            target: MoodTarget::SelfOnly
        }
    );
    // "self Morale loss per hour +0.5" drains it.
    let e = effects("control_bd_spd[000]");
    assert_eq!(
        e[0],
        &Effect::Mood {
            amount: flat(-0.5),
            target: MoodTarget::SelfOnly
        }
    );
}

#[test]
fn reception_coworker_group_condition() {
    let m = mechanics("meet_spd&sami[000]");
    assert_eq!(m.clauses.len(), 3);
    assert_eq!(m.clauses[0].when, Predicate::Always);
    let sami = Predicate::CoworkerIn {
        group: Group::Power(PowerId::new("sami")),
    };
    assert_eq!(m.clauses[1].when, sami);
    assert_eq!(m.clauses[2].when, sami);
    assert_eq!(
        m.clauses[2].effect,
        Effect::Mood {
            amount: flat(-0.5),
            target: MoodTarget::SelfOnly
        }
    );
}

#[test]
fn training_classes_and_spec_level() {
    assert_eq!(
        effects("train_spd_doubleProf[100]"),
        vec![&Effect::TrainingSpeed {
            amount: flat(30.0),
            professions: vec![Profession::Caster, Profession::Medic],
            subclass: None,
            spec_level: None,
        }]
    );
    let e = effects("train_spd&profession2[320]");
    assert_eq!(
        e[1],
        &Effect::TrainingSpeed {
            amount: flat(45.0),
            professions: vec![Profession::Guard],
            subclass: None,
            spec_level: Some(3),
        }
    );
    let e = effects("train_spd&profession3[140]");
    assert_eq!(
        e[1],
        &Effect::TrainingSpeed {
            amount: flat(45.0),
            professions: vec![Profession::Sniper],
            subclass: Some("Marksman".into()),
            spec_level: None,
        }
    );
}

#[test]
fn workshop_material_and_cost_filters() {
    assert_eq!(
        effects("workshop_formula_probability[100]"),
        vec![&Effect::ByproductRate {
            amount: flat(70.0),
            material: MaterialFilter::Product(ProductType::Evolve),
            base_cost: None,
        }]
    );
    assert_eq!(
        effects("workshop_formula_cost3[110]"),
        vec![&Effect::WorkshopMoodCost {
            change: CostChange::Delta(-1.0),
            material: MaterialFilter::Product(ProductType::Evolve),
            cost: Some(CostFilter::Exactly(4)),
        }]
    );
    assert_eq!(
        effects("workshop_proc_cost[000]"),
        vec![&Effect::ByproductRate {
            amount: flat(50.0),
            material: MaterialFilter::Any,
            base_cost: Some(CostFilter::Exactly(8)),
        }]
    );
}

#[test]
fn resources_are_named_not_dropped() {
    // Worldly Plight → productivity, per 4 points.
    assert_eq!(
        effects("manu_prod_spd_bd[300]"),
        vec![&Effect::Productivity {
            amount: Amount::PerCount {
                per: 1.0,
                step: 3.0,
                counter: Counter::Resource {
                    resource: "worldly_plight".into()
                },
                max_count: None,
                max_total: None,
            },
            product: None,
            scope: Scope::ThisRoom,
        }]
    );
    let e = effects("dorm_rec_bd_n1[100]");
    assert!(matches!(
        e[0],
        Effect::ConvertResource { from, to, .. } if from == "measure" && to == "perception_information"
    ));
}

#[test]
fn partial_skills_name_their_gaps() {
    let m = mechanics("trade_ord_wt&cost[000]");
    assert!(!m.is_fully_modeled());
    let gaps = m.unmodeled_parts();
    assert!(gaps.iter().any(|g| g.contains("higher-yield")), "{gaps:?}");
    // The quantified half still parsed.
    assert!(m.clauses.iter().any(|c| matches!(
        c.effect,
        Effect::Mood {
            target: MoodTarget::SelfOnly,
            ..
        }
    )));
}

// ---- cross-check against upstream's efficiency hint ---------------------------

/// Upstream's `efficiency` field is a display-only percentage the client
/// uses to sort skills. For skills whose primary effect is a single flat
/// percentage of the matching kind, it must equal what we parsed. Skills
/// whose hint describes something the sort ignores (conditional bonuses,
/// ramps, morale costs) are excluded.
#[test]
fn parsed_magnitudes_agree_with_efficiency_hint() {
    let mut checked = 0;
    let mut mismatches = Vec::new();
    for skill in loaded().data.skills.values() {
        if skill.efficiency_hint == 0 {
            continue;
        }
        let Some(m) = &skill.mechanics else { continue };
        // Primary effect: the first unconditional clause of the room's
        // headline stat with a flat amount.
        let primary = m.clauses.iter().find_map(|c| {
            if c.when != Predicate::Always {
                return None;
            }
            let kind_matches = matches!(
                (&c.effect, skill.room_type),
                (Effect::Productivity { .. }, RoomType::Manufacture)
                    | (Effect::OrderEfficiency { .. }, RoomType::Trading)
                    | (Effect::DroneRecovery { .. }, RoomType::Power)
                    | (Effect::ClueSpeed { .. }, RoomType::Meeting)
                    | (Effect::ContactSpeed { .. }, RoomType::Hire)
                    | (Effect::TrainingSpeed { .. }, RoomType::Training)
                    | (Effect::ByproductRate { .. }, RoomType::Workshop)
            );
            match c.effect.amount() {
                Some(Amount::Flat { value }) if kind_matches => Some(*value),
                _ => None,
            }
        });
        let Some(value) = primary else { continue };
        checked += 1;
        if (value - f64::from(skill.efficiency_hint)).abs() > 1e-9 {
            mismatches.push(format!(
                "{}: hint {} vs parsed {} — {}",
                skill.id,
                skill.efficiency_hint,
                value,
                skill.description.plain()
            ));
        }
    }
    assert!(checked > 200, "only {checked} skills cross-checked");
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn every_other_excludes_the_skill_owner() {
    // "for every other Rhine Lab Operator in base (…, caps at 5),
    // additional charging speed +3%".
    let e = effects("power_rec_rhine[000]");
    assert_eq!(e.len(), 2, "{e:?}");
    match e[1] {
        Effect::DroneRecovery {
            amount:
                Amount::PerCount {
                    per,
                    counter:
                        Counter::Operators {
                            group: Group::Power(p),
                            scope: CountScope::Base,
                            excluding_self: true,
                        },
                    max_count,
                    ..
                },
        } => {
            assert_eq!(*per, 3.0);
            assert_eq!(p.as_str(), "rhine");
            assert_eq!(*max_count, Some(5.0));
        }
        other => panic!("{other:?}"),
    }
    // Without "other", the owner counts.
    let m = mechanics("control_mp_cost&faction[990]");
    assert!(matches!(
        m.clauses[0].effect.amount(),
        Some(Amount::PerCount {
            counter: Counter::Operators {
                excluding_self: false,
                ..
            },
            ..
        })
    ));
}
