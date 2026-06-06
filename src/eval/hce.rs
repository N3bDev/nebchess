//! Tapered HCE. Every term function takes a Tracer; the engine calls with
//! NullTracer (zero cost), the tuner with CollectingTracer. ALL parameter
//! reads go through PARAMS[idx] + trace.record(idx, sign) IN THE SAME
//! STATEMENT GROUP — that invariant is what keeps the tuner honest.

use crate::board::{attacks, Bitboard, Color, Move, PieceType, Position};
use crate::eval::eval_params::PARAMS;
use crate::eval::manifest as m;
use crate::eval::trace::{NullTracer, Tracer};
use crate::eval::Evaluator;

// ---- Pawn structure masks (const-built, same while-loop idiom as attacks.rs) ----

const FILE_A: u64 = 0x0101010101010101;

/// adjacent_files[file]: the 1-2 neighboring file(s) as a bitboard (all ranks).
const fn build_adjacent_files() -> [u64; 8] {
    let mut t = [0u64; 8];
    let mut f = 0usize;
    while f < 8 {
        let mut adj = 0u64;
        if f > 0 {
            adj |= FILE_A << (f - 1);
        }
        if f < 7 {
            adj |= FILE_A << (f + 1);
        }
        t[f] = adj;
        f += 1;
    }
    t
}

/// forward_file[color][sq]: same file, all ranks strictly ahead of sq (for doubled detection).
/// color 0 = White (higher ranks ahead), color 1 = Black (lower ranks ahead).
const fn build_forward_file() -> [[u64; 64]; 2] {
    let mut t = [[0u64; 64]; 2];
    let mut sq = 0usize;
    while sq < 64 {
        let file = sq % 8;
        let rank = sq / 8;
        let col = FILE_A << file; // all 8 squares of this file
                                  // White forward: ranks strictly above (rank+1 .. 7)
        let above_mask: u64 = if rank < 7 {
            u64::MAX << ((rank + 1) * 8)
        } else {
            0
        };
        t[0][sq] = col & above_mask;
        // Black forward: ranks strictly below (0 .. rank-1)
        let below_mask: u64 = if rank > 0 {
            u64::MAX >> ((8 - rank) * 8)
        } else {
            0
        };
        t[1][sq] = col & below_mask;
        sq += 1;
    }
    t
}

/// passed_mask[color][sq]: same file + adjacent files, all ranks strictly ahead.
/// A pawn on sq is passed if (passed_mask[color][sq] & enemy_pawns).is_empty().
const fn build_passed_mask() -> [[u64; 64]; 2] {
    let adj = build_adjacent_files();
    let mut t = [[0u64; 64]; 2];
    let mut sq = 0usize;
    while sq < 64 {
        let file = sq % 8;
        let rank = sq / 8;
        let col = FILE_A << file;
        let span = col | adj[file]; // own file + adjacent files
                                    // White: strictly above rank
        let above_mask: u64 = if rank < 7 {
            u64::MAX << ((rank + 1) * 8)
        } else {
            0
        };
        t[0][sq] = span & above_mask;
        // Black: strictly below rank
        let below_mask: u64 = if rank > 0 {
            u64::MAX >> ((8 - rank) * 8)
        } else {
            0
        };
        t[1][sq] = span & below_mask;
        sq += 1;
    }
    t
}

static ADJACENT_FILES: [u64; 8] = build_adjacent_files();
static FORWARD_FILE: [[u64; 64]; 2] = build_forward_file();
static PASSED_MASK: [[u64; 64]; 2] = build_passed_mask();

#[inline]
fn passed_mask(color: Color, sq: crate::board::Square) -> Bitboard {
    Bitboard(PASSED_MASK[color.index()][sq.index()])
}

#[inline]
fn adjacent_files(file: u8) -> Bitboard {
    Bitboard(ADJACENT_FILES[file as usize])
}

#[inline]
fn forward_file(color: Color, sq: crate::board::Square) -> Bitboard {
    Bitboard(FORWARD_FILE[color.index()][sq.index()])
}

// ---- Pawn hash ----
const PAWN_HASH_SIZE: usize = 16384; // entries; (u64, i32, i32) = 16B -> 256KB

/// Game phase: N/B=1, R=2, Q=4 per piece, capped at 24 (opening) .. 0 (bare kings).
pub fn phase(pos: &Position) -> i32 {
    let mut p = 0;
    for color in [Color::White, Color::Black] {
        p += pos.piece_bb(color, PieceType::Knight).count() as i32;
        p += pos.piece_bb(color, PieceType::Bishop).count() as i32;
        p += 2 * pos.piece_bb(color, PieceType::Rook).count() as i32;
        p += 4 * pos.piece_bb(color, PieceType::Queen).count() as i32;
    }
    p.min(24)
}

