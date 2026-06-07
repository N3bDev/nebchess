//! Static Exchange Evaluation (swap algorithm). Returns the material outcome
//! in centipawns (SEE-local values, decoupled from the tuned eval) of the
//! capture sequence on `mv.to()`, assuming both sides capture with their
//! least valuable attacker and stop when continuing loses material.
//!
//! Approximations (documented, standard at this level):
//! - promotions during the exchange are not modeled (the moved piece keeps its
//!   value);
//! - pins are ignored (a pinned defender still "defends").
//!
//! SEE values are SEE-local consts, decoupled from the tuned eval — same
//! rationale as the pinned ordering values in `mod.rs` (a retune must not
//! silently reshape SEE pruning).

use crate::board::attacks;
use crate::board::{Bitboard, Color, Move, PieceType, Position, Square};

const SEE_VALS: [i32; 6] = [100, 320, 330, 500, 900, 20_000];

/// All pieces of BOTH colors attacking `sq` under occupancy `occ`. Sliders are
/// resolved against `occ`, so removing a front piece between calls reveals the
/// x-ray attacker behind it.
fn attackers_to(pos: &Position, sq: Square, occ: Bitboard) -> Bitboard {
    // Pawn attackers: white pawns attack `sq` iff they sit on the squares a
    // black pawn on `sq` would attack, and vice versa.
    (attacks::pawn_attacks(Color::Black, sq) & pos.piece_bb(Color::White, PieceType::Pawn))
        | (attacks::pawn_attacks(Color::White, sq) & pos.piece_bb(Color::Black, PieceType::Pawn))
        | (attacks::knight_attacks(sq)
            & (pos.piece_bb(Color::White, PieceType::Knight)
                | pos.piece_bb(Color::Black, PieceType::Knight)))
        | (attacks::king_attacks(sq)
            & (pos.piece_bb(Color::White, PieceType::King)
                | pos.piece_bb(Color::Black, PieceType::King)))
        | (attacks::bishop_attacks(sq, occ)
            & (pos.piece_bb(Color::White, PieceType::Bishop)
                | pos.piece_bb(Color::Black, PieceType::Bishop)
                | pos.piece_bb(Color::White, PieceType::Queen)
                | pos.piece_bb(Color::Black, PieceType::Queen)))
        | (attacks::rook_attacks(sq, occ)
            & (pos.piece_bb(Color::White, PieceType::Rook)
                | pos.piece_bb(Color::Black, PieceType::Rook)
                | pos.piece_bb(Color::White, PieceType::Queen)
                | pos.piece_bb(Color::Black, PieceType::Queen)))
}

