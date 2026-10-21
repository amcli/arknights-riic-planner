//! Shift policies: who goes to rest and who takes over.
//!
//! A [`RotationPolicy`] is asked after every tick and answers with
//! [`SimAction`]s, which the loop applies through [`Assignment`]'s checked
//! mutators. Policies never see or touch simulator internals beyond the
//! read-only [`SimView`].

use std::collections::{BTreeMap, BTreeSet};

use ak_domain::{Assignment, BaseConfig, GameData, OperatorId, RoomType, Slot};
use serde::{Deserialize, Serialize};

use crate::sim::OpState;

/// Read-only view of the run handed to a policy.
#[derive(Debug, Clone, Copy)]
pub struct SimView<'a> {
    /// Simulated hours since the start.
    pub hour: f64,
    /// Game data.
    pub data: &'a GameData,
    /// The base.
    pub base: &'a BaseConfig,
    /// Who is where right now.
    pub assignment: &'a Assignment,
    /// Per-operator state, including benched operators.
    pub states: &'a BTreeMap<OperatorId, OpState>,
}

impl SimView<'_> {
    /// Current morale of an operator the run tracks.
    pub fn mood(&self, op: &str) -> Option<f64> {
        self.states.get(op).map(|s| s.mood)
    }

    /// True at maximum morale.
    pub fn is_full(&self, op: &str) -> bool {
        self.states
            .get(op)
            .is_some_and(|s| s.mood >= s.max_mood - 1e-9)
    }

    /// Kind of the room a slot belongs to.
    pub fn kind_of(&self, slot: &Slot) -> Option<RoomType> {
        self.base.room(slot.room.as_str()).map(|r| r.kind)
    }

    /// First empty Dormitory slot in a planned assignment.
    pub fn vacant_dorm(&self, plan: &Assignment) -> Option<Slot> {
        plan.vacancies().find(|s| {
            self.base
                .room(s.room.as_str())
                .is_some_and(|r| r.kind == RoomType::Dormitory)
        })
    }
}

/// A change a policy asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimAction {
    /// Station an operator at a slot (from wherever they are, or from the
    /// bench). Whoever was there is benched.
    Move { operator: OperatorId, to: Slot },
    /// Take an operator off the base.
    Bench { operator: OperatorId },
    /// Exchange the occupants of two slots.
    Swap { a: Slot, b: Slot },
}

/// Decides shifts.
pub trait RotationPolicy {
    /// Operators the policy may bring in who are not in the initial
    /// assignment; the run tracks their morale from the start.
    fn extra_operators(&self) -> Vec<OperatorId> {
        Vec::new()
    }

    /// Actions to apply now. Called after every tick.
    fn next_actions(&mut self, view: &SimView<'_>) -> Vec<SimAction>;
}

/// Nobody ever moves. The naive baseline.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRotation;

impl RotationPolicy for NoRotation {
    fn next_actions(&mut self, _view: &SimView<'_>) -> Vec<SimAction> {
        Vec::new()
    }
}

/// Two-shift rotation on morale thresholds.
///
/// A working operator whose morale drops to `swap_out` goes to rest, and
/// the best-rested substitute at or above `swap_in` takes their slot.
/// Substitutes are `bench` operators, off the base or resting in a
/// Dormitory, and anyone else resting in a Dormitory except its support:
/// those whose skills raise others' morale there, or whom another
/// operator's skill needs there (see [`crate::MoodRate::supports`]). The
/// rest of the residents are the second shift. The tired operator takes a
/// vacant Dormitory slot, or, when every bed is taken, trades places with a
/// substitute resting in one; with neither, they stay and exhaust. When the
/// original occupant recovers to `swap_in` they trade places with whoever
/// covers their slot.
#[derive(Debug, Clone)]
pub struct MoodThreshold {
    /// Move out at or below this morale.
    pub swap_out: f64,
    /// Return (or become eligible) at or above this morale.
    pub swap_in: f64,
    /// Substitutes, not in the initial assignment.
    pub bench: Vec<OperatorId>,
    /// Resting operators and the slot they return to.
    home: BTreeMap<OperatorId, Slot>,
    /// Operators covering someone else's slot, who have no slot to return
    /// to when they rest.
    covering: BTreeSet<OperatorId>,
}

impl MoodThreshold {
    /// Constructs the policy.
    pub fn new(swap_out: f64, swap_in: f64, bench: Vec<OperatorId>) -> Self {
        MoodThreshold {
            swap_out,
            swap_in,
            bench,
            home: BTreeMap::new(),
            covering: BTreeSet::new(),
        }
    }