/// Shared add-term helper (promoted from closure in T2+ for all term functions).
#[inline]
fn add_term<T: Tracer>(idx: usize, sign: i32, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let (pmg, peg) = PARAMS[idx];
    *mg += sign * pmg;
    *eg += sign * peg;
    t.record(idx, sign as i8);
}

/// Pawn structure terms: passed, connected passers, isolated, doubled.
fn pawn_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        let own = pos.piece_bb(color, PieceType::Pawn);
        let enemy = pos.piece_bb(color.flip(), PieceType::Pawn);
        for sq in own {
            let rel_rank = if color == Color::White {
                sq.rank()
            } else {
                7 - sq.rank()
            } as usize;
            // Passed pawn: no enemy pawns on same or adjacent files ahead
            if (passed_mask(color, sq) & enemy).is_empty() {
                // rel_rank for a pawn is always 1..=6; PASSED indexes 0..=5
                add_term(m::PASSED + rel_rank - 1, sign, t, mg, eg);
                // Connected passer: own pawn on adjacent file (any rank)
                if (adjacent_files(sq.file()) & own).any() {
                    add_term(m::PASSED_CONNECTED, sign, t, mg, eg);
                }
            }
            // Isolated: no own pawn on adjacent files
            if (adjacent_files(sq.file()) & own).is_empty() {
                add_term(m::ISOLATED, sign, t, mg, eg);
            }
            // Doubled: own pawn strictly ahead on same file
            if (forward_file(color, sq) & own).any() {
                add_term(m::DOUBLED, sign, t, mg, eg);
            }
        }
    }
}

// ---- T3: mobility ----

/// All squares attacked by `color`'s pawns (whole-set shift, no per-pawn loop).
#[inline]
fn pawn_attack_set(pos: &Position, color: Color) -> Bitboard {
    let p = pos.piece_bb(color, PieceType::Pawn);
    match color {
        Color::White => p.north_east() | p.north_west(),
        Color::Black => p.south_east() | p.south_west(),
    }
}

/// Safe-mobility per piece: count attacked squares that are neither occupied by
/// our own pieces nor attacked by an enemy pawn, indexing a per-piece table.
/// A 0-mobility piece reads the most negative cell — that IS the trapped-piece
/// term. Depends on ALL pieces (slider occupancy), so this is NOT pawn-cacheable:
/// it runs in the fresh path of Hce::evaluate and in eval_terms (tuner path).
fn mobility_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let occ = pos.occ_all();
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        // safe = not our own pieces, not attacked by an enemy pawn
        let safe = !pos.occ(color) & !pawn_attack_set(pos, color.flip());
        for sq in pos.piece_bb(color, PieceType::Knight) {
            let n = (attacks::knight_attacks(sq) & safe).count() as usize;
            add_term(m::MOB_KNIGHT + n, sign, t, mg, eg);
        }
        for sq in pos.piece_bb(color, PieceType::Bishop) {
            let n = (attacks::bishop_attacks(sq, occ) & safe).count() as usize;
            add_term(m::MOB_BISHOP + n, sign, t, mg, eg);
        }
        for sq in pos.piece_bb(color, PieceType::Rook) {
            let n = (attacks::rook_attacks(sq, occ) & safe).count() as usize;
            add_term(m::MOB_ROOK + n, sign, t, mg, eg);
        }
        for sq in pos.piece_bb(color, PieceType::Queen) {
            let n = (attacks::queen_attacks(sq, occ) & safe).count() as usize;
            add_term(m::MOB_QUEEN + n, sign, t, mg, eg);
        }
    }
}

// ---- T4: king safety ----

/// All 8 squares of file `f` as a bitboard.
#[inline]
fn file_mask(f: u8) -> Bitboard {
    Bitboard(FILE_A << f)
}

/// The single rank `ahead` steps in front of `ksq`, color-relative, as a
/// full-rank bitboard. White advances toward higher ranks, Black toward lower.
/// Returns an empty bitboard when the target rank is off the board (so a king
/// already on the back two ranks simply has no shield there).
#[inline]
fn shield_rank(color: Color, ksq: crate::board::Square, ahead: i8) -> Bitboard {
    let kr = ksq.rank() as i8;
    let r = match color {
        Color::White => kr + ahead,
        Color::Black => kr - ahead,
    };
    if (0..8).contains(&r) {
        Bitboard(0xFFu64 << (r * 8))
    } else {
        Bitboard::EMPTY
    }
}

