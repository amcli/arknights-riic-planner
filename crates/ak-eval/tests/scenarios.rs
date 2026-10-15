//! Layer 4 golden scenarios against the pinned snapshot.
//!
//! Each test pins one verified game rule to the number it must produce.
//! Where the rule comes from is documented in `ak_eval::rules`; the
//! operators used are chosen at run time by property (e.g. "has no Factory
//! skills") so the tests survive data bumps.

use std::sync::OnceLock;

use ak_data::{Loaded, Strictness, load_default};
use ak_domain::*;
use ak_eval::*;

fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(|| load_default(Strictness::Strict).expect("pinned snapshot loads"))
}

fn data() -> &'static GameData {
    &loaded().data
}

fn by_name(name: &str) -> OperatorId {
    data()
        .operators
        .values()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("operator {name}"))
        .id
        .clone()
}

/// Operators none of whose maxed skills target `kind`, so in a room of that
/// kind they contribute only the per-operator base bonus.
fn without_skills_for(kind: RoomType, n: usize) -> Vec<OperatorId> {
    without_skills_for_excluding(kind, n, &[])
}

fn without_skills_for_excluding(
    kind: RoomType,
    n: usize,
    exclude: &[OperatorId],
) -> Vec<OperatorId> {
    data()
        .operators
        .values()
        .filter(|o| !exclude.contains(&o.id))
        .filter(|o| {
            o.max_buffs().iter().all(|b| {
                data()
                    .skills
                    .get(b.as_str())
                    .is_none_or(|s| s.room_type != kind)
            })
        })
        .map(|o| o.id.clone())
        .take(n)
        .collect()
}

/// A Control Center, the given rooms, then as many level-3 Power Plants as
/// it takes to power them (the game refuses bases that overdraw).
fn base(extra: Vec<Room>) -> BaseConfig {
    let mut rooms = vec![Room::new("cc", RoomType::Control, 5)];
    rooms.extend(extra);
    let mut b = BaseConfig::new(rooms);
    let mut i = 0;
    while b.count_of(RoomType::Power) < 3 {
        let (supply, demand) = b.power_balance(data());
        if supply >= demand {
            break;
        }
        i += 1;
        b.rooms
            .push(Room::new(format!("pp{i}"), RoomType::Power, 3));
    }
    b
}

fn gold_factory(id: &str, level: u8) -> Room {
    Room::new(id, RoomType::Manufacture, level).with_formula("4")
}

fn assign(base: &BaseConfig, placements: &[(&str, &[OperatorId])]) -> Assignment {
    let mut a = Assignment::empty(base, data());
    for (room, ops) in placements {
        for (i, op) in ops.iter().enumerate() {
            a.place(&Slot::new(*room, u8::try_from(i).unwrap()), op.clone())
                .unwrap();
        }
    }
    a
}

fn request(base: BaseConfig, assignment: Assignment) -> SimRequest {
    SimRequest {
        base,
        assignment,
        roster: Roster::everyone_maxed(data()),
        config: SimConfig::default(),
        rotation: Rotation::None,
    }
}

fn maxed() -> Roster {
    Roster::everyone_maxed(data())
}

/// True when the only events are rig workers running out of morale at the
/// very end of a 24-hour run: a lone worker drains 1.0/h from 24.
fn quiet(events: &[SimEvent]) -> bool {
    events
        .iter()
        .all(|e| matches!(e.kind, EventKind::Exhausted { .. }) && e.hour > 23.9)
}

#[track_caller]
fn approx(actual: f64, expected: f64, tol: f64) {
    assert!(
        (actual - expected).abs() <= tol,
        "expected {expected}, got {actual}"
    );
}

// ---- Factory -----------------------------------------------------------

#[test]
fn one_skill_less_operator_is_the_baseline() {
    // Pure Gold takes 72 min (cost_point 4320 s) at 100%: 20 per day, and
    // the one operator adds the 1% base bonus.
    let op = without_skills_for(RoomType::Manufacture, 1);
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &op)]);
    let r = simulate(data(), &request(b, a)).unwrap();
    approx(r.totals.gold_produced, 20.2, 1e-6);
    assert_eq!(r.rooms[1].initial_stat_pct, Some(101.0));
    assert!(quiet(&r.events), "{:?}", r.events);
}

#[test]
fn unstaffed_rooms_do_no_work_except_power_plants() {
    // wiki.gg Factory, Trading Post: they "will only function when at least
    // one Operator is assigned"; Human Resources Office likewise. wiki.gg
    // Power Plant: drones charge unstaffed, one per 6 minutes.
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("t1", RoomType::Trading, 3),
        Room::new("h1", RoomType::Hire, 3),
    ]);
    let a = Assignment::empty(&b, data());
    let mut req = request(b.clone(), a.clone());
    req.config.initial_gold = 50.0;
    let r = simulate(data(), &req).unwrap();
    approx(r.totals.gold_produced, 0.0, 1e-12);
    approx(r.totals.lmd, 0.0, 1e-12);
    approx(r.totals.contacts, 0.0, 1e-12);
    for room in &r.rooms {
        let expected = match room.kind {
            RoomType::Power => Some(100.0),
            RoomType::Manufacture | RoomType::Trading | RoomType::Hire => Some(0.0),
            _ => None,
        };
        assert_eq!(room.initial_stat_pct, expected, "{}", room.id);
    }
    let plants = b.count_of(RoomType::Power) as f64;
    approx(r.totals.drones, 24.0 * 10.0 * plants, 1e-6);

    // One operator anywhere in the Factory is enough, even at zero morale:
    // only their skills stop.
    let op = without_skills_for(RoomType::Manufacture, 1);
    let staffed = assign(&b, &[("f1", &op)]);
    let snap = evaluate(data(), &b, &staffed, &maxed(), &SimConfig::default()).unwrap();
    approx(snap.rooms[1].productivity_pct, 101.0, 1e-9);
    let mut tired = maxed();
    tired.insert(
        op[0].clone(),
        RosterEntry {
            promotion: UnlockCond::MAX,
            mood: Some(0.0),
        },
    );
    let config = SimConfig {
        initial_mood: MoodPolicy::Roster,
        ..SimConfig::default()
    };
    let snap = evaluate(data(), &b, &staffed, &tired, &config).unwrap();
    approx(snap.rooms[1].productivity_pct, 100.0, 1e-9);
}

#[test]
fn each_working_operator_adds_one_percent_and_headcount_relieves_drain() {
    // PRTS 制造站: +1% per working operator; 3 operators drain 0.9/h each.
    let ops = without_skills_for(RoomType::Manufacture, 3);
    assert_eq!(ops.len(), 3);
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &ops)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    approx(snap.rooms[1].productivity_pct, 103.0, 1e-9);
    assert_eq!(snap.mood.len(), 3);
    for m in &snap.mood {
        approx(m.base, -0.9, 1e-9);
        approx(m.skills, 0.0, 1e-9);
    }

    let r = simulate(data(), &request(b, a)).unwrap();
    // 24 h × 0.9/h = 21.6 drained: 2.4 left, nobody exhausted.
    for o in &r.operators {
        approx(o.final_mood, 2.4, 1e-6);
        approx(o.hours_working, 24.0, 1e-9);
    }
    approx(r.totals.gold_produced, 20.6, 1e-6);
    assert!(r.events.is_empty(), "{:?}", r.events);
}

#[test]
fn two_operators_drain_slightly_less() {
    let ops = without_skills_for(RoomType::Manufacture, 2);
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &ops)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    for m in &snap.mood {
        approx(m.base, -0.95, 1e-9);
    }
}

#[test]
fn standardization_alpha_adds_fifteen_points() {
    // An operator whose only active skill at E0 L1 is Standardization α.
    let target = BuffId::new("manu_prod_spd[000]");
    let op = data()
        .operators
        .values()
        .find(|o| o.active_buffs(UnlockCond::BASE) == vec![&target])
        .expect("someone has only Standardization α at E0 L1")
        .id
        .clone();
    let b = base(vec![gold_factory("f1", 1)]);
    let a = assign(&b, &[("f1", std::slice::from_ref(&op))]);
    let mut roster = Roster::new();
    roster.insert(op.clone(), RosterEntry::at(UnlockCond::BASE));
    let snap = evaluate(data(), &b, &a, &roster, &SimConfig::default()).unwrap();
    approx(snap.rooms[1].productivity_pct, 116.0, 1e-9);
    assert_eq!(snap.contributions.len(), 1);
    assert_eq!(snap.contributions[0].skill, target);
    assert!(snap.warnings.is_empty(), "{:?}", snap.warnings);
}

