//! Exhaustive enumeration for small spaces: the ground truth the annealer
//! is checked against.

use ak_domain::Assignment;

use crate::anneal::TopK;
use crate::evaluator::Evaluator;
use crate::result::SolveError;
use crate::space::Space;

/// Scores every distinct assignment of the pool to the variable slots,
/// starting from `start` (whose variable slots must be empty), and feeds
/// `top`. Returns how many were scored.
pub fn enumerate(
    space: &Space,
    start: &Assignment,
    evaluator: &mut dyn Evaluator,
    top: &mut TopK,
) -> Result<u64, SolveError> {
    let groups: Vec<(Vec<usize>, bool)> = space
        .groups()
        .map(|(slots, ordered)| (slots.to_vec(), ordered))
        .collect();
    let mut walk = Walk {
        space,
        groups: &groups,
        evaluator,
        top,
        assignment: start.clone(),
        used: vec![false; space.pool.len()],
        scored: 0,
    };
    walk.group(0, 0, 0)?;
    Ok(walk.scored)
}

struct Walk<'a> {
    space: &'a Space,
    groups: &'a [(Vec<usize>, bool)],
    evaluator: &'a mut dyn Evaluator,
    top: &'a mut TopK,
    assignment: Assignment,
    used: Vec<bool>,
    scored: u64,
}

impl Walk<'_> {
    /// Fills group `gi` from its slot `si`, choosing operators with pool
    /// index at least `min_op` (so an unordered room is filled in one order
    /// only), then moves on.
    fn group(&mut self, gi: usize, si: usize, min_op: usize) -> Result<(), SolveError> {
        if gi == self.groups.len() {
            let score = self.evaluator.score(&self.assignment)?;
            self.scored += 1;
            let key = self.space.canonical(&self.assignment);
            self.top.insert(score, key, self.assignment.clone());
            return Ok(());
        }
        let (slots, ordered) = (&self.groups[gi].0, self.groups[gi].1);
        if si == slots.len() {
            return self.group(gi + 1, 0, 0);
        }
        // Leave this slot, and the rest of the room, empty.
        self.group(gi + 1, 0, 0)?;
        let slot = self.space.slots[slots[si]].clone();
        let first = if ordered { 0 } else { min_op };
        for p in first..self.space.pool.len() {
            if self.used[p] {
                continue;
            }
            self.used[p] = true;
            let op = self.space.pool[p].clone();
            let placed = self.assignment.place(&slot, op).is_ok();
            if placed {
                let next_min = if ordered { 0 } else { p + 1 };
                let r = self.group(gi, si + 1, next_min);
                self.assignment.remove(&slot);
                r?;
            }
            self.used[p] = false;
        }
        Ok(())
    }
}
