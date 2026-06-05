//! Search behavior tests: mate finding, draw handling, limit respect.

use nebchess::board::{movegen::find_uci_move, Position};
use nebchess::eval::Hce;
use nebchess::search::{limits::Limits, SearchThread, MATE, MATE_BOUND};
use std::time::Instant;

fn searcher(fen: &str) -> SearchThread<Hce> {
    SearchThread::new(Position::from_fen(fen).unwrap(), Hce::new())
}

#[test]
fn finds_mate_in_one() {
    // back-rank: 1.Ra8#
    let mut st = searcher("6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1");
    let (best, score) = st.search_to_depth(2);
    assert_eq!(best.unwrap().to_string(), "a1a8");
    assert_eq!(score, MATE - 1, "mate at ply 1");
}

#[test]
fn finds_mate_in_two() {
    // KR vs K: 1.Kb6! Kb8 2.Rh8# (1.Rh8+? Ka7 escapes; 1.Rh7 Kb8 2.Rh8+ Ka7 escapes)
    let mut st = searcher("k7/8/2K5/8/8/8/8/7R w - - 0 1");
    let (best, score) = st.search_to_depth(4);
    assert_eq!(score, MATE - 3, "mate at ply 3");
    assert_eq!(best.unwrap().to_string(), "c6b6");
}

#[test]
fn stalemate_scores_draw() {
    // black to move, Kh8 has no moves, not in check
    let mut st = searcher("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
    let (best, score) = st.search_to_depth(3);
    assert!(best.is_none(), "no legal moves");
    assert!(score.abs() <= 1, "draw jitter only, got {score}");
}

#[test]
fn qsearch_resolves_hanging_queen() {
    // Qd1xd8 wins a queen outright; depth 1 + qsearch must see it
    let mut st = searcher("3q1k2/8/8/8/8/8/8/3Q1K2 w - - 0 1");
    let (best, score) = st.search_to_depth(1);
    assert_eq!(best.unwrap().to_string(), "d1d8");
    assert!(score > 700, "won a queen, got {score}");
}

#[test]
fn depth_one_returns_a_legal_move() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let (best, _) = st.search_to_depth(1);
    let pos = Position::startpos();
    let mv = best.expect("must produce a move");
    assert!(
        find_uci_move(&pos, &mv.to_string()).is_some(),
        "bestmove must be legal"
    );
}

#[test]
fn node_limit_stops_search() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    st.set_node_limit(Some(10_000));
    let (_best, _) = st.search_to_depth(99);
    assert!(
        st.nodes < 10_000 + 5_000,
        "polling cadence overshoot bounded, got {}",
        st.nodes
    );
}

#[test]
fn fifty_move_draw_scored_in_search() {
    // halfmove already at 100: any deeper node should resolve as draw-ish.
    // KQ vs K would otherwise be a huge score; the rule caps it.
    // (Black king on a8: NOT attacked by Qf6 — the position must be legal.)
    let mut st = searcher("k7/8/5Q2/8/8/8/8/K7 w - - 100 90");
    let (_best, score) = st.search_to_depth(3);
    // root itself is exempt (ply 0); children all return draw — score ~0,
    // far below the +900-ish a live queen would give
    assert!(
        score.abs() <= 1,
        "fifty-move children cap score, got {score}"
    );
}

#[test]
fn en_prise_king_fen_does_not_panic() {
    // ILLEGAL input position (black king already capturable, white to move):
    // GUIs can send such FENs; the engine must not crash and should report
    // a mate-class score for the capture.
    let mut st = searcher("7k/8/5Q2/8/8/8/8/K7 w - - 100 90");
    let (best, score) = st.search_to_depth(3);
    assert!(best.is_some());
    assert!(
        score > MATE_BOUND,
        "king capture scores as won, got {score}"
    );
}

#[test]
fn movetime_is_respected() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let limits = Limits {
        movetime: Some(100),
        ..Limits::default()
    };
    let t0 = Instant::now();
    let best = st.iterate(&limits, |_| {});
    let elapsed = t0.elapsed().as_millis();
    assert!(best.is_some());
    assert!(elapsed < 600, "movetime 100 took {elapsed}ms");
}

#[test]
fn depth_limit_caps_iterations() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let limits = Limits {
        depth: Some(3),
        ..Limits::default()
    };
    let mut depths = Vec::new();
    st.iterate(&limits, |i| depths.push(i.depth));
    assert_eq!(depths, vec![1, 2, 3]);
}

