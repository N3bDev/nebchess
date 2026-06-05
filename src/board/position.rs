//! Position state: bitboards + mailbox + game state + Zobrist key.
//! All mutation goes through here; movegen only reads.

use crate::board::attacks;
use crate::board::zobrist::KEYS;
use crate::board::{Bitboard, CastlingRights, Color, Move, Piece, PieceType, Square};

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Debug, PartialEq, Eq)]
pub enum FenError {
    MissingFields,
    BadBoard,
    BadColor,
    BadCastling,
    BadEpSquare,
    BadCounter,
    BadKings,
}

impl std::fmt::Display for FenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid FEN: {self:?}")
    }
}
impl std::error::Error for FenError {}

// constructed by make() in the next commit
#[allow(dead_code)]
pub(crate) struct Undo {
    pub mv: Move,
    pub captured: Option<Piece>,
    pub castling: CastlingRights,
    pub ep: Option<Square>,
    pub halfmove: u16,
    pub key: u64,
}

pub struct Position {
    pub(crate) pieces: [Bitboard; 12],
    pub(crate) occ: [Bitboard; 2],
    pub(crate) occ_all: Bitboard,
    pub(crate) mailbox: [Option<Piece>; 64],
    pub(crate) stm: Color,
    pub(crate) castling: CastlingRights,
    pub(crate) ep: Option<Square>,
    pub(crate) halfmove: u16,
    pub(crate) fullmove: u16,
    pub(crate) key: u64,
    // populated by make() in the next commit
    #[allow(dead_code)]
    pub(crate) undo_stack: Vec<Undo>,
}

impl Position {
    fn new_empty() -> Position {
        Position {
            pieces: [Bitboard::EMPTY; 12],
            occ: [Bitboard::EMPTY; 2],
            occ_all: Bitboard::EMPTY,
            mailbox: [None; 64],
            stm: Color::White,
            castling: CastlingRights::NONE,
            ep: None,
            halfmove: 0,
            fullmove: 1,
            key: 0,
            undo_stack: Vec::with_capacity(256),
        }
    }

    pub fn startpos() -> Position {
        Position::from_fen(START_FEN).expect("startpos FEN is valid")
    }

    // ---- accessors ----
    #[inline]
    pub fn stm(&self) -> Color {
        self.stm
    }
    #[inline]
    pub fn key(&self) -> u64 {
        self.key
    }
    #[inline]
    pub fn castling(&self) -> CastlingRights {
        self.castling
    }
    #[inline]
    pub fn ep(&self) -> Option<Square> {
        self.ep
    }
    #[inline]
    pub fn halfmove(&self) -> u16 {
        self.halfmove
    }
    #[inline]
    pub fn fullmove(&self) -> u16 {
        self.fullmove
    }
    #[inline]
    pub fn piece_on(&self, sq: Square) -> Option<Piece> {
        self.mailbox[sq.index()]
    }
    #[inline]
    pub fn piece_bb(&self, color: Color, pt: PieceType) -> Bitboard {
        self.pieces[Piece::new(color, pt).index()]
    }
    #[inline]
    pub fn occ(&self, color: Color) -> Bitboard {
        self.occ[color.index()]
    }
    #[inline]
    pub fn occ_all(&self) -> Bitboard {
        self.occ_all
    }
    #[inline]
    pub fn king_sq(&self, color: Color) -> Square {
        self.piece_bb(color, PieceType::King).lsb()
    }

    // ---- raw piece plumbing (no Zobrist) ----
    #[inline]
    pub(crate) fn put_raw(&mut self, p: Piece, sq: Square) {
        debug_assert!(self.mailbox[sq.index()].is_none());
        self.pieces[p.index()].set(sq);
        self.occ[p.color().index()].set(sq);
        self.occ_all.set(sq);
        self.mailbox[sq.index()] = Some(p);
    }
    // used by make() in the next commit
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn remove_raw(&mut self, sq: Square) -> Piece {
        let p = self.mailbox[sq.index()].expect("remove_raw: empty square");
        self.pieces[p.index()].clear(sq);
        self.occ[p.color().index()].clear(sq);
        self.occ_all.clear(sq);
        self.mailbox[sq.index()] = None;
        p
    }
    // used by make() in the next commit
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn move_raw(&mut self, from: Square, to: Square) {
        let p = self.remove_raw(from);
        self.put_raw(p, to);
    }

