//! A small deterministic generator (PCG32), so that a seed fully determines
//! a run on every platform and Rust version. No dependency, no global
//! state.

/// PCG32: 64-bit state, 32-bit output.
#[derive(Debug, Clone)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    /// A generator for `seed` on `stream`. Different streams with the same
    /// seed give independent sequences (used for restarts).
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Pcg32 {
            state: 0,
            inc: (stream << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    /// Next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform integer in `0..n`. `n` must be positive.
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        ((u64::from(self.next_u32()) * n as u64) >> 32) as usize
    }

    /// Uniform float in `[0, 1)`.
    pub fn f64(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_stream_independent() {
        let mut x = Pcg32::new(42, 0);
        let mut y = Pcg32::new(42, 0);
        let mut z = Pcg32::new(42, 1);
        let xs: Vec<u32> = (0..8).map(|_| x.next_u32()).collect();
        let ys: Vec<u32> = (0..8).map(|_| y.next_u32()).collect();
        let zs: Vec<u32> = (0..8).map(|_| z.next_u32()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
        let mut r = Pcg32::new(7, 3);
        for _ in 0..1000 {
            assert!(r.below(5) < 5);
            let f = r.f64();
            assert!((0.0..1.0).contains(&f));
        }
    }
}