/// King safety: enemy pieces touching the king zone (by type), pawn shield
/// state on the king's file and its neighbors, and open/semi-open files near
/// the king. Depends on ALL pieces (slider occupancy, both armies) -> NOT
/// pawn-cacheable: like mobility, it runs fresh in Hce::evaluate AND in the
/// tuner path (eval_terms).
///
/// SIGN CONVENTION: every feature is recorded in the white-relative frame with
/// `sign = +1` when the WHITE king is the subject and `-1` when the black king
/// is. The tuner learns whether each parameter is good or bad for the king's
/// owner; penalties therefore come out NEGATIVE on their own (a white-king
/// danger feature, recorded +1, gets a negative tuned weight). Shield bonuses
/// stay positive. The shield-file loop spans `kfile-1..=kfile+1` clamped to
/// [0,7], so a king on the a/h file is scored over 2 files, not 3 — intended,
/// matching standard HCE practice.
fn king_safety_terms<T: Tracer>(pos: &Position, t: &mut T, mg: &mut i32, eg: &mut i32) {
    let occ = pos.occ_all();
    for (color, sign) in [(Color::White, 1i32), (Color::Black, -1i32)] {
        let ksq = pos.king_sq(color);
        let zone = attacks::king_attacks(ksq) | ksq.bb();
        let enemy = color.flip();
        // Enemy attackers touching the king zone, counted per piece type.
        for (pt, slot) in [
            (PieceType::Knight, 0usize),
            (PieceType::Bishop, 1),
            (PieceType::Rook, 2),
            (PieceType::Queen, 3),
        ] {
            for sq in pos.piece_bb(enemy, pt) {
                let att = match pt {
                    PieceType::Knight => attacks::knight_attacks(sq),
                    PieceType::Bishop => attacks::bishop_attacks(sq, occ),
                    PieceType::Rook => attacks::rook_attacks(sq, occ),
                    _ => attacks::queen_attacks(sq, occ),
                };
                if (att & zone).any() {
                    add_term(m::KS_ATTACKER + slot, sign, t, mg, eg);
                }
            }
        }
        // Pawn shield + file state on the king's file and its neighbors.
        let own_pawns = pos.piece_bb(color, PieceType::Pawn);
        let all_pawns = own_pawns | pos.piece_bb(enemy, PieceType::Pawn);
        let kfile = ksq.file() as i8;
        // Shield = our own pawns on the two ranks directly ahead of the king,
        // checked per-rank (no shield_span union — the per-rank intersection
        // is equivalent and clearer; reported as the implementer's choice).
        let r1 = shield_rank(color, ksq, 1);
        let r2 = shield_rank(color, ksq, 2);
        for f in (kfile - 1).max(0)..=(kfile + 1).min(7) {
            let file_bb = file_mask(f as u8);
            // Shield state, nearest rank first (one rank ahead, then two).
            if (own_pawns & file_bb & r1).any() {
                add_term(m::KS_SHIELD, sign, t, mg, eg); // pawn one rank ahead
            } else if (own_pawns & file_bb & r2).any() {
                add_term(m::KS_SHIELD + 1, sign, t, mg, eg); // pawn two ranks ahead
            } else {
                add_term(m::KS_SHIELD + 2, sign, t, mg, eg); // shield missing
            }
            // Open / semi-open file near the king.
            if (all_pawns & file_bb).is_empty() {
                add_term(m::KS_OPEN_FILE, sign, t, mg, eg);
            } else if (own_pawns & file_bb).is_empty() {
                add_term(m::KS_SEMI_FILE, sign, t, mg, eg);
            }
        }
    }
}

