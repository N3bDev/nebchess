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

#[derive(Clone)]
pub(crate) struct Undo {
    pub mv: Move,
    pub captured: Option<Piece>,
    pub castling: CastlingRights,
    pub ep: Option<Square>,
    pub halfmove: u16,
    pub key: u64,
    pub pawn_key: u64,
}

#[derive(Clone)]
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
    pub(crate) pawn_key: u64,
    pub(crate) undo_stack: Vec<Undo>,
    pub(crate) key_history: Vec<u64>,
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
            pawn_key: 0,
            undo_stack: Vec::with_capacity(256),
            key_history: Vec::with_capacity(256),
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
    pub fn pawn_key(&self) -> u64 {
        self.pawn_key
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
    #[inline]
    pub(crate) fn remove_raw(&mut self, sq: Square) -> Piece {
        let p = self.mailbox[sq.index()].expect("remove_raw: empty square");
        self.pieces[p.index()].clear(sq);
        self.occ[p.color().index()].clear(sq);
        self.occ_all.clear(sq);
        self.mailbox[sq.index()] = None;
        p
    }
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
        if p.piece_type() == PieceType::Pawn {
            self.pawn_key ^= KEYS.pieces[p.index()][sq.index()];
        }
    }
    #[inline]
    pub(crate) fn remove_piece(&mut self, sq: Square) -> Piece {
        let p = self.remove_raw(sq);
        self.key ^= KEYS.pieces[p.index()][sq.index()];
        if p.piece_type() == PieceType::Pawn {
            self.pawn_key ^= KEYS.pieces[p.index()][sq.index()];
        }
        p
    }
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

    /// Pawn-only key recomputation (debug verification; never in hot paths).
    pub fn compute_pawn_key(&self) -> u64 {
        let mut key = 0u64;
        for color in [Color::White, Color::Black] {
            let p = Piece::new(color, PieceType::Pawn);
            for sq in self.piece_bb(color, PieceType::Pawn) {
                key ^= KEYS.pieces[p.index()][sq.index()];
            }
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
        debug_assert_eq!(pos.pawn_key, pos.compute_pawn_key());
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

    /// Is `sq` attacked by any piece of color `by`?
    pub fn square_attacked(&self, sq: Square, by: Color) -> bool {
        let occ = self.occ_all;
        // a `by`-pawn attacks sq iff a pawn of the OTHER color on sq would attack it (symmetry)
        (attacks::pawn_attacks(by.flip(), sq) & self.piece_bb(by, PieceType::Pawn)).any()
            || (attacks::knight_attacks(sq) & self.piece_bb(by, PieceType::Knight)).any()
            || (attacks::king_attacks(sq) & self.piece_bb(by, PieceType::King)).any()
            || (attacks::bishop_attacks(sq, occ)
                & (self.piece_bb(by, PieceType::Bishop) | self.piece_bb(by, PieceType::Queen)))
            .any()
            || (attacks::rook_attacks(sq, occ)
                & (self.piece_bb(by, PieceType::Rook) | self.piece_bb(by, PieceType::Queen)))
            .any()
    }

    #[inline]
    pub fn in_check(&self, color: Color) -> bool {
        self.square_attacked(self.king_sq(color), color.flip())
    }

    /// Applies a pseudo-legal move. Returns false (state unchanged) if it
    /// would leave the mover's king attacked.
    pub fn make(&mut self, mv: Move) -> bool {
        let stm = self.stm;
        let (from, to, flag) = (mv.from(), mv.to(), mv.flag());
        let captured = if flag == Move::EN_PASSANT {
            Some(Piece::new(stm.flip(), PieceType::Pawn))
        } else {
            self.mailbox[to.index()]
        };
        self.undo_stack.push(Undo {
            mv,
            captured,
            castling: self.castling,
            ep: self.ep,
            halfmove: self.halfmove,
            key: self.key,
            pawn_key: self.pawn_key,
        });
        self.key_history.push(self.key);

        let mover = self.mailbox[from.index()].expect("make: empty from-square");
        debug_assert_eq!(mover.color(), stm);

        // clear stale EP state
        if let Some(ep) = self.ep.take() {
            self.key ^= KEYS.ep_file[ep.file() as usize];
        }

        // remove captured piece
        if mv.is_capture() {
            if flag == Move::EN_PASSANT {
                // captured pawn sits on the from-rank, to-file
                self.remove_piece(Square::from_fr(to.file(), from.rank()));
            } else {
                self.remove_piece(to);
            }
        }

        // move the piece (promotions swap the pawn for the new piece)
        if mv.is_promotion() {
            self.remove_piece(from);
            self.put_piece(Piece::new(stm, mv.promotion_piece_type()), to);
        } else {
            self.move_piece(from, to);
        }

        // flag-specific side effects
        match flag {
            Move::KING_CASTLE => {
                let (rf, rt) = match stm {
                    Color::White => (Square::H1, Square::F1),
                    Color::Black => (Square::H8, Square::F8),
                };
                self.move_piece(rf, rt);
            }
            Move::QUEEN_CASTLE => {
                let (rf, rt) = match stm {
                    Color::White => (Square::A1, Square::D1),
                    Color::Black => (Square::A8, Square::D8),
                };
                self.move_piece(rf, rt);
            }
            Move::DOUBLE_PUSH => {
                let ep_sq = Square::from_fr(from.file(), (from.rank() + to.rank()) / 2);
                if self.ep_capturable(ep_sq, stm) {
                    self.ep = Some(ep_sq);
                    self.key ^= KEYS.ep_file[ep_sq.file() as usize];
                }
            }
            _ => {}
        }

        // castling rights: per-square masks handle king moves, rook moves,
        // and rook captures uniformly
        const CASTLE_MASK: [u8; 64] = {
            let mut m = [0b1111u8; 64];
            m[0] = 0b1101; // a1: clear WQ
            m[4] = 0b1100; // e1: clear WK|WQ
            m[7] = 0b1110; // h1: clear WK
            m[56] = 0b0111; // a8: clear BQ
            m[60] = 0b0011; // e8: clear BK|BQ
            m[63] = 0b1011; // h8: clear BK
            m
        };
        let old_castling = self.castling.bits();
        self.castling
            .mask(CASTLE_MASK[from.index()] & CASTLE_MASK[to.index()]);
        self.key ^=
            KEYS.castling[old_castling as usize] ^ KEYS.castling[self.castling.bits() as usize];

        // clocks
        self.halfmove = if mover.piece_type() == PieceType::Pawn || mv.is_capture() {
            0
        } else {
            self.halfmove + 1
        };
        if stm == Color::Black {
            self.fullmove += 1;
        }

        // side to move
        self.stm = stm.flip();
        self.key ^= KEYS.black_to_move;

        debug_assert_eq!(self.key, self.compute_key());
        debug_assert_eq!(self.pawn_key, self.compute_pawn_key());

        // legality: mover's king must not be attacked
        if self.square_attacked(self.king_sq(stm), self.stm) {
            self.unmake();
            return false;
        }
        true
    }

    /// Exactly reverses the last make(). Key/castling/ep/halfmove restored
    /// from the undo record; piece moves reversed with raw (keyless) ops.
    pub fn unmake(&mut self) {
        self.key_history.pop();
        let u = self.undo_stack.pop().expect("unmake: empty undo stack");
        let mv = u.mv;
        let flag = mv.flag();
        self.stm = self.stm.flip(); // back to the mover
        let stm = self.stm;
        if stm == Color::Black {
            self.fullmove -= 1;
        }

        // reverse the piece movement
        if mv.is_promotion() {
            self.remove_raw(mv.to());
            self.put_raw(Piece::new(stm, PieceType::Pawn), mv.from());
        } else {
            self.move_raw(mv.to(), mv.from());
        }

        // reverse side effects
        match flag {
            Move::KING_CASTLE => {
                let (rf, rt) = match stm {
                    Color::White => (Square::H1, Square::F1),
                    Color::Black => (Square::H8, Square::F8),
                };
                self.move_raw(rt, rf);
            }
            Move::QUEEN_CASTLE => {
                let (rf, rt) = match stm {
                    Color::White => (Square::A1, Square::D1),
                    Color::Black => (Square::A8, Square::D8),
                };
                self.move_raw(rt, rf);
            }
            Move::EN_PASSANT => {
                // captured pawn returns to from-rank, to-file
                self.put_raw(
                    Piece::new(stm.flip(), PieceType::Pawn),
                    Square::from_fr(mv.to().file(), mv.from().rank()),
                );
            }
            _ => {}
        }
        if flag != Move::EN_PASSANT {
            if let Some(captured) = u.captured {
                self.put_raw(captured, mv.to());
            }
        }

        self.castling = u.castling;
        self.ep = u.ep;
        self.halfmove = u.halfmove;
        self.key = u.key;
        self.pawn_key = u.pawn_key;
    }

    /// Has the current position occurred before within the reversible-move
    /// window? (Twofold; search treats this as a draw, spec §3.)
    pub fn is_repetition(&self) -> bool {
        let n = self.key_history.len();
        let lookback = (self.halfmove as usize).min(n);
        // same side to move only: ancestors at distance 2, 4, ...
        let mut d = 2;
        while d <= lookback {
            if self.key_history[n - d] == self.key {
                return true;
            }
            d += 2;
        }
        false
    }

    /// 50-move rule (100 halfmoves). Mate-precedence is the caller's job:
    /// a mated side at halfmove >= 100 is still mated (search checks moves).
    #[inline]
    pub fn is_fifty_move_draw(&self) -> bool {
        self.halfmove >= 100
    }

    /// Null move: pass the turn (search-only device; illegal in chess).
    /// Pairs strictly with unmake_null — NEVER with unmake().
    pub fn make_null(&mut self) {
        self.undo_stack.push(Undo {
            mv: Move::NULL,
            captured: None,
            castling: self.castling,
            ep: self.ep,
            halfmove: self.halfmove,
            key: self.key,
            pawn_key: self.pawn_key,
        });
        self.key_history.push(self.key);
        if let Some(ep) = self.ep.take() {
            self.key ^= KEYS.ep_file[ep.file() as usize];
        }
        self.halfmove += 1;
        self.stm = self.stm.flip();
        self.key ^= KEYS.black_to_move;
        debug_assert_eq!(self.key, self.compute_key());
    }

    pub fn unmake_null(&mut self) {
        self.key_history.pop();
        let u = self
            .undo_stack
            .pop()
            .expect("unmake_null: empty undo stack");
        debug_assert_eq!(u.mv, Move::NULL, "unmake_null paired with a real make");
        self.stm = self.stm.flip();
        self.castling = u.castling;
        self.ep = u.ep;
        self.halfmove = u.halfmove;
        self.key = u.key;
        self.pawn_key = u.pawn_key;
    }

    /// Anything beyond king+pawns for `color` (zugzwang guard for null-move).
    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        (self.piece_bb(color, PieceType::Knight)
            | self.piece_bb(color, PieceType::Bishop)
            | self.piece_bb(color, PieceType::Rook)
            | self.piece_bb(color, PieceType::Queen))
        .any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(pos: &Position, from: &str, to: &str, flag: u16) -> Move {
        let _ = pos;
        Move::new(
            Square::from_name(from).unwrap(),
            Square::from_name(to).unwrap(),
            flag,
        )
    }

    #[test]
    fn square_attacked_basics() {
        let pos = Position::startpos();
        // e3 attacked by white pawns (d2/f2), g1-knight covers f3
        assert!(pos.square_attacked(Square::from_name("e3").unwrap(), Color::White));
        assert!(pos.square_attacked(Square::from_name("f3").unwrap(), Color::White));
        // e4 attacked by nobody at start
        assert!(!pos.square_attacked(Square::from_name("e4").unwrap(), Color::White));
        assert!(!pos.square_attacked(Square::from_name("e4").unwrap(), Color::Black));
        assert!(!pos.in_check(Color::White));
        // sliding attacks through occupancy
        let pos = Position::from_fen("4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1").unwrap();
        assert!(pos.square_attacked(Square::from_name("e5").unwrap(), Color::Black)); // re8
        assert!(!pos.square_attacked(Square::E1, Color::Black)); // blocked by Re2
    }

    /// make+unmake must restore FEN, key, and stack depth exactly.
    fn assert_make_unmake(fen: &str, m: Move, expect_legal: bool) {
        let mut pos = Position::from_fen(fen).unwrap();
        let key_before = pos.key();
        let legal = pos.make(m);
        assert_eq!(legal, expect_legal, "legality of {m} in {fen}");
        if legal {
            assert_eq!(pos.key(), pos.compute_key(), "incremental key after {m}");
            pos.unmake();
        }
        assert_eq!(pos.to_fen(), fen, "state restore after {m}");
        assert_eq!(pos.key(), key_before, "key restore after {m}");
        assert_eq!(pos.key(), pos.compute_key());
    }

    #[test]
    fn make_unmake_quiet_and_double_push() {
        let pos = Position::startpos();
        assert_make_unmake(START_FEN, mv(&pos, "g1", "f3", Move::QUIET), true);
        assert_make_unmake(START_FEN, mv(&pos, "e2", "e4", Move::DOUBLE_PUSH), true);
    }

    #[test]
    fn double_push_sets_ep_only_when_capturable() {
        let mut pos = Position::startpos();
        assert!(pos.make(Move::new(
            Square::from_name("e2").unwrap(),
            Square::from_name("e4").unwrap(),
            Move::DOUBLE_PUSH
        )));
        assert_eq!(pos.ep(), None, "no black pawn adjacent to e4");
        // black pawn on d4 -> e2e4 must set ep = e3
        let mut pos =
            Position::from_fen("rnbqkbnr/ppp1pppp/8/8/3p4/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
                .unwrap();
        assert!(pos.make(Move::new(
            Square::from_name("e2").unwrap(),
            Square::from_name("e4").unwrap(),
            Move::DOUBLE_PUSH
        )));
        assert_eq!(pos.ep(), Some(Square::from_name("e3").unwrap()));
        assert_eq!(pos.key(), pos.compute_key());
    }

    #[test]
    fn make_unmake_captures() {
        let fen = "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let pos = Position::from_fen(fen).unwrap();
        assert_make_unmake(fen, mv(&pos, "e4", "d5", Move::CAPTURE), true);
    }

    #[test]
    fn make_unmake_en_passant() {
        let fen = "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let m = mv(&pos, "c4", "d3", Move::EN_PASSANT);
        assert_make_unmake(fen, m, true);
        // verify the captured pawn vanishes from d4 (not d3)
        let mut pos = Position::from_fen(fen).unwrap();
        assert!(pos.make(m));
        assert_eq!(pos.piece_on(Square::from_name("d4").unwrap()), None);
        assert_eq!(
            pos.piece_on(Square::from_name("d3").unwrap()),
            Some(Piece::new(Color::Black, PieceType::Pawn))
        );
    }

    #[test]
    fn make_unmake_castling_moves_rook() {
        let fen = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_make_unmake(fen, mv(&pos, "e1", "g1", Move::KING_CASTLE), true);
        assert_make_unmake(fen, mv(&pos, "e1", "c1", Move::QUEEN_CASTLE), true);
        let mut pos = Position::from_fen(fen).unwrap();
        assert!(pos.make(mv(&pos, "e1", "g1", Move::KING_CASTLE)));
        assert_eq!(
            pos.piece_on(Square::F1),
            Some(Piece::new(Color::White, PieceType::Rook))
        );
        assert_eq!(pos.piece_on(Square::H1), None);
        assert!(!pos.castling().has(CastlingRights::WK));
        assert!(!pos.castling().has(CastlingRights::WQ));
        assert!(pos.castling().has(CastlingRights::BK));
    }

    #[test]
    fn rook_capture_strips_castling_right() {
        // Ra1xa8: black loses queenside right, white loses queenside right (rook moved)
        let fen = "r3k2r/8/8/8/8/8/6B1/R3K2R w KQkq - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let m = mv(&pos, "a1", "a8", Move::CAPTURE);
        assert!(pos.make(m));
        assert!(!pos.castling().has(CastlingRights::BQ), "a8 rook captured");
        assert!(!pos.castling().has(CastlingRights::WQ), "a1 rook moved");
        assert!(pos.castling().has(CastlingRights::BK));
        assert_eq!(pos.key(), pos.compute_key());
        pos.unmake();
        assert_eq!(pos.to_fen(), fen, "rook capture restore");
        assert_eq!(pos.key(), pos.compute_key());
        assert!(pos.castling().has(CastlingRights::BQ) && pos.castling().has(CastlingRights::WQ));
    }

    #[test]
    fn make_unmake_chain_restores_exactly() {
        // mixed flags over 6 plies; cumulative drift would survive single-move tests
        let mut pos = Position::startpos();
        let start_key = pos.key();
        let start_pawn_key = pos.pawn_key();
        let moves = [
            ("g1", "f3", Move::QUIET),
            ("g8", "f6", Move::QUIET),
            ("e2", "e4", Move::DOUBLE_PUSH),
            ("e7", "e5", Move::DOUBLE_PUSH),
            ("f1", "c4", Move::QUIET),
            ("f8", "c5", Move::QUIET),
        ];
        for (f, t, flag) in moves {
            assert!(pos.make(mv(&pos, f, t, flag)));
            assert_eq!(pos.key(), pos.compute_key(), "drift after {f}{t}");
            assert_eq!(
                pos.pawn_key(),
                pos.compute_pawn_key(),
                "pawn key drift after {f}{t}"
            );
        }
        for _ in 0..moves.len() {
            pos.unmake();
        }
        assert_eq!(pos.to_fen(), START_FEN);
        assert_eq!(pos.key(), start_key);
        assert_eq!(pos.pawn_key(), start_pawn_key);
    }

    #[test]
    fn pawn_key_changes_on_pawn_moves_not_knight() {
        let mut pos = Position::startpos();
        let pk_start = pos.pawn_key();
        // knight move: pawn key unchanged
        assert!(pos.make(mv(&pos, "g1", "f3", Move::QUIET)));
        assert_eq!(pos.pawn_key(), pk_start, "knight move must not change pawn key");
        pos.unmake();
        // pawn move: pawn key must change
        assert!(pos.make(mv(&pos, "e2", "e4", Move::DOUBLE_PUSH)));
        assert_ne!(pos.pawn_key(), pk_start, "pawn double push must change pawn key");
        assert_eq!(pos.pawn_key(), pos.compute_pawn_key());
        pos.unmake();
        assert_eq!(pos.pawn_key(), pk_start, "pawn key restored after unmake");
    }

    #[test]
    fn pawn_key_survives_null_move() {
        let mut pos = Position::startpos();
        let pk = pos.pawn_key();
        pos.make_null();
        assert_eq!(pos.pawn_key(), pk, "null move must not change pawn key");
        pos.unmake_null();
        assert_eq!(pos.pawn_key(), pk);
    }

    #[test]
    fn make_unmake_promotions() {
        // c7 pawn: push-promote to empty c8, or capture-promote the b8 knight.
        let fen = "1n2k3/2P5/8/8/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_make_unmake(fen, mv(&pos, "c7", "b8", Move::PROMO_CAP_Q), true);
        assert_make_unmake(fen, mv(&pos, "c7", "c8", Move::PROMO_N), true);
        let mut pos = Position::from_fen(fen).unwrap();
        assert!(pos.make(mv(&pos, "c7", "c8", Move::PROMO_Q)));
        assert_eq!(
            pos.piece_on(Square::C8),
            Some(Piece::new(Color::White, PieceType::Queen))
        );
    }

    #[test]
    fn illegal_move_rejected_and_state_unchanged() {
        // Re2 is pinned by Re8; moving it off the e-file is illegal
        let fen = "4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_make_unmake(fen, mv(&pos, "e2", "a2", Move::QUIET), false);
        // staying on the file is legal
        assert_make_unmake(fen, mv(&pos, "e2", "e5", Move::QUIET), true);
    }

    #[test]
    fn halfmove_and_fullmove_clocks() {
        let mut pos = Position::startpos();
        pos.make(mv(&pos, "g1", "f3", Move::QUIET));
        assert_eq!(pos.halfmove(), 1);
        assert_eq!(pos.fullmove(), 1);
        pos.make(mv(&pos, "g8", "f6", Move::QUIET));
        assert_eq!(pos.halfmove(), 2);
        assert_eq!(pos.fullmove(), 2); // increments after black moves
        pos.make(mv(&pos, "e2", "e4", Move::DOUBLE_PUSH));
        assert_eq!(pos.halfmove(), 0, "pawn move resets");
    }

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

    #[test]
    fn en_passant_exposing_rank_pin_rejected() {
        // The most famous make/unmake trap: exd6 e.p. removes BOTH pawns from
        // rank 5, exposing Ka5 to Rh5. Must be rejected.
        let fen = "8/8/8/K2pP2r/8/8/8/4k3 w - d6 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(
            pos.ep(),
            Some(Square::from_name("d6").unwrap()),
            "ep parsed"
        );
        assert_make_unmake(fen, mv(&pos, "e5", "d6", Move::EN_PASSANT), false);
    }

    #[test]
    fn repetition_detected_via_history() {
        let mut pos = Position::startpos();
        assert!(!pos.is_repetition());
        // Ng1f3 Ng8f6 Nf3g1 Nf6g8 -> startpos repeated (one fold)
        for (f, t) in [("g1", "f3"), ("g8", "f6"), ("f3", "g1"), ("f6", "g8")] {
            assert!(pos.make(mv(&pos, f, t, Move::QUIET)));
        }
        assert!(pos.is_repetition(), "back at startpos: repetition");
        pos.unmake();
        assert!(!pos.is_repetition());
    }

    #[test]
    fn pawn_move_cuts_repetition_scope() {
        let mut pos = Position::startpos();
        assert!(pos.make(mv(&pos, "e2", "e4", Move::DOUBLE_PUSH)));
        assert!(pos.make(mv(&pos, "e7", "e5", Move::DOUBLE_PUSH)));
        // shuffle knights back to the post-e4e5 position
        for (f, t) in [("g1", "f3"), ("g8", "f6"), ("f3", "g1"), ("f6", "g8")] {
            assert!(pos.make(mv(&pos, f, t, Move::QUIET)));
        }
        assert!(pos.is_repetition(), "post-e4e5 position repeated");
        // but startpos itself is NOT reachable as a repetition (pawn moves reset)
        // halfmove clock is 4 here; history scan must not cross the e7e5 boundary
        assert_eq!(pos.halfmove(), 4);
    }

    #[test]
    fn history_survives_clone_and_unmake_restores_len() {
        let mut pos = Position::startpos();
        assert!(pos.make(mv(&pos, "g1", "f3", Move::QUIET)));
        let snapshot = pos.clone();
        assert_eq!(snapshot.key(), pos.key());
        assert!(pos.make(mv(&pos, "g8", "f6", Move::QUIET)));
        pos.unmake();
        assert_eq!(pos.key(), snapshot.key());
        assert!(!pos.is_repetition());
    }

    #[test]
    fn fifty_move_counter_draw_helper() {
        // artificial position with halfmove=99: one quiet move crosses 100
        let mut pos = Position::from_fen("4k3/8/8/8/8/8/8/R3K3 w Q - 99 80").unwrap();
        assert!(!pos.is_fifty_move_draw());
        assert!(pos.make(mv(&pos, "a1", "a2", Move::QUIET)));
        assert_eq!(pos.halfmove(), 100);
        assert!(pos.is_fifty_move_draw());
    }

    #[test]
    fn null_move_roundtrip() {
        let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let key = pos.key();
        pos.make_null();
        assert_ne!(pos.key(), key, "stm flip must change the key");
        assert_eq!(pos.stm(), Color::Black);
        assert_eq!(pos.key(), pos.compute_key());
        pos.unmake_null();
        assert_eq!(pos.to_fen(), fen);
        assert_eq!(pos.key(), key);
    }

    #[test]
    fn null_move_clears_ep_and_restores_it() {
        let fen = "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1"; // capturable ep kept
        let mut pos = Position::from_fen(fen).unwrap();
        assert!(pos.ep().is_some());
        pos.make_null();
        assert_eq!(pos.ep(), None, "null clears ep");
        assert_eq!(pos.key(), pos.compute_key());
        pos.unmake_null();
        assert_eq!(pos.ep(), Some(Square::from_name("d3").unwrap()));
        assert_eq!(pos.to_fen(), fen);
    }

    #[test]
    fn null_move_interleaves_with_real_moves() {
        let mut pos = Position::startpos();
        let key0 = pos.key();
        assert!(pos.make(mv(&pos, "e2", "e4", Move::DOUBLE_PUSH)));
        pos.make_null();
        assert!(pos.make(mv(&pos, "d2", "d3", Move::QUIET))); // white again after null
        pos.unmake();
        pos.unmake_null();
        pos.unmake();
        assert_eq!(pos.key(), key0);
        assert_eq!(pos.to_fen(), START_FEN);
    }

    #[test]
    fn non_pawn_material_detection() {
        let pos = Position::startpos();
        assert!(pos.has_non_pawn_material(Color::White));
        let pos = Position::from_fen("4k3/4p3/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        assert!(!pos.has_non_pawn_material(Color::White), "K+P only");
        assert!(!pos.has_non_pawn_material(Color::Black));
        let pos = Position::from_fen("4k3/8/8/8/8/8/4P3/4KN2 w - - 0 1").unwrap();
        assert!(pos.has_non_pawn_material(Color::White));
    }
}
