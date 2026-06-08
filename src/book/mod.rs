//! PolyGlot opening book: key composition, on-disk reader, weighted pick.
//!
//! The book format is the de-facto standard (`*.bin`): 16-byte big-endian
//! entries `{ key: u64, move: u16, weight: u16, learn: u32 }`, sorted by key.
//! Keys are composed from the vendored 781-entry [`polyglot_random::RANDOM`]
//! table (see that file for the four-subarray layout). This module is a
//! root-level move source consulted before search; it does NOT touch the
//! engine's own Zobrist keys ([`crate::board::zobrist`]) — those are unrelated.

pub mod polyglot_random;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::board::movegen::find_uci_move;
use crate::board::{CastlingRights, Color, Move, Piece, PieceType, Position, Square};
use polyglot_random::RANDOM;

/// Computes the standard PolyGlot Zobrist key for `pos`.
///
/// Mirrors the format spec exactly: `key = piece ^ castle ^ enpassant ^ turn`.
/// The en-passant term is included only when an enemy pawn stands adjacent to
/// the just-pushed pawn (legality of the capture is irrelevant) — which is the
/// precise condition under which [`Position::ep`] returns `Some` (our parser
/// canonicalises the FEN ep square the same way).
pub fn polyglot_key(pos: &Position) -> u64 {
    let mut key = 0u64;

    // --- pieces: offset = 64*kind + 8*row + file (row=rank, file as-is) ---
    // PolyGlot's kind_of_piece interleaves colours: bp=0, wp=1, bn=2, wn=3, ...
    for sq_i in 0..64u8 {
        let sq = Square::new(sq_i);
        if let Some(p) = pos.piece_on(sq) {
            let kind = polyglot_kind(p);
            let offset = 64 * kind + 8 * sq.rank() as usize + sq.file() as usize;
            key ^= RANDOM[offset];
        }
    }

    // --- castling: 768=WK(short), 769=WQ(long), 770=BK(short), 771=BQ(long) ---
    let c = pos.castling();
    if c.has(CastlingRights::WK) {
        key ^= RANDOM[768];
    }
    if c.has(CastlingRights::WQ) {
        key ^= RANDOM[769];
    }
    if c.has(CastlingRights::BK) {
        key ^= RANDOM[770];
    }
    if c.has(CastlingRights::BQ) {
        key ^= RANDOM[771];
    }

    // --- en passant: 772..780 indexed by the pushed pawn's file ---
    if let Some(ep) = pos.ep() {
        key ^= RANDOM[772 + ep.file() as usize];
    }

    // --- turn: 780 is XORed only when white is to move ---
    if pos.stm() == Color::White {
        key ^= RANDOM[780];
    }

    key
}

/// Maps our `Piece` to PolyGlot's `kind_of_piece` index (colour-interleaved:
/// black=even, white=odd; pawn,knight,bishop,rook,queen,king order).
#[inline]
fn polyglot_kind(p: Piece) -> usize {
    let base = match p.piece_type() {
        PieceType::Pawn => 0,
        PieceType::Knight => 2,
        PieceType::Bishop => 4,
        PieceType::Rook => 6,
        PieceType::Queen => 8,
        PieceType::King => 10,
    };
    // black gets the even slot, white the odd one (+1)
    base + (p.color() == Color::White) as usize
}

/// One decoded book entry (16 bytes on disk, big-endian).
#[derive(Clone, Copy, Debug)]
struct Entry {
    key: u64,
    mv: u16,
    weight: u16,
}

/// A loaded PolyGlot book: entries sorted by key (binary-searchable).
pub struct Book {
    entries: Vec<Entry>,
}

