//! Pseudo-legal move generation. Castling is generated fully legal;
//! everything else is filtered by Position::make().

use crate::board::attacks;
use crate::board::{Bitboard, CastlingRights, Color, Move, MoveList, PieceType, Position, Square};

/// Appends pseudo-legal moves for the side to move to `list`.
/// Castling moves are fully legal (clear path + unattacked transit verified);
/// all other moves may leave the king in check — callers filter via `Position::make`.
pub fn generate_moves(pos: &Position, list: &mut MoveList) {
    let stm = pos.stm();
    let own = pos.occ(stm);
    let enemy = pos.occ(stm.flip());
    let occ = pos.occ_all();

    pawn_moves(pos, list, stm, enemy, occ);

    for from in pos.piece_bb(stm, PieceType::Knight) {
        push_targets(list, from, attacks::knight_attacks(from) & !own, enemy);
    }
    for from in pos.piece_bb(stm, PieceType::Bishop) {
        push_targets(list, from, attacks::bishop_attacks(from, occ) & !own, enemy);
    }
    for from in pos.piece_bb(stm, PieceType::Rook) {
        push_targets(list, from, attacks::rook_attacks(from, occ) & !own, enemy);
    }
    for from in pos.piece_bb(stm, PieceType::Queen) {
        push_targets(list, from, attacks::queen_attacks(from, occ) & !own, enemy);
    }
    let king = pos.king_sq(stm);
    push_targets(list, king, attacks::king_attacks(king) & !own, enemy);

    castling_moves(pos, list, stm, occ);
}

#[inline]
fn push_targets(list: &mut MoveList, from: Square, targets: Bitboard, enemy: Bitboard) {
    for to in targets {
        let flag = if enemy.contains(to) {
            Move::CAPTURE
        } else {
            Move::QUIET
        };
        list.push(Move::new(from, to, flag));
    }
}

fn pawn_moves(pos: &Position, list: &mut MoveList, stm: Color, enemy: Bitboard, occ: Bitboard) {
    // ranks are from-square ranks
    let (up, start_rank, promo_rank): (i8, u8, u8) = match stm {
        Color::White => (8, 1, 6),
        Color::Black => (-8, 6, 1),
    };

    for from in pos.piece_bb(stm, PieceType::Pawn) {
        let rank = from.rank();
        let one = Square::new((from.index() as i8 + up) as u8);

        // pushes
        if !occ.contains(one) {
            if rank == promo_rank {
                for flag in [Move::PROMO_Q, Move::PROMO_R, Move::PROMO_B, Move::PROMO_N] {
                    list.push(Move::new(from, one, flag));
                }
            } else {
                list.push(Move::new(from, one, Move::QUIET));
                if rank == start_rank {
                    let two = Square::new((one.index() as i8 + up) as u8);
                    if !occ.contains(two) {
                        list.push(Move::new(from, two, Move::DOUBLE_PUSH));
                    }
                }
            }
        }

        // captures
        let att = attacks::pawn_attacks(stm, from);
        for to in att & enemy {
            if rank == promo_rank {
                for flag in [
                    Move::PROMO_CAP_Q,
                    Move::PROMO_CAP_R,
                    Move::PROMO_CAP_B,
                    Move::PROMO_CAP_N,
                ] {
                    list.push(Move::new(from, to, flag));
                }
            } else {
                list.push(Move::new(from, to, Move::CAPTURE));
            }
        }

        // en passant (Position guarantees ep is Some only when capturable)
        if let Some(ep) = pos.ep() {
            if att.contains(ep) {
                list.push(Move::new(from, ep, Move::EN_PASSANT));
            }
        }
    }
}

