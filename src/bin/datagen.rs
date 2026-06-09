//! plan-9: self-play training-data generator for NNUE. Dev-only binary.
//! Emits `FEN | cp_white | wdl_white` text shards from engine self-play.
//! Reproducible given (--seed, --threads, --games). No GPU, no new deps.

use nebchess::board::{generate_moves, movegen::find_first_legal, Move, MoveList, Position};
use nebchess::board::types::Color;
use nebchess::eval::Hce;
use nebchess::search::limits::Limits;
use nebchess::search::SearchThread;
use nebchess::tb::{Tb, Wdl};

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

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    WhiteWin,
    Draw,
    BlackWin,
}

#[allow(dead_code)]
fn outcome_to_wdl(o: Outcome) -> f32 {
    match o {
        Outcome::WhiteWin => 1.0,
        Outcome::Draw => 0.5,
        Outcome::BlackWin => 0.0,
    }
}

/// Side-to-move-relative TB result -> white-relative outcome.
#[allow(dead_code)]
fn wdl_to_outcome(stm: Color, w: Wdl) -> Outcome {
    match w {
        Wdl::Draw => Outcome::Draw,
        Wdl::Win => if stm == Color::White { Outcome::WhiteWin } else { Outcome::BlackWin },
        Wdl::Loss => if stm == Color::White { Outcome::BlackWin } else { Outcome::WhiteWin },
    }
}

/// Natural game end for the side to move. Mate/stalemate first (terminal),
/// then the draw rules. None if the game is ongoing.
#[allow(dead_code)]
fn terminal_outcome(pos: &mut Position) -> Option<Outcome> {
    if find_first_legal(pos).is_none() {
        return Some(if pos.in_check(pos.stm()) {
            if pos.stm() == Color::White { Outcome::BlackWin } else { Outcome::WhiteWin }
        } else {
            Outcome::Draw // stalemate
        });
    }
    if pos.is_fifty_move_draw() || pos.is_repetition() || pos.is_insufficient_material() {
        return Some(Outcome::Draw);
    }
    None
}

/// Scores at or above this magnitude are mate/saturated and are not recorded.
#[allow(dead_code)]
const MATE_THRESHOLD: i32 = 29_000; // mirrors search::MATE_BOUND

/// Convert a side-to-move-relative centipawn score to white-relative.
#[allow(dead_code)]
fn cp_white(stm: Color, score_cp: i32) -> i32 {
    if stm == Color::White { score_cp } else { -score_cp }
}

/// Smart-fen-skipping: record only QUIET, non-saturated positions where the
/// side to move is not in check (the net learns quiet eval; search handles tactics).
#[allow(dead_code)]
fn should_record(pos: &Position, best: Move, score_cp: i32) -> bool {
    !pos.in_check(pos.stm()) && !best.is_capture() && score_cp.abs() < MATE_THRESHOLD
}

#[allow(dead_code)]
#[derive(Clone)]
struct Config {
    soft_nodes: u64,
    opening_plies: usize,
    max_plies: usize,
    resign_cp: i32,
    resign_plies: i32,
}

impl Default for Config {
    fn default() -> Config {
        Config { soft_nodes: 5_000, opening_plies: 8, max_plies: 400, resign_cp: 1_000, resign_plies: 8 }
    }
}