impl Book {
    /// Reads the whole book file into memory. Entries are validated to be
    /// 16 bytes each; a trailing partial entry is rejected. The file is
    /// re-sorted by key defensively (well-formed books are already sorted).
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Book> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Book::from_bytes(&bytes)
    }

    /// Parses raw book bytes (shared by `open` and tests).
    fn from_bytes(bytes: &[u8]) -> io::Result<Book> {
        if !bytes.len().is_multiple_of(16) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "polyglot book size is not a multiple of 16 bytes",
            ));
        }
        let mut entries = Vec::with_capacity(bytes.len() / 16);
        for chunk in bytes.chunks_exact(16) {
            entries.push(Entry {
                key: u64::from_be_bytes(chunk[0..8].try_into().unwrap()),
                mv: u16::from_be_bytes(chunk[8..10].try_into().unwrap()),
                weight: u16::from_be_bytes(chunk[10..12].try_into().unwrap()),
                // chunk[12..16] is the "learn" field — read but unused
            });
        }
        entries.sort_by_key(|e| e.key);
        Ok(Book { entries })
    }

    /// Number of entries (testing/diagnostics).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the book is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Picks a book move for `pos`, or `None` if the position is not in the
    /// book. Among the matching entries, selection is weighted-random using a
    /// xorshift seeded by `rng_key` (the caller passes a value that is
    /// deterministic per position-per-game but varies across games, e.g.
    /// `pos.key() ^ game_ply`). Zero-weight entries are treated as weight 1 so
    /// they remain reachable. The decoded PolyGlot move is translated into our
    /// `Move` (including the e1h1-style castling convention) by resolving it
    /// against the legal move list — entries that do not resolve are skipped.
    pub fn pick(&self, pos: &Position, rng_key: u64) -> Option<Move> {
        let key = polyglot_key(pos);
        // first matching index (binary search lands somewhere inside the run)
        let lo = self.entries.partition_point(|e| e.key < key);
        let matches = self.entries[lo..]
            .iter()
            .take_while(|e| e.key == key)
            .filter_map(|e| decode_move(pos, e.mv).map(|mv| (mv, e.weight.max(1) as u64)));

        // weighted reservoir-free pass: pick the move whose cumulative weight
        // band contains a single random draw in [0, total).
        let candidates: Vec<(Move, u64)> = matches.collect();
        if candidates.is_empty() {
            return None;
        }
        let total: u64 = candidates.iter().map(|(_, w)| w).sum();
        let mut state = rng_key | 1; // xorshift must not be seeded with 0
        state = xorshift64(state);
        let mut pick = state % total;
        for (mv, w) in &candidates {
            if pick < *w {
                return Some(*mv);
            }
            pick -= *w;
        }
        // unreachable (pick < total), but return the last as a safe fallback
        candidates.last().map(|(mv, _)| *mv)
    }
}

/// xorshift64 (Marsaglia) — std-only PRNG for weighted book selection.
#[inline]
fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// Decodes a PolyGlot 16-bit move against `pos`, returning our `Move`.
///
/// Bit layout: to_file(0..3), to_row(3..6), from_file(6..9), from_row(9..12),
/// promo(12..15). Castling is encoded as the king capturing its own rook
/// (e1h1 / e1a1 / e8h8 / e8a8); we recognise those four king-move shapes and
/// emit the UCI castling string our `find_uci_move` understands. All moves are
/// resolved through `find_uci_move` so the returned `Move` carries the correct
/// flag (capture / double-push / castle) for `make`.
fn decode_move(pos: &Position, raw: u16) -> Option<Move> {
    if raw == 0 {
        return None; // a1a1 sentinel: ignore
    }
    let to_file = (raw & 0x7) as u8;
    let to_row = ((raw >> 3) & 0x7) as u8;
    let from_file = ((raw >> 6) & 0x7) as u8;
    let from_row = ((raw >> 9) & 0x7) as u8;
    let promo = (raw >> 12) & 0x7;

    let from = Square::from_fr(from_file, from_row);
    let to = Square::from_fr(to_file, to_row);

    // Castling: king on e1/e8 "captures" its rook on a/h of the same rank.
    // Translate to the GUI king-target square (g/c file) before resolving.
    let to = translate_castle_target(pos, from, to);

    let mut uci = format!("{from}{to}");
    if promo != 0 {
        uci.push(match promo {
            1 => 'n',
            2 => 'b',
            3 => 'r',
            4 => 'q',
            _ => return None,
        });
    }
    find_uci_move(pos, &uci)
}

