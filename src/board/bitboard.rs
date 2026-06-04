//! u64 bitboard, bit 0 = a1 (LERF).

use crate::board::Square;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub struct Bitboard(pub u64);

pub const FILE_A: Bitboard = Bitboard(0x0101010101010101);
pub const FILE_H: Bitboard = Bitboard(0x8080808080808080);
pub const RANK_1: Bitboard = Bitboard(0x00000000000000FF);
pub const RANK_2: Bitboard = Bitboard(0x000000000000FF00);
pub const RANK_4: Bitboard = Bitboard(0x00000000FF000000);
pub const RANK_5: Bitboard = Bitboard(0x000000FF00000000);
pub const RANK_7: Bitboard = Bitboard(0x00FF000000000000);
pub const RANK_8: Bitboard = Bitboard(0xFF00000000000000);

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    #[inline]
    pub const fn any(self) -> bool {
        self.0 != 0
    }
    #[inline]
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }
    #[inline]
    pub const fn contains(self, sq: Square) -> bool {
        self.0 & (1u64 << sq.index()) != 0
    }
    #[inline]
    pub fn set(&mut self, sq: Square) {
        self.0 |= 1u64 << sq.index();
    }
    #[inline]
    pub fn clear(&mut self, sq: Square) {
        self.0 &= !(1u64 << sq.index());
    }
    /// Lowest set square. Caller guarantees non-empty (debug-asserted).
    #[inline]
    pub fn lsb(self) -> Square {
        debug_assert!(self.any());
        Square::new(self.0.trailing_zeros() as u8)
    }
    #[inline]
    pub fn pop_lsb(&mut self) -> Square {
        let sq = self.lsb();
        self.0 &= self.0 - 1;
        sq
    }
    // Directional shifts; file masks stop horizontal wrap-around.
    #[inline]
    pub const fn north(self) -> Bitboard {
        Bitboard(self.0 << 8)
    }
    #[inline]
    pub const fn south(self) -> Bitboard {
        Bitboard(self.0 >> 8)
    }
    #[inline]
    pub const fn east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) << 1)
    }
    #[inline]
    pub const fn west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) >> 1)
    }
    #[inline]
    pub const fn north_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) << 9)
    }
    #[inline]
    pub const fn north_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) << 7)
    }
    #[inline]
    pub const fn south_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) >> 7)
    }
    #[inline]
    pub const fn south_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) >> 9)
    }
}

impl Square {
    #[inline]
    pub const fn bb(self) -> Bitboard {
        Bitboard(1u64 << self.index())
    }
}

macro_rules! impl_bit_op {
    ($trait:ident, $fn:ident, $assign_trait:ident, $assign_fn:ident, $op:tt) => {
        impl std::ops::$trait for Bitboard {
            type Output = Bitboard;
            #[inline]
            fn $fn(self, rhs: Bitboard) -> Bitboard {
                Bitboard(self.0 $op rhs.0)
            }
        }
        impl std::ops::$assign_trait for Bitboard {
            #[inline]
            fn $assign_fn(&mut self, rhs: Bitboard) {
                self.0 = self.0 $op rhs.0;
            }
        }
    };
}
impl_bit_op!(BitAnd, bitand, BitAndAssign, bitand_assign, &);
impl_bit_op!(BitOr, bitor, BitOrAssign, bitor_assign, |);
impl_bit_op!(BitXor, bitxor, BitXorAssign, bitxor_assign, ^);

impl std::ops::Not for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

impl Iterator for Bitboard {
    type Item = Square;
    #[inline]
    fn next(&mut self) -> Option<Square> {
        // UFCS: plain `self.any()` would resolve to Iterator::any(&mut self, predicate)
        Bitboard::any(*self).then(|| self.pop_lsb())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Square;

    #[test]
    fn set_clear_contains() {
        let mut bb = Bitboard::EMPTY;
        assert!(bb.is_empty());
        bb.set(Square::E1);
        assert!(bb.contains(Square::E1));
        assert!(!bb.contains(Square::A1));
        assert_eq!(bb.count(), 1);
        bb.clear(Square::E1);
        assert!(bb.is_empty());
    }

    #[test]
    fn shifts_respect_edges() {
        // a pawn-ish single bit on h4 shifted "east" must vanish, not wrap to a5
        assert_eq!(Square::new(31).bb().east(), Bitboard::EMPTY); // h4
        assert_eq!(Square::new(24).bb().west(), Bitboard::EMPTY); // a4
        assert_eq!(Square::E1.bb().north(), Square::new(12).bb()); // e1 -> e2
        assert_eq!(Square::E1.bb().south(), Bitboard::EMPTY);
        assert_eq!(Square::H8.bb().north(), Bitboard::EMPTY);
    }

    #[test]
    fn pop_lsb_ascending() {
        let mut bb = Square::A1.bb() | Square::E1.bb() | Square::H8.bb();
        assert_eq!(bb.pop_lsb(), Square::A1);
        assert_eq!(bb.pop_lsb(), Square::E1);
        assert_eq!(bb.pop_lsb(), Square::H8);
        assert!(bb.is_empty());
    }

    #[test]
    fn iterator_yields_all() {
        let bb = Bitboard(0x8100000000000081); // corners
        let squares: Vec<Square> = bb.into_iter().collect();
        assert_eq!(
            squares,
            vec![Square::A1, Square::H1, Square::A8, Square::H8]
        );
    }
}