#[test]
fn exhausted_operator_stops_contributing() {
    // One operator, no CC: drains 1.0/h, hits zero at hour 24 exactly, so
    // run 30 h: productivity is 101 for 24 h and 100 for 6 h.
    let ops = without_skills_for(RoomType::Manufacture, 1);
    let b = base(vec![gold_factory("f1", 1)]);
    let a = assign(&b, &[("f1", &ops)]);
    let mut req = request(b, a);
    req.config.horizon_hours = 30.0;
    let r = simulate(data(), &req).unwrap();
    let op = &r.operators[0];
    approx(op.final_mood, 0.0, 1e-9);
    approx(op.hours_working, 24.0, 1e-6);
    approx(op.hours_exhausted, 6.0, 1e-6);
    approx(
        r.rooms[1].average_stat_pct.unwrap(),
        (101.0 * 24.0 + 100.0 * 6.0) / 30.0,
        1e-6,
    );
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Exhausted { .. }))
    );
}

// ---- Control Center and Dormitory ---------------------------------------

#[test]
fn control_center_relieves_every_working_operator() {
    // wiki.gg Control Center: each stationed operator gives −0.05/h to
    // everyone in the base, including the Control Center itself.
    let cc = without_skills_for(RoomType::Control, 5);
    let worker = without_skills_for_excluding(RoomType::Manufacture, 1, &cc);
    let b = base(vec![gold_factory("f1", 1)]);
    let a = assign(&b, &[("cc", &cc), ("f1", &worker)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    let w = snap.mood.iter().find(|m| m.operator == worker[0]).unwrap();
    approx(w.base, -0.75, 1e-9);
    let c = snap.mood.iter().find(|m| m.operator == cc[0]).unwrap();
    approx(c.base, -0.75, 1e-9);
}

#[test]
fn dormitory_recovery_matches_published_formula() {
    // 1.5 + 0.1 × level + 0.0004 × ambience (wiki.gg Dormitory): level 5 at
    // 5000 ambience is 4.0/h; level 1 bare is 1.6/h.
    let op = without_skills_for(RoomType::Dormitory, 1).remove(0);
    let b = base(vec![
        Room::new("d5", RoomType::Dormitory, 5).with_ambience(5000),
        Room::new("d1", RoomType::Dormitory, 1),
    ]);
    let mut roster = maxed();
    roster.entries.get_mut(&op).unwrap().mood = Some(10.0);
    let config = SimConfig {
        initial_mood: MoodPolicy::Roster,
        ..SimConfig::default()
    };

    let a = assign(&b, &[("d5", std::slice::from_ref(&op))]);
    let snap = evaluate(data(), &b, &a, &roster, &config).unwrap();
    approx(snap.mood[0].base, 4.0, 1e-9);

    let a1 = assign(&b, &[("d1", std::slice::from_ref(&op))]);
    let snap1 = evaluate(data(), &b, &a1, &roster, &config).unwrap();
    approx(snap1.mood[0].base, 1.6, 1e-9);

    let mut req = request(b, a);
    req.roster = roster;
    req.config = SimConfig {
        horizon_hours: 2.0,
        initial_mood: MoodPolicy::Roster,
        ..SimConfig::default()
    };
    let r = simulate(data(), &req).unwrap();
    approx(r.operators[0].final_mood, 18.0, 1e-6);
    approx(r.operators[0].hours_resting, 2.0, 1e-9);
}

#[test]
fn team_rainbow_zeroes_control_center_drain() {
    // wiki.gg: with Ash, Blitz, Frost and Tachanka in the Control Center
    // its drain is exactly zero; a fifth operator makes it recover. This
    // is the evidence that "each Team Rainbow operator" counts the owner.
    let r6: Vec<OperatorId> = ["Ash", "Blitz", "Frost", "Tachanka"]
        .iter()
        .map(|n| by_name(n))
        .collect();
    let b = base(vec![]);
    let a = assign(&b, &[("cc", &r6)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    for m in &snap.mood {
        approx(m.base, -0.8, 1e-9);
        approx(m.total, 0.0, 1e-9);
    }

    let extra = without_skills_for_excluding(RoomType::Control, 1, &r6);
    let mut five = r6.clone();
    five.push(extra[0].clone());
    let a = assign(&b, &[("cc", &five)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    for m in &snap.mood {
        approx(m.total, 0.05, 1e-9);
    }
}

// ---- Trading Post -------------------------------------------------------

#[test]
fn trading_post_throughput_matches_order_table() {
    // Level 3 mix: E[LMD] 1450, E[gold] 2.9, E[time] 203.4 min, at the 101%
    // one skill-less operator gives.
    let op = without_skills_for(RoomType::Trading, 1);
    let b = base(vec![Room::new("t1", RoomType::Trading, 3)]);
    let a = assign(&b, &[("t1", &op)]);
    let mut req = request(b, a);
    req.config.initial_gold = 1000.0;
    let r = simulate(data(), &req).unwrap();
    let orders = 24.0 * 60.0 / 203.4 * 1.01;
    approx(r.totals.orders_completed, orders, 1e-6);
    approx(r.totals.lmd, orders * 1450.0, 1e-3);
    approx(r.totals.gold_consumed, orders * 2.9, 1e-6);
    approx(r.totals.gold_in_depot, 1000.0 - orders * 2.9, 1e-6);
    assert!(quiet(&r.events), "{:?}", r.events);
}

#[test]
fn trading_post_lives_off_the_factories() {
    // One gold Factory at 101% makes 20.2/day; a level-3 post at 101% wants
    // 20.73. Every unit produced is sold and the post reports starvation.
    let maker = without_skills_for(RoomType::Manufacture, 1);
    let seller = without_skills_for_excluding(RoomType::Trading, 1, &maker);
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("t1", RoomType::Trading, 3),
    ]);
    let a = assign(&b, &[("f1", &maker), ("t1", &seller)]);
    let r = simulate(data(), &request(b, a)).unwrap();
    approx(r.totals.gold_produced, 20.2, 1e-6);
    approx(r.totals.gold_consumed, 20.2, 1e-6);
    approx(r.totals.lmd, 20.2 / 2.9 * 1450.0, 1e-3);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::InputStarved { .. }))
    );
}

// ---- Collection and storage ---------------------------------------------

#[test]
fn periodic_collection_caps_factory_storage() {
    // A level-1 store holds 24 volume, and Pure Gold takes 2 (PRTS 制造站):
    // 12 units, which is 14.4 h of output at 100%, 14.26 h at the 101% one
    // skill-less operator gives.
    let op = without_skills_for(RoomType::Manufacture, 1);
    let b = base(vec![gold_factory("f1", 1)]);
    let a = assign(&b, &[("f1", &op)]);
    let mut req = request(b, a);
    req.config.horizon_hours = 48.0;
    req.config.collection = CollectionPolicy::EveryHours { hours: 48.0 };
    let r = simulate(data(), &req).unwrap();
    approx(r.totals.gold_produced, 12.0, 1e-6);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::StorageFull { .. }))
    );
    let f = &r.rooms[1];
    approx(f.hours_blocked, 48.0 - 14.4 / 1.01, 1e-6);
    approx(f.in_storage, 0.0, 1e-9);
}

// ---- Rotation -----------------------------------------------------------