/// Static exchange evaluation of the capture `mv`, from the side-to-move's
/// perspective. Positive = the capture sequence wins material.
pub fn see(pos: &Position, mv: Move) -> i32 {
    let to = mv.to();
    let from = mv.from();
    let is_ep = mv.flag() == Move::EN_PASSANT;
    let mover = pos
        .piece_on(from)
        .expect("see: no piece on from-square")
        .piece_type();

    let mut gain = [0i32; 32];
    // Victim value (ep: the captured pawn is NOT on `to`).
    gain[0] = if is_ep {
        SEE_VALS[PieceType::Pawn.index()]
    } else {
        match pos.piece_on(to) {
            Some(p) => SEE_VALS[p.piece_type().index()],
            None => 0, // quiet move: the exchange starts with our piece exposed
        }
    };

    let mut occ = pos.occ_all() ^ from.bb(); // mover leaves its square
    if is_ep {
        // Captured pawn sits on the from-rank, to-file (matches make()).
        let cap_sq = Square::from_fr(to.file(), from.rank());
        occ ^= cap_sq.bb();
    }
    let mut attackers = attackers_to(pos, to, occ) & occ;
    let mut stm = pos.stm().flip();
    let mut victim_val = SEE_VALS[mover.index()]; // piece now on `to`, captured next
    let mut depth = 0usize;

    loop {
        let my_attackers = attackers & pos.occ(stm) & occ;
        if my_attackers.is_empty() {
            break;
        }
        // least valuable attacker of the side to move
        let mut lva = None;
        for pt in PieceType::ALL {
            let s = my_attackers & pos.piece_bb(stm, pt);
            if Bitboard::any(s) {
                lva = Some((s.lsb(), pt));
                break;
            }
        }
        let (sq, pt) = lva.expect("my_attackers non-empty => an LVA exists");
        depth += 1;
        gain[depth] = victim_val - gain[depth - 1];
        // If our best achievable result here already loses relative to standing
        // pat at the previous ply, neither side gains by continuing.
        if gain[depth].max(-gain[depth - 1]) < 0 {
            break;
        }
        victim_val = SEE_VALS[pt.index()];
        occ ^= sq.bb();
        // slider x-rays revealed by the capturer's departure
        attackers |= attackers_to(pos, to, occ);
        attackers &= occ;
        stm = stm.flip();
    }
    // negamax fold-back: each side, walking outward, may decline the recapture
    while depth > 0 {
        gain[depth - 1] = -((-gain[depth - 1]).max(gain[depth]));
        depth -= 1;
    }
    gain[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::movegen::find_uci_move;

    // Values: pawn 100, knight 320, bishop 330, rook 500, queen 900 (SEE-local,
    // decoupled from eval — same rationale as the pinned ordering VICTIM_VALS).
    #[test]
    fn pawn_takes_defended_pawn_is_zero() {
        // e4xd5, d5 defended by c6 pawn: PxP, PxP -> 100 - 100 = 0
        let pos = Position::from_fen("4k3/8/2p5/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
        let mv = find_uci_move(&pos, "e4d5").unwrap();
        assert_eq!(see(&pos, mv), 0);
    }

    #[test]
    fn rook_takes_defended_pawn_loses_exchange() {
        // Rxd5 (pawn), d5 defended by e6 pawn: +100 - 500 = -400
        let pos = Position::from_fen("4k3/8/4p3/3p4/8/8/8/3RK3 w - - 0 1").unwrap();
        let mv = find_uci_move(&pos, "d1d5").unwrap();
        assert_eq!(see(&pos, mv), -400);
    }

    #[test]
    fn xray_battery_loses_eighty() {
        // R+R battery (Rd3 mover, Rd2 x-ray behind on the d-file) vs a single
        // knight defender on f6 (f6 DEFENDS d5 — the plan's e6 does NOT; see
        // the implementer's re-derivation note). Swap:
        //   Rxd5 (+100), Nxd5 reveals the second rook, Rxd5 (+320).
        //   gain = [100], victim_val=500(rook on d5)
        //   d1: Nf6 takes rook -> gain[1] = 500 - 100 = 400, victim_val=320
        //   d2: Rd2 takes knight -> gain[2] = 320 - 400 = -80 -> prune+stop
        //   fold: gain[1] = -max(-400,-80) = 80 ; gain[0] = -max(-100,80) = -80
        let pos = Position::from_fen("4k3/8/5n2/3p4/8/3R4/3R4/4K3 w - - 0 1").unwrap();
        let mv = find_uci_move(&pos, "d3d5").unwrap();
        assert_eq!(see(&pos, mv), -80);
    }

    #[test]
    fn en_passant_capture_sees_pawn() {
        // ep: captured pawn is NOT on the to-square; undefended -> +100
        let pos = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").unwrap();
        let mv = find_uci_move(&pos, "e5d6").unwrap();
        assert_eq!(see(&pos, mv), 100);
    }

    #[test]
    fn queen_grabs_poisoned_pawn() {
        // Qxd5 defended by a knight on f6 (again: f6 defends d5, the plan's e6
        // does not). Qxd5 (+100), Nxd5 wins the queen: fold -> -800.
        let pos = Position::from_fen("4k3/8/5n2/3p4/8/8/8/3QK3 w - - 0 1").unwrap();
        let mv = find_uci_move(&pos, "d1d5").unwrap();
        assert_eq!(see(&pos, mv), -800);
    }
}