/// White-relative (mg, eg) accumulation over all terms.
/// This path is UNCACHED — used by the tuner (CollectingTracer) and as the
/// reference for the transparency test. The engine's Hce::evaluate uses a
/// pawn hash for the pawn structure terms.
pub fn eval_terms<T: Tracer>(pos: &Position, t: &mut T) -> (i32, i32) {
    let (mut mg, mut eg) = (0i32, 0i32);

    const PST: [usize; 6] = [
        m::PST_PAWN,
        m::PST_KNIGHT,
        m::PST_BISHOP,
        m::PST_ROOK,
        m::PST_QUEEN,
        m::PST_KING,
    ];
    for pt in PieceType::ALL {
        for sq in pos.piece_bb(Color::White, pt) {
            add_term(m::MATERIAL + pt.index(), 1, t, &mut mg, &mut eg);
            add_term(PST[pt.index()] + (sq.index() ^ 56), 1, t, &mut mg, &mut eg);
        }
        for sq in pos.piece_bb(Color::Black, pt) {
            add_term(m::MATERIAL + pt.index(), -1, t, &mut mg, &mut eg);
            add_term(PST[pt.index()] + sq.index(), -1, t, &mut mg, &mut eg);
        }
    }
    // T2: pawn structure (uncached — tuner path)
    pawn_terms(pos, t, &mut mg, &mut eg);
    // T3: mobility (not pawn-cacheable — depends on all pieces)
    mobility_terms(pos, t, &mut mg, &mut eg);
    // T4: king safety (not pawn-cacheable — depends on all pieces)
    king_safety_terms(pos, t, &mut mg, &mut eg);
    // T5 threats append here
    (mg, eg)
}

/// Blend (mg, eg) by phase; result is white-relative.
#[inline]
fn blend(mg: i32, eg: i32, ph: i32) -> i32 {
    (mg * ph + eg * (24 - ph)) / 24
}

/// Non-pawn terms only (material + PST for all pieces), white-relative.
/// Used by eval_terms_cached to compute the non-pawn portion fresh.
fn eval_non_pawn_terms(pos: &Position) -> (i32, i32) {
    let (mut mg, mut eg) = (0i32, 0i32);
    const PST: [usize; 6] = [
        m::PST_PAWN,
        m::PST_KNIGHT,
        m::PST_BISHOP,
        m::PST_ROOK,
        m::PST_QUEEN,
        m::PST_KING,
    ];
    for pt in PieceType::ALL {
        for sq in pos.piece_bb(Color::White, pt) {
            let (pmg, peg) = PARAMS[m::MATERIAL + pt.index()];
            mg += pmg;
            eg += peg;
            let (pmg, peg) = PARAMS[PST[pt.index()] + (sq.index() ^ 56)];
            mg += pmg;
            eg += peg;
        }
        for sq in pos.piece_bb(Color::Black, pt) {
            let (pmg, peg) = PARAMS[m::MATERIAL + pt.index()];
            mg -= pmg;
            eg -= peg;
            let (pmg, peg) = PARAMS[PST[pt.index()] + sq.index()];
            mg -= pmg;
            eg -= peg;
        }
    }
    (mg, eg)
}

/// Compute pawn terms with a pawn hash (replace-always policy).
/// The cache entry stores (pawn_key, mg, eg). A 0-key entry is empty.
fn pawn_terms_cached(hash: &mut [(u64, i32, i32)], pos: &Position) -> (i32, i32) {
    let key = pos.pawn_key();
    let slot = (key as usize) & (PAWN_HASH_SIZE - 1);
    let entry = hash[slot];
    if entry.0 == key {
        return (entry.1, entry.2);
    }
    // Miss: compute and store (uncached, NullTracer)
    let (mut mg, mut eg) = (0i32, 0i32);
    pawn_terms(pos, &mut NullTracer, &mut mg, &mut eg);
    hash[slot] = (key, mg, eg);
    (mg, eg)
}

/// Blend by phase and flip to side-to-move-relative.
pub fn evaluate_white_relative(pos: &Position) -> i32 {
    let (mg, eg) = eval_terms(pos, &mut NullTracer);
    let ph = phase(pos);
    blend(mg, eg, ph)
}

pub struct Hce {
    /// Pawn hash table: (pawn_key, mg, eg); replace-always policy.
    /// The TRACED path (eval_terms, used by the tuner) bypasses this cache
    /// entirely — caches and trace records don't mix.
    pawn_hash: Vec<(u64, i32, i32)>,
}

impl Default for Hce {
    fn default() -> Hce {
        Hce::new()
    }
}

impl Hce {
    pub fn new() -> Hce {
        Hce {
            pawn_hash: vec![(0u64, 0i32, 0i32); PAWN_HASH_SIZE],
        }
    }
}

impl Evaluator for Hce {
    fn refresh(&mut self, _pos: &Position) {}
    fn on_make(&mut self, _mv: Move, _pos: &Position) {}
    fn on_unmake(&mut self, _mv: Move, _pos: &Position) {}

