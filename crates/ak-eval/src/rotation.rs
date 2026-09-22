//! Shift policies: who goes to rest and who takes over.
//!
//! A [`RotationPolicy`] is asked after every tick and answers with
//! [`SimAction`]s, which the loop applies through [`Assignment`]'s checked
//! mutators. Policies never see or touch simulator internals beyond the
//! read-only [`SimView`].

use std::collections::BTreeMap;

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
/// A working operator whose morale drops to `swap_out` moves to a vacant
/// Dormitory slot (if none is free, they stay and exhaust). Their slot is
/// filled by the best-rested `bench` operator at or above `swap_in`, if
/// any. When the original occupant recovers to `swap_in` they return, and
/// the substitute goes back to a Dormitory or the bench.
#[derive(Debug, Clone)]
pub struct MoodThreshold {
    /// Move out at or below this morale.
    pub swap_out: f64,
    /// Return (or become eligible) at or above this morale.
    pub swap_in: f64,
    /// Substitutes, not in the initial assignment.
    pub bench: Vec<OperatorId>,
    home: BTreeMap<OperatorId, Slot>,
}

impl MoodThreshold {
    /// Constructs the policy.
    pub fn new(swap_out: f64, swap_in: f64, bench: Vec<OperatorId>) -> Self {
        MoodThreshold {
            swap_out,
            swap_in,
            bench,
            home: BTreeMap::new(),
        }
    }
}

impl RotationPolicy for MoodThreshold {
    fn extra_operators(&self) -> Vec<OperatorId> {
        self.bench.clone()
    }

    fn next_actions(&mut self, v: &SimView<'_>) -> Vec<SimAction> {
        let mut plan = v.assignment.clone();
        let mut actions = Vec::new();

        // 1. Rested primaries go home; substitutes step aside.
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
            if plan.locate(op.as_str()).as_ref() == Some(&home) {
                self.home.remove(&op);
                continue;
            }
            let occupant = plan
                .slots(home.room.as_str())
                .and_then(|s| s.get(usize::from(home.index)))
                .cloned()
                .flatten();
            if let Some(sub) = occupant {
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

        // 2. Tired workers rest; the bench fills in.
        let tired: Vec<(Slot, OperatorId)> = plan
            .iter()
            .filter(|(slot, op)| {
                v.kind_of(slot).is_some_and(RoomType::is_work_area)
                    && v.mood(op.as_str()).is_some_and(|m| m <= self.swap_out)
            })
            .map(|(slot, op)| (slot, op.clone()))
            .collect();
        for (slot, op) in tired {
            let Some(dorm) = v.vacant_dorm(&plan) else {
                break;
            };
            if plan.move_to(&op, &dorm).is_err() {
                continue;
            }
            actions.push(SimAction::Move {
                operator: op.clone(),
                to: dorm,
            });
            if !self.bench.contains(&op) {
                self.home.insert(op, slot.clone());
            }
            let sub = self
                .bench
                .iter()
                .filter(|b| {
                    let resting = plan
                        .locate(b.as_str())
                        .is_none_or(|s| v.kind_of(&s) == Some(RoomType::Dormitory));
                    resting && v.mood(b.as_str()).is_some_and(|m| m >= self.swap_in)
                })
                .max_by(|a, b| {
                    v.mood(a.as_str())
                        .unwrap_or(0.0)
                        .total_cmp(&v.mood(b.as_str()).unwrap_or(0.0))
                })
                .cloned();
            if let Some(sub) = sub
                && plan.move_to(&sub, &slot).is_ok()
            {
                actions.push(SimAction::Move {
                    operator: sub,
                    to: slot,
                });
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