#[test]
fn mood_threshold_rotation_prevents_exhaustion() {
    let pool = without_skills_for(RoomType::Manufacture, 6);
    let (workers, bench) = pool.split_at(3);
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("d1", RoomType::Dormitory, 5).with_ambience(5000),
    ]);
    let a = assign(&b, &[("f1", workers)]);
    let mut req = request(b, a);
    req.config.horizon_hours = 72.0;
    req.rotation = Rotation::MoodThreshold {
        swap_out: 4.0,
        swap_in: 20.0,
        bench: bench.to_vec(),
    };
    let r = simulate(data(), &req).unwrap();
    assert!(
        !r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Exhausted { .. })),
        "{:?}",
        r.events
    );
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Moved { .. }))
    );
    let one_tick = 0.9 * 5.0 / 60.0;
    for o in &r.operators {
        assert!(o.min_mood >= 4.0 - one_tick - 1e-9, "{o:?}");
        assert!(o.hours_working > 0.0, "{o:?}");
    }
    // The Factory never runs short-handed.
    assert!(r.rooms[1].average_stat_pct.unwrap() > 102.9);
    approx(r.totals.gold_produced, 20.6 * 3.0, 0.1);
}

// ---- Honesty ------------------------------------------------------------

#[test]
fn unmodeled_parts_are_reported_not_dropped() {
    let mut reported = 0;
    for partial in loaded().report.mechanics.partial_skills.iter().take(40) {
        let skill = data().skill(partial.id.as_str()).unwrap();
        let Some(op) = data()
            .operators
            .values()
            .find(|o| o.max_buffs().iter().any(|b| **b == partial.id))
        else {
            continue;
        };
        let (b, room) = if skill.room_type == RoomType::Control {
            (base(vec![]), "cc")
        } else {
            let level = data().facility(skill.room_type).unwrap().max_level();
            let mut room = Room::new("x", skill.room_type, level);
            if skill.room_type == RoomType::Training {
                room = room.with_training(TrainingJob {
                    profession: op.profession,
                    subclass: None,
                    spec_level: 1,
                });
            }
            (base(vec![room]), "x")
        };
        let a = assign(&b, &[(room, std::slice::from_ref(&op.id))]);
        let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
        if !snap.warnings.is_empty() {
            reported += 1;
            // The model cannot say what the skill does, so its owner counts.
            assert!(
                counted(&snap, &op.id),
                "{} warned but did not count",
                partial.id
            );
        }
    }
    assert!(
        reported > 10,
        "only {reported} partial skills produced a warning"
    );
}

#[test]
fn random_assignments_stay_in_bounds() {
    let ops: Vec<&OperatorId> = data().operators.keys().collect();
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("f2", RoomType::Manufacture, 3).with_formula("1"),
        Room::new("t1", RoomType::Trading, 3),
        Room::new("p1", RoomType::Power, 3),
        Room::new("d1", RoomType::Dormitory, 5).with_ambience(2000),
        Room::new("m1", RoomType::Meeting, 3),
        Room::new("h1", RoomType::Hire, 3),
    ]);
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..8 {
        let mut a = Assignment::empty(&b, data());
        let vacancies: Vec<Slot> = a.vacancies().collect();
        for slot in vacancies {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let op = ops[usize::try_from(seed >> 33).unwrap() % ops.len()];
            let _ = a.place(&slot, op.clone());
        }
        let r = simulate(data(), &request(b.clone(), a)).unwrap();
        assert!(r.totals.lmd >= 0.0 && r.totals.exp >= 0.0);
        assert!(r.totals.gold_produced >= 0.0 && r.totals.drones >= 0.0);
        for o in &r.operators {
            assert!((0.0..=24.0).contains(&o.final_mood), "{o:?}");
            assert!((0.0..=24.0).contains(&o.min_mood), "{o:?}");
        }
        for room in &r.rooms {
            if let Some(p) = room.average_stat_pct {
                assert!(p >= 0.0, "{room:?}");
            }
        }
        // Serialisable round trip.
        let json = serde_json::to_string(&r).unwrap();
        let back: SimResult = serde_json::from_str(&json).unwrap();
        approx(back.totals.lmd, r.totals.lmd, 1e-6);
        assert_eq!(back.rooms.len(), r.rooms.len());
        // One level-3 Power Plant charges 10 drones an hour at 100%.
        assert!(r.totals.drones >= 240.0 - 1e-6, "{}", r.totals.drones);
    }
}

// ---- Training Room and Office --------------------------------------------

fn training_room(with_job: bool) -> Room {
    let room = Room::new("tr", RoomType::Training, 3);
    if with_job {
        room.with_training(TrainingJob {
            profession: Profession::Guard,
            subclass: None,
            spec_level: 1,
        })
    } else {
        room
    }
}

#[test]
fn training_room_only_the_assistant_works() {
    // PRTS 训练室: only the assistant's skills apply, the trainee does not
    // drain morale, and the assistant works only while training runs.
    // Franka's maxed Training skill is +50% for Guards.
    let franka = by_name("Franka");
    let other = without_skills_for_excluding(RoomType::Training, 1, std::slice::from_ref(&franka))
        .remove(0);
    let b = base(vec![training_room(true)]);
    let a = assign(&b, &[("tr", &[franka.clone(), other.clone()])]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    approx(snap.rooms[1].training_speed_pct, 100.0 + 5.0 + 50.0, 1e-9);
    let rate =
        |s: &Snapshot, id: &OperatorId| s.mood.iter().find(|m| &m.operator == id).unwrap().clone();
    approx(rate(&snap, &franka).total, -1.0, 1e-9);
    assert!(rate(&snap, &other).idle);
    approx(rate(&snap, &other).total, 0.0, 1e-9);

    // Franka as the trainee contributes nothing.
    let swapped = assign(&b, &[("tr", &[other.clone(), franka.clone()])]);
    let snap = evaluate(data(), &b, &swapped, &maxed(), &SimConfig::default()).unwrap();
    approx(snap.rooms[1].training_speed_pct, 105.0, 1e-9);

    // No training job: the assistant is idle too.
    let b = base(vec![training_room(false)]);
    let a = assign(&b, &[("tr", std::slice::from_ref(&franka))]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    assert!(snap.mood[0].idle);
    approx(snap.rooms[1].training_speed_pct, 100.0, 1e-9);
}

#[test]
fn training_completes_after_base_hours_over_speed() {
    // Specialisation 1 is 8 base hours; at 155% it takes 8 / 1.55 hours,
    // after which the assistant stops working.
    let franka = by_name("Franka");
    let b = base(vec![training_room(true)]);
    let a = assign(&b, &[("tr", std::slice::from_ref(&franka))]);
    let mut req = request(b, a);
    req.config.horizon_hours = 10.0;
    let r = simulate(data(), &req).unwrap();
    let tr = &r.rooms[1];
    approx(tr.training_completed_hour.unwrap(), 8.0 / 1.55, 1e-6);
    approx(tr.training_progress_hours, 8.0, 1e-9);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::TrainingCompleted { .. }))
    );
    let f = r.operators.iter().find(|o| o.id == franka).unwrap();
    assert!(
        f.hours_idle > 10.0 - 8.0 / 1.55 - 5.0 / 60.0 - 1e-9,
        "{f:?}"
    );
    approx(f.hours_idle + f.hours_working, 10.0, 1e-9);
}

#[test]
fn office_contacts_follow_the_twelve_hour_base() {
    // PRTS 办公室: 12 h per contact at 100%, +5% per working operator, at
    // most 3 stored.
    let op = without_skills_for(RoomType::Hire, 1).remove(0);
    let b = base(vec![Room::new("h1", RoomType::Hire, 3)]);
    let a = assign(&b, &[("h1", std::slice::from_ref(&op))]);
    let r = simulate(data(), &request(b.clone(), a.clone())).unwrap();
    approx(r.rooms[1].initial_stat_pct.unwrap(), 105.0, 1e-9);
    approx(r.totals.contacts, 24.0 * 1.05 / 12.0, 1e-6);

    let mut req = request(b, a);
    req.config.horizon_hours = 72.0;
    req.config.collection = CollectionPolicy::EveryHours { hours: 72.0 };
    let r = simulate(data(), &req).unwrap();
    approx(r.totals.contacts, 3.0, 1e-9);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::StorageFull { .. }))
    );
}

// ---- Stacking and scaling -------------------------------------------------

