//! plan-9: self-play training-data generator for NNUE. Dev-only binary.
//! Emits `FEN | cp_white | wdl_white` text shards from engine self-play.
//! Reproducible given (--seed, --threads, --games). No GPU, no new deps.

/// Seeded SplitMix64 (adapted from src/bin/find_magics.rs).
#[allow(dead_code)]
struct Rng(u64);
#[allow(dead_code)]
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, n)`. `n` must be > 0.
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn main() {
    eprintln!("datagen: see plan-9; run with --help (subcommands land in later tasks)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let a: Vec<u64> = { let mut r = Rng::new(42); (0..5).map(|_| r.next_u64()).collect() };
        let b: Vec<u64> = { let mut r = Rng::new(42); (0..5).map(|_| r.next_u64()).collect() };
        let c: Vec<u64> = { let mut r = Rng::new(43); (0..5).map(|_| r.next_u64()).collect() };
        assert_eq!(a, b, "same seed -> same stream");
        assert_ne!(a, c, "different seed -> different stream");

        let mut r = Rng::new(7);
        for _ in 0..1000 {
            assert!(r.below(13) < 13);
        }
    }
}