    // ---- keyed piece plumbing (raw + Zobrist) ----
    #[inline]
    pub(crate) fn put_piece(&mut self, p: Piece, sq: Square) {
        self.put_raw(p, sq);
        self.key ^= KEYS.pieces[p.index()][sq.index()];
    }
    // used by make() in the next commit
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn remove_piece(&mut self, sq: Square) -> Piece {
        let p = self.remove_raw(sq);
        self.key ^= KEYS.pieces[p.index()][sq.index()];
        p
    }
    // used by make() in the next commit
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn move_piece(&mut self, from: Square, to: Square) {
        let p = self.remove_piece(from);
        self.put_piece(p, to);
    }

    /// Full key recomputation. Debug verification only — never in hot paths.
    pub fn compute_key(&self) -> u64 {
        let mut key = 0u64;
        for (i, bb) in self.pieces.iter().enumerate() {
            for sq in *bb {
                key ^= KEYS.pieces[i][sq.index()];
            }
        }
        key ^= KEYS.castling[self.castling.bits() as usize];
        if let Some(ep) = self.ep {
            key ^= KEYS.ep_file[ep.file() as usize];
        }
        if self.stm == Color::Black {
            key ^= KEYS.black_to_move;
        }
        key
    }

    /// EP convention: square counts only if an enemy pawn can pseudo-legally
    /// capture onto it. `pusher` = side that just double-pushed.
    pub(crate) fn ep_capturable(&self, ep_sq: Square, pusher: Color) -> bool {
        (attacks::pawn_attacks(pusher, ep_sq) & self.piece_bb(pusher.flip(), PieceType::Pawn)).any()
    }

    pub fn from_fen(fen: &str) -> Result<Position, FenError> {
        let mut fields = fen.split_whitespace();
        let board = fields.next().ok_or(FenError::MissingFields)?;
        let color = fields.next().ok_or(FenError::MissingFields)?;
        let castling = fields.next().ok_or(FenError::MissingFields)?;
        let ep = fields.next().ok_or(FenError::MissingFields)?;
        let halfmove = fields.next().unwrap_or("0");
        let fullmove = fields.next().unwrap_or("1");

        let mut pos = Position::new_empty();

        // piece placement: ranks 8..1 separated by '/'
        let ranks: Vec<&str> = board.split('/').collect();
        if ranks.len() != 8 {
            return Err(FenError::BadBoard);
        }
        for (i, rank_str) in ranks.iter().enumerate() {
            let rank = 7 - i as u8;
            let mut file = 0u8;
            for c in rank_str.chars() {
                if let Some(d) = c.to_digit(10) {
                    file += d as u8;
                } else {
                    let p = Piece::from_char(c).ok_or(FenError::BadBoard)?;
                    if file >= 8 {
                        return Err(FenError::BadBoard);
                    }
                    pos.put_piece(p, Square::from_fr(file, rank));
                    file += 1;
                }
            }
            if file != 8 {
                return Err(FenError::BadBoard);
            }
        }

        pos.stm = match color {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err(FenError::BadColor),
        };

        if castling != "-" {
            for c in castling.chars() {
                let right = match c {
                    'K' => CastlingRights::WK,
                    'Q' => CastlingRights::WQ,
                    'k' => CastlingRights::BK,
                    'q' => CastlingRights::BQ,
                    _ => return Err(FenError::BadCastling),
                };
                pos.castling |= right;
            }
        }

        if ep != "-" {
            let sq = Square::from_name(ep).ok_or(FenError::BadEpSquare)?;
            if sq.rank() != 2 && sq.rank() != 5 {
                return Err(FenError::BadEpSquare);
            }
            // pusher is the side that is NOT to move
            if pos.ep_capturable(sq, pos.stm.flip()) {
                pos.ep = Some(sq);
            }
        }

        pos.halfmove = halfmove.parse().map_err(|_| FenError::BadCounter)?;
        pos.fullmove = fullmove.parse().map_err(|_| FenError::BadCounter)?;

        if pos.piece_bb(Color::White, PieceType::King).count() != 1
            || pos.piece_bb(Color::Black, PieceType::King).count() != 1
        {
            return Err(FenError::BadKings);
        }

        // piece placement already XORed via put_piece; add the rest
        pos.key ^= KEYS.castling[pos.castling.bits() as usize];
        if let Some(ep) = pos.ep {
            pos.key ^= KEYS.ep_file[ep.file() as usize];
        }
        if pos.stm == Color::Black {
            pos.key ^= KEYS.black_to_move;
        }
        debug_assert_eq!(pos.key, pos.compute_key());
        Ok(pos)
    }

