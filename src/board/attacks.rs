//! All attack computation lives here. Leapers: const-built lookup tables.
//! Sliders: magic bitboards (Task 7).

use crate::board::{Bitboard, Color, Square};

const FILE_A: u64 = 0x0101010101010101;
const FILE_B: u64 = FILE_A << 1;
const FILE_G: u64 = FILE_A << 6;
const FILE_H: u64 = FILE_A << 7;

const fn build_knight() -> [u64; 64] {
    let mut t = [0u64; 64];
    let mut i = 0;
    while i < 64 {
        let b = 1u64 << i;
        t[i] = ((b << 17) & !FILE_A)
            | ((b << 15) & !FILE_H)
            | ((b << 10) & !(FILE_A | FILE_B))
            | ((b << 6) & !(FILE_G | FILE_H))
            | ((b >> 17) & !FILE_H)
            | ((b >> 15) & !FILE_A)
            | ((b >> 10) & !(FILE_G | FILE_H))
            | ((b >> 6) & !(FILE_A | FILE_B));
        i += 1;
    }
    t
}

const fn build_king() -> [u64; 64] {
    let mut t = [0u64; 64];
    let mut i = 0;
    while i < 64 {
        let b = 1u64 << i;
        let horiz = ((b << 1) & !FILE_A) | ((b >> 1) & !FILE_H);
        let row = b | horiz;
        t[i] = (horiz | (row << 8) | (row >> 8)) & !b;
        i += 1;
    }
    t
}

const fn build_pawn() -> [[u64; 64]; 2] {
    let mut t = [[0u64; 64]; 2];
    let mut i = 0;
    while i < 64 {
        let b = 1u64 << i;
        t[0][i] = ((b << 9) & !FILE_A) | ((b << 7) & !FILE_H); // white
        t[1][i] = ((b >> 7) & !FILE_A) | ((b >> 9) & !FILE_H); // black
        i += 1;
    }
    t
}

static KNIGHT: [u64; 64] = build_knight();
static KING: [u64; 64] = build_king();
static PAWN: [[u64; 64]; 2] = build_pawn();

#[inline]
pub fn knight_attacks(sq: Square) -> Bitboard {
    Bitboard(KNIGHT[sq.index()])
}

#[inline]
pub fn king_attacks(sq: Square) -> Bitboard {
    Bitboard(KING[sq.index()])
}

/// Squares a pawn of `color` ON `sq` attacks.
#[inline]
pub fn pawn_attacks(color: Color, sq: Square) -> Bitboard {
    Bitboard(PAWN[color.index()][sq.index()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Bitboard, Color, Square};

    fn bb_of(names: &[&str]) -> Bitboard {
        let mut bb = Bitboard::EMPTY;
        for n in names {
            bb.set(Square::from_name(n).unwrap());
        }
        bb
    }

    #[test]
    fn knight_attack_patterns() {
        assert_eq!(
            knight_attacks(Square::from_name("d4").unwrap()),
            bb_of(&["b3", "b5", "c2", "c6", "e2", "e6", "f3", "f5"])
        );
        // corner: no wrap-around
        assert_eq!(knight_attacks(Square::A1), bb_of(&["b3", "c2"]));
        assert_eq!(knight_attacks(Square::H8), bb_of(&["f7", "g6"]));
    }

    #[test]
    fn king_attack_patterns() {
        assert_eq!(
            king_attacks(Square::E1),
            bb_of(&["d1", "d2", "e2", "f1", "f2"])
        );
        assert_eq!(king_attacks(Square::A8), bb_of(&["a7", "b7", "b8"]));
    }

    #[test]
    fn pawn_attack_patterns() {
        let e4 = Square::from_name("e4").unwrap();
        assert_eq!(pawn_attacks(Color::White, e4), bb_of(&["d5", "f5"]));
        assert_eq!(pawn_attacks(Color::Black, e4), bb_of(&["d3", "f3"]));
        // edges don't wrap
        let a2 = Square::from_name("a2").unwrap();
        assert_eq!(pawn_attacks(Color::White, a2), bb_of(&["b3"]));
        let h7 = Square::from_name("h7").unwrap();
        assert_eq!(pawn_attacks(Color::Black, h7), bb_of(&["g6"]));
    }
}
