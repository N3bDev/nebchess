//! Tapered HCE. Every term function takes a Tracer; the engine calls with
//! NullTracer (zero cost), the tuner with CollectingTracer. ALL parameter
//! reads go through PARAMS[idx] + trace.record(idx, sign) IN THE SAME
//! STATEMENT GROUP — that invariant is what keeps the tuner honest.

use crate::board::{Bitboard, Color, Move, PieceType, Position};
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
    // T3 mobility; T4 king safety; T5 threats append here
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
        // Non-pawn terms computed fresh; pawn terms via hash
        let (np_mg, np_eg) = eval_non_pawn_terms(pos);
        let (pw_mg, pw_eg) = pawn_terms_cached(&mut self.pawn_hash, pos);
        let white = blend(np_mg + pw_mg, np_eg + pw_eg, ph);
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
            // cached path (cold)
            let cached_cold = {
                let white = {
                    let np = eval_non_pawn_terms(&pos);
                    let pw = pawn_terms_cached(&mut hce.pawn_hash, &pos);
                    blend(np.0 + pw.0, np.1 + pw.1, ph)
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
}
