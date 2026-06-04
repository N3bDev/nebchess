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
        t[i] =
            // +2r+1f (wrap H->A: strip A)
            ((b << 17) & !FILE_A)
            // +2r-1f (wrap A->H: strip H)
            | ((b << 15) & !FILE_H)
            // +1r+2f (double-wrap H: strip A,B)
            | ((b << 10) & !(FILE_A | FILE_B))
            // +1r-2f (double-wrap A: strip G,H)
            | ((b << 6) & !(FILE_G | FILE_H))
            // -2r-1f (wrap A->H: strip H)
            | ((b >> 17) & !FILE_H)
            // -2r+1f (wrap H->A: strip A)
            | ((b >> 15) & !FILE_A)
            // -1r-2f (double-wrap A: strip G,H)
            | ((b >> 10) & !(FILE_G | FILE_H))
            // -1r+2f (double-wrap H: strip A,B)
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

pub const ROOK_DELTAS: [(i8, i8); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];
pub const BISHOP_DELTAS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Reference slider attack generation by ray walking. Slow; used only by the
/// magic finder, table construction, and tests — never in the search.
pub fn sliding_attacks_slow(sq: Square, occ: Bitboard, deltas: &[(i8, i8); 4]) -> Bitboard {
    let mut attacks = Bitboard::EMPTY;
    for &(df, dr) in deltas {
        let (mut f, mut r) = (sq.file() as i8 + df, sq.rank() as i8 + dr);
        while (0..8).contains(&f) && (0..8).contains(&r) {
            let s = Square::from_fr(f as u8, r as u8);
            attacks.set(s);
            if occ.contains(s) {
                break; // blocker square included; ray stops behind it
            }
            f += df;
            r += dr;
        }
    }
    attacks
}

/// Relevant-occupancy mask: ray squares whose occupancy can change the attack
/// set — i.e. every ray square except the final edge square of each ray.
pub fn relevant_mask(sq: Square, deltas: &[(i8, i8); 4]) -> Bitboard {
    let mut mask = Bitboard::EMPTY;
    for &(df, dr) in deltas {
        let (mut f, mut r) = (sq.file() as i8 + df, sq.rank() as i8 + dr);
        // include the square only if the NEXT step is still on the board
        while (0..8).contains(&(f + df)) && (0..8).contains(&(r + dr)) {
            mask.set(Square::from_fr(f as u8, r as u8));
            f += df;
            r += dr;
        }
    }
    mask
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

    #[test]
    fn leaper_popcounts_exhaustive() {
        // popcount distribution catches any future mask typo
        for i in 0..64u8 {
            let sq = Square::new(i);
            let (f, r) = (i % 8, i / 8);
            let edge_dist = |x: u8| x.min(7 - x);
            let (fd, rd) = (edge_dist(f), edge_dist(r));
            let expected_knight = match (fd.min(rd), fd.max(rd)) {
                (0, 0) => 2,
                (0, 1) => 3,
                (0, _) => 4,
                (1, 1) => 4,
                (1, _) => 6,
                _ => 8,
            };
            assert_eq!(knight_attacks(sq).count(), expected_knight, "knight {sq}");
            let expected_king = match (fd.min(rd), fd.max(rd)) {
                (0, 0) => 3,
                (0, _) => 5,
                _ => 8,
            };
            assert_eq!(king_attacks(sq).count(), expected_king, "king {sq}");
        }
    }

    #[test]
    fn slow_rook_rays() {
        // empty board from a1: full file + rank minus a1 itself = 14 squares
        let a = sliding_attacks_slow(Square::A1, Bitboard::EMPTY, &ROOK_DELTAS);
        assert_eq!(a.count(), 14);
        // blocker on d4 stops the ray (blocker square included, beyond excluded)
        let d4 = Square::from_name("d4").unwrap();
        let d1 = Square::D1;
        let occ = d4.bb();
        let a = sliding_attacks_slow(d1, occ, &ROOK_DELTAS);
        assert!(a.contains(d4));
        assert!(!a.contains(Square::from_name("d5").unwrap()));
        assert!(a.contains(Square::from_name("d2").unwrap()));
        assert!(a.contains(Square::A1) && a.contains(Square::H1));
    }

    #[test]
    fn slow_bishop_rays() {
        let d4 = Square::from_name("d4").unwrap();
        let a = sliding_attacks_slow(d4, Bitboard::EMPTY, &BISHOP_DELTAS);
        assert_eq!(a.count(), 13);
        let occ = bb_of(&["f6"]);
        let a = sliding_attacks_slow(d4, occ, &BISHOP_DELTAS);
        assert!(a.contains(Square::from_name("f6").unwrap()));
        assert!(!a.contains(Square::from_name("g7").unwrap()));
    }

    #[test]
    fn relevant_mask_bit_counts() {
        // Known values: rook a1 = 12 relevant bits, rook d4 = 10, bishop d4 = 9, bishop a1 = 6
        assert_eq!(relevant_mask(Square::A1, &ROOK_DELTAS).count(), 12);
        let d4 = Square::from_name("d4").unwrap();
        assert_eq!(relevant_mask(d4, &ROOK_DELTAS).count(), 10);
        assert_eq!(relevant_mask(d4, &BISHOP_DELTAS).count(), 9);
        assert_eq!(relevant_mask(Square::A1, &BISHOP_DELTAS).count(), 6);
    }
}
