//! Simulated annealing with geometric cooling, and the bounded set of
//! finalists both searches feed.

use ak_domain::Assignment;

use crate::control::{CHECK_EVERY, Tracker};
use crate::evaluator::{Evaluator, Score};
use crate::result::{SolveError, SolverConfig};
use crate::rng::Pcg32;
use crate::space::Space;

/// One finalist.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Its inner score.
    pub score: Score,
    /// Its canonical key (see [`Space::canonical`]).
    pub key: String,
    /// Who is where.
    pub assignment: Assignment,
}

/// The `k` best distinct assignments seen so far, best first.
#[derive(Debug, Clone)]
pub struct TopK {
    k: usize,
    entries: Vec<Entry>,
}

impl TopK {
    /// Room for `k` finalists.
    pub fn new(k: usize) -> Self {
        TopK {
            k: k.max(1),
            entries: Vec::new(),
        }
    }

    /// Offers a candidate. Returns true if it is kept. Finalists have
    /// distinct scores: a rearrangement that scores exactly like one already
    /// kept is dropped, so the list shows trade-offs rather than the same
    /// crew shuffled between identical rooms.
    pub fn insert(&mut self, score: Score, key: String, assignment: Assignment) -> bool {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            if score.value > self.entries[i].score.value {
                self.entries[i].score = score;
                self.entries[i].assignment = assignment;
                self.sort();
            }
            return false;
        }
        let tolerance = 1e-9 * score.value.abs().max(1.0);
        if self
            .entries
            .iter()
            .any(|e| (e.score.value - score.value).abs() <= tolerance)
        {
            return false;
        }
        if self.entries.len() < self.k {
            self.entries.push(Entry {
                score,
                key,
                assignment,
            });
            self.sort();
            return true;
        }
        let worst = self
            .entries
            .last()
            .map_or(f64::NEG_INFINITY, |e| e.score.value);
        if score.value > worst {
            self.entries.pop();
            self.entries.push(Entry {
                score,
                key,
                assignment,
            });
            self.sort();
            return true;
        }
        false
    }

    fn sort(&mut self) {
        // Ties break on the key so that a run is reproducible.
        self.entries.sort_by(|a, b| {
            b.score
                .value
                .total_cmp(&a.score.value)
                .then_with(|| a.key.cmp(&b.key))
        });
    }

    /// The best finalist.
    pub fn best(&self) -> Option<&Entry> {
        self.entries.first()
    }

    /// Finalists, best first.
    pub fn into_entries(self) -> Vec<Entry> {
        self.entries
    }
}

/// What one annealing run did.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnealReport {
    /// Candidates scored.
    pub evaluations: u64,
    /// Moves accepted.
    pub accepted: u64,
    /// The starting temperature used.
    pub initial_temperature: f64,
    /// Steps taken before the budget ran out or the run finished.
    pub steps: u32,
}

/// One annealing run from `start`, feeding `top`. `stream` is the restart
/// index: it picks the random stream and places this run's steps in the
/// progress count. The run ends early when `tracker` says stop.
pub fn anneal(
    space: &Space,
    evaluator: &mut dyn Evaluator,
    start: &Assignment,
    cfg: &SolverConfig,
    top: &mut TopK,
    stream: u64,
    tracker: &mut Tracker<'_>,
) -> Result<AnnealReport, SolveError> {
    let mut rng = Pcg32::new(cfg.seed, stream);
    let offset = stream as f64 * f64::from(cfg.iterations);
    let mut current = start.clone();
    let mut current_score = evaluator.score(&current)?;
    let mut evaluations = 1;
    tracker.saw(current_score.value);
    top.insert(
        current_score.clone(),
        space.canonical(&current),
        current.clone(),
    );

    let initial_temperature = match cfg.initial_temperature {
        Some(t) => t,
        None => {
            // Typical size of one move's effect.
            let mut deltas = Vec::new();
            for _ in 0..32 {
                let Some(cand) = space.propose(&current, &mut rng) else {
                    break;
                };
                let s = evaluator.score(&cand)?;
                evaluations += 1;
                tracker.saw(s.value);
                let d = (s.value - current_score.value).abs();
                if d > 0.0 {
                    deltas.push(d);
                }
                top.insert(s, space.canonical(&cand), cand);
            }
            if deltas.is_empty() {
                1.0
            } else {
                deltas.sort_by(f64::total_cmp);
                deltas[deltas.len() / 2]
            }
        }
    };
    let alpha = cfg
        .final_temperature_ratio
        .powf(1.0 / f64::from(cfg.iterations));
    let mut temperature = initial_temperature;
    let mut accepted = 0;
    let mut steps = 0;

    for step in 0..cfg.iterations {
        if u64::from(step).is_multiple_of(CHECK_EVERY)
            && tracker.checkpoint(offset + f64::from(step), evaluator.evaluations())
        {
            break;
        }
        steps = step + 1;
        let Some(cand) = space.propose(&current, &mut rng) else {
            break;
        };
        let s = evaluator.score(&cand)?;
        evaluations += 1;
        tracker.saw(s.value);
        let delta = s.value - current_score.value;
        let accept = delta >= 0.0 || rng.f64() < (delta / temperature).exp();
        top.insert(s.clone(), space.canonical(&cand), cand.clone());
        if accept {
            current = cand;
            current_score = s;
            accepted += 1;
        }
        temperature *= alpha;
    }
    tracker.report(offset + f64::from(steps), evaluator.evaluations());

    Ok(AnnealReport {
        evaluations,
        accepted,
        initial_temperature,
        steps,
    })
}