    fn evaluate(&mut self, pos: &Position) -> i32 {
        let ph = phase(pos);
        // Non-pawn material/PST computed fresh; pawn structure via hash.
        let (np_mg, np_eg) = eval_non_pawn_terms(pos);
        let (pw_mg, pw_eg) = pawn_terms_cached(&mut self.pawn_hash, pos);
        // Mobility and king safety depend on all pieces -> NOT pawn-cacheable:
        // compute fresh (NullTracer = zero cost) so the engine path matches
        // eval_terms exactly.
        let (mut fresh_mg, mut fresh_eg) = (0i32, 0i32);
        mobility_terms(pos, &mut NullTracer, &mut fresh_mg, &mut fresh_eg);
        king_safety_terms(pos, &mut NullTracer, &mut fresh_mg, &mut fresh_eg);
        let white = blend(np_mg + pw_mg + fresh_mg, np_eg + pw_eg + fresh_eg, ph);
        if pos.stm() == Color::White {
            white
        } else {
            -white
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Position;
    use crate::eval::eval_params::PARAMS;
    use crate::eval::manifest::{self, TOTAL_PAIRS};
    use crate::eval::trace::CollectingTracer;

    #[test]
    fn params_len_matches_total_pairs() {
        assert_eq!(
            PARAMS.len(),
            TOTAL_PAIRS,
            "eval_params.rs length {} doesn't match manifest TOTAL_PAIRS {}",
            PARAMS.len(),
            TOTAL_PAIRS
        );
    }

    #[test]
    fn phase_startpos_is_24() {
        let pos = Position::startpos();
        assert_eq!(phase(&pos), 24, "startpos has full 24-point phase");
    }

    #[test]
    fn phase_bare_kings_is_zero() {
        // Bare kings: both kings only, no other pieces
        let fen = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(phase(&pos), 0, "bare kings have 0 phase");
    }

    #[test]
    fn startpos_is_balanced() {
        let mut e = Hce::new();
        let pos = Position::startpos();
        assert_eq!(e.evaluate(&pos), 0, "symmetric position must be 0");
    }

    #[test]
    fn eval_is_stm_relative() {
        // same physical position, both side-to-move variants: scores negate
        // NOTE: since mg==eg in the seed (no tapering divergence), the eval is
        // phase-independent, so stm negation holds exactly at this seed stage.
        let w = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1";
        let b = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let mut e = Hce::new();
        let sw = e.evaluate(&Position::from_fen(w).unwrap());
        let sb = e.evaluate(&Position::from_fen(b).unwrap());
        assert_eq!(sw, -sb);
        // e2->e4 is a PST improvement for White
        assert!(sw > 0, "White improved by e4, White to move: positive");
    }

    #[test]
    fn material_dominates_pst() {
        // White is a clean knight up; score from White's view >> 200cp
        let fen = "rnbqkb1r/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let mut e = Hce::new();
        let s = e.evaluate(&Position::from_fen(fen).unwrap());
        assert!(s > 200, "knight-up should exceed 200cp, got {s}");
        assert!(s < 500, "but not exceed knight+max-pst, got {s}");
    }

    #[test]
    fn hooks_are_callable_noops() {
        // the seam contract: search calls these unconditionally from M2 on
        let mut e = Hce::new();
        let mut pos = Position::startpos();
        e.refresh(&pos);
        let before = e.evaluate(&pos);
        let mv = crate::board::movegen::find_uci_move(&pos, "e2e4").unwrap();
        assert!(pos.make(mv));
        e.on_make(mv, &pos);
        pos.unmake();
        e.on_unmake(mv, &pos);
        assert_eq!(e.evaluate(&pos), before, "no-op hooks don't corrupt eval");
    }

    #[test]
    fn mirrored_position_negates() {
        // asymmetric position and its color-flipped mirror: stm-relative
        // scores must be equal (White's edge becomes Black's edge).
        let orig = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 1";
        let flip = "rnbqk2r/pppp1ppp/5n2/2b1p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 0 1";
        let mut e = Hce::new();
        let a = e.evaluate(&Position::from_fen(orig).unwrap());
        let b = e.evaluate(&Position::from_fen(flip).unwrap());
        assert_eq!(a, b, "color-flip symmetry violated: {a} vs {b}");
    }

    // ---- Step 2.3: pawn hash transparency test ----

    /// Hce::evaluate (cached path) must equal the uncached traced result blended
    /// by phase, for a set of varied FENs.
    #[test]
    fn pawn_hash_transparent_to_uncached() {
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "4k3/8/8/3P4/8/8/8/4K3 w - - 0 1", // lone passed pawn
            "4k3/p7/8/P7/8/8/8/4K3 w - - 0 1", // both sides, white passed
            "r1bqkbnr/pp1ppppp/2n5/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 3",
            "8/2p2pp1/3k4/p7/P1P5/8/5PPP/4K3 w - - 0 1", // complex pawn structure
        ];
        let mut hce = Hce::new();
        for fen in fens {
            let pos = Position::from_fen(fen).unwrap();
            let ph = phase(&pos);
            // uncached traced path (what the tuner sees)
            let (mg_t, eg_t) = eval_terms(&pos, &mut crate::eval::trace::NullTracer);
            let uncached = blend(mg_t, eg_t, ph);
            // cached path (cold) — mirrors Hce::evaluate
            // (np + pawn-hash + fresh mobility + fresh king safety)
            let cached_cold = {
                let white = {
                    let np = eval_non_pawn_terms(&pos);
                    let pw = pawn_terms_cached(&mut hce.pawn_hash, &pos);
                    let (mut fresh_mg, mut fresh_eg) = (0i32, 0i32);
                    mobility_terms(&pos, &mut NullTracer, &mut fresh_mg, &mut fresh_eg);
                    king_safety_terms(&pos, &mut NullTracer, &mut fresh_mg, &mut fresh_eg);
                    blend(np.0 + pw.0 + fresh_mg, np.1 + pw.1 + fresh_eg, ph)
                };
                if pos.stm() == Color::White {
                    white
                } else {
                    -white
                }
            };
            // stm-relative comparison
            let uncached_stm = if pos.stm() == Color::White {
                uncached
            } else {
                -uncached
            };
            assert_eq!(
                cached_cold, uncached_stm,
                "pawn hash cold miss mismatch on FEN: {fen}"
            );
            // warm (cache hit)
            let cached_warm = hce.evaluate(&pos);
            assert_eq!(
                cached_warm, uncached_stm,
                "pawn hash warm hit mismatch on FEN: {fen}"
            );
        }
    }

    // ---- Step 2.4: trace-based feature record tests ----

    fn features_at(fen: &str, idx: usize) -> Vec<i8> {
        let pos = Position::from_fen(fen).unwrap();
        let mut tr = CollectingTracer::default();
        eval_terms(&pos, &mut tr);
        tr.features
            .iter()
            .filter(|&&(i, _)| i as usize == idx)
            .map(|&(_, s)| s)
            .collect()
    }

    /// White lone passed pawn on d5 (rank 5 from white's perspective = rel_rank 4,
    /// PASSED offset 3). Bare-kings position.
    /// spec example: "4k3/8/8/3P4/8/8/8/4K3 w"
    #[test]
    fn passed_pawn_records_correct_rank() {
        let fen = "4k3/8/8/3P4/8/8/8/4K3 w - - 0 1";
        // d5 = rank 4 (0-indexed), white rel_rank = 4, PASSED index = PASSED + 4 - 1 = PASSED + 3
        let expected_idx = manifest::PASSED + 3;
        let signs = features_at(fen, expected_idx);
        assert_eq!(
            signs,
            vec![1i8],
            "d5 pawn: exactly one PASSED+3 record with sign +1"
        );
        // No lower-rank PASSED records
        for r in 0..3usize {
            let s = features_at(fen, manifest::PASSED + r);
            assert!(s.is_empty(), "no PASSED+{r} record expected");
        }
    }

    /// Isolated pawn test: white e-pawn alone (no d or f pawn).
    #[test]
    fn isolated_pawn_detected() {
        // White e2 pawn, black king, no other pawns
        let fen = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
        let signs = features_at(fen, manifest::ISOLATED);
        assert_eq!(signs, vec![1i8], "lone e2 pawn is isolated");
        // No DOUBLED
        assert!(
            features_at(fen, manifest::DOUBLED).is_empty(),
            "no doubled pawn"
        );
    }

    /// Doubled pawn test: white pawns on e2 and e4.
    #[test]
    fn doubled_pawn_detected() {
        let fen = "4k3/8/8/8/4P3/8/4P3/4K3 w - - 0 1";
        let doubled = features_at(fen, manifest::DOUBLED);
        // The rear pawn (e2) has the front pawn (e4) ahead: one DOUBLED +1 record.
        // The front pawn (e4) has no pawn ahead: no DOUBLED for it.
        assert_eq!(doubled, vec![1i8], "one doubled record for the rear pawn");
    }

    /// Connected passers: two white passed pawns on adjacent files.
    #[test]
    fn connected_passer_detected() {
        // White d5 and e5 pawns; no black pawns. Both are passed and adjacent.
        let fen = "4k3/8/8/3PP3/8/8/8/4K3 w - - 0 1";
        // Both d5 and e5 are passed (rel_rank 4, PASSED+3) and adjacent to each other
        let passed = features_at(fen, manifest::PASSED + 3);
        assert_eq!(passed.len(), 2, "two passed pawns at rel_rank 5");
        let connected = features_at(fen, manifest::PASSED_CONNECTED);
        assert_eq!(connected.len(), 2, "both passers record PASSED_CONNECTED");
        assert!(connected.iter().all(|&s| s == 1), "all signs +1 for white");
    }

    /// Black passed pawn: verify sign is -1 (black's advantage is subtracted).
    #[test]
    fn black_passed_pawn_sign_is_negative() {
        // Black pawn on d4 (rank 3, black rel_rank = 7-3 = 4, PASSED+3), no white pawns
        let fen = "4k3/8/8/8/3p4/8/8/4K3 w - - 0 1";
        let signs = features_at(fen, manifest::PASSED + 3);
        assert_eq!(signs, vec![-1i8], "black passed pawn gets sign -1");
    }

    // ---- Step 3.4: mobility trace tests ----

    /// Helper: collect (idx, sign) records whose idx falls in [base, base+len).
    fn records_in_table(fen: &str, base: usize, len: usize) -> Vec<(usize, i8)> {
        let pos = Position::from_fen(fen).unwrap();
        let mut tr = CollectingTracer::default();
        eval_terms(&pos, &mut tr);
        tr.features
            .iter()
            .map(|&(i, s)| (i as usize, s))
            .filter(|&(i, _)| i >= base && i < base + len)
            .collect()
    }

    /// Startpos: each knight has exactly 2 safe squares. The b1/g1 knights reach
    /// a3,c3 and f3,h3 respectively; none are attacked by an enemy pawn (enemy
    /// pawns on rank 7 attack rank 6) and none are own-occupied. So every knight
    /// records MOB_KNIGHT+2 — two white (+1) and two black (-1).
    #[test]
    fn startpos_knight_mobility_is_two() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let recs = records_in_table(fen, manifest::MOB_KNIGHT, 9);
        // All four knight records must be exactly at MOB_KNIGHT+2.
        assert_eq!(recs.len(), 4, "four knights total, got {recs:?}");
        for &(idx, _) in &recs {
            assert_eq!(
                idx,
                manifest::MOB_KNIGHT + 2,
                "every startpos knight has 2 safe squares; got idx {idx}"
            );
        }
        let white = recs.iter().filter(|&&(_, s)| s == 1).count();
        let black = recs.iter().filter(|&&(_, s)| s == -1).count();
        assert_eq!((white, black), (2, 2), "two white +1, two black -1");
    }