    pub fn to_fen(&self) -> String {
        let mut out = String::with_capacity(90);
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match self.mailbox[Square::from_fr(file, rank).index()] {
                    Some(p) => {
                        if empty > 0 {
                            out.push(char::from_digit(empty, 10).unwrap());
                            empty = 0;
                        }
                        out.push(p.to_char());
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                out.push(char::from_digit(empty, 10).unwrap());
            }
            if rank > 0 {
                out.push('/');
            }
        }
        out.push(' ');
        out.push(if self.stm == Color::White { 'w' } else { 'b' });
        out.push(' ');
        if self.castling == CastlingRights::NONE {
            out.push('-');
        } else {
            for (right, c) in [
                (CastlingRights::WK, 'K'),
                (CastlingRights::WQ, 'Q'),
                (CastlingRights::BK, 'k'),
                (CastlingRights::BQ, 'q'),
            ] {
                if self.castling.has(right) {
                    out.push(c);
                }
            }
        }
        out.push(' ');
        match self.ep {
            Some(sq) => out.push_str(&sq.to_string()),
            None => out.push('-'),
        }
        out.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const KIWIPETE: &str =
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

    #[test]
    fn startpos_roundtrip() {
        let pos = Position::startpos();
        assert_eq!(pos.to_fen(), START_FEN);
        assert_eq!(pos.stm(), Color::White);
        assert_eq!(pos.castling(), CastlingRights::ALL);
        assert_eq!(pos.ep(), None);
        assert_eq!(pos.occ_all().count(), 32);
        assert_eq!(
            pos.piece_on(Square::E1),
            Some(Piece::new(Color::White, PieceType::King))
        );
        assert_eq!(pos.king_sq(Color::Black), Square::E8);
    }

    #[test]
    fn kiwipete_parse() {
        let pos = Position::from_fen(KIWIPETE).unwrap();
        assert_eq!(pos.occ_all().count(), 32);
        assert_eq!(pos.occ(Color::White).count(), 16);
        assert_eq!(pos.occ(Color::Black).count(), 16);
        assert_eq!(pos.to_fen(), KIWIPETE);
    }

    #[test]
    fn fen_roundtrip_suite() {
        // canonical FENs (EP only when capturable)
        for fen in [
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1", // EP with capturer present: kept
        ] {
            let pos = Position::from_fen(fen).unwrap();
            assert_eq!(pos.to_fen(), fen, "roundtrip failed");
        }
    }

    #[test]
    fn ep_square_filtered_when_uncapturable() {
        // after 1.e4 there is no black pawn on d4/f4 -> ep must canonicalize to None
        let pos = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
            .unwrap();
        assert_eq!(pos.ep(), None);
        assert!(pos.to_fen().contains(" b KQkq - 0 1"));
        // capturer exists -> kept
        let pos = Position::from_fen("8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1").unwrap();
        assert_eq!(pos.ep(), Some(Square::from_name("d3").unwrap()));
    }

    #[test]
    fn key_matches_recompute_and_differs_by_stm() {
        let a = Position::startpos();
        assert_eq!(a.key(), a.compute_key());
        let b =
            Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1").unwrap();
        assert_eq!(b.key(), b.compute_key());
        assert_ne!(a.key(), b.key());
        assert_eq!(a.key() ^ b.key(), crate::board::zobrist::KEYS.black_to_move);
    }

    #[test]
    fn bad_fens_rejected() {
        for fen in [
            "",                                                          // empty
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP w KQkq - 0 1",           // 7 ranks
            "rnbqkbnr/pppppppp/9/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",  // rank sums to 9
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR x KQkq - 0 1",  // bad stm
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KZkq - 0 1",  // bad castling char
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq e9 0 1", // bad ep square
            "8/8/8/8/8/8/8/8 w - - 0 1",                                 // no kings
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - x 1",  // bad counter
        ] {
            assert!(Position::from_fen(fen).is_err(), "accepted bad FEN: {fen}");
        }
    }
}
