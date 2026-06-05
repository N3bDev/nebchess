//! Search behavior tests: mate finding, draw handling, limit respect.

use nebchess::board::{movegen::find_uci_move, Position};
use nebchess::eval::Hce;
use nebchess::search::{SearchThread, MATE, MATE_BOUND};

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