#[test]
fn control_center_trading_bonus_applies_once() {
    // "all Trading Posts' order efficiency +7% (only the most effective one
    // will take effect …)": two holders give +7%, not +14%.
    let holders: Vec<OperatorId> = data()
        .operators
        .values()
        .filter(|o| {
            o.max_buffs()
                .iter()
                .any(|b| b.family() == "control_tra_spd")
        })
        .map(|o| o.id.clone())
        .take(2)
        .collect();
    assert_eq!(holders.len(), 2);
    let b = base(vec![Room::new("t1", RoomType::Trading, 3)]);
    let a = assign(&b, &[("cc", &holders)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    let from_cc: f64 = snap
        .contributions
        .iter()
        .filter(|c| c.room.as_str() == "t1" && c.skill.family() == "control_tra_spd")
        .map(|c| c.value)
        .sum();
    approx(from_cc, 7.0, 1e-9);
}

/// First maxed skill tier of `family` held by any operator, with its owner.
fn holder_of(family: &str) -> (OperatorId, &'static BaseSkill) {
    data()
        .operators
        .values()
        .find_map(|o| {
            o.max_buffs()
                .into_iter()
                .find(|b| b.family() == family)
                .map(|b| (o.id.clone(), &data().skills[b.as_str()]))
        })
        .unwrap_or_else(|| panic!("nobody holds {family}"))
}

fn per_count_value(skill: &BaseSkill) -> f64 {
    skill
        .mechanics
        .as_ref()
        .unwrap()
        .clauses
        .iter()
        .find_map(|c| match c.effect.amount() {
            Some(Amount::PerCount { per, .. }) => Some(*per),
            _ => None,
        })
        .unwrap()
}

#[test]
fn automation_zeroes_only_coworkers_skill_bonuses() {
    // "the productivity contributed by all other Operators in that Factory
    // becomes 0 (excluding productivity granted based on facility count),
    // but each Power Plant increases that Factory's productivity by +N%".
    // A coworker's Standardization α is zeroed; the Control Center's +2%
    // is not.
    let (auto, auto_skill) = holder_of("manu_prod_spd&power");
    let (cc_op, _) = holder_of("control_prod_spd");
    let std_skill = BuffId::new("manu_prod_spd[000]");
    let std_op = data()
        .operators
        .values()
        .find(|o| o.active_buffs(UnlockCond::BASE) == vec![&std_skill] && o.id != auto)
        .unwrap()
        .id
        .clone();
    let mut roster = maxed();
    roster.insert(std_op.clone(), RosterEntry::at(UnlockCond::BASE));
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(
        &b,
        &[
            ("cc", std::slice::from_ref(&cc_op)),
            ("f1", &[auto.clone(), std_op.clone()]),
        ],
    );
    let snap = evaluate(data(), &b, &a, &roster, &SimConfig::default()).unwrap();

    let find = |op: &OperatorId, skill: &str| {
        snap.contributions
            .iter()
            .find(|c| &c.operator == op && c.skill.as_str() == skill)
            .unwrap_or_else(|| panic!("no contribution from {op} {skill}"))
            .clone()
    };
    let std_c = find(&std_op, "manu_prod_spd[000]");
    approx(std_c.value, 0.0, 1e-9);
    assert_eq!(std_c.scaled_from, Some(15.0));
    let cc_c = snap
        .contributions
        .iter()
        .find(|c| c.operator == cc_op && c.room.as_str() == "f1")
        .unwrap();
    approx(cc_c.value, 2.0, 1e-9);
    assert_eq!(cc_c.scaled_from, None);
    let powers = b.count_of(RoomType::Power) as f64;
    let auto_c = find(&auto, auto_skill.id.as_str());
    approx(auto_c.value, per_count_value(auto_skill) * powers, 1e-9);
    let sum: f64 = snap
        .contributions
        .iter()
        .filter(|c| c.room.as_str() == "f1")
        .map(|c| c.value)
        .sum();
    approx(snap.rooms[1].productivity_pct, 102.0 + sum, 1e-9);
}

#[test]
fn knight_bonus_counts_knights_in_each_factory() {
    // "all Knight Operators assigned to Factories gain productivity +N%":
    // each Factory counts its own Knights, not the Control Center's.
    let (owner, skill) = holder_of("control_prod_fraction");
    let per = per_count_value(skill);
    let pool = without_skills_for_excluding(RoomType::Manufacture, 3, std::slice::from_ref(&owner));
    let mut config = SimConfig::default();
    config
        .memberships
        .tags
        .insert("knight".into(), pool[..2].iter().cloned().collect());
    let b = base(vec![gold_factory("f1", 3), gold_factory("f2", 3)]);
    let a = assign(
        &b,
        &[
            ("cc", std::slice::from_ref(&owner)),
            ("f1", &pool[..2]),
            ("f2", &pool[2..3]),
        ],
    );
    let snap = evaluate(data(), &b, &a, &maxed(), &config).unwrap();
    let bonus = |room: &str| -> f64 {
        snap.contributions
            .iter()
            .filter(|c| c.room.as_str() == room && c.skill == skill.id)
            .map(|c| c.value)
            .sum()
    };
    approx(bonus("f1"), 2.0 * per, 1e-9);
    approx(bonus("f2"), 0.0, 1e-9);
}

#[test]
fn every_other_rhine_lab_operator_excludes_the_owner() {
    let (owner, skill) = holder_of("power_rec_rhine");
    let rhine = PowerId::new("rhine");
    assert!(
        data().operators[owner.as_str()]
            .powers()
            .any(|p| *p == rhine)
    );
    let others: Vec<OperatorId> = data()
        .operators
        .values()
        .filter(|o| o.id != owner && o.powers().any(|p| *p == rhine))
        .map(|o| o.id.clone())
        .take(2)
        .collect();
    assert_eq!(others.len(), 2);
    let b = base(vec![
        Room::new("p1", RoomType::Power, 3),
        Room::new("d1", RoomType::Dormitory, 5),
    ]);
    let a = assign(&b, &[("p1", std::slice::from_ref(&owner)), ("d1", &others)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    let flat: f64 = skill
        .mechanics
        .as_ref()
        .unwrap()
        .clauses
        .iter()
        .filter_map(|c| match c.effect.amount() {
            Some(Amount::Flat { value }) => Some(*value),
            _ => None,
        })
        .sum();
    let total: f64 = snap
        .contributions
        .iter()
        .filter(|c| c.skill == skill.id)
        .map(|c| c.value)
        .sum();
    approx(total, flat + per_count_value(skill) * 2.0, 1e-9);
}

#[test]
fn a_trading_post_needs_three_kjerag_operators_for_the_bonus() {
    // "All Trading Posts with 3 Kjerag Operators assigned gain order
    // acquisition efficiency +10%". Kjerag is the nation (the CN text of
    // `cc.g.karlan` is 谢拉格), so members outside the Karlan Trade group
    // count; only those are used here, so a wrong mapping shows.
    let (owner, skill) = holder_of("control_tra_limit&spd3");
    let has = |o: &Operator, p: &str| o.powers().any(|x| x.as_str() == p);
    let kjerag: Vec<OperatorId> = data()
        .operators
        .values()
        .filter(|o| o.id != owner && has(o, "kjerag") && !has(o, "karlan"))
        .map(|o| o.id.clone())
        .take(5)
        .collect();
    assert_eq!(kjerag.len(), 5);
    let b = base(vec![
        Room::new("t1", RoomType::Trading, 3),
        Room::new("t2", RoomType::Trading, 3),
    ]);
    let a = assign(
        &b,
        &[
            ("cc", std::slice::from_ref(&owner)),
            ("t1", &kjerag[..3]),
            ("t2", &kjerag[3..]),
        ],
    );
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    let bonus = |room: &str| -> f64 {
        snap.contributions
            .iter()
            .filter(|c| c.room.as_str() == room && c.skill == skill.id)
            .map(|c| c.value)
            .sum()
    };
    approx(bonus("t1"), per_count_value(skill), 1e-9);
    approx(bonus("t2"), 0.0, 1e-9);
}

#[test]
fn every_operator_in_the_factory_adds_capacity_including_the_owner() {
    // "… but every Operator in that Factory increases that Factory's
    // Capacity limit by +5": three Operators, three times the amount.
    let (owner, skill) = holder_of("manu_prod_spd&manu");
    let per = skill
        .mechanics
        .as_ref()
        .unwrap()
        .clauses
        .iter()
        .find_map(|c| match &c.effect {
            Effect::Capacity {
                amount: Amount::PerCount { per, .. },
                ..
            } => Some(*per),
            _ => None,
        })
        .unwrap();
    let pool = without_skills_for_excluding(RoomType::Manufacture, 2, std::slice::from_ref(&owner));
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &[owner, pool[0].clone(), pool[1].clone()])]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    let added: f64 = snap
        .contributions
        .iter()
        .filter(|c| c.skill == skill.id && c.stat == StatKind::Capacity)
        .map(|c| c.value)
        .sum();
    approx(added, 3.0 * per, 1e-9);
    let f1 = snap.rooms.iter().find(|r| r.id.as_str() == "f1").unwrap();
    let level_3 = f64::from(data().constants.manufacture.phases[2].output_capacity);
    approx(f1.capacity, level_3 + 3.0 * per, 1e-9);
}

// ---- Base validation and Factory inputs ---------------------------------

#[test]
fn base_validation_enforces_power_and_layout() {
    let data = data();
    let cc = || Room::new("cc", RoomType::Control, 5);
    let b = BaseConfig::new(vec![cc(), gold_factory("f1", 3), gold_factory("f2", 3)]);
    assert_eq!(
        b.validate(data),
        Err(BaseError::PowerDeficit {
            supply: 0,
            demand: 120
        })
    );

    // The standard 2-4-3 layout balances exactly: three Power Plants supply
    // 810 against two Trading Posts, four Factories, four Dormitories and
    // the four function rooms.
    let mut rooms = vec![cc()];
    rooms.extend((0..3).map(|i| Room::new(format!("p{i}"), RoomType::Power, 3)));
    rooms.extend((0..2).map(|i| Room::new(format!("t{i}"), RoomType::Trading, 3)));
    rooms.extend((0..4).map(|i| gold_factory(&format!("f{i}"), 3)));
    rooms.extend((0..4).map(|i| Room::new(format!("d{i}"), RoomType::Dormitory, 5)));
    rooms.push(Room::new("ws", RoomType::Workshop, 3));
    rooms.push(Room::new("of", RoomType::Hire, 3));
    rooms.push(Room::new("tr", RoomType::Training, 3));
    rooms.push(Room::new("rr", RoomType::Meeting, 3));
    let b = BaseConfig::new(rooms);
    assert_eq!(b.power_balance(data), (810, 810));
    assert_eq!(b.validate(data), Ok(()));

    // Ten production rooms do not fit the nine production slots.
    let mut rooms = vec![cc()];
    rooms.extend((0..3).map(|i| Room::new(format!("p{i}"), RoomType::Power, 3)));
    rooms.extend((0..5).map(|i| gold_factory(&format!("f{i}"), 3)));
    rooms.extend((0..2).map(|i| Room::new(format!("t{i}"), RoomType::Trading, 3)));
    assert_eq!(
        BaseConfig::new(rooms).validate(data),
        Err(BaseError::NoSlots {
            category: RoomCategory::Output,
            rooms: 10,
            slots: 9
        })
    );
}

#[test]
fn input_bound_formulas_need_a_stock() {
    // Dualchips take one second a batch, so output is set by inputs.
    let op = without_skills_for(RoomType::Manufacture, 1);
    let b = base(vec![
        Room::new("f1", RoomType::Manufacture, 3).with_formula("5"),
    ]);
    let a = assign(&b, &[("f1", &op)]);
    let r = simulate(data(), &request(b.clone(), a.clone())).unwrap();
    assert!(r.totals.items.is_empty());
    assert!(
        r.warnings
            .iter()
            .any(|w| matches!(w, SimWarning::InputBoundFormula { .. }))
    );

    let mut req = request(b, a);
    req.config.input_stock = Some(
        [(ItemId::new("3212"), 10.0), (ItemId::new("32001"), 3.0)]
            .into_iter()
            .collect(),
    );
    let r = simulate(data(), &req).unwrap();
    // Three catalysts make three Dualchips from six chips.
    approx(r.totals.items[&ItemId::new("3213")], 3.0, 1e-9);
    approx(r.totals.consumed[&ItemId::new("3212")], 6.0, 1e-9);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::InputStarved { .. }))
    );
}