    /// A corner-trapped bishop with exactly 0 safe squares records MOB_BISHOP+0.
    /// White bishop a1, white pawn b2: the bishop's only attacked square is b2
    /// (own-occupied -> not safe), the ray is blocked there, so safe count = 0.
    /// FEN legality: white Ka1?? no — kings apart. Use white Kc1, black Kc8.
    #[test]
    fn trapped_bishop_records_zero_mobility() {
        // White: Ba1, Pb2, Kc1.  Black: Kc8.  White to move (irrelevant to eval).
        let fen = "2k5/8/8/8/8/8/1P6/B1K5 w - - 0 1";
        let recs = records_in_table(fen, manifest::MOB_BISHOP, 14);
        assert_eq!(
            recs,
            vec![(manifest::MOB_BISHOP, 1i8)],
            "trapped a1 bishop (pawn b2) has 0 safe squares -> exactly one MOB_BISHOP+0, sign +1"
        );
    }

    // ---- Step 4.3: king-safety trace tests ----

    /// Collect (idx, sign) records in [base, base+len) restricted to one sign
    /// (used to isolate the WHITE king's features from the BLACK king's, since
    /// neighboring shield-file loops can overlap on a shared file).
    fn ks_records(fen: &str, base: usize, len: usize, want_sign: i8) -> Vec<usize> {
        let pos = Position::from_fen(fen).unwrap();
        let mut tr = CollectingTracer::default();
        eval_terms(&pos, &mut tr);
        tr.features
            .iter()
            .filter(|&&(i, s)| (i as usize) >= base && (i as usize) < base + len && s == want_sign)
            .map(|&(i, _)| i as usize)
            .collect()
    }

