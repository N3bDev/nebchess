//! Tapered HCE. Every term function takes a Tracer; the engine calls with
//! NullTracer (zero cost), the tuner with CollectingTracer. ALL parameter
//! reads go through PARAMS[idx] + trace.record(idx, sign) IN THE SAME
//! STATEMENT GROUP — that invariant is what keeps the tuner honest.

use crate::board::{Color, Move, PieceType, Position};
use crate::eval::eval_params::PARAMS;
use crate::eval::manifest as m;
use crate::eval::trace::{NullTracer, Tracer};
use crate::eval::Evaluator;

/// Game phase: N/B=1, R=2, Q=4 per piece, capped at 24 (opening) .. 0 (bare kings).
pub fn phase(pos: &Position) -> i32 {
    let mut p = 0;
    for color in [Color::White, Color::Black] {
        p += pos.piece_bb(color, PieceType::Knight).count() as i32;
        p += pos.piece_bb(color, PieceType::Bishop).count() as i32;
        p += 2 * pos.piece_bb(color, PieceType::Rook).count() as i32;
        p += 4 * pos.piece_bb(color, PieceType::Queen).count() as i32;
    }
    p.min(24)
}

/// Shared add-term helper (promoted from closure in T2+ for all term functions).
#[inline]
fn add_term<T: Tracer>(idx: usize, sign: i32, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let (pmg, peg) = PARAMS[idx];
    *mg += sign * pmg;
    *eg += sign * peg;
    t.record(idx, sign as i8);
}

/// White-relative (mg, eg) accumulation over all terms.
pub fn eval_terms<T: Tracer>(pos: &Position, t: &mut T) -> (i32, i32) {
    let (mut mg, mut eg) = (0i32, 0i32);

    const PST: [usize; 6] = [
        m::PST_PAWN,
        m::PST_KNIGHT,
        m::PST_BISHOP,
        m::PST_ROOK,
        m::PST_QUEEN,
        m::PST_KING,
    ];
    for pt in PieceType::ALL {
        for sq in pos.piece_bb(Color::White, pt) {
            add_term(m::MATERIAL + pt.index(), 1, t, &mut mg, &mut eg);
            add_term(PST[pt.index()] + (sq.index() ^ 56), 1, t, &mut mg, &mut eg);
        }
        for sq in pos.piece_bb(Color::Black, pt) {
            add_term(m::MATERIAL + pt.index(), -1, t, &mut mg, &mut eg);
            add_term(PST[pt.index()] + sq.index(), -1, t, &mut mg, &mut eg);
        }
    }
    // T2 pawn-structure terms append here; T3 mobility; T4 king safety; T5 threats
    (mg, eg)
}

/// Blend by phase and flip to side-to-move-relative.
pub fn evaluate_white_relative(pos: &Position) -> i32 {
    let (mg, eg) = eval_terms(pos, &mut NullTracer);
    let ph = phase(pos);
    (mg * ph + eg * (24 - ph)) / 24
}

#[derive(Default)]
pub struct Hce;

impl Hce {
    pub fn new() -> Hce {
        Hce
    }
}

impl Evaluator for Hce {
    fn refresh(&mut self, _pos: &Position) {}
    fn on_make(&mut self, _mv: Move, _pos: &Position) {}
    fn on_unmake(&mut self, _mv: Move, _pos: &Position) {}

    fn evaluate(&mut self, pos: &Position) -> i32 {
        let white = evaluate_white_relative(pos);
        if pos.stm() == Color::White {
            white
        } else {
            -white
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Position;
    use crate::eval::eval_params::PARAMS;
    use crate::eval::manifest::TOTAL_PAIRS;

    #[test]
    fn params_len_matches_total_pairs() {
        assert_eq!(
            PARAMS.len(),
            TOTAL_PAIRS,
            "eval_params.rs length {} doesn't match manifest TOTAL_PAIRS {}",
            PARAMS.len(),
            TOTAL_PAIRS
        );
    }

    #[test]
    fn phase_startpos_is_24() {
        let pos = Position::startpos();
        assert_eq!(phase(&pos), 24, "startpos has full 24-point phase");
    }

    #[test]
    fn phase_bare_kings_is_zero() {
        // Bare kings: both kings only, no other pieces
        let fen = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(phase(&pos), 0, "bare kings have 0 phase");
    }

    #[test]
    fn startpos_is_balanced() {
        let mut e = Hce::new();
        let pos = Position::startpos();
        assert_eq!(e.evaluate(&pos), 0, "symmetric position must be 0");
    }

    #[test]
    fn eval_is_stm_relative() {
        // same physical position, both side-to-move variants: scores negate
        // NOTE: since mg==eg in the seed (no tapering divergence), the eval is
        // phase-independent, so stm negation holds exactly at this seed stage.
        let w = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1";
        let b = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let mut e = Hce::new();
        let sw = e.evaluate(&Position::from_fen(w).unwrap());
        let sb = e.evaluate(&Position::from_fen(b).unwrap());
        assert_eq!(sw, -sb);
        // e2->e4 is a PST improvement for White
        assert!(sw > 0, "White improved by e4, White to move: positive");
    }

    #[test]
    fn material_dominates_pst() {
        // White is a clean knight up; score from White's view >> 200cp
        let fen = "rnbqkb1r/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let mut e = Hce::new();
        let s = e.evaluate(&Position::from_fen(fen).unwrap());
        assert!(s > 200, "knight-up should exceed 200cp, got {s}");
        assert!(s < 500, "but not exceed knight+max-pst, got {s}");
    }

    #[test]
    fn hooks_are_callable_noops() {
        // the seam contract: search calls these unconditionally from M2 on
        let mut e = Hce::new();
        let mut pos = Position::startpos();
        e.refresh(&pos);
        let before = e.evaluate(&pos);
        let mv = crate::board::movegen::find_uci_move(&pos, "e2e4").unwrap();
        assert!(pos.make(mv));
        e.on_make(mv, &pos);
        pos.unmake();
        e.on_unmake(mv, &pos);
        assert_eq!(e.evaluate(&pos), before, "no-op hooks don't corrupt eval");
    }

    #[test]
    fn mirrored_position_negates() {
        // asymmetric position and its color-flipped mirror: stm-relative
        // scores must be equal (White's edge becomes Black's edge).
        let orig = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 1";
        let flip = "rnbqk2r/pppp1ppp/5n2/2b1p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 0 1";
        let mut e = Hce::new();
        let a = e.evaluate(&Position::from_fen(orig).unwrap());
        let b = e.evaluate(&Position::from_fen(flip).unwrap());
        assert_eq!(a, b, "color-flip symmetry violated: {a} vs {b}");
    }
}
