//! Syzygy tablebase probing — a thin wrapper over `pyrrhic-rs` (the project's
//! single external dependency; see Cargo.toml for the policy exception).
//!
//! The public surface here is OURS and fixed regardless of the crate's version:
//! [`Tb::init`], [`Tb::probe_wdl`], [`Tb::probe_root`], and the [`Wdl`] enum.
//! The internals adapt to pyrrhic-rs 0.2's `TableBases<EngineAdapter>` API.
//!
//! Square / bitboard convention: pyrrhic-rs uses LERF (a1 = bit 0 .. h8 = bit
//! 63), identical to NebChess — bitboards pass through with no transform. The
//! ONE mismatch is `Color` (pyrrhic: Black=0/White=1; ours: White=0/Black=1),
//! handled in the adapter's `pawn_attacks`.

use crate::board::{attacks, Bitboard, Color, Move, PieceType, Position, Square};
use pyrrhic_rs::{Color as TbColor, DtzProbeValue, EngineAdapter, TableBases, WdlProbeResult};

/// Win/Draw/Loss from the side-to-move's perspective. The 50-move-cursed
/// variants (`CursedWin`, `BlessedLoss`) collapse to `Draw`: WDL probes only
/// fire at `halfmove == 0` (see `probeable_wdl`), and root probes hand pyrrhic
/// the real halfmove clock — in both cases a cursed result means the win/loss
/// is NOT reachable inside the 50-move rule, so scoring it a draw is the safe,
/// never-overclaiming choice.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Wdl {
    Win,
    Draw,
    Loss,
}

impl From<WdlProbeResult> for Wdl {
    fn from(w: WdlProbeResult) -> Wdl {
        match w {
            WdlProbeResult::Win => Wdl::Win,
            WdlProbeResult::Loss => Wdl::Loss,
            // Draw, and the cursed/blessed pair (see the enum doc).
            _ => Wdl::Draw,
        }
    }
}

/// Zero-sized adapter wiring pyrrhic-rs's probe callbacks to NebChess's attack
/// tables. All methods are pure functions of (square, occupancy). `Clone` is
/// required by the `EngineAdapter` bound (the crate carries it as `PhantomData`).
#[derive(Clone, Copy)]
struct Adapter;

#[inline]
fn sq(square: u64) -> Square {
    Square::new(square as u8)
}

impl EngineAdapter for Adapter {
    fn pawn_attacks(color: TbColor, square: u64) -> u64 {
        // pyrrhic Color: Black=0, White=1 — translate to ours.
        let c = match color {
            TbColor::White => Color::White,
            TbColor::Black => Color::Black,
        };
        attacks::pawn_attacks(c, sq(square)).0
    }
    fn knight_attacks(square: u64) -> u64 {
        attacks::knight_attacks(sq(square)).0
    }
    fn bishop_attacks(square: u64, occupied: u64) -> u64 {
        attacks::bishop_attacks(sq(square), Bitboard(occupied)).0
    }
    fn rook_attacks(square: u64, occupied: u64) -> u64 {
        attacks::rook_attacks(sq(square), Bitboard(occupied)).0
    }
    fn queen_attacks(square: u64, occupied: u64) -> u64 {
        attacks::queen_attacks(sq(square), Bitboard(occupied)).0
    }
    fn king_attacks(square: u64) -> u64 {
        attacks::king_attacks(sq(square)).0
    }
}

/// Loaded Syzygy tablebases plus the largest piece count they cover.
pub struct Tb {
    inner: TableBases<Adapter>,
    max_men: u32,
}

/// Per-position bitboards in the layout every pyrrhic probe wants.
struct Bits {
    white: u64,
    black: u64,
    kings: u64,
    queens: u64,
    rooks: u64,
    bishops: u64,
    knights: u64,
    pawns: u64,
}