#[test]
fn shard_formula_spends_lmd() {
    // Formula 13: one Originium Shard per hour at 100% for two Orirock
    // Cubes and 1600 LMD; one skill-less operator makes it 101%.
    let op = without_skills_for(RoomType::Manufacture, 1);
    let b = base(vec![
        Room::new("f1", RoomType::Manufacture, 3).with_formula("13"),
    ]);
    let a = assign(&b, &[("f1", &op)]);
    let r = simulate(data(), &request(b, a)).unwrap();
    approx(r.totals.items[&ItemId::new("3141")], 24.24, 1e-6);
    approx(r.totals.lmd_spent, 24.24 * 1600.0, 1e-3);
    approx(r.totals.shards_in_depot, 24.24, 1e-6);
}

/// Sum of an operator's maxed Dormitory mood clauses with a given target,
/// for unconditional flat amounts. Panics on anything else, so the test
/// below only runs on skills it understands.
fn dorm_mood(op: &OperatorId, target: &MoodTarget) -> f64 {
    let mut total = 0.0;
    for b in data().operators[op.as_str()].max_buffs() {
        let s = &data().skills[b.as_str()];
        if s.room_type != RoomType::Dormitory {
            continue;
        }
        for c in &s.mechanics.as_ref().unwrap().clauses {
            match c {
                Clause {
                    when: Predicate::Always,
                    effect:
                        Effect::Mood {
                            amount: Amount::Flat { value },
                            target: t,
                        },
                } => {
                    if t == target {
                        total += value;
                    }
                }
                other => panic!("{} has an unexpected clause {other:?}", s.id),
            }
        }
    }
    total
}

#[test]
fn dorm_group_recovery_takes_only_the_strongest_across_families() {
    // "(Only the strongest effect of this type takes place)" is per effect
    // type. "Laziness" (self -0.1, everyone +0.2) and a plain whole-room
    // recovery are different skill families of the same type, so only the
    // larger whole-room part applies; Laziness's self part still does.
    let (a, _) = holder_of("dorm_rec_all");
    // Laziness is tiers [000] and [001] of this family; later tiers are a
    // different skill (self plus everyone else).
    let b = data()
        .operators
        .values()
        .find(|o| {
            o.max_buffs()
                .iter()
                .any(|x| x.as_str().starts_with("dorm_rec_all&oneself[00"))
        })
        .expect("a Laziness holder")
        .id
        .clone();
    let a_all = dorm_mood(&a, &MoodTarget::AllInRoom);
    let b_all = dorm_mood(&b, &MoodTarget::AllInRoom);
    let b_self = dorm_mood(&b, &MoodTarget::SelfOnly);
    assert!(a_all > 0.0 && b_all > 0.0 && b_self < 0.0);
    let room = base(vec![Room::new("d1", RoomType::Dormitory, 1)]);
    let asg = assign(&room, &[("d1", &[a.clone(), b.clone()])]);
    let snap = evaluate(data(), &room, &asg, &maxed(), &SimConfig::default()).unwrap();
    let skills = |id: &OperatorId| snap.mood.iter().find(|m| &m.operator == id).unwrap().skills;
    let best = a_all.max(b_all);
    approx(skills(&a), best, 1e-9);
    approx(skills(&b), best + b_self, 1e-9);
}

// ---- Who counts --------------------------------------------------------------

/// Whether `op` counted in `snap`: see `MoodRate::counted`.
fn counted(snap: &Snapshot, op: &OperatorId) -> bool {
    snap.mood
        .iter()
        .find(|m| &m.operator == op)
        .unwrap_or_else(|| panic!("{op} is not stationed"))
        .counted
}