    /// The best-rested operator who can take over a slot, with the
    /// Dormitory slot they rest in (`None` when off the base). With
    /// `in_dorm`, only those resting in a Dormitory, who can trade places
    /// with a tired worker when no bed is free. Ties go to those in a
    /// Dormitory, since taking one of them frees a bed.
    fn substitute(
        &self,
        v: &SimView<'_>,
        plan: &Assignment,
        in_dorm: bool,
    ) -> Option<(OperatorId, Option<Slot>)> {
        let dorm = |slot: &Slot| v.kind_of(slot) == Some(RoomType::Dormitory);
        // Where everyone is, once: the bench can run to hundreds.
        let at: BTreeMap<&OperatorId, Slot> = plan.iter().map(|(slot, op)| (op, slot)).collect();
        let mut pool: Vec<(OperatorId, Option<Slot>)> = Vec::new();
        for b in &self.bench {
            match at.get(b) {
                None if !in_dorm => pool.push((b.clone(), None)),
                Some(slot) if dorm(slot) => pool.push((b.clone(), Some(slot.clone()))),
                _ => {}
            }
        }
        for (slot, op) in plan.iter() {
            let supports = v
                .states
                .get(op)
                .is_none_or(|st| st.supported_in.contains(&slot.room));
            if dorm(&slot) && !supports && !self.bench.contains(op) && !self.home.contains_key(op) {
                pool.push((op.clone(), Some(slot)));
            }
        }
        let mood = |op: &OperatorId| v.mood(op.as_str()).unwrap_or(0.0);
        pool.into_iter()
            .filter(|(op, _)| mood(op) >= self.swap_in)
            .max_by(|a, b| {
                mood(&a.0)
                    .total_cmp(&mood(&b.0))
                    .then(a.1.is_some().cmp(&b.1.is_some()))
            })
    }
}

impl RotationPolicy for MoodThreshold {
    fn extra_operators(&self) -> Vec<OperatorId> {
        self.bench.clone()
    }

    fn next_actions(&mut self, v: &SimView<'_>) -> Vec<SimAction> {
        let mut plan = v.assignment.clone();
        let mut actions = Vec::new();

        // 1. Rested operators go home, trading places with whoever covers
        //    their slot.
        let homes: Vec<(OperatorId, Slot)> = self
            .home
            .iter()
            .map(|(o, s)| (o.clone(), s.clone()))
            .collect();
        for (op, home) in homes {
            let rested =
                v.mood(op.as_str()).is_some_and(|m| m >= self.swap_in) || v.is_full(op.as_str());
            if !rested {
                continue;
            }
            let at = plan.locate(op.as_str());
            if at.as_ref() == Some(&home) {
                self.home.remove(&op);
                continue;
            }
            let occupant = plan
                .slots(home.room.as_str())
                .and_then(|s| s.get(usize::from(home.index)))
                .cloned()
                .flatten();
            match (occupant, at) {
                (Some(sub), Some(bed)) => {
                    if plan.swap(&home, &bed).is_ok() {
                        actions.push(SimAction::Swap { a: home, b: bed });
                        self.covering.remove(&sub);
                        self.home.remove(&op);
                    }
                }
                (occupant, _) => {
                    if let Some(sub) = occupant {
                        self.covering.remove(&sub);
                        match v.vacant_dorm(&plan) {
                            Some(dorm) => {
                                if plan.move_to(&sub, &dorm).is_ok() {
                                    actions.push(SimAction::Move {
                                        operator: sub,
                                        to: dorm,
                                    });
                                }
                            }
                            None => {
                                if let Some(at) = plan.locate(sub.as_str()) {
                                    plan.remove(&at);
                                }
                                actions.push(SimAction::Bench { operator: sub });
                            }
                        }
                    }
                    if plan.move_to(&op, &home).is_ok() {
                        actions.push(SimAction::Move {
                            operator: op.clone(),
                            to: home,
                        });
                        self.home.remove(&op);
                    }
                }
            }
        }

        // 2. Tired workers rest and a substitute takes over: the worker takes
        //    a vacant bed, or trades places with a substitute resting in one.
        let tired: Vec<(Slot, OperatorId)> = plan
            .iter()
            .filter(|(slot, op)| {
                v.kind_of(slot).is_some_and(RoomType::is_work_area)
                    && v.mood(op.as_str()).is_some_and(|m| m <= self.swap_out)
            })
            .map(|(slot, op)| (slot, op.clone()))
            .collect();
        for (slot, op) in tired {
            let bed = v.vacant_dorm(&plan);
            let sub = self.substitute(v, &plan, bed.is_none());
            match (bed, sub) {
                (Some(bed), sub) => {
                    if plan.move_to(&op, &bed).is_err() {
                        continue;
                    }
                    actions.push(SimAction::Move {
                        operator: op.clone(),
                        to: bed,
                    });
                    if let Some((sub, _)) = sub
                        && plan.move_to(&sub, &slot).is_ok()
                    {
                        actions.push(SimAction::Move {
                            operator: sub.clone(),
                            to: slot.clone(),
                        });
                        self.covering.insert(sub);
                    }
                }
                (None, Some((sub, Some(sub_bed)))) => {
                    if plan.swap(&slot, &sub_bed).is_err() {
                        continue;
                    }
                    actions.push(SimAction::Swap {
                        a: slot.clone(),
                        b: sub_bed,
                    });
                    self.covering.insert(sub);
                }
                // Nowhere to rest, and nothing changed, so the same holds for
                // every tired worker after this one.
                _ => break,
            }
            // A substitute who tires has no slot of their own to return to.
            if !self.covering.remove(&op) && !self.bench.contains(&op) {
                self.home.insert(op, slot);
            }
        }
        actions
    }
}

/// Serialisable choice of policy.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rotation {
    /// [`NoRotation`].
    #[default]
    None,
    /// [`MoodThreshold`].
    MoodThreshold {
        swap_out: f64,
        swap_in: f64,
        #[serde(default)]
        bench: Vec<OperatorId>,
    },
}

impl Rotation {
    /// A fresh policy instance.
    pub fn policy(&self) -> Box<dyn RotationPolicy> {
        match self {
            Rotation::None => Box::new(NoRotation),
            Rotation::MoodThreshold {
                swap_out,
                swap_in,
                bench,
            } => Box::new(MoodThreshold::new(*swap_out, *swap_in, bench.clone())),
        }
    }
}
