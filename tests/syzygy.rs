//! Syzygy tablebase correctness suite (step 5.4).
//!
//! These tests need the 3-4-5-men tables on disk (`tools/download-syzygy.sh`
//! into `tools/tb`). CI has no tables, so the verdict tests are `#[ignore]` by
//! default AND additionally skip-if-absent (so `cargo test -- --ignored` on a
//! tableless box is still green). Run locally with:
//!     cargo test --release --test syzygy -- --ignored
//!
//! The non-ignored `init_is_graceful_without_tables` test asserts the
//! feature-off path works everywhere (the contract CI actually exercises).

use nebchess::board::{movegen::find_uci_move, Position};
use nebchess::tb::{Tb, Wdl};

const TB_PATH: &str = "tools/tb";

/// Load the local tables, or `None` if absent. The verdict tests bail out
/// quietly when this is `None` so a tableless environment never red-fails.
fn tables() -> Option<Tb> {
    Tb::init(TB_PATH)
}

fn wdl(tb: &Tb, fen: &str) -> Option<Wdl> {
    Tb::probe_wdl(tb, &Position::from_fen(fen).unwrap())
}

/// Feature-off contract — runs in CI (no tables). An empty / bad path yields
/// `None` and never panics.
#[test]
fn init_is_graceful_without_tables() {
    assert!(Tb::init("").is_none(), "empty SyzygyPath -> off");
    assert!(
        Tb::init("/no/such/syzygy/dir").is_none(),
        "missing dir -> off, no panic"
    );
}

#[test]
#[ignore = "needs tools/tb tables"]
fn kqvk_is_a_win() {
    let Some(tb) = tables() else { return };
    // Ke1, Qd1 vs Ke8 — legal (queen checks nothing), an elementary win.
    assert_eq!(wdl(&tb, "4k3/8/8/8/8/8/8/3QK3 w - - 0 1"), Some(Wdl::Win));
}

#[test]
#[ignore = "needs tools/tb tables"]
fn krvk_is_a_win() {
    let Some(tb) = tables() else { return };
    assert_eq!(wdl(&tb, "4k3/8/8/8/8/8/8/3RK3 w - - 0 1"), Some(Wdl::Win));
}

#[test]
#[ignore = "needs tools/tb tables"]
fn kvk_is_a_draw() {
    // Also caught by step 5.0's insufficient-material rule; verified here via
    // the tablebase too.
    let Some(tb) = tables() else { return };
    assert_eq!(wdl(&tb, "4k3/8/8/8/8/8/8/4K3 w - - 0 1"), Some(Wdl::Draw));
}

#[test]
#[ignore = "needs tools/tb tables"]
fn kpvk_central_pawn_is_a_win() {
    // The spec's required position: white to move escorts the e-pawn home.
    let Some(tb) = tables() else { return };
    assert_eq!(wdl(&tb, "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1"), Some(Wdl::Win));
}

#[test]
#[ignore = "needs tools/tb tables"]
fn kpvk_same_pawn_but_defender_to_move_is_a_draw() {
    // Identical board, BLACK to move: the defending king steps in front of the
    // e-pawn and holds the draw. The clean win/draw pair off a single pawn.
    let Some(tb) = tables() else { return };
    assert_eq!(wdl(&tb, "4k3/8/8/8/8/8/4P3/4K3 b - - 0 1"), Some(Wdl::Draw));
}

#[test]
#[ignore = "needs tools/tb tables"]
fn kpvk_rook_pawn_is_a_draw() {
    // a-pawn with the defending king able to reach the corner — a textbook
    // KPvK draw (a second, structurally different draw from the one above).
    let Some(tb) = tables() else { return };
    assert_eq!(wdl(&tb, "8/8/8/8/8/k7/P7/K7 w - - 0 1"), Some(Wdl::Draw));
}

#[test]
#[ignore = "needs tools/tb tables"]
fn root_probe_returns_a_legal_winning_move() {
    // DTZ root probe must hand back a move that (a) is legal in the position
    // and (b) carries the Win verdict. Checks the from/to/promo -> legal-move
    // reconstruction path end to end.
    let Some(tb) = tables() else { return };
    let fen = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let (mv, w) = Tb::probe_root(&tb, &pos).expect("root probe hits a 3-man win");
    assert_eq!(w, Wdl::Win, "central KPvK is won");
    assert!(
        find_uci_move(&pos, &mv.to_string()).is_some(),
        "root move {mv} must be legal in the position"
    );
}

#[test]
#[ignore = "needs tools/tb tables"]
fn root_probe_promotion_move_is_legal() {
    // A pawn one step from queening: the DTZ move is very likely a promotion,
    // exercising the promo-flag reconstruction. Whatever move it returns must
    // be legal and the verdict a Win.
    let Some(tb) = tables() else { return };
    let fen = "8/4P3/4k3/8/8/8/8/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let (mv, w) = Tb::probe_root(&tb, &pos).expect("root probe hits");
    assert_eq!(w, Wdl::Win);
    assert!(
        find_uci_move(&pos, &mv.to_string()).is_some(),
        "root move {mv} must be legal"
    );
}