fn castling_moves(pos: &Position, list: &mut MoveList, stm: Color, occ: Bitboard) {
    let enemy = stm.flip();
    let (ks_right, qs_right, e, f, g, d, c, b) = match stm {
        Color::White => (
            CastlingRights::WK,
            CastlingRights::WQ,
            Square::E1,
            Square::F1,
            Square::G1,
            Square::D1,
            Square::C1,
            Square::new(1), // b1
        ),
        Color::Black => (
            CastlingRights::BK,
            CastlingRights::BQ,
            Square::E8,
            Square::F8,
            Square::G8,
            Square::D8,
            Square::C8,
            Square::new(57), // b8
        ),
    };

    if pos.castling().has(ks_right)
        && !occ.contains(f)
        && !occ.contains(g)
        && !pos.square_attacked(e, enemy)
        && !pos.square_attacked(f, enemy)
        && !pos.square_attacked(g, enemy)
    {
        list.push(Move::new(e, g, Move::KING_CASTLE));
    }
    if pos.castling().has(qs_right)
        && !occ.contains(d)
        && !occ.contains(c)
        && !occ.contains(b) // b-file square must be empty but may be attacked
        && !pos.square_attacked(e, enemy)
        && !pos.square_attacked(d, enemy)
        && !pos.square_attacked(c, enemy)
    {
        list.push(Move::new(e, c, Move::QUEEN_CASTLE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Move, Position};

    /// pseudo-legal gen + make/unmake filter = legal move list
    fn legal_moves(fen: &str) -> Vec<Move> {
        let mut pos = Position::from_fen(fen).unwrap();
        let mut list = MoveList::new();
        generate_moves(&pos, &mut list);
        let mut legal = Vec::new();
        for &mv in list.iter() {
            if pos.make(mv) {
                pos.unmake();
                legal.push(mv);
            }
        }
        legal
    }

    #[test]
    fn legal_move_counts_standard_positions() {
        // perft(1) of the six standard suite positions (spec §10.1)
        let cases = [
            (
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                20,
            ),
            (
                "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
                48,
            ),
            ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", 14),
            (
                "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
                6,
            ),
            (
                "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
                44,
            ),
            (
                "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
                46,
            ),
        ];
        for (fen, expected) in cases {
            assert_eq!(legal_moves(fen).len(), expected, "fen: {fen}");
        }
    }

    #[test]
    fn castling_generated_when_legal() {
        let moves = legal_moves("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
        let uci: Vec<String> = moves.iter().map(|m| m.to_string()).collect();
        assert!(uci.contains(&"e1g1".to_string()));
        assert!(uci.contains(&"e1c1".to_string()));
        // flags must be castle flags, not quiet
        assert!(moves
            .iter()
            .any(|m| m.to_string() == "e1g1" && m.flag() == Move::KING_CASTLE));
        assert!(moves
            .iter()
            .any(|m| m.to_string() == "e1c1" && m.flag() == Move::QUEEN_CASTLE));
    }

    #[test]
    fn castling_blocked_through_attacked_square() {
        // black rook on f2 attacks f1: kingside out, queenside still legal
        let moves = legal_moves("r3k2r/8/8/8/8/8/5r2/R3K2R w KQkq - 0 1");
        let uci: Vec<String> = moves.iter().map(|m| m.to_string()).collect();
        assert!(!uci.contains(&"e1g1".to_string()));
        assert!(uci.contains(&"e1c1".to_string()));
    }

    #[test]
    fn castling_not_generated_while_in_check() {
        let moves = legal_moves("r3k2r/8/8/8/8/8/4r3/R3K2R w KQkq - 0 1");
        let uci: Vec<String> = moves.iter().map(|m| m.to_string()).collect();
        assert!(!uci.contains(&"e1g1".to_string()));
        assert!(!uci.contains(&"e1c1".to_string()));
    }

    #[test]
    fn en_passant_generated() {
        let moves = legal_moves("8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1");
        assert!(moves
            .iter()
            .any(|m| m.to_string() == "c4d3" && m.flag() == Move::EN_PASSANT));
    }

    #[test]
    fn promotions_generated_all_four_plus_captures() {
        // pos5: white pawn d7, black bishop c8, black queen d8 (push blocked)
        let moves = legal_moves("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8");
        let promo_caps: Vec<&Move> = moves
            .iter()
            .filter(|m| m.to_string().starts_with("d7c8"))
            .collect();
        assert_eq!(promo_caps.len(), 4, "n/b/r/q capture-promotions on c8");
        assert!(promo_caps
            .iter()
            .all(|m| m.is_promotion() && m.is_capture()));
        assert!(
            !moves.iter().any(|m| m.to_string().starts_with("d7d8")),
            "push blocked by queen"
        );
    }

    #[test]
    fn double_push_blocked_by_occupied_squares() {
        // piece directly in front: neither single nor double push
        let moves = legal_moves("4k3/8/8/8/8/4n3/4P3/4K3 w - - 0 1");
        let uci: Vec<String> = moves.iter().map(|m| m.to_string()).collect();
        assert!(!uci.contains(&"e2e3".to_string()));
        assert!(!uci.contains(&"e2e4".to_string()));
        // piece on the double-push target only: single push ok, double blocked
        let moves = legal_moves("4k3/8/8/8/4n3/8/4P3/4K3 w - - 0 1");
        let uci: Vec<String> = moves.iter().map(|m| m.to_string()).collect();
        assert!(uci.contains(&"e2e3".to_string()));
        assert!(!uci.contains(&"e2e4".to_string()));
    }
}
