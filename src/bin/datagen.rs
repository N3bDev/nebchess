//! plan-9: self-play training-data generator for NNUE. Dev-only binary.
//! Emits `FEN | cp_white | wdl_white` text shards from engine self-play.
//! Reproducible given (--seed, --threads, --games). No GPU, no new deps.

use nebchess::board::{generate_moves, Move, MoveList, Position};

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

/// Pick a uniformly-random LEGAL move (generate_moves is pseudo-legal; filter via make/unmake).
#[allow(dead_code)]
fn random_legal_move(pos: &mut Position, rng: &mut Rng) -> Option<Move> {
    let mut pseudo = MoveList::new();
    generate_moves(pos, &mut pseudo);
    let mut legal: Vec<Move> = Vec::with_capacity(pseudo.len());
    for &mv in pseudo.iter() {
        if pos.make(mv) {
            pos.unmake();
            legal.push(mv);
        }
    }
    if legal.is_empty() {
        None
    } else {
        Some(legal[rng.below(legal.len())])
    }
}

/// Play `plies` random legal half-moves from the start position. Returns None if a
/// terminal position (mate/stalemate) is hit during the opening (caller skips the game).
#[allow(dead_code)]
fn play_random_opening(rng: &mut Rng, plies: usize) -> Option<Position> {
    let mut pos = Position::startpos();
    for _ in 0..plies {
        let mv = random_legal_move(&mut pos, rng)?;
        pos.make(mv);
    }
    Some(pos)
}

fn main() {
    eprintln!("datagen: see plan-9; run with --help (subcommands land in later tasks)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebchess::board::types::Color;

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

    #[test]
    fn random_legal_move_is_legal_and_deterministic() {
        let pick = |seed: u64| {
            let mut pos = Position::startpos();
            let mut rng = Rng::new(seed);
            random_legal_move(&mut pos, &mut rng)
        };
        let mv = pick(1).expect("startpos has legal moves");
        let mut pos = Position::startpos();
        assert!(pos.make(mv), "returned move must be legal");
        pos.unmake();
        assert_eq!(pick(1), pick(1), "same seed -> same pick");
    }

    #[test]
    fn random_opening_applies_requested_plies() {
        let mut rng = Rng::new(99);
        let pos = play_random_opening(&mut rng, 8).expect("8-ply opening from startpos");
        assert_eq!(pos.stm(), Color::White); // 8 half-moves -> White to move again
        assert!(pos.to_fen().split_whitespace().count() >= 6, "valid FEN");

        let mut rng_a = Rng::new(5);
        let mut rng_b = Rng::new(5);
        assert_eq!(
            play_random_opening(&mut rng_a, 8).map(|p| p.to_fen()),
            play_random_opening(&mut rng_b, 8).map(|p| p.to_fen()),
            "same seed -> same opening"
        );
    }
}