    /// Castled white king with an intact pawn shield: Kg1 with pawns f2/g2/h2.
    /// White's shield loop spans files f,g,h; each has an own pawn one rank
    /// ahead (rank 2) -> three KS_SHIELD+0 records (sign +1), and none of those
    /// files is open or semi-open for white. Black king is parked on a8 with an
    /// a7 pawn so its own shield/file records land on the a/b files (sign -1),
    /// well clear of white's f/g/h files.
    #[test]
    fn castled_king_full_shield() {
        // White: Kg1, Pf2,Pg2,Ph2.  Black: Ka8, Pa7.  Legal, no side-effect check.
        let fen = "k7/p7/8/8/8/8/5PPP/6K1 w - - 0 1";
        // Validate legality before asserting.
        let pos = Position::from_fen(fen).expect("test FEN must be legal");
        assert_eq!(pos.king_sq(Color::White), crate::board::Square::G1);

        // Exactly three KS_SHIELD+0 (sign +1) for white; none at +1 or +2.
        let shield0 = ks_records(fen, manifest::KS_SHIELD, 1, 1);
        assert_eq!(
            shield0.len(),
            3,
            "Kg1 with f2/g2/h2: three shield-pawn-one-rank-ahead records, got {shield0:?}"
        );
        assert!(
            ks_records(fen, manifest::KS_SHIELD + 1, 1, 1).is_empty(),
            "no rel-rank-3 shield records for white"
        );
        assert!(
            ks_records(fen, manifest::KS_SHIELD + 2, 1, 1).is_empty(),
            "no missing-shield records for white"
        );
        // No open/semi files among white's f/g/h (all carry a white pawn).
        assert!(
            ks_records(fen, manifest::KS_OPEN_FILE, 1, 1).is_empty(),
            "no open file near the white king"
        );
        assert!(
            ks_records(fen, manifest::KS_SEMI_FILE, 1, 1).is_empty(),
            "no semi-open file near the white king"
        );
    }

