//! The search space: which slots may change, who may fill them, and the
//! moves that take one assignment to a neighbouring one.
//!
//! Every move goes through [`Assignment`]'s checked mutators, so the search
//! cannot produce an operator in two places or a slot beyond a room's
//! capacity. Slots within a room are interchangeable (except the Training
//! Room's two roles), so swaps within a room are never proposed and
//! [`Space::canonical`] treats permutations within a room as one
//! assignment.

use std::collections::BTreeSet;

use ak_domain::{
    Assignment, BaseConfig, GameData, OperatorId, RoomId, RoomType, Roster, Slot,
    TRAINING_TRAINEE_SLOT,
};

use crate::result::SolveError;
use crate::rng::Pcg32;

/// One room's variable slots.
struct Group {
    /// Indexes into [`Space::slots`].
    slots: Vec<usize>,
    /// Whether slot order matters (Training Room).
    ordered: bool,
}

/// The search space.
pub struct Space {
    /// Slots the solver may change.
    pub slots: Vec<Slot>,
    /// Operators the solver may place, in a fixed order.
    pub pool: Vec<OperatorId>,
    /// Slots held fixed.
    pub locked: usize,
    groups: Vec<Group>,
    slot_group: Vec<usize>,
    rooms: Vec<(RoomId, bool)>,
}

impl Space {
    /// Builds the space. `initial` must fit the base; its operators in
    /// variable slots join the pool, and its operators in locked slots
    /// leave it.
    pub fn new(
        data: &GameData,
        base: &BaseConfig,
        roster: &Roster,
        pool: Option<&[OperatorId]>,
        initial: &Assignment,
        locked: &[Slot],
    ) -> Result<Self, SolveError> {
        base.validate(data).map_err(ak_eval::SimError::from)?;
        initial.check(base, data).map_err(ak_eval::SimError::from)?;
        for l in locked {
            let ok = base
                .room(l.room.as_str())
                .is_some_and(|r| l.index < r.capacity(data));
            if !ok {
                return Err(SolveError::BadLockedSlot(l.clone()));
            }
        }

        let mut slots = Vec::new();
        let mut groups = Vec::new();
        let mut slot_group = Vec::new();
        let mut rooms = Vec::new();
        let mut fixed_ops: BTreeSet<OperatorId> = BTreeSet::new();
        let mut locked_count = 0;
        for room in &base.rooms {
            let ordered = room.kind == RoomType::Training;
            rooms.push((room.id.clone(), ordered));
            let mut group = Group {
                slots: Vec::new(),
                ordered,
            };
            for i in 0..room.capacity(data) {
                let slot = Slot::new(room.id.clone(), i);
                let fixed = locked.contains(&slot)
                    || (room.kind == RoomType::Training && i == TRAINING_TRAINEE_SLOT);
                if fixed {
                    locked_count += 1;
                    if let Some(op) = initial
                        .slots(room.id.as_str())
                        .and_then(|s| s.get(usize::from(i)))
                        .cloned()
                        .flatten()
                    {
                        fixed_ops.insert(op);
                    }
                    continue;
                }
                slot_group.push(groups.len());
                group.slots.push(slots.len());
                slots.push(slot);
            }
            groups.push(group);
        }
        if slots.is_empty() {
            return Err(SolveError::EmptySpace);
        }

        let mut seen: BTreeSet<&OperatorId> = BTreeSet::new();
        let mut pool_ops: Vec<OperatorId> = Vec::new();
        let candidates: Vec<&OperatorId> = match pool {
            Some(list) => list.iter().collect(),
            None => roster.entries.keys().collect(),
        };
        for op in candidates {
            if !data.operators.contains_key(op.as_str()) {
                return Err(SolveError::UnknownOperator(op.clone()));
            }
            if fixed_ops.contains(op) || !seen.insert(op) {
                continue;
            }
            pool_ops.push(op.clone());
        }
        for (slot, op) in initial.iter() {
            if slots.contains(&slot) && !pool_ops.contains(op) {
                if !data.operators.contains_key(op.as_str()) {
                    return Err(SolveError::UnknownOperator(op.clone()));
                }
                pool_ops.push(op.clone());
            }
        }

        Ok(Space {
            slots,
            pool: pool_ops,
            locked: locked_count,
            groups,
            slot_group,
            rooms,
        })
    }