#[test]
fn an_operator_whose_skills_do_nothing_there_is_filler() {
    // An order efficiency holder counts in a Trading Post; an operator with
    // no Trading Post skills adds only the +1% anyone would.
    let (trader, _) = holder_of("trade_ord_spd");
    let filler =
        without_skills_for_excluding(RoomType::Trading, 1, std::slice::from_ref(&trader)).remove(0);
    let b = base(vec![Room::new("t1", RoomType::Trading, 3)]);
    let a = assign(&b, &[("t1", &[trader.clone(), filler.clone()])]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    assert!(counted(&snap, &trader));
    assert!(!counted(&snap, &filler));

    // Over a run, the report names the room they were filler in.
    let r = simulate(data(), &request(b, a)).unwrap();
    let report = |id: &OperatorId| r.operators.iter().find(|o| &o.id == id).unwrap();
    assert!(report(&trader).filler_in.is_empty());
    assert_eq!(report(&filler).filler_in, vec![RoomId::new("t1")]);
}

#[test]
fn operators_a_skill_picks_out_by_group_count() {
    // "All Knight Operators assigned to Factories gain productivity": with
    // its owner in the Control Center the Knights count, though their own
    // skills do nothing in a Factory. A non-Knight beside them does not.
    let (owner, _) = holder_of("control_prod_fraction");
    let pool = without_skills_for_excluding(RoomType::Manufacture, 3, std::slice::from_ref(&owner));
    let mut config = SimConfig::default();
    config
        .memberships
        .tags
        .insert("knight".into(), pool[..2].iter().cloned().collect());
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("cc", std::slice::from_ref(&owner)), ("f1", &pool)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &config).unwrap();
    assert!(counted(&snap, &owner));
    assert!(counted(&snap, &pool[0]) && counted(&snap, &pool[1]));
    assert!(!counted(&snap, &pool[2]));

    // Without the owner, nothing picks them out.
    let a = assign(&b, &[("f1", &pool)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &config).unwrap();
    assert!(pool.iter().all(|op| !counted(&snap, op)));
}

#[test]
fn a_coworker_whose_bonus_automation_zeroes_is_filler() {
    // Automation zeroes the productivity skills of the others in its
    // Factory, so a Standardization holder beside it adds nothing that an
    // operator without skills would not. The Automation holder counts (here
    // through their own per-Power-Plant bonus).
    let (auto, _) = holder_of("manu_prod_spd&power");
    let std_skill = BuffId::new("manu_prod_spd[000]");
    let std_op = data()
        .operators
        .values()
        .find(|o| o.active_buffs(UnlockCond::BASE) == vec![&std_skill] && o.id != auto)
        .unwrap()
        .id
        .clone();
    let mut roster = maxed();
    roster.insert(std_op.clone(), RosterEntry::at(UnlockCond::BASE));
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &[auto.clone(), std_op.clone()])]);
    let snap = evaluate(data(), &b, &a, &roster, &SimConfig::default()).unwrap();
    assert!(counted(&snap, &auto));
    assert!(!counted(&snap, &std_op));

    // On their own, Standardization counts.
    let a = assign(&b, &[("f1", std::slice::from_ref(&std_op))]);
    let snap = evaluate(data(), &b, &a, &roster, &SimConfig::default()).unwrap();
    assert!(counted(&snap, &std_op));
}

#[test]
fn a_morale_skill_counts_its_owner() {
    // Restoring everyone's morale in a Dormitory counts; resting beside them
    // does not.
    let (healer, _) = holder_of("dorm_rec_all");
    let resting =
        without_skills_for_excluding(RoomType::Dormitory, 1, std::slice::from_ref(&healer))
            .remove(0);
    let b = base(vec![Room::new("d1", RoomType::Dormitory, 5)]);
    let a = assign(&b, &[("d1", &[healer.clone(), resting.clone()])]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    assert!(counted(&snap, &healer));
    assert!(!counted(&snap, &resting));

    // Resting is not working: nobody in a Dormitory is filler.
    let r = simulate(data(), &request(b, a)).unwrap();
    for o in &r.operators {
        assert!(o.worked_in.is_empty() && o.filler_in.is_empty(), "{o:?}");
    }
}

#[test]
fn a_trainee_is_never_filler() {
    // A trainee is idle, so never works: whatever their skills, they are
    // not filler. An assistant with no Training Room skills is.
    let pool = without_skills_for(RoomType::Training, 2);
    let b = base(vec![training_room(true)]);
    let a = assign(&b, &[("tr", &pool)]);
    let r = simulate(data(), &request(b, a)).unwrap();
    let report = |id: &OperatorId| r.operators.iter().find(|o| &o.id == id).unwrap();
    assert_eq!(report(&pool[0]).filler_in, vec![RoomId::new("tr")]);
    assert!(report(&pool[1]).filler_in.is_empty());
}

#[test]
fn a_named_coworker_counts() {
    // Alanna's "Here to Help!": Precious Metal productivity +15% when Warmy
    // is in the same Factory. Before Elite 2 Warmy has no Factory skill of
    // her own, yet she counts; anyone else in her place does not.
    let alanna = by_name("Alanna");
    let warmy = by_name("Warmy");
    let mut roster = maxed();
    roster.insert(warmy.clone(), RosterEntry::at(UnlockCond::BASE));
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &[alanna.clone(), warmy.clone()])]);
    let snap = evaluate(data(), &b, &a, &roster, &SimConfig::default()).unwrap();
    assert!(counted(&snap, &warmy));

    let other =
        without_skills_for_excluding(RoomType::Manufacture, 1, &[alanna.clone(), warmy.clone()])
            .remove(0);
    let a = assign(&b, &[("f1", &[alanna, other.clone()])]);
    let snap = evaluate(data(), &b, &a, &roster, &SimConfig::default()).unwrap();
    assert!(!counted(&snap, &other));
}

#[test]
fn a_resource_skill_counts_who_it_counts() {
    // Chongyue gains Worldly Plight for each Sui operator in a work area.
    // The resource is not simulated, but Nian (Sui, no Power Plant skills)
    // still counts in a Power Plant; a non-Sui operator there does not.
    let chongyue = by_name("Chongyue");
    let nian = by_name("Nian");
    let sui = |id: &OperatorId| {
        data().operators[id.as_str()]
            .powers()
            .any(|p| p.as_str() == "sui")
    };
    assert!(sui(&chongyue) && sui(&nian));
    let b = base(vec![Room::new("p1", RoomType::Power, 3)]);
    let a = assign(
        &b,
        &[
            ("cc", std::slice::from_ref(&chongyue)),
            ("p1", std::slice::from_ref(&nian)),
        ],
    );
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    assert!(counted(&snap, &nian));

    let other = without_skills_for_excluding(RoomType::Power, 40, &[chongyue.clone(), nian])
        .into_iter()
        .find(|o| !sui(o))
        .unwrap();
    let a = assign(
        &b,
        &[
            ("cc", std::slice::from_ref(&chongyue)),
            ("p1", std::slice::from_ref(&other)),
        ],
    );
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    assert!(!counted(&snap, &other));
}

#[test]
fn nobody_working_in_the_reception_room_is_filler() {
    // Its clue speed also rises with each occupant's rarity and promotion,
    // which is not modelled, so the model cannot say anyone there does
    // nothing, skills or not.
    let op = without_skills_for(RoomType::Meeting, 1);
    let b = base(vec![Room::new("r1", RoomType::Meeting, 3)]);
    let a = assign(&b, &[("r1", &op)]);
    let snap = evaluate(data(), &b, &a, &maxed(), &SimConfig::default()).unwrap();
    assert!(counted(&snap, &op[0]));
    let r = simulate(data(), &request(b, a)).unwrap();
    let report = r.operators.iter().find(|o| o.id == op[0]).unwrap();
    assert_eq!(report.worked_in, vec![RoomId::new("r1")]);
    assert!(report.filler_in.is_empty());
}