    /// Stripped white king: Kg1 with NO f/g/h pawns, and an enemy queen+rook
    /// bearing on the king zone. White's shield loop (files f,g,h) finds no own
    /// pawn anywhere ahead -> three KS_SHIELD+2 (missing) records (sign +1).
    /// All three files are pawnless -> three KS_OPEN_FILE records. The black
    /// queen (h4: hits h1/h2/f2 in the zone) and rook (f8: hits f1 down the
    /// open f-file) both touch the king zone -> KS_ATTACKER records (slots Q
    /// and R). Neither piece checks the g1 king (no line to g1), so the FEN is
    /// a legal white-to-move position with no side-effect check.
    #[test]
    fn stripped_king_open_files_and_attackers() {
        // White: Kg1.  Black: Ka8, Qh4, Rf8.  White to move, not in check.
        let fen = "k4r2/8/8/8/7q/8/8/6K1 w - - 0 1";
        let pos = Position::from_fen(fen).expect("test FEN must be legal");
        assert_eq!(pos.king_sq(Color::White), crate::board::Square::G1);

        // Three missing-shield records (sign +1) on f/g/h; none nearer.
        let missing = ks_records(fen, manifest::KS_SHIELD + 2, 1, 1);
        assert_eq!(
            missing.len(),
            3,
            "Kg1 with no f/g/h pawns: three missing-shield records, got {missing:?}"
        );
        assert!(
            ks_records(fen, manifest::KS_SHIELD, 1, 1).is_empty(),
            "no one-rank-ahead shield for the stripped king"
        );
        assert!(
            ks_records(fen, manifest::KS_SHIELD + 1, 1, 1).is_empty(),
            "no two-ranks-ahead shield for the stripped king"
        );
        // Three open files (f/g/h have no pawns of either color).
        let open = ks_records(fen, manifest::KS_OPEN_FILE, 1, 1);
        assert_eq!(
            open.len(),
            3,
            "all three files near the stripped king are open, got {open:?}"
        );
        // King-zone attackers present: the black queen and rook both bear on g1.
        let attackers = ks_records(fen, manifest::KS_ATTACKER, 4, 1);
        assert!(
            !attackers.is_empty(),
            "enemy queen+rook on the g-file must touch the king zone"
        );
        assert!(
            attackers.contains(&(manifest::KS_ATTACKER + 3)),
            "the black queen (slot 3) attacks the zone, got {attackers:?}"
        );
        assert!(
            attackers.contains(&(manifest::KS_ATTACKER + 2)),
            "the black rook (slot 2) attacks the zone, got {attackers:?}"
        );
    }
}
