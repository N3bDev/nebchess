//! 16-bit move encoding: bits 0-5 to, 6-11 from, 12-15 flag.

use crate::board::{PieceType, Square};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Move(u16);

impl Move {
    pub const QUIET: u16 = 0;
    pub const DOUBLE_PUSH: u16 = 1;
    pub const KING_CASTLE: u16 = 2;
    pub const QUEEN_CASTLE: u16 = 3;
    pub const CAPTURE: u16 = 4;
    pub const EN_PASSANT: u16 = 5;
    pub const PROMO_N: u16 = 8;
    pub const PROMO_B: u16 = 9;
    pub const PROMO_R: u16 = 10;
    pub const PROMO_Q: u16 = 11;
    pub const PROMO_CAP_N: u16 = 12;
    pub const PROMO_CAP_B: u16 = 13;
    pub const PROMO_CAP_R: u16 = 14;
    pub const PROMO_CAP_Q: u16 = 15;

    /// All-zero move; never a legal chess move (a1->a1). Used as a sentinel.
    pub const NULL: Move = Move(0);

    #[inline]
    pub const fn new(from: Square, to: Square, flag: u16) -> Move {
        Move((to.index() as u16) | ((from.index() as u16) << 6) | (flag << 12))
    }
    #[inline]
    pub const fn to(self) -> Square {
        Square::new((self.0 & 0x3F) as u8)
    }
    // chess "from-square", not a conversion
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub const fn from(self) -> Square {
        Square::new(((self.0 >> 6) & 0x3F) as u8)
    }
    #[inline]
    pub const fn flag(self) -> u16 {
        self.0 >> 12
    }
    #[inline]
    pub const fn is_capture(self) -> bool {
        self.flag() & 4 != 0
    }
    #[inline]
    pub const fn is_promotion(self) -> bool {
        self.flag() & 8 != 0
    }

    /// Raw 16-bit encoding (engine-internal: TT storage). Round-trips with
    /// `from_raw`; a raw value from a corrupted/collided TT entry decodes to
    /// SOME move — consumers must validate against generated moves.
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub const fn from_raw(raw: u16) -> Move {
        Move(raw)
    }
    /// Only valid when is_promotion().
    #[inline]
    pub const fn promotion_piece_type(self) -> PieceType {
        match self.flag() & 3 {
            0 => PieceType::Knight,
            1 => PieceType::Bishop,
            2 => PieceType::Rook,
            _ => PieceType::Queen,
        }
    }
}

impl std::fmt::Display for Move {
    /// UCI long algebraic: e2e4, e7e8q. Castling as king move (e1g1).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.from(), self.to())?;
        if self.is_promotion() {
            let c = match self.promotion_piece_type() {
                PieceType::Knight => 'n',
                PieceType::Bishop => 'b',
                PieceType::Rook => 'r',
                _ => 'q',
            };
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

/// Fixed-capacity move list; 256 exceeds the known maximum legal moves (218).
pub struct MoveList {
    moves: [Move; 256],
    len: usize,
}

impl MoveList {
    #[inline]
    pub fn new() -> MoveList {
        MoveList {
            moves: [Move::NULL; 256],
            len: 0,
        }
    }
    #[inline]
    pub fn push(&mut self, mv: Move) {
        debug_assert!(self.len < 256);
        self.moves[self.len] = mv;
        self.len += 1;
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Move> {
        self.moves[..self.len].iter()
    }
    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{PieceType, Square};

    #[test]
    fn pack_unpack_roundtrip() {
        let e2 = Square::from_name("e2").unwrap();
        let e4 = Square::from_name("e4").unwrap();
        let mv = Move::new(e2, e4, Move::DOUBLE_PUSH);
        assert_eq!(mv.from(), e2);
        assert_eq!(mv.to(), e4);
        assert_eq!(mv.flag(), Move::DOUBLE_PUSH);
        assert!(!mv.is_capture());
        assert!(!mv.is_promotion());
    }

    #[test]
    fn flag_predicates() {
        let a = Square::from_name("d4").unwrap();
        let b = Square::from_name("e5").unwrap();
        assert!(Move::new(a, b, Move::CAPTURE).is_capture());
        assert!(Move::new(a, b, Move::EN_PASSANT).is_capture());
        assert!(!Move::new(a, b, Move::KING_CASTLE).is_capture());
        let p = Move::new(a, b, Move::PROMO_CAP_Q);
        assert!(p.is_capture() && p.is_promotion());
        assert_eq!(p.promotion_piece_type(), PieceType::Queen);
        assert_eq!(
            Move::new(a, b, Move::PROMO_N).promotion_piece_type(),
            PieceType::Knight
        );
    }

    #[test]
    fn uci_display() {
        let e7 = Square::from_name("e7").unwrap();
        let e8 = Square::from_name("e8").unwrap();
        assert_eq!(Move::new(e7, e8, Move::PROMO_Q).to_string(), "e7e8q");
        let e2 = Square::from_name("e2").unwrap();
        let e4 = Square::from_name("e4").unwrap();
        assert_eq!(Move::new(e2, e4, Move::DOUBLE_PUSH).to_string(), "e2e4");
        assert_eq!(
            Move::new(Square::E1, Square::G1, Move::KING_CASTLE).to_string(),
            "e1g1"
        );
    }

    #[test]
    fn movelist_push_iter() {
        let mut list = MoveList::new();
        assert_eq!(list.len(), 0);
        let mv = Move::new(Square::E1, Square::G1, Move::KING_CASTLE);
        list.push(mv);
        list.push(mv);
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|&m| m == mv));
    }

    #[test]
    fn raw_roundtrip() {
        let mv = Move::new(Square::E1, Square::G1, Move::KING_CASTLE);
        assert_eq!(Move::from_raw(mv.raw()), mv);
        assert_eq!(Move::from_raw(0), Move::NULL);
    }

    #[test]
    fn as_mut_slice_allows_reordering() {
        let mut list = MoveList::new();
        let a = Move::new(Square::E1, Square::G1, Move::KING_CASTLE);
        let b = Move::new(Square::A1, Square::H8, Move::QUIET);
        list.push(a);
        list.push(b);
        list.as_mut_slice().swap(0, 1);
        assert_eq!(list.as_slice()[0], b);
        assert_eq!(list.as_slice()[1], a);
    }
}