#[test]
fn a_scaler_counts_when_it_has_something_to_scale() {
    // With Automation cut down to "others' productivity becomes 0" alone, its
    // holder adds nothing of their own, yet counts for zeroing a coworker's
    // Standardization.
    let mut d = data().clone();
    for s in d.skills.values_mut() {
        if s.id.as_str().starts_with("manu_prod_spd&power") {
            let clauses = &mut s.mechanics.as_mut().unwrap().clauses;
            assert!(matches!(
                clauses[0].effect,
                Effect::ScaleOthersContribution { .. }
            ));
            clauses.truncate(1);
        }
    }
    let (auto, _) = holder_of("manu_prod_spd&power");
    let std_skill = BuffId::new("manu_prod_spd[000]");
    let std_op = d
        .operators
        .values()
        .find(|o| o.active_buffs(UnlockCond::BASE) == vec![&std_skill] && o.id != auto)
        .unwrap()
        .id
        .clone();
    let mut roster = maxed();
    roster.insert(std_op.clone(), RosterEntry::at(UnlockCond::BASE));
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &[auto.clone(), std_op.clone()])]);
    let snap = evaluate(&d, &b, &a, &roster, &SimConfig::default()).unwrap();
    assert!(counted(&snap, &auto));
    assert!(!counted(&snap, &std_op));

    // Alone, it has nothing to scale.
    let a = assign(&b, &[("f1", std::slice::from_ref(&auto))]);
    let snap = evaluate(&d, &b, &a, &roster, &SimConfig::default()).unwrap();
    assert!(!counted(&snap, &auto));
}

#[test]
fn a_report_stored_before_filler_tracking_still_reads() {
    // Stored solves keep their simulation; one saved before `worked_in` and
    // `filler_in` existed must still load, with neither known.
    let op = without_skills_for(RoomType::Manufacture, 1);
    let b = base(vec![gold_factory("f1", 3)]);
    let a = assign(&b, &[("f1", &op)]);
    let r = simulate(data(), &request(b, a)).unwrap();
    let mut v = serde_json::to_value(&r).unwrap();
    for o in v["operators"].as_array_mut().unwrap() {
        let o = o.as_object_mut().unwrap();
        o.remove("worked_in");
        o.remove("filler_in");
    }
    let old: SimResult = serde_json::from_value(v).unwrap();
    assert!(
        old.operators
            .iter()
            .all(|o| o.worked_in.is_empty() && o.filler_in.is_empty())
    );
}

// ---- Dormitory shifts --------------------------------------------------------

/// Operators with no skills for a Factory or a Dormitory.
fn plain_for_shifts(n: usize, exclude: &[OperatorId]) -> Vec<OperatorId> {
    let dorm = without_skills_for_excluding(RoomType::Dormitory, 400, exclude);
    without_skills_for_excluding(RoomType::Manufacture, 400, exclude)
        .into_iter()
        .filter(|o| dorm.contains(o))
        .take(n)
        .collect()
}

/// Workers in two Factories (three in the first) and five rested residents
/// filling every bed, with nobody on the bench.
fn full_dorm(residents: &[OperatorId], workers: &[OperatorId]) -> SimRequest {
    let b = base(vec![
        gold_factory("f1", 3),
        gold_factory("f2", 3),
        Room::new("d1", RoomType::Dormitory, 5).with_ambience(5000),
    ]);
    let (first, second) = workers.split_at(workers.len().min(3));
    let a = assign(&b, &[("f1", first), ("f2", second), ("d1", residents)]);
    let mut req = request(b, a);
    req.config.horizon_hours = 72.0;
    req.rotation = Rotation::MoodThreshold {
        swap_out: 4.0,
        swap_in: 20.0,
        bench: Vec::new(),
    };
    req
}

#[test]
fn residents_take_shifts_when_every_bed_is_taken() {
    // The five workers tire (the two in f2 about an hour before the three in
    // f1) and trade places with the five rested residents, who are the
    // second shift; rested again, they trade back. Nobody is benched and
    // nobody runs out of morale.
    let pool = plain_for_shifts(10, &[]);
    let (workers, residents) = pool.split_at(5);
    let r = simulate(data(), &full_dorm(residents, workers)).unwrap();
    assert!(
        !r.events.iter().any(|e| matches!(
            &e.kind,
            EventKind::Exhausted { .. } | EventKind::Benched { .. }
        )),
        "{:?}",
        r.events
    );
    for o in &r.operators {
        assert!(o.min_mood > 0.0, "{o:?}");
        assert!(o.hours_working > 0.0 && o.hours_resting > 0.0, "{o:?}");
    }
    for room in &r.rooms[1..3] {
        assert!(room.average_stat_pct.unwrap() > 101.9, "{room:?}");
    }
    // A trade is two moves at one instant, each into the other's slot.
    let moved: Vec<(f64, &Option<Slot>, &Slot)> = r
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Moved { from, to, .. } => Some((e.hour, from, to)),
            _ => None,
        })
        .collect();
    let ((h1, from1, to1), (h2, from2, to2)) = (moved[0], moved[1]);
    approx(h1, h2, 1e-12);
    assert!(
        from1
            .as_ref()
            .is_some_and(|s| s.room.as_str().starts_with('f'))
    );
    assert_eq!(to1.room.as_str(), "d1");
    assert_eq!(from2.as_ref(), Some(to1));
    assert_eq!(Some(to2), from1.as_ref());
}

#[test]
fn dormitory_support_stays_put() {
    // Indigo restores morale to another resident, so she stays even while
    // everyone is rested; Reed only speeds his own recovery, so he takes a
    // shift like anyone else. Four workers tire, the one in f2 first and then
    // the three in f1: one for each resident but Indigo.
    let indigo = by_name("Indigo");
    let reed = by_name("Reed");
    let pool = plain_for_shifts(7, &[indigo.clone(), reed.clone()]);
    let (workers, rest) = pool.split_at(4);
    // Indigo last: were she eligible, ties would pick her first.
    let residents = [
        reed.clone(),
        rest[0].clone(),
        rest[1].clone(),
        rest[2].clone(),
        indigo.clone(),
    ];
    let req = full_dorm(&residents, workers);

    let snap = evaluate(data(), &req.base, &req.assignment, &maxed(), &req.config).unwrap();
    let rate = |id: &OperatorId| {
        snap.mood
            .iter()
            .find(|m| &m.operator == id)
            .unwrap()
            .clone()
    };
    assert!(rate(&indigo).supports);
    assert!(!rate(&reed).supports && rate(&reed).counted);

    let r = simulate(data(), &req).unwrap();
    let report = |id: &OperatorId| r.operators.iter().find(|o| &o.id == id).unwrap();
    approx(report(&indigo).hours_working, 0.0, 1e-12);
    assert!(report(&reed).hours_working > 0.0);
    assert!(
        !r.events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Exhausted { .. }))
    );
}

/// Morale state for driving a policy by hand.
fn at_mood(mood: f64) -> OpState {
    OpState {
        mood,
        max_mood: 24.0,
        initial_mood: mood,
        min_mood: mood,
        hours_in_room: 0.0,
        hours_working: 0.0,
        hours_resting: 0.0,
        hours_exhausted: 0.0,
        hours_idle: 0.0,
        hours_benched: 0.0,
        worked_in: Default::default(),
        counted_in: Default::default(),
        supported_in: Default::default(),
    }
}