/// If `from`->`to` is a PolyGlot castling encoding (king takes own rook), maps
/// the destination to the conventional king square (e1g1/e1c1/e8g8/e8c8). All
/// other moves pass through unchanged. The king-present guard avoids treating
/// a genuine rook-square capture by a non-king as castling.
fn translate_castle_target(pos: &Position, from: Square, to: Square) -> Square {
    let is_king = pos
        .piece_on(from)
        .is_some_and(|p| p.piece_type() == PieceType::King);
    if !is_king {
        return to;
    }
    match (from, to) {
        (Square::E1, Square::H1) => Square::G1,
        (Square::E1, Square::A1) => Square::C1,
        (Square::E8, Square::H8) => Square::G8,
        (Square::E8, Square::A8) => Square::C8,
        _ => to,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replays UCI moves from the start position and returns the result.
    fn after(moves: &[&str]) -> Position {
        let mut pos = Position::startpos();
        for m in moves {
            let mv = find_uci_move(&pos, m).expect("legal test move");
            assert!(pos.make(mv), "test move {m} must be legal");
        }
        pos
    }

    #[test]
    fn random_table_endpoints() {
        // The vendored table must be bit-identical to every other engine's.
        assert_eq!(RANDOM.len(), 781);
        assert_eq!(RANDOM[0], 0x9D39247E33776D41);
        assert_eq!(RANDOM[780], 0xF8D626AAAF278509);
    }

    /// The published reference FEN/key pairs from the format spec. If ANY of
    /// these mismatch, the key composition is wrong — these pin correctness.
    #[test]
    fn anchor_keys_match_published_values() {
        let cases: &[(&[&str], u64)] = &[
            (&[], 0x463B96181691FC9C),
            (&["e2e4"], 0x823C9B50FD114196),
            (&["e2e4", "d7d5"], 0x0756B94461C50FB0),
            (&["e2e4", "d7d5", "e4e5"], 0x662FAFB965DB29D4),
            // the en-passant gotcha: f6 ep IS hashed (white pawn on e5 adjacent)
            (&["e2e4", "d7d5", "e4e5", "f7f5"], 0x22A48B5A8E47FF78),
            (
                &["e2e4", "d7d5", "e4e5", "f7f5", "e1e2"],
                0x652A607CA3F242C1,
            ),
            (
                &["e2e4", "d7d5", "e4e5", "f7f5", "e1e2", "e8f7"],
                0x00FDD303C946BDD9,
            ),
            // bonus ep cases from the spec (c3 ep, then a-file castling rights)
            (
                &["a2a4", "b7b5", "h2h4", "b5b4", "c2c4"],
                0x3C8123EA7B067637,
            ),
            (
                &["a2a4", "b7b5", "h2h4", "b5b4", "c2c4", "b4c3", "a1a3"],
                0x5C3F9B829B279560,
            ),
        ];
        for (moves, expect) in cases {
            let pos = after(moves);
            assert_eq!(
                polyglot_key(&pos),
                *expect,
                "polyglot key mismatch after {moves:?}\n  fen: {}",
                pos.to_fen()
            );
        }
    }

    /// Builds a raw 16-byte big-endian entry.
    fn entry_bytes(key: u64, mv: u16, weight: u16, learn: u32) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&key.to_be_bytes());
        b[8..10].copy_from_slice(&mv.to_be_bytes());
        b[10..12].copy_from_slice(&weight.to_be_bytes());
        b[12..16].copy_from_slice(&learn.to_be_bytes());
        b
    }

    /// Encodes a from/to/promo triple into the PolyGlot 16-bit move.
    fn encode_move(from: Square, to: Square, promo: u16) -> u16 {
        (to.file() as u16)
            | ((to.rank() as u16) << 3)
            | ((from.file() as u16) << 6)
            | ((from.rank() as u16) << 9)
            | (promo << 12)
    }

    #[test]
    fn hand_built_book_round_trips() {
        // Three entries, deliberately written out of key order to exercise the
        // defensive sort and the binary search.
        let start = polyglot_key(&Position::startpos());
        let after_e4 = polyglot_key(&after(&["e2e4"]));
        let e2e4 = encode_move(Square::from_name("e2").unwrap(), Square::from_fr(4, 3), 0);
        let d2d4 = encode_move(Square::from_name("d2").unwrap(), Square::from_fr(3, 3), 0);
        let g8f6 = encode_move(
            Square::from_name("g8").unwrap(),
            Square::from_name("f6").unwrap(),
            0,
        );

        let mut raw = Vec::new();
        // out-of-order on purpose
        raw.extend_from_slice(&entry_bytes(after_e4, g8f6, 10, 0));
        raw.extend_from_slice(&entry_bytes(start, e2e4, 7, 0));
        raw.extend_from_slice(&entry_bytes(start, d2d4, 0, 0)); // zero weight -> floor 1

        let book = Book::from_bytes(&raw).unwrap();
        assert_eq!(book.len(), 3);

        // startpos has two candidates (e2e4 w7, d2d4 w1). Sweep rng_keys and
        // confirm both appear and only those two ever come back.
        let startpos = Position::startpos();
        let mut seen = std::collections::HashSet::new();
        for k in 0..2000u64 {
            let mv = book.pick(&startpos, k).expect("startpos is in book");
            seen.insert(mv.to_string());
        }
        assert_eq!(
            seen,
            ["e2e4".to_string(), "d2d4".to_string()]
                .into_iter()
                .collect(),
            "only the two start entries, weighted"
        );
        // e2e4 (weight 7) should dominate d2d4 (weight 1)
        let e4_hits = (0..2000u64)
            .filter(|&k| book.pick(&startpos, k).unwrap().to_string() == "e2e4")
            .count();
        assert!(
            e4_hits > 1400,
            "weight-7 move should dominate weight-1: {e4_hits}/2000"
        );

        // the after-e4 position resolves the knight move
        let pos_e4 = after(&["e2e4"]);
        assert_eq!(book.pick(&pos_e4, 0).unwrap().to_string(), "g8f6");

        // a position not in the book returns None
        let pos_other = after(&["e2e4", "e7e5"]);
        assert!(book.pick(&pos_other, 0).is_none());
    }

    #[test]
    fn castling_move_decodes_to_our_flag() {
        // White: king on e1, rooks on a1/h1 -> e1h1 / e1a1 encodings.
        let pos = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let key = polyglot_key(&pos);
        let short = encode_move(Square::E1, Square::H1, 0);
        let long = encode_move(Square::E1, Square::A1, 0);
        let raw = [entry_bytes(key, short, 5, 0), entry_bytes(key, long, 5, 0)].concat();
        let book = Book::from_bytes(&raw).unwrap();
        let mut kinds = std::collections::HashSet::new();
        for k in 0..400u64 {
            let mv = book.pick(&pos, k).unwrap();
            kinds.insert(mv.to_string());
            // each must carry the proper castle flag (not a plain king move)
            match mv.to_string().as_str() {
                "e1g1" => assert_eq!(mv.flag(), Move::KING_CASTLE),
                "e1c1" => assert_eq!(mv.flag(), Move::QUEEN_CASTLE),
                other => panic!("unexpected castle decode {other}"),
            }
        }
        assert_eq!(
            kinds,
            ["e1g1".to_string(), "e1c1".to_string()]
                .into_iter()
                .collect(),
        );

        // Black: e8h8 -> e8g8 king-castle.
        let pos_b = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1").unwrap();
        let bk = polyglot_key(&pos_b);
        let bshort = encode_move(Square::E8, Square::H8, 0);
        let book_b = Book::from_bytes(&entry_bytes(bk, bshort, 1, 0)).unwrap();
        let mv = book_b.pick(&pos_b, 0).unwrap();
        assert_eq!(mv.to_string(), "e8g8");
        assert_eq!(mv.flag(), Move::KING_CASTLE);
    }

    #[test]
    fn promotion_move_decodes() {
        // c7c8=Q encoded with promo=4.
        let pos = Position::from_fen("4k3/2P5/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let key = polyglot_key(&pos);
        let c7c8q = encode_move(
            Square::from_name("c7").unwrap(),
            Square::from_name("c8").unwrap(),
            4,
        );
        let book = Book::from_bytes(&entry_bytes(key, c7c8q, 1, 0)).unwrap();
        let mv = book.pick(&pos, 0).unwrap();
        assert_eq!(mv.to_string(), "c7c8q");
        assert!(mv.is_promotion());
        assert_eq!(mv.promotion_piece_type(), PieceType::Queen);
    }

    #[test]
    fn odd_sized_book_is_rejected() {
        let raw = [0u8; 17];
        assert!(Book::from_bytes(&raw).is_err());
    }

    #[test]
    fn zero_move_entry_is_skipped() {
        // an all-zero move (a1a1 sentinel) must not be returned
        let start = polyglot_key(&Position::startpos());
        let book = Book::from_bytes(&entry_bytes(start, 0, 9, 0)).unwrap();
        assert!(book.pick(&Position::startpos(), 0).is_none());
    }
}