impl Tb {
    /// Load tablebases from `path` (colon-separated dirs accepted). Returns
    /// `None` for an empty path or a directory holding no usable tables; never
    /// panics, so a bad `SyzygyPath` simply leaves the feature off.
    ///
    /// IMPORTANT (pyrrhic-rs 0.2 quirk): `TableBases::new` succeeds and reports
    /// `max_pieces() == 7` even when the directory contains NO tables — the
    /// `TB_LARGEST == 0` guard never fires in this version. So `max_pieces` is
    /// not a usable presence check. We instead VALIDATE with a canary probe of
    /// KQvK (present in every standard 3-4-5 set, an unambiguous Win); if that
    /// misses, the directory has no real tables and we report `None`.
    ///
    /// NOTE: pyrrhic-rs keeps a process-global singleton — only the first
    /// successful `init` in a process binds the path (a later `init` returns
    /// `AlreadyInitialized` -> `None`). The engine sets `SyzygyPath` once, so
    /// this is a non-issue in practice; documented for the test harness.
    pub fn init(path: &str) -> Option<Tb> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let inner = TableBases::<Adapter>::new(path).ok()?;
        let max_men = inner.max_pieces();
        if max_men < 3 {
            return None;
        }
        let tb = Tb { inner, max_men };
        // Canary: KQvK (white Ke1, Qd1; black Ke8) must resolve to a Win. A
        // miss means the path holds no actual tables despite a happy `new`. The
        // FEN must be LEGAL (the side NOT to move cannot be in check) or Fathom
        // returns ProbeFailed — here the queen on d1 checks nothing.
        let canary =
            Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").expect("canary FEN is valid");
        match tb.probe_wdl(&canary) {
            Some(Wdl::Win) => Some(tb),
            _ => None,
        }
    }

    /// Largest piece count the loaded tables cover (e.g. 5 for a 3-4-5 set).
    #[inline]
    pub fn max_men(&self) -> u32 {
        self.max_men
    }

    /// Total pieces on the board.
    #[inline]
    fn piece_count(pos: &Position) -> u32 {
        pos.occ_all().count()
    }

    /// Structural conditions ANY probe needs: few enough pieces and no
    /// castling rights (Syzygy positions never have them). Deliberately silent
    /// on the 50-move counter — WDL probes add that via [`Self::probeable_wdl`],
    /// while root probes must run at any halfmove value (see [`Tb::probe_root`]).
    #[inline]
    fn probeable_men(&self, pos: &Position) -> bool {
        Self::piece_count(pos) <= self.max_men
            && pos.castling() == crate::board::CastlingRights::NONE
    }

    /// WDL probes are only sound at `halfmove == 0`: WDL tables ignore the
    /// 50-move counter, so at `halfmove > 0` a cursed win (NOT reachable
    /// inside the 50-move rule from here) would be misreported as a clean win.
    #[inline]
    fn probeable_wdl(&self, pos: &Position) -> bool {
        self.probeable_men(pos) && pos.halfmove() == 0
    }

    /// Bitboards for a probe call. `white`/`black` are color occupancies; the
    /// piece-type masks span BOTH colors (pyrrhic splits color by occupancy).
    fn bits(pos: &Position) -> Bits {
        let pt = |t: PieceType| (pos.piece_bb(Color::White, t) | pos.piece_bb(Color::Black, t)).0;
        Bits {
            white: pos.occ(Color::White).0,
            black: pos.occ(Color::Black).0,
            kings: pt(PieceType::King),
            queens: pt(PieceType::Queen),
            rooks: pt(PieceType::Rook),
            bishops: pt(PieceType::Bishop),
            knights: pt(PieceType::Knight),
            pawns: pt(PieceType::Pawn),
        }
    }

    #[inline]
    fn ep_square(pos: &Position) -> u32 {
        // Fathom/pyrrhic convention: ep target square index, 0 = none.
        pos.ep().map_or(0, |s| s.index() as u32)
    }

    /// WDL probe (side-to-move relative). `None` when the position isn't a
    /// legal probe target or the lookup misses (e.g. a 4-man table absent).
    pub fn probe_wdl(&self, pos: &Position) -> Option<Wdl> {
        if !self.probeable_wdl(pos) {
            return None;
        }
        let b = Self::bits(pos);
        self.inner
            .probe_wdl(
                b.white,
                b.black,
                b.kings,
                b.queens,
                b.rooks,
                b.bishops,
                b.knights,
                b.pawns,
                Self::ep_square(pos),
                pos.stm() == Color::White,
            )
            .ok()
            .map(Wdl::from)
    }

    /// DTZ root probe: the tablebase's recommended move plus its WDL. `None`
    /// when the position isn't probeable or the root lookup fails / has no move
    /// (stalemate / checkmate sentinels). The returned [`Move`] is rebuilt via
    /// the legal move generator so its flags (capture / promo / ep / castle)
    /// are correct — the raw (from, to, promo) from Syzygy carries no flags.
    ///
    /// Unlike [`Tb::probe_wdl`], this MUST fire at any halfmove value: pyrrhic
    /// gets the true clock (`pos.halfmove()` below) and its DTZ root logic
    /// handles rule 50 itself — it picks the minimal-DTZ winning move and
    /// scores the result against the remaining budget (`dtz_to_wdl(rule50,
    /// dtz)`). Gating this probe on `halfmove == 0` caused a live-game loss of
    /// a win: in pawnless TB endings (KRvK etc.) no move ever resets the
    /// counter, so the engine played exactly ONE tablebase move at entry and
    /// then shuffled to a 50-move draw from mate-in-11 (game b86gNzRp,
    /// 2026-06-10).
    pub fn probe_root(&self, pos: &Position) -> Option<(Move, Wdl)> {
        if !self.probeable_men(pos) {
            return None;
        }
        let b = Self::bits(pos);
        let res = self
            .inner
            .probe_root(
                b.white,
                b.black,
                b.kings,
                b.queens,
                b.rooks,
                b.bishops,
                b.knights,
                b.pawns,
                pos.halfmove() as u32,
                Self::ep_square(pos),
                pos.stm() == Color::White,
            )
            .ok()?;
        let dtz = match res.root {
            DtzProbeValue::DtzResult(d) => d,
            // Stalemate / Checkmate / Failed: no move to play from the TB.
            _ => return None,
        };
        let from = Square::new(dtz.from_square);
        let to = Square::new(dtz.to_square);
        let promo = match dtz.promotion {
            pyrrhic_rs::Piece::Queen => Some(PieceType::Queen),
            pyrrhic_rs::Piece::Rook => Some(PieceType::Rook),
            pyrrhic_rs::Piece::Bishop => Some(PieceType::Bishop),
            pyrrhic_rs::Piece::Knight => Some(PieceType::Knight),
            _ => None,
        };
        let mv = match_legal_move(pos, from, to, promo)?;
        Some((mv, Wdl::from(dtz.wdl)))
    }
}

