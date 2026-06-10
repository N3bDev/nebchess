pub mod accumulator;
pub mod net;

use accumulator::AccPair;
use net::Network;

use crate::board::{Move, Position};
use crate::board::types::{Color, Piece, PieceType, Square};
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

    /// The shipped network, embedded at compile time.
    pub fn embedded() -> NnueEvaluator {
        NnueEvaluator::from_bytes(include_bytes!("net2.bin"))
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

    fn on_make(&mut self, mv: Move, pos: &Position) {
        // pos is AFTER pos.make(mv). Push a copy of the current accumulator, then apply deltas.
        self.top += 1;
        self.stack[self.top] = self.stack[self.top - 1];
        let net = &self.net;
        let acc = &mut self.stack[self.top];

        let from = mv.from();
        let to = mv.to();
        let moved = pos.piece_on(to).expect("a piece on the destination after make");

        if mv.is_promotion() {
            let pawn = Piece::new(moved.color(), PieceType::Pawn);
            acc.sub(net, pawn, from);
            acc.add(net, moved, to); // moved == the promoted piece
            if mv.is_capture() {
                let cap = pos.undo_stack.last().expect("undo entry").captured.expect("captured piece");
                acc.sub(net, cap, to);
            }
        } else if mv.flag() == Move::KING_CASTLE || mv.flag() == Move::QUEEN_CASTLE {
            acc.sub(net, moved, from);
            acc.add(net, moved, to);
            let (rf, rt) = castle_rook_squares(to);
            let rook = Piece::new(moved.color(), PieceType::Rook);
            acc.sub(net, rook, rf);
            acc.add(net, rook, rt);
        } else if mv.flag() == Move::EN_PASSANT {
            acc.sub(net, moved, from);
            acc.add(net, moved, to);
            let cap_sq = Square::from_fr(to.file(), from.rank());
            let cap_pawn = Piece::new(moved.color().flip(), PieceType::Pawn);
            acc.sub(net, cap_pawn, cap_sq);
        } else {
            // quiet or normal capture
            acc.sub(net, moved, from);
            acc.add(net, moved, to);
            if mv.is_capture() {
                let cap = pos.undo_stack.last().expect("undo entry").captured.expect("captured piece");
                acc.sub(net, cap, to);
            }
        }
    }

    fn on_unmake(&mut self, _mv: Move, _pos: &Position) {
        self.top -= 1;
    }

    fn evaluate(&mut self, pos: &Position) -> i32 {
        let acc = &self.stack[self.top];
        let (us, them) = match pos.stm() {
            Color::White => (&acc.white, &acc.black),
            Color::Black => (&acc.black, &acc.white),
        };
        self.net.out(us, them)
    }
}

/// Rook from/to squares for a castling move, given the king's destination square.
fn castle_rook_squares(king_to: Square) -> (Square, Square) {
    match king_to.index() {
        6  => (Square::new(7),  Square::new(5)),   // e1g1: h1->f1
        2  => (Square::new(0),  Square::new(3)),   // e1c1: a1->d1
        62 => (Square::new(63), Square::new(61)),  // e8g8: h8->f8
        58 => (Square::new(56), Square::new(59)),  // e8c8: a8->d8
        _  => unreachable!("castle king_to must be c1/g1/c8/g8"),
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

    use crate::board::{generate_moves, MoveList};

    #[test]
    fn incremental_matches_refresh() {
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut inc = NnueEvaluator::from_bytes(&bytes); // updated via on_make
        let mut chk = NnueEvaluator::from_bytes(&bytes); // refreshed each step
        let mut pos = Position::startpos();
        inc.refresh(&pos);
        let mut s = 0xC0FFEEu64;
        for _ in 0..60 {
            let mut list = MoveList::new();
            generate_moves(&pos, &mut list);
            let mut legal = Vec::new();
            for &m in list.iter() { if pos.make(m) { pos.unmake(); legal.push(m); } }
            if legal.is_empty() { break; }
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let mv = legal[(s as usize) % legal.len()];
            pos.make(mv);
            inc.on_make(mv, &pos);
            chk.refresh(&pos);
            assert_eq!(inc.evaluate(&pos), chk.evaluate(&pos),
                       "incremental != refresh after {}", mv);
        }
    }

    // Find the first LEGAL move from `pos` matching `pick` (pos is left unchanged).
    fn target(pos: &mut Position, pick: impl Fn(Move) -> bool) -> Move {
        let mut list = MoveList::new();
        generate_moves(pos, &mut list);
        for &m in list.iter() {
            if pos.make(m) {
                pos.unmake();
                if pick(m) { return m; }
            }
        }
        panic!("no matching legal move in this position");
    }

    // Refresh, make the picked move + on_make, and assert incremental == a fresh refresh.
    fn check_inc_eq_refresh(fen: &str, pick: impl Fn(Move) -> bool) {
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut inc = NnueEvaluator::from_bytes(&bytes);
        let mut chk = NnueEvaluator::from_bytes(&bytes);
        let mut pos = Position::from_fen(fen).unwrap();
        let mv = target(&mut pos, pick);
        inc.refresh(&pos);          // accumulator for the pre-move position
        pos.make(mv);
        inc.on_make(mv, &pos);      // incremental update
        chk.refresh(&pos);          // from-scratch on the post-move position
        assert_eq!(inc.evaluate(&pos), chk.evaluate(&pos), "inc != refresh after {:?} from {fen}", mv);
    }

    #[test]
    fn incremental_promotion() {
        check_inc_eq_refresh("k7/4P3/8/8/8/8/8/4K3 w - - 0 1", |m| m.is_promotion() && !m.is_capture());
    }
    #[test]
    fn incremental_promotion_capture() {
        check_inc_eq_refresh("1n2k3/P7/8/8/8/8/8/4K3 w - - 0 1", |m| m.is_promotion() && m.is_capture());
    }
    #[test]
    fn incremental_en_passant() {
        check_inc_eq_refresh("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", |m| m.flag() == Move::EN_PASSANT);
    }
    #[test]
    fn incremental_castle_kingside() {
        check_inc_eq_refresh("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", |m| m.flag() == Move::KING_CASTLE);
    }
    #[test]
    fn incremental_castle_queenside() {
        check_inc_eq_refresh("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", |m| m.flag() == Move::QUEEN_CASTLE);
    }

    #[test]
    fn null_move_needs_no_handling() {
        // Search makes null moves WITHOUT calling eval hooks. For a plain-768 net a null changes
        // no piece-square feature, so the accumulator at `top` stays valid and evaluate() reads
        // the flipped stm. Prove: incremental-after-null == refresh-on-null-position.
        let Ok(bytes) = std::fs::read(TOY) else { return };
        let mut inc = NnueEvaluator::from_bytes(&bytes);
        let mut chk = NnueEvaluator::from_bytes(&bytes);
        let mut pos = Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3").unwrap();
        inc.refresh(&pos);
        pos.make_null();          // no eval hook, mirroring the search
        chk.refresh(&pos);
        assert_eq!(inc.evaluate(&pos), chk.evaluate(&pos), "accumulator wrong across a null move");
        pos.unmake_null();
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