    /// Distinct assignments of the pool to the variable slots, with
    /// permutations within a room counted once. Saturates at `1e300`.
    pub fn size_estimate(&self) -> f64 {
        // ways[k]: ways to place k specific operators into the groups seen so
        // far, unordered within a group.
        let capacity: usize = self.groups.iter().map(|g| g.slots.len()).sum();
        let mut ways = vec![0.0; capacity + 1];
        ways[0] = 1.0;
        let mut placed_max = 0;
        for g in &self.groups {
            let cap = g.slots.len();
            let mut next = vec![0.0; capacity + 1];
            for (k, w) in ways.iter().enumerate().take(placed_max + 1) {
                if *w == 0.0 {
                    continue;
                }
                for j in 0..=cap {
                    let arrangements = if g.ordered {
                        falling(k + j, j)
                    } else {
                        choose(k + j, j)
                    };
                    next[k + j] += w * arrangements;
                }
            }
            placed_max += cap;
            ways = next;
        }
        let p = self.pool.len();
        let mut total = 0.0;
        for (k, w) in ways.iter().enumerate() {
            if k > p {
                break;
            }
            total += choose(p, k) * w;
        }
        total.min(1e300)
    }

    /// A key that is equal for assignments that differ only by the order of
    /// operators within a room (other than the Training Room).
    pub fn canonical(&self, a: &Assignment) -> String {
        let mut key = String::new();
        for (room, ordered) in &self.rooms {
            let mut names: Vec<&str> = a
                .slots(room.as_str())
                .into_iter()
                .flatten()
                .map(|o| o.as_ref().map_or("", |op| op.as_str()))
                .collect();
            if !ordered {
                names.sort_unstable();
            }
            key.push_str(room.as_str());
            key.push(':');
            key.push_str(&names.join(","));
            key.push('|');
        }
        key
    }

    /// Empties every variable slot.
    pub fn clear_variable(&self, a: &mut Assignment) {
        for slot in &self.slots {
            a.remove(slot);
        }
    }

