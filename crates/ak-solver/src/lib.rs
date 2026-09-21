//! Assignment solvers (Layer 5).
//!
//! **Status: placeholder.** Planned in order:
//!
//! 1. exhaustive search for small instances (the ground truth);
//! 2. simulated annealing over the full base, parallel restarts via `rayon`;
//! 3. optionally tabu search.
//!
//! The solver is generic over an `Evaluator` so a cheap steady-state
//! approximation can rank candidates and the full `ak-eval` simulation can
//! score the final top-K. It returns the top-K solutions, not just the best.

#![forbid(unsafe_code)]