/// Play one self-play game; push `(fen, cp_white, wdl_white)` for each kept position.
/// Reuses the caller's SearchThread (and its TT) across games for throughput; this is
/// deterministic per worker (single-threaded) and benign at ~5k nodes.
#[allow(dead_code)]
fn play_game(st: &mut SearchThread<Hce>, rng: &mut Rng, cfg: &Config,
             tb: Option<&Tb>, out: &mut Vec<(String, i32, f32)>) {
    // 1. Random opening (skip the game if it dead-ends during the opening).
    let Some(opening) = play_random_opening(rng, cfg.opening_plies) else { return };
    st.pos = opening;

    // 2. Self-play.
    let mut records: Vec<(String, i32)> = Vec::new();
    let mut outcome: Option<Outcome> = None;
    let mut resign_run = 0i32;

    for _ in 0..cfg.max_plies {
        if let Some(o) = terminal_outcome(&mut st.pos) {
            outcome = Some(o);
            break;
        }
        if let Some(tb) = tb {
            if let Some(w) = tb.probe_wdl(&st.pos) {
                outcome = Some(wdl_to_outcome(st.pos.stm(), w));
                break;
            }
        }

        let limits = Limits {
            soft_nodes: Some(cfg.soft_nodes),
            nodes: Some(cfg.soft_nodes.saturating_mul(8)), // hard safety ceiling
            ..Limits::default()
        };
        let mut score = 0i32;
        let best = st.iterate(&limits, |info| score = info.score);
        let Some(mv) = best else { break };

        if should_record(&st.pos, mv, score) {
            records.push((st.pos.to_fen(), cp_white(st.pos.stm(), score)));
        }

        // Resign adjudication: a sustained large white-relative edge ends the game.
        let wcp = cp_white(st.pos.stm(), score);
        resign_run = if wcp.abs() >= cfg.resign_cp { resign_run + 1 } else { 0 };
        if resign_run >= cfg.resign_plies {
            outcome = Some(if wcp > 0 { Outcome::WhiteWin } else { Outcome::BlackWin });
            break;
        }

        st.pos.make(mv);
    }

    // 3. Label every recorded position with the game result.
    let wdl = outcome_to_wdl(outcome.unwrap_or(Outcome::Draw));
    for (fen, cp) in records {
        out.push((fen, cp, wdl));
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
    fn terminal_outcome_detects_endings() {
        // Fool's mate: White to move, checkmated -> Black wins.
        let mut mate = Position::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3").unwrap();
        assert_eq!(terminal_outcome(&mut mate), Some(Outcome::BlackWin));

        // Stalemate: Black to move, not in check, no legal moves -> Draw.
        let mut stale = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(terminal_outcome(&mut stale), Some(Outcome::Draw));

        // KvK insufficient material -> Draw.
        let mut kvk = Position::from_fen("8/8/4k3/8/8/3K4/8/8 w - - 0 1").unwrap();
        assert_eq!(terminal_outcome(&mut kvk), Some(Outcome::Draw));

        // Start position is ongoing.
        let mut start = Position::startpos();
        assert_eq!(terminal_outcome(&mut start), None);
    }

    #[test]
    fn wdl_maps_to_white_relative_outcome() {
        assert_eq!(wdl_to_outcome(Color::White, Wdl::Win), Outcome::WhiteWin);
        assert_eq!(wdl_to_outcome(Color::Black, Wdl::Win), Outcome::BlackWin);
        assert_eq!(wdl_to_outcome(Color::White, Wdl::Loss), Outcome::BlackWin);
        assert_eq!(wdl_to_outcome(Color::Black, Wdl::Loss), Outcome::WhiteWin);
        assert_eq!(wdl_to_outcome(Color::White, Wdl::Draw), Outcome::Draw);
    }

    #[test]
    fn cp_white_flips_for_black() {
        assert_eq!(cp_white(Color::White, 30), 30);
        assert_eq!(cp_white(Color::Black, 30), -30);
    }

    #[test]
    fn filter_skips_check_capture_and_saturated() {
        use nebchess::board::types::Square;

        let e2 = Square::from_name("e2").unwrap();
        let e4 = Square::from_name("e4").unwrap();
        let quiet = Move::new(e2, e4, Move::QUIET);
        let capture = Move::new(e2, e4, Move::CAPTURE);

        let start = Position::startpos();
        assert!(should_record(&start, quiet, 25), "quiet, in-bounds score -> record");
        assert!(!should_record(&start, capture, 25), "best move is a capture -> skip");
        assert!(!should_record(&start, quiet, 30_000), "saturated/mate score -> skip");

        // Side to move in check -> skip.
        let in_check = Position::from_fen("4k3/8/8/8/7q/8/8/4K3 w - - 0 1").unwrap();
        assert!(!should_record(&in_check, quiet, 25), "stm in check -> skip");
    }

    #[test]
    fn play_game_is_deterministic_and_consistent() {
        let cfg = Config { soft_nodes: 400, opening_plies: 4, max_plies: 60, ..Config::default() };

        let run = |seed: u64| {
            let mut st = SearchThread::<Hce>::new(Position::startpos(), Hce::new());
            let mut rng = Rng::new(seed);
            let mut out = Vec::new();
            play_game(&mut st, &mut rng, &cfg, None, &mut out);
            out
        };

        let a = run(123);
        let b = run(123);
        assert_eq!(a, b, "same seed -> identical game/records");

        if let Some((_, _, wdl0)) = a.first() {
            for (fen, cp, wdl) in &a {
                assert!(Position::from_fen(fen).is_ok(), "recorded FEN must parse: {fen}");
                assert!(*wdl == 0.0 || *wdl == 0.5 || *wdl == 1.0, "wdl in set");
                assert_eq!(wdl, wdl0, "one game -> one result label");
                assert!(cp.abs() < MATE_THRESHOLD, "no saturated scores recorded");
            }
        }
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