#[test]
fn tiny_node_budget_still_returns_legal_move() {
    let mut st = searcher("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
    // a 1-node budget forces the earliest possible abort path; the
    // first-legal fallback must still produce a legal bestmove
    let limits = Limits {
        nodes: Some(1),
        ..Limits::default()
    };
    let best = st.iterate(&limits, |_| {}).expect("legal moves exist");
    let pos = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    assert!(find_uci_move(&pos, &best.to_string()).is_some());
}

#[test]
fn clock_allocation_returns_promptly() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let limits = Limits {
        wtime: Some(1_000),
        ..Limits::default()
    }; // soft ~33ms, hard ~132ms
    let t0 = Instant::now();
    st.iterate(&limits, |_| {});
    assert!(t0.elapsed().as_millis() < 700);
}

#[test]
fn mate_found_exits_early() {
    let mut st = searcher("6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1");
    let limits = Limits::default(); // no limits at all
    let t0 = Instant::now();
    let best = st.iterate(&limits, |_| {});
    assert_eq!(best.unwrap().to_string(), "a1a8");
    assert!(t0.elapsed().as_secs() < 5, "mate-bound early exit");
}

#[test]
fn no_legal_moves_returns_none() {
    // stalemate on the board
    let mut st = searcher("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
    let best = st.iterate(&Limits::default(), |_| {});
    assert!(best.is_none());
}

// --- Task 3: TT cutoff + store behavior tests ---

#[test]
fn tt_makes_research_cheap_and_stable() {
    let mut st = searcher("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1");
    let (best1, score1) = st.search_to_depth(6);
    let nodes_first = st.nodes;
    let (best2, score2) = st.search_to_depth(6); // same thread: warm TT
    let nodes_second = st.nodes - nodes_first;
    assert_eq!(best1, best2, "warm-TT re-search must agree");
    assert_eq!(score1, score2);
    assert!(
        nodes_second * 4 < nodes_first,
        "warm TT should slash nodes: {nodes_first} then {nodes_second}"
    );
}

#[test]
fn mate_scores_survive_tt_round_trips() {
    let mut st = searcher("k7/8/2K5/8/8/8/8/7R w - - 0 1");
    let (_b1, s1) = st.search_to_depth(4);
    assert_eq!(s1, MATE - 3);
    let (b2, s2) = st.search_to_depth(4); // warm TT: ply-adjust must hold
    assert_eq!(s2, MATE - 3, "mate distance corrupted through the TT");
    assert_eq!(b2.unwrap().to_string(), "c6b6");
}

#[test]
fn tiny_tt_collision_storm_is_sound() {
    // 1MB table + deep-ish search = heavy collisions; the gate is soundness
    // (no panics, legal move, sane score), not strength
    let mut st =
        searcher("r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10");
    st.set_tt(std::sync::Arc::new(nebchess::search::tt::Tt::new(1)));
    let (best, score) = st.search_to_depth(7);
    assert!(best.is_some());
    assert!(
        score.abs() < 1000,
        "quiet position, sane score, got {score}"
    );
}

// --- Task 4: TT-move ordering + MVV-LVA king attacker fix ---

#[test]
fn junk_tt_move_is_ignored_not_played() {
    // poison the TT entry for the root position with a junk move encoding,
    // then search: the engine must neither panic nor emit an illegal move
    let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    let mut st = searcher(fen);
    let tt = std::sync::Arc::new(nebchess::search::tt::Tt::new(1));
    let key = nebchess::board::Position::from_fen(fen).unwrap().key();
    // raw 0xFFFF decodes to h8->h8 promo-capture nonsense: never generated
    tt.store(
        key,
        nebchess::board::Move::from_raw(0xFFFF),
        500,
        nebchess::search::tt::EVAL_NONE,
        12,
        nebchess::search::tt::Bound::Lower,
        0,
    );
    st.set_tt(tt);
    let (best, _) = st.search_to_depth(4);
    let pos = nebchess::board::Position::from_fen(fen).unwrap();
    assert!(
        nebchess::board::movegen::find_uci_move(&pos, &best.unwrap().to_string()).is_some(),
        "junk TT move leaked into play"
    );
}

#[test]
fn tt_ordering_reduces_nodes() {
    // search depth 6 cold, then depth 7: the depth-6 TT moves should steer
    // depth 7 well below a cold depth-7 search
    let fen = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";
    let mut warm = searcher(fen);
    warm.search_to_depth(6);
    let nodes_before_7 = warm.nodes;
    warm.search_to_depth(7);
    let warm_7 = warm.nodes - nodes_before_7;
    let mut cold = searcher(fen);
    cold.search_to_depth(7);
    assert!(
        warm_7 < cold.nodes,
        "TT-move ordering should beat cold search: warm {warm_7} vs cold {}",
        cold.nodes
    );
}
