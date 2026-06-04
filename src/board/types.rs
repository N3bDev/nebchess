//! Fundamental board types. Square indexing is LERF: a1=0 .. h8=63.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    #[inline]
    pub const fn flip(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum PieceType {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl PieceType {
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const ALL: [PieceType; 6] = [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ];
}

/// A colored piece. Index layout: white P,N,B,R,Q,K = 0..=5, black = 6..=11.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Piece(u8);

impl Piece {
    #[inline]
    pub const fn new(color: Color, pt: PieceType) -> Piece {
        Piece((color as u8) * 6 + pt as u8)
    }
    /// # Panics
    /// In debug builds, panics if `i >= 12`. In release the assertion is
    /// elided; callers must provide a valid index (0..=11) — an invalid
    /// `Piece` misclassifies silently and panics at `to_char()`.
    #[inline]
    pub const fn from_index(i: usize) -> Piece {
        debug_assert!(i < 12);
        Piece(i as u8)
    }
    #[inline]
    pub const fn color(self) -> Color {
        if self.0 < 6 {
            Color::White
        } else {
            Color::Black
        }
    }
    #[inline]
    pub const fn piece_type(self) -> PieceType {
        match self.0 % 6 {
            0 => PieceType::Pawn,
            1 => PieceType::Knight,
            2 => PieceType::Bishop,
            3 => PieceType::Rook,
            4 => PieceType::Queen,
            _ => PieceType::King,
        }
    }
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
    /// FEN char: uppercase = white, lowercase = black.
    pub const fn to_char(self) -> char {
        const CHARS: [char; 12] = ['P', 'N', 'B', 'R', 'Q', 'K', 'p', 'n', 'b', 'r', 'q', 'k'];
        CHARS[self.0 as usize]
    }
    pub fn from_char(c: char) -> Option<Piece> {
        "PNBRQKpnbrqk".find(c).map(|i| Piece(i as u8))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Square(u8);

impl Square {
    #[inline]
    pub const fn new(index: u8) -> Square {
        debug_assert!(index < 64);
        Square(index)
    }
    #[inline]
    pub const fn from_fr(file: u8, rank: u8) -> Square {
        debug_assert!(file < 8 && rank < 8);
        Square(rank * 8 + file)
    }
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
    #[inline]
    pub const fn file(self) -> u8 {
        self.0 & 7
    }
    #[inline]
    pub const fn rank(self) -> u8 {
        self.0 >> 3
    }
    pub fn from_name(name: &str) -> Option<Square> {
        let b = name.as_bytes();
        if b.len() != 2 {
            return None;
        }
        let (f, r) = (b[0].wrapping_sub(b'a'), b[1].wrapping_sub(b'1'));
        (f < 8 && r < 8).then(|| Square::from_fr(f, r))
    }
    pub const A1: Square = Square(0);
    pub const C1: Square = Square(2);
    pub const D1: Square = Square(3);
    pub const E1: Square = Square(4);
    pub const F1: Square = Square(5);
    pub const G1: Square = Square(6);
    pub const H1: Square = Square(7);
    pub const A8: Square = Square(56);
    pub const C8: Square = Square(58);
    pub const D8: Square = Square(59);
    pub const E8: Square = Square(60);
    pub const F8: Square = Square(61);
    pub const G8: Square = Square(62);
    pub const H8: Square = Square(63);
}

impl std::fmt::Display for Square {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", (b'a' + self.file()) as char, self.rank() + 1)
    }
}

/// 4-bit castling rights mask.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct CastlingRights(u8);

impl CastlingRights {
    pub const NONE: CastlingRights = CastlingRights(0);
    pub const WK: CastlingRights = CastlingRights(1);
    pub const WQ: CastlingRights = CastlingRights(2);
    pub const BK: CastlingRights = CastlingRights(4);
    pub const BQ: CastlingRights = CastlingRights(8);
    pub const ALL: CastlingRights = CastlingRights(0b1111);

    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }
    /// Returns `true` if *any* bit in `rights` is set in `self`.
    #[inline]
    pub const fn has(self, rights: CastlingRights) -> bool {
        self.0 & rights.0 != 0
    }
    #[inline]
    pub fn remove(&mut self, rights: CastlingRights) {
        self.0 &= !rights.0;
    }
    /// rights &= mask (used with the per-square mask table in make()).
    #[inline]
    pub fn mask(&mut self, keep: u8) {
        self.0 &= keep;
    }
}

impl std::ops::BitOr for CastlingRights {
    type Output = CastlingRights;
    fn bitor(self, rhs: CastlingRights) -> CastlingRights {
        CastlingRights(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for CastlingRights {
    fn bitor_assign(&mut self, rhs: CastlingRights) {
        self.0 |= rhs.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_roundtrip() {
        let e4 = Square::from_name("e4").unwrap();
        assert_eq!(e4.index(), 28); // e=4, rank 4 -> 4 + 3*8
        assert_eq!(e4.file(), 4);
        assert_eq!(e4.rank(), 3);
        assert_eq!(e4.to_string(), "e4");
        assert_eq!(Square::from_fr(4, 3), e4);
        assert_eq!(Square::A1.index(), 0);
        assert_eq!(Square::H8.index(), 63);
        assert!(Square::from_name("i9").is_none());
        assert!(Square::from_name("e").is_none());
        assert_eq!(Square::from_fr(0, 0), Square::A1);
        assert_eq!(Square::from_fr(7, 7), Square::H8);
        assert!(Square::from_name("A1").is_none()); // uppercase rejected
    }

    #[test]
    fn piece_composition() {
        let bn = Piece::new(Color::Black, PieceType::Knight);
        assert_eq!(bn.color(), Color::Black);
        assert_eq!(bn.piece_type(), PieceType::Knight);
        assert_eq!(bn.index(), 7); // white 0..=5, black 6..=11
        assert_eq!(Piece::from_index(7), bn);
        assert_eq!(bn.to_char(), 'n');
        assert_eq!(Piece::from_char('n'), Some(bn));
        assert_eq!(
            Piece::from_char('K'),
            Some(Piece::new(Color::White, PieceType::King))
        );
        assert_eq!(Piece::from_char('x'), None);
    }

    #[test]
    fn color_flip() {
        assert_eq!(Color::White.flip(), Color::Black);
        assert_eq!(Color::Black.flip(), Color::White);
    }

    #[test]
    fn castling_rights() {
        let mut cr = CastlingRights::ALL;
        assert!(cr.has(CastlingRights::WK));
        cr.remove(CastlingRights::WK | CastlingRights::WQ);
        assert!(!cr.has(CastlingRights::WK));
        assert!(cr.has(CastlingRights::BQ));
        assert_eq!(CastlingRights::NONE.bits(), 0);
        assert_eq!(CastlingRights::ALL.bits(), 0b1111);
    }
}
