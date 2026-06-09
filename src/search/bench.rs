//! Deterministic bench (spec §10.2): fixed positions, fixed depth, no time
//! control. The total node count fingerprints search behavior — it goes in
//! every engine-affecting commit message as "Bench: N" and CI re-verifies it.

use std::time::Instant;

use crate::board::Position;
use crate::eval::NnueEvaluator;
use crate::search::SearchThread;

pub const BENCH_DEPTH: i32 = 6;

pub const BENCH_FENS: [&str; 12] = [
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
    "r3k2r/1b4bq/8/8/8/8/7B/R3K2R w KQkq - 0 1",
    "2K2r2/4P3/8/8/8/8/8/3k4 w - - 0 1",
    "8/8/1P2K3/8/2n5/1q6/8/5k2 b - - 0 1",
    "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1",
    "4k3/8/8/8/8/8/8/R3K3 w Q - 0 1",
    "7k/5Q2/6K1/8/8/8/8/8 w - - 0 1",
];

pub fn run() {
    let start = Instant::now();
    let mut total: u64 = 0;
    for (i, fen) in BENCH_FENS.iter().enumerate() {
        let pos = Position::from_fen(fen).expect("bench FEN");
        let mut st = SearchThread::new(pos, NnueEvaluator::embedded());
        let (_best, _score) = st.search_to_depth(BENCH_DEPTH);
        println!("position {:>2}: {:>10} nodes", i + 1, st.nodes);
        total += st.nodes;
    }
    let ms = start.elapsed().as_millis().max(1);
    println!("nps: {}", total as u128 * 1000 / ms);
    println!("Bench: {total}");
}