#[test]
fn a_tired_substitute_does_not_take_over_the_slot() {
    // A substitute who tires before the worker they cover has rested goes to
    // rest with no claim on the slot: once rested, they stay out of the
    // worker's way.
    let ops = plain_for_shifts(6, &[]);
    let (worker, residents) = (&ops[0], &ops[1..]);
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("d1", RoomType::Dormitory, 5),
    ]);
    let mut a = assign(
        &b,
        &[("f1", std::slice::from_ref(worker)), ("d1", residents)],
    );
    let work = Slot::new("f1", 0);
    let mut moods: Vec<(OperatorId, f64)> = vec![(worker.clone(), 4.0)];
    moods.extend(
        residents
            .iter()
            .cloned()
            .zip([24.0, 21.0, 21.0, 21.0, 21.0]),
    );
    let mut policy = MoodThreshold::new(4.0, 20.0, Vec::new());
    let mut step = |a: &mut Assignment, moods: &[(OperatorId, f64)]| {
        let states = moods
            .iter()
            .map(|(id, m)| (id.clone(), at_mood(*m)))
            .collect();
        let view = SimView {
            hour: 0.0,
            data: data(),
            base: &b,
            assignment: a,
            states: &states,
        };
        let actions = policy.next_actions(&view);
        for action in &actions {
            if let SimAction::Swap { a: x, b: y } = action {
                a.swap(x, y).unwrap();
            }
        }
        actions
    };
    let who = |a: &Assignment, slot: &Slot| {
        a.slots(slot.room.as_str()).unwrap()[usize::from(slot.index)].clone()
    };

    // The worker trades places with the best-rested resident.
    step(&mut a, &moods);
    let cover = who(&a, &work).unwrap();
    assert_eq!(cover, residents[0]);
    // The cover tires before the worker has rested: another resident steps in.
    for (id, m) in &mut moods {
        *m = if id == worker {
            10.0
        } else if *id == cover {
            4.0
        } else {
            21.0
        };
    }
    step(&mut a, &moods);
    let second = who(&a, &work).unwrap();
    assert!(second != cover && second != *worker);
    // The worker, rested, goes home.
    for (id, m) in &mut moods {
        *m = if id == worker {
            20.0
        } else if *id == cover {
            10.0
        } else {
            15.0
        };
    }
    step(&mut a, &moods);
    assert_eq!(who(&a, &work).as_ref(), Some(worker));
    // The first cover, rested too, leaves the worker be.
    for (id, m) in &mut moods {
        *m = if id == worker {
            18.0
        } else if *id == cover {
            24.0
        } else {
            15.0
        };
    }
    assert!(step(&mut a, &moods).is_empty());
    assert_eq!(who(&a, &work).as_ref(), Some(worker));
}

#[test]
fn residents_cover_while_beds_are_free() {
    // With beds to spare, a tired worker takes a free one and a rested
    // resident takes the slot: the residents are the second shift whether or
    // not the Dormitory is full.
    let pool = plain_for_shifts(6, &[]);
    let (workers, residents) = pool.split_at(3);
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("d1", RoomType::Dormitory, 5).with_ambience(5000),
    ]);
    let a = assign(&b, &[("f1", workers), ("d1", residents)]);
    let mut req = request(b, a);
    req.config.horizon_hours = 72.0;
    req.rotation = Rotation::MoodThreshold {
        swap_out: 4.0,
        swap_in: 20.0,
        bench: Vec::new(),
    };
    let r = simulate(data(), &req).unwrap();
    assert!(
        !r.events.iter().any(|e| matches!(
            &e.kind,
            EventKind::Exhausted { .. } | EventKind::Benched { .. }
        )),
        "{:?}",
        r.events
    );
    for o in &r.operators {
        assert!(o.hours_working > 0.0 && o.hours_resting > 0.0, "{o:?}");
    }
    assert!(r.rooms[1].average_stat_pct.unwrap() > 102.9);
}

#[test]
fn the_weaker_of_two_healers_takes_shifts() {
    // Shining and Myrrh each restore morale to one other resident, but only
    // the stronger effect of the type applies: Myrrh would never reach
    // anyone, so she is the second shift. She sits first, where the stand-in
    // newcomer must not make her look like support.
    let (shining, myrrh) = (by_name("Shining"), by_name("Myrrh"));
    let pool = plain_for_shifts(7, &[shining.clone(), myrrh.clone()]);
    let (workers, rest) = pool.split_at(4);
    let residents = [
        myrrh.clone(),
        shining.clone(),
        rest[0].clone(),
        rest[1].clone(),
        rest[2].clone(),
    ];
    let req = full_dorm(&residents, workers);
    let snap = evaluate(data(), &req.base, &req.assignment, &maxed(), &req.config).unwrap();
    let rate = |id: &OperatorId| {
        snap.mood
            .iter()
            .find(|m| &m.operator == id)
            .unwrap()
            .clone()
    };
    assert!(rate(&shining).supports && !rate(&myrrh).supports);

    let r = simulate(data(), &req).unwrap();
    let report = |id: &OperatorId| r.operators.iter().find(|o| &o.id == id).unwrap();
    approx(report(&shining).hours_working, 0.0, 1e-12);
    assert!(report(&myrrh).hours_working > 0.0);
}

#[test]
fn a_resident_another_skill_needs_stays() {
    // Sankta Miksaparato's clue bonus needs Fiammetta in a Dormitory, so
    // Fiammetta stays put, though she raises nobody's morale.
    let fiammetta = by_name("Fiammetta");
    let mixer = by_name("Sankta Miksaparato");
    let pool = plain_for_shifts(8, &[fiammetta.clone(), mixer.clone()]);
    let (workers, rest) = pool.split_at(4);
    let residents = [
        rest[0].clone(),
        rest[1].clone(),
        rest[2].clone(),
        rest[3].clone(),
        fiammetta.clone(),
    ];
    let mut req = full_dorm(&residents, workers);
    req.base.rooms.push(Room::new("r1", RoomType::Meeting, 3));
    req.assignment = assign(
        &req.base,
        &[
            ("f1", &workers[..3]),
            ("f2", &workers[3..]),
            ("d1", &residents),
            ("r1", std::slice::from_ref(&mixer)),
        ],
    );
    let snap = evaluate(data(), &req.base, &req.assignment, &maxed(), &req.config).unwrap();
    let rate = snap.mood.iter().find(|m| m.operator == fiammetta).unwrap();
    assert!(rate.supports);
    let r = simulate(data(), &req).unwrap();
    let report = r.operators.iter().find(|o| o.id == fiammetta).unwrap();
    approx(report.hours_working, 0.0, 1e-12);
}

#[test]
fn the_policy_passes_over_dormitory_support() {
    // Even as the best-rested resident, support is not a substitute.
    let ops = plain_for_shifts(3, &[]);
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("d1", RoomType::Dormitory, 5),
    ]);
    let a = assign(&b, &[("f1", &ops[..1]), ("d1", &ops[1..])]);
    let mut support = at_mood(24.0);
    support.supported_in.insert(RoomId::new("d1"));
    let states = [
        (ops[0].clone(), at_mood(4.0)),
        (ops[1].clone(), support),
        (ops[2].clone(), at_mood(21.0)),
    ]
    .into_iter()
    .collect();
    let view = SimView {
        hour: 0.0,
        data: data(),
        base: &b,
        assignment: &a,
        states: &states,
    };
    let actions = MoodThreshold::new(4.0, 20.0, Vec::new()).next_actions(&view);
    let work = Slot::new("f1", 0);
    let moved = |who: &OperatorId| {
        actions.iter().any(
            |act| matches!(act, SimAction::Move { operator, to } if operator == who && *to == work),
        )
    };
    assert!(moved(&ops[2]), "{actions:?}");
    assert!(!moved(&ops[1]), "{actions:?}");
}

#[test]
fn a_swap_restarts_both_operators_time_in_room() {
    // Ramping skills count hours since entering the room, so a trade starts
    // the clock again for both.
    struct SwapOnce {
        done: bool,
        after: Option<Vec<f64>>,
    }
    impl RotationPolicy for SwapOnce {
        fn next_actions(&mut self, v: &SimView<'_>) -> Vec<SimAction> {
            let (a, b) = (Slot::new("f1", 0), Slot::new("d1", 0));
            if !self.done && v.hour >= 6.0 {
                self.done = true;
                return vec![SimAction::Swap { a, b }];
            }
            if self.done && self.after.is_none() {
                let at = |s: &Slot| {
                    v.assignment.slots(s.room.as_str()).unwrap()[usize::from(s.index)]
                        .clone()
                        .unwrap()
                };
                self.after = Some(
                    [a, b]
                        .iter()
                        .map(|s| v.states[&at(s)].hours_in_room)
                        .collect(),
                );
            }
            Vec::new()
        }
    }
    let pool = plain_for_shifts(2, &[]);
    let b = base(vec![
        gold_factory("f1", 3),
        Room::new("d1", RoomType::Dormitory, 5),
    ]);
    let a = assign(&b, &[("f1", &pool[..1]), ("d1", &pool[1..])]);
    let mut policy = SwapOnce {
        done: false,
        after: None,
    };
    simulate_with(data(), &b, &a, &maxed(), &SimConfig::default(), &mut policy).unwrap();
    for hours in policy.after.unwrap() {
        assert!(hours < 0.5, "{hours}");
    }
}
