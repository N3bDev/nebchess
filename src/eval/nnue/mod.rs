pub mod accumulator;
pub mod net;

use accumulator::AccPair;
use net::Network;

use crate::board::{Move, Position};
use crate::board::types::{Color, Piece, PieceType};
use crate::eval::Evaluator;
use crate::search::MAX_PLY;

pub struct NnueEvaluator {
    net: Box<Network>,
    stack: Box<[AccPair]>, // length MAX_PLY + 1
    top: usize,
}

impl NnueEvaluator {
    pub fn from_bytes(bytes: &[u8]) -> NnueEvaluator {
        let net = Network::from_bytes(bytes);
        let stack = vec![AccPair::fresh(&net); MAX_PLY + 1].into_boxed_slice();
        NnueEvaluator { net, stack, top: 0 }
    }
}

impl Evaluator for NnueEvaluator {
    fn refresh(&mut self, pos: &Position) {
        self.top = 0;
        let net = &self.net;
        let acc = &mut self.stack[0];
        *acc = AccPair::fresh(net);
        for color in [Color::White, Color::Black] {
            for pt in [PieceType::Pawn, PieceType::Knight, PieceType::Bishop,
                       PieceType::Rook, PieceType::Queen, PieceType::King] {
                for sq in pos.piece_bb(color, pt) {
                    acc.add(net, Piece::new(color, pt), sq);
                }
            }
        }
    }

    fn on_make(&mut self, _mv: Move, _pos: &Position) { /* Task 5 */ }
    fn on_unmake(&mut self, _mv: Move, _pos: &Position) { /* Task 5 */ }

    fn evaluate(&mut self, pos: &Position) -> i32 {
        let acc = &self.stack[self.top];
        let (us, them) = match pos.stm() {
            Color::White => (&acc.white, &acc.black),
            Color::Black => (&acc.black, &acc.white),
        };
        self.net.out(us, them)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::Piece;

    const TOY: &str = "tools/trainer/checkpoints/toy-5/quantised.bin";

    // Naive, independent eval: build both halves from the board, run the forward.
    fn naive_eval(net: &Network, pos: &Position) -> i32 {
        let mut acc = AccPair::fresh(net);
        for color in [Color::White, Color::Black] {
            for pt in [PieceType::Pawn, PieceType::Knight, PieceType::Bishop,
                       PieceType::Rook, PieceType::Queen, PieceType::King] {
                for sq in pos.piece_bb(color, pt) { acc.add(net, Piece::new(color, pt), sq); }
            }
        }
        let (us, them) = match pos.stm() {
            Color::White => (&acc.white, &acc.black),
            Color::Black => (&acc.black, &acc.white),
        };
        net.out(us, them)
    }

    const FENS: &[&str] = &[
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
        "8/2k5/8/8/8/5K2/6Q1/8 b - - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ];

    #[test]
    fn refresh_eval_matches_naive() {
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut e = NnueEvaluator::from_bytes(&bytes);
        for fen in FENS {
            let pos = Position::from_fen(fen).unwrap();
            e.refresh(&pos);
            assert_eq!(e.evaluate(&pos), naive_eval(&e.net, &pos), "fen {fen}");
        }
    }

    #[test]
    fn material_edge_has_sane_sign() {
        // White is up a full QUEEN (black's d8 queen removed). Sign must track side-to-move.
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut e = NnueEvaluator::from_bytes(&bytes);
        let pos_w = Position::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        e.refresh(&pos_w);
        let white_to_move = e.evaluate(&pos_w);
        let pos_b = Position::from_fen("rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();
        e.refresh(&pos_b);
        let black_to_move = e.evaluate(&pos_b);
        assert!(white_to_move > 0, "White up a queen, White to move -> positive (got {white_to_move})");
        assert!(black_to_move < 0, "White up a queen, Black to move -> negative (got {black_to_move})");
    }
}