    /// Pool operators not currently assigned anywhere.
    pub fn unassigned<'a>(&'a self, a: &Assignment) -> Vec<&'a OperatorId> {
        self.pool
            .iter()
            .filter(|op| a.locate(op.as_str()).is_none())
            .collect()
    }

    /// A random neighbour: swap two slots, replace an occupant with a bench
    /// operator, fill an empty slot, or clear a slot. `None` only if no move
    /// applies at all.
    pub fn propose(&self, current: &Assignment, rng: &mut Pcg32) -> Option<Assignment> {
        for _ in 0..24 {
            let mut a = current.clone();
            let roll = rng.below(100);
            let changed = if roll < 40 {
                self.swap(&mut a, rng)
            } else if roll < 70 {
                self.replace(&mut a, rng)
            } else if roll < 90 {
                self.fill(&mut a, rng)
            } else {
                self.clear(&mut a, rng)
            };
            if changed {
                return Some(a);
            }
        }
        // The random draws kept picking moves that do not apply here (with
        // the whole pool placed, replace and fill never do). That is rare,
        // and it does not mean the search is stuck, so choose among the
        // moves that exist.
        let mut all = self.neighbours(current);
        if all.is_empty() {
            None
        } else {
            let pick = rng.below(all.len());
            Some(all.swap_remove(pick))
        }
    }

    /// Every assignment one move away from `current`.
    pub fn neighbours(&self, current: &Assignment) -> Vec<Assignment> {
        let mut out = Vec::new();
        for i in 0..self.slots.len() {
            for j in i + 1..self.slots.len() {
                let same_group = self.slot_group[i] == self.slot_group[j];
                if same_group && !self.groups[self.slot_group[i]].ordered {
                    continue;
                }
                let (si, sj) = (&self.slots[i], &self.slots[j]);
                if !is_occupied(current, si) && !is_occupied(current, sj) {
                    continue;
                }
                let mut a = current.clone();
                if a.swap(si, sj).is_ok() {
                    out.push(a);
                }
            }
        }
        let bench = self.unassigned(current);
        for slot in &self.slots {
            let occupied = is_occupied(current, slot);
            for op in &bench {
                let mut a = current.clone();
                let moved = if occupied {
                    a.move_to(op, slot).is_ok()
                } else {
                    a.place(slot, (*op).clone()).is_ok()
                };
                if moved {
                    out.push(a);
                }
            }
            if occupied {
                let mut a = current.clone();
                if a.remove(slot).is_some() {
                    out.push(a);
                }
            }
        }
        out
    }

    fn swap(&self, a: &mut Assignment, rng: &mut Pcg32) -> bool {
        if self.slots.len() < 2 {
            return false;
        }
        let i = rng.below(self.slots.len());
        let j = rng.below(self.slots.len());
        if i == j {
            return false;
        }
        let same_group = self.slot_group[i] == self.slot_group[j];
        if same_group && !self.groups[self.slot_group[i]].ordered {
            return false;
        }
        let (si, sj) = (&self.slots[i], &self.slots[j]);
        let occupied = |s: &Slot| {
            a.slots(s.room.as_str())
                .and_then(|v| v.get(usize::from(s.index)))
                .is_some_and(Option::is_some)
        };
        if !occupied(si) && !occupied(sj) {
            return false;
        }
        a.swap(si, sj).is_ok()
    }

    fn replace(&self, a: &mut Assignment, rng: &mut Pcg32) -> bool {
        let occupied: Vec<&Slot> = self
            .slots
            .iter()
            .filter(|s| {
                a.slots(s.room.as_str())
                    .and_then(|v| v.get(usize::from(s.index)))
                    .is_some_and(Option::is_some)
            })
            .collect();
        let bench = self.unassigned(a);
        if occupied.is_empty() || bench.is_empty() {
            return false;
        }
        let slot = occupied[rng.below(occupied.len())].clone();
        let op = bench[rng.below(bench.len())].clone();
        a.move_to(&op, &slot).is_ok()
    }

    fn fill(&self, a: &mut Assignment, rng: &mut Pcg32) -> bool {
        let empty: Vec<&Slot> = self
            .slots
            .iter()
            .filter(|s| {
                a.slots(s.room.as_str())
                    .and_then(|v| v.get(usize::from(s.index)))
                    .is_some_and(Option::is_none)
            })
            .collect();
        let bench = self.unassigned(a);
        if empty.is_empty() || bench.is_empty() {
            return false;
        }
        let slot = empty[rng.below(empty.len())].clone();
        let op = bench[rng.below(bench.len())].clone();
        a.place(&slot, op).is_ok()
    }

    fn clear(&self, a: &mut Assignment, rng: &mut Pcg32) -> bool {
        let occupied: Vec<&Slot> = self
            .slots
            .iter()
            .filter(|s| {
                a.slots(s.room.as_str())
                    .and_then(|v| v.get(usize::from(s.index)))
                    .is_some_and(Option::is_some)
            })
            .collect();
        if occupied.is_empty() {
            return false;
        }
        let slot = occupied[rng.below(occupied.len())].clone();
        a.remove(&slot).is_some()
    }

    /// Variable slots grouped by room, as `(slot indexes, ordered)`.
    pub(crate) fn groups(&self) -> impl Iterator<Item = (&[usize], bool)> {
        self.groups.iter().map(|g| (g.slots.as_slice(), g.ordered))
    }
}

fn is_occupied(a: &Assignment, s: &Slot) -> bool {
    a.slots(s.room.as_str())
        .and_then(|v| v.get(usize::from(s.index)))
        .is_some_and(Option::is_some)
}

fn choose(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut r = 1.0;
    for i in 0..k {
        r = r * (n - i) as f64 / (i + 1) as f64;
    }
    r
}

fn falling(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    (0..k).map(|i| (n - i) as f64).product()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binomials() {
        assert_eq!(choose(6, 3), 20.0);
        assert_eq!(choose(6, 0), 1.0);
        assert_eq!(choose(3, 5), 0.0);
        assert_eq!(falling(5, 2), 20.0);
    }
}
