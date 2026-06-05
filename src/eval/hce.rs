//! M2 hand-crafted eval: material + single-phase PSTs. Full-scan evaluate;
//! the trait hooks are no-ops (NNUE will use them; incremental PST tracking
//! is a possible M3+ optimization, deliberately not done yet — YAGNI).

use crate::board::{Color, Move, PieceType, Position};
use crate::eval::psqt::{MATERIAL, TABLES};
use crate::eval::Evaluator;

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
        let mut score = 0i32; // White-relative accumulation
        for pt in PieceType::ALL {
            let val = MATERIAL[pt.index()];
            let table = TABLES[pt.index()];
            for sq in pos.piece_bb(Color::White, pt) {
                score += val + table[sq.index() ^ 56];
            }
            for sq in pos.piece_bb(Color::Black, pt) {
                score -= val + table[sq.index()];
            }
        }
        if pos.stm() == Color::White {
            score
        } else {
            -score
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Position;

    #[test]
    fn startpos_is_balanced() {
        let mut e = Hce::new();
        let pos = Position::startpos();
        assert_eq!(e.evaluate(&pos), 0, "symmetric position must be 0");
    }

    #[test]
    fn eval_is_stm_relative() {
        // same physical position, both side-to-move variants: scores negate
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
        assert!(s < 450, "but not exceed knight+max-pst, got {s}");
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