/// Find the unique legal move matching `(from, to[, promo])` by generating the
/// move list — this recovers the correct flag bits (capture / en-passant /
/// castle / promotion) that the bare squares from Syzygy don't encode.
fn match_legal_move(
    pos: &Position,
    from: Square,
    to: Square,
    promo: Option<PieceType>,
) -> Option<Move> {
    let mut list = crate::board::MoveList::new();
    crate::board::generate_moves(pos, &mut list);
    list.iter().copied().find(|m| {
        m.from() == from
            && m.to() == to
            && match promo {
                Some(pt) => m.is_promotion() && m.promotion_piece_type() == pt,
                None => !m.is_promotion(),
            }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::movegen::find_first_legal;
    use std::sync::Mutex;

    /// pyrrhic-rs keeps a process-global singleton (see `Tb::init`): two tests
    /// initializing concurrently make the loser get `AlreadyInitialized` ->
    /// `None` and silently skip. Every test that calls `Tb::init` takes this
    /// lock so init/probe/drop cycles are serialized and deterministic.
    static TB_GATE: Mutex<()> = Mutex::new(());

    /// Local 3-4-5 Syzygy tables (gitignored, absent in CI — callers skip on
    /// `None`, matching the `tests/syzygy.rs` pattern). Paths are relative to
    /// the crate root, which is the cwd for `cargo test --lib`.
    fn tables() -> Option<Tb> {
        Tb::init("tools/tb")
    }

    /// A bad / empty `SyzygyPath` must yield `None` gracefully (CI has no
    /// tables; the engine ships with the feature off by default). This is the
    /// non-ignored smoke test the correctness suite leans on.
    #[test]
    fn init_none_on_empty_or_bad_path() {
        let _gate = TB_GATE.lock().unwrap();
        assert!(Tb::init("").is_none(), "empty path -> None");
        assert!(Tb::init("   ").is_none(), "whitespace path -> None");
        assert!(
            Tb::init("/nonexistent/syzygy/path/xyz").is_none(),
            "missing directory -> None (no panic)"
        );
    }

    /// Root probes must fire with the 50-move counter RUNNING (here hmc=30):
    /// pyrrhic receives the true clock and handles rule 50 itself. The old
    /// shared `halfmove == 0` gate returned `None` here — the b86gNzRp bug.
    #[test]
    fn root_probe_fires_at_nonzero_halfmove() {
        let _gate = TB_GATE.lock().unwrap();
        let Some(tb) = tables() else { return };
        let pos = Position::from_fen("8/8/8/4k3/8/8/4K3/4R3 w - - 30 1").unwrap();
        let (mv, wdl) = tb
            .probe_root(&pos)
            .expect("KRvK root probe must hit at halfmove > 0");
        assert_eq!(wdl, Wdl::Win, "KRvK with 70 plies of budget is a clean win");
        let mut check = pos.clone();
        assert!(check.make(mv), "TB root move {mv} must be legal");
    }

    /// The b86gNzRp regression, end to end: enter KRvK mid-count (hmc=20) and
    /// drive BOTH sides by root probes alone. Every move raises the counter
    /// (no pawn/capture resets exist in KRvK), so each probe runs at
    /// halfmove > 0; with the old gate the winning side got exactly one TB
    /// move and then nothing. The win must convert to mate before the 50-move
    /// rule (halfmove 100) and within a sane ply budget.
    #[test]
    fn krk_converts_within_fifty_move_budget() {
        let _gate = TB_GATE.lock().unwrap();
        let Some(tb) = tables() else { return };
        let mut pos = Position::from_fen("8/8/8/4k3/8/8/4K3/4R3 w - - 20 1").unwrap();
        let mut plies = 0;
        while find_first_legal(&mut pos).is_some() {
            assert!(plies < 60, "no mate within 60 plies — conversion stalled");
            assert!(
                pos.halfmove() < 100,
                "hit the 50-move rule before mate — the b86gNzRp regression"
            );
            let mv = match tb.probe_root(&pos) {
                Some((mv, _)) => mv,
                None => {
                    // The defender may lack a TB verdict only at terminal
                    // sentinels (handled by the loop guard); the WINNING side
                    // must always get a move — that miss IS the bug.
                    assert_eq!(
                        pos.stm(),
                        Color::Black,
                        "winning side lost its TB root move mid-conversion"
                    );
                    find_first_legal(&mut pos).unwrap()
                }
            };
            assert!(pos.make(mv), "TB move {mv} must be legal");
            plies += 1;
        }
        assert!(
            pos.in_check(pos.stm()),
            "game ended without mate (stalemate?) after {plies} plies"
        );
        assert_eq!(pos.stm(), Color::Black, "the defender is the mated side");
        assert!(pos.halfmove() < 100, "mate must beat the 50-move counter");
        println!(
            "KRvK converted to mate in {plies} plies (final hmc {})",
            pos.halfmove()
        );
    }

    /// WDL variant collapse (the cursed/blessed pair -> Draw).
    #[test]
    fn wdl_from_pyrrhic_collapses_cursed() {
        assert_eq!(Wdl::from(WdlProbeResult::Win), Wdl::Win);
        assert_eq!(Wdl::from(WdlProbeResult::Loss), Wdl::Loss);
        assert_eq!(Wdl::from(WdlProbeResult::Draw), Wdl::Draw);
        assert_eq!(Wdl::from(WdlProbeResult::CursedWin), Wdl::Draw);
        assert_eq!(Wdl::from(WdlProbeResult::BlessedLoss), Wdl::Draw);
    }
}
