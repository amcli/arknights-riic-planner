//! Layer 5 tests against the pinned snapshot.
//!
//! The exhaustive search is the ground truth; the annealer is checked
//! against it on a space small enough to enumerate, and for its contracts
//! (determinism, locks, validity, never worse than the start, time budget)
//! on a full base.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use ak_data::{Loaded, Strictness, load_default};
use ak_domain::*;
use ak_eval::*;
use ak_solver::*;

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

/// A Control Center, the given rooms, then enough level-3 Power Plants to
/// power them.
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

/// Operators whose maxed Factory skills are nothing but unconditional flat
/// productivity for any product in their own room: `(id, total)`, largest
/// first, ties by id.
fn plain_factory_ops(n: usize) -> Vec<(OperatorId, f64)> {
    let data = data();
    let mut out = Vec::new();
    for op in data.operators.values() {
        let mut total = 0.0;
        let mut plain = true;
        let mut any = false;
        for b in op.max_buffs() {
            let s = &data.skills[b.as_str()];
            if s.room_type != RoomType::Manufacture {
                continue;
            }
            let Some(m) = &s.mechanics else {
                plain = false;
                break;
            };
            for c in &m.clauses {
                match (&c.when, &c.effect) {
                    (
                        Predicate::Always,
                        Effect::Productivity {
                            amount: Amount::Flat { value },
                            product: None,
                            scope: Scope::ThisRoom,
                        },
                    ) => {
                        total += value;
                        any = true;
                    }
                    _ => plain = false,
                }
            }
        }
        if plain && any && total > 0.0 {
            out.push((op.id.clone(), total));
        }
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.truncate(n);
    out
}

/// Operators none of whose maxed skills target `kind`.
fn without_skills_for(kind: RoomType, n: usize, exclude: &[OperatorId]) -> Vec<OperatorId> {
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

fn request(
    base: BaseConfig,
    pool: Vec<OperatorId>,
    initial: Option<Assignment>,
    locked: Vec<Slot>,
    solver: SolverConfig,
) -> SolveRequest {
    SolveRequest {
        base,
        roster: Roster::everyone_maxed(data()),
        pool: Some(pool),
        initial,
        locked,
        config: SimConfig::default(),
        rotation: Rotation::None,
        objective: Objective::default(),
        solver,
    }
}

fn occupants(a: &Assignment, room: &str) -> BTreeSet<OperatorId> {
    a.occupants(room).cloned().collect()
}

fn example_request() -> SimRequest {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/requests/243-base.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn example_solve(solver: SolverConfig) -> SolveRequest {
    let sim = example_request();
    SolveRequest {
        base: sim.base,
        roster: sim.roster,
        pool: None,
        initial: Some(sim.assignment),
        locked: Vec::new(),
        config: sim.config,
        rotation: sim.rotation,
        objective: Objective::default(),
        solver,
    }
}

#[track_caller]
fn approx(actual: f64, expected: f64, tol: f64) {
    assert!(
        (actual - expected).abs() <= tol,
        "expected {expected}, got {actual}"
    );
}

/// One EXP Factory whose crew is the only thing to decide; the Control
/// Center and Power Plant slots are locked empty.
fn crew_problem() -> (SolveRequest, Vec<OperatorId>) {
    // Only these three plain operators enter the pool, alongside three with
    // no Factory skills, so they are the unique optimum whatever ties exist
    // outside the pool.
    let plain = plain_factory_ops(3);
    assert_eq!(plain.len(), 3, "{plain:?}");
    let best: Vec<OperatorId> = plain.iter().map(|(id, _)| id.clone()).collect();
    let dull = without_skills_for(RoomType::Manufacture, 3, &best);
    let b = base(vec![
        Room::new("f1", RoomType::Manufacture, 3).with_formula("1"),
    ]);
    let mut locked: Vec<Slot> = (0..5).map(|i| Slot::new("cc", i)).collect();
    for r in b.rooms_of(RoomType::Power) {
        locked.push(Slot::new(r.id.clone(), 0));
    }
    let mut pool = best.clone();
    pool.extend(dull);
    let req = request(b, pool, None, locked, SolverConfig::default());
    (req, best)
}

#[test]
fn exhaustive_picks_the_best_factory_crew() {
    let (mut req, best) = crew_problem();
    req.solver.strategy = Strategy::Exhaustive;
    let r = solve(data(), &req).unwrap();
    assert_eq!(r.strategy, Strategy::Exhaustive);
    assert_eq!(r.space.variable_slots, 3);
    assert_eq!(r.space.pool, 6);
    // Three slots, six candidates, order within the room irrelevant:
    // 1 + 6 + 15 + 20 assignments, plus the starting one.
    approx(r.space.estimated_size, 42.0, 1e-9);
    assert_eq!(r.evaluations, 43);

    let top = &r.candidates[0];
    assert_eq!(occupants(&top.assignment, "f1"), best.into_iter().collect());
    assert!(top.score > r.initial.score);
    assert!(top.simulated);
    assert!(r.best_simulation.is_some());
    assert!(r.candidates.windows(2).all(|w| w[0].score >= w[1].score));
    for c in &r.candidates {
        c.assignment.check(&req.base, data()).unwrap();
        assert!(c.assignment.occupants("cc").count() == 0);
    }
    // Nothing binds here (no trading, continuous collection, no
    // exhaustion), so the proxy and the simulator agree closely.
    let rel = (top.inner_score - top.score).abs() / top.score;
    assert!(
        rel < 0.02,
        "proxy {} vs simulated {}",
        top.inner_score,
        top.score
    );
}

#[test]
fn annealing_finds_the_exhaustive_optimum() {
    let (mut req, _) = crew_problem();
    req.solver.strategy = Strategy::Exhaustive;
    let truth = solve(data(), &req).unwrap();
    req.solver = SolverConfig {
        strategy: Strategy::Annealing,
        iterations: 200,
        restarts: 2,
        top_k: 3,
        ..SolverConfig::default()
    };
    let r = solve(data(), &req).unwrap();
    assert_eq!(r.strategy, Strategy::Annealing);
    approx(r.candidates[0].score, truth.candidates[0].score, 1e-6);
    assert_eq!(
        occupants(&r.candidates[0].assignment, "f1"),
        occupants(&truth.candidates[0].assignment, "f1")
    );
}

#[test]
fn auto_strategy_switches_on_size() {
    let (mut req, _) = crew_problem();
    req.solver.strategy = Strategy::Auto;
    assert_eq!(solve(data(), &req).unwrap().strategy, Strategy::Exhaustive);
    req.solver.exhaustive_limit = 10.0;
    req.solver.iterations = 50;
    req.solver.restarts = 1;
    assert_eq!(solve(data(), &req).unwrap().strategy, Strategy::Annealing);
    req.solver.strategy = Strategy::Exhaustive;
    assert!(matches!(
        solve(data(), &req),
        Err(SolveError::TooLarge { .. })
    ));
}

#[test]
fn same_seed_same_answer() {
    let pool: Vec<OperatorId> = data().operators.keys().take(12).cloned().collect();
    let b = base(vec![
        Room::new("f1", RoomType::Manufacture, 3).with_formula("4"),
        Room::new("f2", RoomType::Manufacture, 3).with_formula("1"),
        Room::new("t1", RoomType::Trading, 3),
        Room::new("d1", RoomType::Dormitory, 5).with_ambience(3000),
    ]);
    let solver = SolverConfig {
        strategy: Strategy::Annealing,
        iterations: 150,
        restarts: 2,
        top_k: 4,
        seed: 7,
        ..SolverConfig::default()
    };
    let req = request(b, pool, None, Vec::new(), solver);
    let a = solve(data(), &req).unwrap();
    let b = solve(data(), &req).unwrap();
    assert_eq!(a.evaluations, b.evaluations);
    assert_eq!(
        serde_json::to_string(&a.candidates).unwrap(),
        serde_json::to_string(&b.candidates).unwrap()
    );
    let mut other = req.clone();
    other.solver.seed = 8;
    let c = solve(data(), &other).unwrap();
    // A different seed explores differently (evaluation counts can match,
    // the visited candidates should not).
    assert!(
        serde_json::to_string(&a.candidates).unwrap()
            != serde_json::to_string(&c.candidates).unwrap()
            || a.evaluations != c.evaluations
    );
}

#[test]
fn respects_locks_and_pool() {
    let fixed = by_name("Amiya");
    let pool: Vec<OperatorId> = data()
        .operators
        .keys()
        .filter(|id| **id != fixed)
        .take(8)
        .cloned()
        .collect();
    let b = base(vec![
        Room::new("f1", RoomType::Manufacture, 3).with_formula("4"),
        Room::new("t1", RoomType::Trading, 3),
        Room::new("d1", RoomType::Dormitory, 5),
    ]);
    let mut initial = Assignment::empty(&b, data());
    initial.place(&Slot::new("cc", 0), fixed.clone()).unwrap();
    let solver = SolverConfig {
        strategy: Strategy::Annealing,
        iterations: 200,
        restarts: 1,
        top_k: 5,
        ..SolverConfig::default()
    };
    let req = request(
        b,
        pool.clone(),
        Some(initial),
        vec![Slot::new("cc", 0)],
        solver,
    );
    let r = solve(data(), &req).unwrap();
    assert_eq!(r.space.locked_slots, 1);
    assert_eq!(r.space.pool, 8);
    let allowed: BTreeSet<&OperatorId> = pool.iter().chain(std::iter::once(&fixed)).collect();
    for c in r.candidates.iter().chain(std::iter::once(&r.initial)) {
        c.assignment.check(&req.base, data()).unwrap();
        assert_eq!(c.assignment.slots("cc").unwrap()[0].as_ref(), Some(&fixed));
        for op in c.assignment.operators() {
            assert!(allowed.contains(op), "{op} is not in the pool");
        }
    }
}

#[test]
fn never_worse_than_the_start_and_stops_on_budget() {
    let req = example_solve(SolverConfig {
        strategy: Strategy::Annealing,
        iterations: 1_000_000,
        restarts: 3,
        top_k: 3,
        time_budget_ms: Some(1500),
        ..SolverConfig::default()
    });
    let started = std::time::Instant::now();
    let r = solve(data(), &req).unwrap();
    assert!(started.elapsed().as_secs() < 30, "{} ms", r.elapsed_ms);
    assert!(r.evaluations < 1_000_000);
    assert_eq!(r.strategy, Strategy::Annealing);
    assert!(r.space.estimated_size > 1e6);
    assert!(
        r.candidates[0].score >= r.initial.score - 1e-6,
        "best {} < initial {}",
        r.candidates[0].score,
        r.initial.score
    );
    let initial = req.initial.as_ref().unwrap();
    assert!(r.candidates.iter().any(|c| &c.assignment == initial));
    assert!(r.candidates.len() <= 4);
    for c in &r.candidates {
        c.assignment.check(&req.base, data()).unwrap();
        assert!(c.simulated);
    }
    // The Training Room trainee slot is never touched.
    for c in &r.candidates {
        assert_eq!(c.assignment.slots("training").unwrap()[1], None);
    }
}

#[test]
fn proxy_tracks_the_simulator_on_the_example_base() {
    let req = example_solve(SolverConfig::default());
    let initial = req.initial.as_ref().unwrap();
    let pool: Vec<OperatorId> = req.roster.entries.keys().cloned().collect();
    let mut steady = SteadyState::new(
        data(),
        &req.base,
        &req.roster,
        &req.config,
        &req.objective,
        &req.rotation,
    );
    let mut sim = Simulation::new(
        data(),
        &req.base,
        &req.roster,
        &req.config,
        &req.rotation,
        &pool,
        &req.objective,
    );
    let p = steady.score(initial).unwrap();
    let s = sim.score(initial).unwrap();
    assert!(s.value > 0.0);
    let rel = (p.value - s.value).abs() / s.value;
    assert!(rel < 0.05, "proxy {} vs simulated {}", p.value, s.value);
    // Both see the same gold shortage.
    assert!(p.breakdown.gold_net.abs() < 0.5, "{}", p.breakdown.gold_net);
    assert!(s.breakdown.gold_net.abs() < 0.5, "{}", s.breakdown.gold_net);
}

#[test]
fn rotation_prices_morale_relief() {
    // The example base keeps Team Rainbow and Amiya in the Control Center,
    // which takes 0.25/h off every worker's drain. Within 24 h without
    // rotation nobody exhausts, so the proxy scores the base the same with
    // an empty Control Center. Under a shift policy the relief is time the
    // crew no longer spends resting, and the proxy must prefer it.
    let sim = example_request();
    let with_crew = sim.assignment.clone();
    let mut without_crew = with_crew.clone();
    for i in 0..5 {
        without_crew.remove(&Slot::new("cc", i));
    }
    let objective = Objective::default();
    let score = |rotation: &Rotation, horizon: f64, a: &Assignment| {
        let config = SimConfig {
            horizon_hours: horizon,
            ..sim.config.clone()
        };
        let mut steady = SteadyState::new(
            data(),
            &sim.base,
            &sim.roster,
            &config,
            &objective,
            rotation,
        );
        steady.score(a).unwrap()
    };
    let none = Rotation::None;
    let shifts = Rotation::MoodThreshold {
        swap_out: 4.0,
        swap_in: 20.0,
        bench: Vec::new(),
    };

    let a = score(&none, 24.0, &with_crew);
    let b = score(&none, 24.0, &without_crew);
    approx(a.value, b.value, 1e-6 * a.value.abs());
    assert_eq!(a.breakdown.exhausted_hours, 0.0);

    let a = score(&shifts, 72.0, &with_crew);
    let b = score(&shifts, 72.0, &without_crew);
    assert!(
        a.value > b.value,
        "with crew {} vs without {}",
        a.value,
        b.value
    );
    assert_eq!(a.breakdown.exhausted_hours, 0.0);

    // Without rotation over 72 h the workers exhaust, so the crew matters
    // there too, and exhaustion is reported.
    let a = score(&none, 72.0, &with_crew);
    let b = score(&none, 72.0, &without_crew);
    assert!(
        a.value > b.value,
        "with crew {} vs without {}",
        a.value,
        b.value
    );
    assert!(b.breakdown.exhausted_hours > a.breakdown.exhausted_hours);
}

#[test]
fn finalists_have_distinct_scores() {
    let (mut req, _) = crew_problem();
    req.solver.strategy = Strategy::Exhaustive;
    let r = solve(data(), &req).unwrap();
    for pair in r.candidates.windows(2) {
        assert!(pair[0].inner_score > pair[1].inner_score + 1e-9, "{pair:?}");
    }
}
