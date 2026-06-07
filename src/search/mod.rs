//! M3 search: iterative-deepening + TT cutoffs/stores + alpha-beta + qsearch.
//! All mutable search state lives in SearchThread (spec §5.1).

pub mod bench;
pub mod limits;
mod see;
pub mod tt;

use see::see;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::search::tt::Tt;

use crate::board::{generate_moves, Move, MoveList, PieceType, Position};
use crate::eval::Evaluator;
use crate::search::limits::{Limits, TimeManager};

pub const MATE: i32 = 30_000;
/// |score| above this is a mate score (UCI "score mate" conversion).
pub const MATE_BOUND: i32 = 29_000;
pub const INF: i32 = 32_000;
pub const MAX_PLY: usize = 128;

/// Triangular PV table: row[ply] holds the best line found at that ply.
struct PvTable {
    moves: Vec<[Move; MAX_PLY]>,
    len: [usize; MAX_PLY],
}

impl PvTable {
    fn new() -> PvTable {
        PvTable {
            moves: vec![[Move::NULL; MAX_PLY]; MAX_PLY],
            len: [0; MAX_PLY],
        }
    }
    #[inline]
    fn clear_ply(&mut self, ply: usize) {
        self.len[ply] = 0;
    }
    fn update(&mut self, ply: usize, mv: Move) {
        let child_len = if ply + 1 < MAX_PLY {
            self.len[ply + 1]
        } else {
            0
        };
        let (head, tail) = self.moves.split_at_mut(ply + 1);
        let row = &mut head[ply];
        row[0] = mv;
        if child_len > 0 {
            // guard: at ply == MAX_PLY-1 `tail` is empty — indexing it panics
            row[1..=child_len].copy_from_slice(&tail[0][..child_len]);
        }
        self.len[ply] = child_len + 1;
    }
    fn line(&self) -> &[Move] {
        &self.moves[0][..self.len[0]]
    }
}

/// Per-ply search state (spec §5.1). M3 uses `killers` and `current_move`;
/// `static_eval` (M4: RFP/improving) and `excluded_move` (M6: singular
/// extensions) are reserved so their features don't re-layout the stack.
#[derive(Clone, Copy)]
struct StackEntry {
    static_eval: i32,
    current_move: Move,
    /// Piece type that made `current_move` — the continuation-history key for
    /// this ply. The `current_move == Move::NULL` check gates its usage (a
    /// null move stores `PieceType::Pawn` here, which is never read).
    moved_piece: PieceType,
    killers: [Move; 2],
    #[allow(dead_code)] // M6
    excluded_move: Move,
}

impl StackEntry {
    const EMPTY: StackEntry = StackEntry {
        static_eval: 0,
        current_move: Move::NULL,
        moved_piece: PieceType::Pawn,
        killers: [Move::NULL; 2],
        excluded_move: Move::NULL,
    };
}

/// Butterfly history: [side][from][to], bumped depth^2 on quiet beta cutoffs.
/// Fresh per `go` (SearchThread is per-search; cross-move persistence is an
/// M4 refactor — recorded in the plan header).
type HistoryTable = [[[i32; 64]; 64]; 2];

/// Continuation history: indexed `[prev_piece][prev_to][piece][to]`, one table
/// for the move made 1 ply ago and one for 2 plies ago. `i16` saturating with a
/// `depth^2` bonus / `−depth^2` malus, clamped to ±16_000. 6*64*6*64*2 bytes =
/// 294_912 bytes (~288 KiB) per table. Persists across moves (the feature):
/// cleared only at `new()`.
type ContHist = [[[[i16; 64]; 6]; 64]; 6];

/// Saturating-clamp bound for continuation-history entries.
const CONT_HIST_MAX: i16 = 16_000;

/// Heap-allocate a zeroed [`ContHist`] without a 288 KiB stack temporary.
/// `Box::new([[[[0; 64]; 6]; 64]; 6])` relies on the optimizer eliding the
/// stack copy; the vec route guarantees heap construction in every profile
/// (this crate's `[profile.test]` is opt-level 2, but `cargo build` debug is
/// opt-level 0 — the safe route works for both).
fn zeroed_cont_hist() -> Box<ContHist> {
    vec![[[[0i16; 64]; 6]; 64]; 6]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!("len 6 -> [_; 6]"))
}

/// Saturating-add `bonus` into one continuation-history entry, clamped to
/// ±[`CONT_HIST_MAX`]. `parent` is the `(piece, to)` key of the prior ply.
#[inline]
fn bump_cont_hist(
    table: &mut ContHist,
    parent: (PieceType, crate::board::Square),
    piece: PieceType,
    to: crate::board::Square,
    bonus: i16,
) {
    let (pp, pto) = parent;
    let c = &mut table[pp.index()][pto.index()][piece.index()][to.index()];
    *c = c.saturating_add(bonus).clamp(-CONT_HIST_MAX, CONT_HIST_MAX);
}

/// Ordering tiers: TT move (2M) > captures by MVV-LVA (1M+) > quiets (0).
struct MovePicker {
    moves: MoveList,
    scores: [i32; 256],
    cur: usize,
}

/// MVV victim values for ordering only — deliberately decoupled from the
/// tunable eval material (retunes must not silently reshape move ordering).
const VICTIM_VALS: [i32; 6] = [100, 320, 330, 500, 900, 0];

/// LVA values: unlike eval MATERIAL, the king must rank as the MOST
/// expensive attacker (it was 0 there, sorting king-captures first).
const ATTACKER_VALS: [i32; 6] = [100, 320, 330, 500, 900, 10_000];

/// Continuation-history parent key: `(piece, to-square)` of the move made at a
/// previous ply. `None` when that ply is out of range or made a null move.
type ContKey = Option<(PieceType, crate::board::Square)>;

/// Combined quiet-move history score: `butterfly + 2×conthist(1-ply) +
/// conthist(2-ply)`. Single source of truth for BOTH the [`MovePicker`] quiet
/// ordering and the history-driven LMR adjustment in `negamax` — they must read
/// the same number or ordering and reduction disagree.
#[allow(clippy::too_many_arguments)]
#[inline]
fn quiet_history(
    history: &HistoryTable,
    cont_hist1: &ContHist,
    cont_hist2: &ContHist,
    ch1: ContKey,
    ch2: ContKey,
    stm: crate::board::Color,
    piece: PieceType,
    from: crate::board::Square,
    to: crate::board::Square,
) -> i32 {
    let mut s = history[stm.index()][from.index()][to.index()];
    if let Some((pp, pto)) = ch1 {
        s += i32::from(cont_hist1[pp.index()][pto.index()][piece.index()][to.index()]) * 2;
    }
    if let Some((pp, pto)) = ch2 {
        s += i32::from(cont_hist2[pp.index()][pto.index()][piece.index()][to.index()]);
    }
    s
}

impl MovePicker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pos: &Position,
        tt_move: Move,
        killers: [Move; 2],
        history: &HistoryTable,
        cont_hist1: &ContHist,
        cont_hist2: &ContHist,
        ch1: ContKey,
        ch2: ContKey,
        stm: crate::board::Color,
    ) -> MovePicker {
        let mut moves = MoveList::new();
        generate_moves(pos, &mut moves);
        let mut scores = [0i32; 256];
        for (i, &mv) in moves.iter().enumerate() {
            scores[i] = if mv == tt_move && mv != Move::NULL {
                2_000_000 // matched against the GENERATED list = inherent legality
            } else if mv.is_capture() {
                let victim = if mv.flag() == Move::EN_PASSANT {
                    PieceType::Pawn
                } else {
                    pos.piece_on(mv.to()).expect("capture target").piece_type()
                };
                let attacker = pos.piece_on(mv.from()).expect("mover").piece_type();
                1_000_000 + 10 * VICTIM_VALS[victim.index()] - ATTACKER_VALS[attacker.index()]
            } else if mv == killers[0] {
                900_000
            } else if mv == killers[1] {
                899_999
            } else {
                // quiet ordering: butterfly + 2×conthist(1-ply) + conthist(2-ply),
                // via the shared `quiet_history` helper (same score the LMR
                // history adjustment reads).
                let piece = pos.piece_on(mv.from()).expect("mover").piece_type();
                quiet_history(
                    history,
                    cont_hist1,
                    cont_hist2,
                    ch1,
                    ch2,
                    stm,
                    piece,
                    mv.from(),
                    mv.to(),
                )
            };
        }
        MovePicker {
            moves,
            scores,
            cur: 0,
        }
    }

    fn next(&mut self) -> Option<Move> {
        let len = self.moves.len();
        if self.cur >= len {
            return None;
        }
        // selection: swap the best remaining move into position `cur`
        let mut best = self.cur;
        for i in (self.cur + 1)..len {
            if self.scores[i] > self.scores[best] {
                best = i;
            }
        }
        self.moves.as_mut_slice().swap(self.cur, best);
        self.scores.swap(self.cur, best);
        let mv = self.moves.as_slice()[self.cur];
        self.cur += 1;
        Some(mv)
    }
}

/// Per-iteration report for UCI `info` lines.
pub struct IterInfo<'a> {
    pub depth: i32,
    pub score: i32,
    pub nodes: u64,
    pub elapsed_ms: u128,
    pub pv: &'a [Move],
}

pub struct SearchThread<E: Evaluator> {
    pub pos: Position,
    pub eval: E,
    pub nodes: u64,
    stop: Arc<AtomicBool>,
    node_limit: Option<u64>,
    stopped: bool,
    deadline: Option<Instant>,
    overhead_ms: u64,
    pv: PvTable,
    stack: Box<[StackEntry; MAX_PLY]>,
    tt: Arc<Tt>,
    history: Box<HistoryTable>,
    cont_hist1: Box<ContHist>,
    cont_hist2: Box<ContHist>,
    /// Precomputed LMR base reductions: `reductions[depth][move_index]` =
    /// `(0.77 + ln(depth)·ln(move_index) / 2.36)` truncated to `i32`, for
    /// `depth, move_index` in `1..64`. Row/col 0 stay 0 (never indexed: the
    /// LMR guards require `depth >= 3` and `quiet_count >= 3`). Built once in
    /// [`SearchThread::new`]; indices are clamped to 63 at the call site.
    reductions: Box<[[i32; 64]; 64]>,
}

impl<E: Evaluator> SearchThread<E> {
    pub fn new(pos: Position, eval: E) -> SearchThread<E> {
        // Log-formula LMR base reductions, computed once: deeper searches and
        // later moves reduce more (both axes monotone non-decreasing).
        let mut reductions = Box::new([[0i32; 64]; 64]);
        for d in 1..64 {
            for m in 1..64 {
                reductions[d][m] = (0.77 + (d as f64).ln() * (m as f64).ln() / 2.36) as i32;
            }
        }
        SearchThread {
            pos,
            eval,
            nodes: 0,
            stop: Arc::new(AtomicBool::new(false)),
            node_limit: None,
            stopped: false,
            deadline: None,
            overhead_ms: 50,
            pv: PvTable::new(),
            stack: Box::new([StackEntry::EMPTY; MAX_PLY]),
            tt: Arc::new(Tt::new(16)),
            history: Box::new([[[0; 64]; 64]; 2]),
            cont_hist1: zeroed_cont_hist(),
            cont_hist2: zeroed_cont_hist(),
            reductions,
        }
    }

    /// Share this flag with the UCI thread; setting it aborts the search.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }
    pub fn set_stop_flag(&mut self, flag: Arc<AtomicBool>) {
        self.stop = flag;
    }
    pub fn set_node_limit(&mut self, limit: Option<u64>) {
        self.node_limit = limit;
    }
    pub fn set_overhead_ms(&mut self, ms: u64) {
        self.overhead_ms = ms;
    }
    pub fn set_tt(&mut self, tt: Arc<Tt>) {
        self.tt = tt;
    }

    /// Best line from the last completed search call.
    pub fn pv_line(&self) -> &[Move] {
        self.pv.line()
    }
    pub fn was_stopped(&self) -> bool {
        self.stopped
    }

    /// Fixed-depth, full-window search (bench + tests + shallow ID depths).
    pub fn search_to_depth(&mut self, depth: i32) -> (Option<Move>, i32) {
        self.search_root(depth, -INF, INF)
    }

    /// Fixed-depth search with an explicit window (aspiration re-searches).
    fn search_root(&mut self, depth: i32, alpha: i32, beta: i32) -> (Option<Move>, i32) {
        self.eval.refresh(&self.pos);
        self.stopped = false;
        let score = self.negamax(depth, alpha, beta, 0);
        (self.pv.line().first().copied(), score)
    }

    /// Polled every 2048 nodes (spec §5.4): external stop or node budget.
    /// Task 5 extends this with the hard time deadline.
    #[inline]
    fn should_stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.nodes & 2047 == 0 {
            if self.stop.load(Ordering::Relaxed) {
                self.stopped = true;
            }
            if let Some(limit) = self.node_limit {
                if self.nodes >= limit {
                    self.stopped = true;
                }
            }
            if let Some(d) = self.deadline {
                if Instant::now() >= d {
                    self.stopped = true;
                }
            }
        }
        self.stopped
    }

    /// Small jitter (±1cp) instead of flat 0: avoids threefold blindness in
    /// self-play pools (spec §3).
    #[inline]
    fn draw_score(&self) -> i32 {
        1 - (self.nodes as i32 & 2)
    }

    /// Static eval trending up vs two plies ago (same side) — margin scaler.
    fn improving(&self, ply: usize) -> bool {
        ply >= 2 && self.stack[ply].static_eval > self.stack[ply - 2].static_eval
    }

    /// 50-move rule with mate precedence: a mated side at halfmove >= 100
    /// is still mated.
    fn fifty_move_score(&mut self, ply: usize) -> i32 {
        let mut list = MoveList::new();
        generate_moves(&self.pos, &mut list);
        let mut any_legal = false;
        for &mv in list.iter() {
            if self.pos.make(mv) {
                self.pos.unmake();
                any_legal = true;
                break;
            }
        }
        if !any_legal && self.pos.in_check(self.pos.stm()) {
            -(MATE - ply as i32)
        } else {
            self.draw_score()
        }
    }

    fn negamax(&mut self, depth: i32, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.pv.clear_ply(ply);
        if self.should_stop() {
            return 0;
        }
        // stm has no king: unreachable through legal make() flows, but GUI
        // FENs can be illegal (enemy king en prise). Score as already-mated
        // (stm-relative) so the capturer prefers it — and never crash.
        if self
            .pos
            .piece_bb(self.pos.stm(), PieceType::King)
            .is_empty()
        {
            return -(MATE - ply as i32);
        }
        if ply > 0 {
            if self.pos.is_repetition() {
                return self.draw_score();
            }
            if self.pos.is_fifty_move_draw() {
                return self.fifty_move_score(ply);
            }
        }
        if depth <= 0 {
            return self.qsearch(alpha, beta, ply);
        }
        // interior node (horizon nodes are counted by qsearch)
        self.nodes += 1;
        if ply >= MAX_PLY - 1 {
            return self.eval.evaluate(&self.pos);
        }

        // TT probe. We always probe for the tt_move (used in Task 4 move
        // ordering); at ply > 0 a sufficient-depth hit may short-circuit the
        // full search (grafting). KNOWN CAVEAT: TT grafting can interact with
        // path-dependent draw scores (repetition/50-move). The draw checks
        // above run BEFORE the probe, bounding the damage; this is universal
        // practice at this engine level.
        let tt_hit = self.tt.probe(self.pos.key(), ply);
        if ply > 0 {
            if let Some(ref h) = tt_hit {
                if h.depth >= depth {
                    match h.bound {
                        tt::Bound::Exact => return h.score,
                        tt::Bound::Lower if h.score >= beta => return h.score,
                        tt::Bound::Upper if h.score <= alpha => return h.score,
                        _ => {}
                    }
                }
            }
        }

        let tt_move = tt_hit.as_ref().map_or(Move::NULL, |h| h.mv);
        let in_check = self.pos.in_check(self.pos.stm());
        // static eval: reuse the TT's cached value when present (identical by
        // determinism), else compute; populate the stack slot (spec §5.1)
        let static_eval = match tt_hit {
            Some(ref h) if h.eval != tt::EVAL_NONE => h.eval,
            _ => self.eval.evaluate(&self.pos),
        };
        self.stack[ply].static_eval = static_eval;

        let improving = self.improving(ply);
        // reverse futility: shallow node already beating beta by a margin.
        // Guards: not in a mate/mated search (beta/static_eval in non-mate
        // range only) so we never truncate forced-mate lines.
        if ply > 0
            && !in_check
            && depth <= 6
            && beta.abs() < MATE_BOUND
            && static_eval.abs() < MATE_BOUND
        {
            let margin = (if improving { 60 } else { 80 }) * depth;
            if static_eval - margin >= beta {
                return static_eval;
            }
        }

        // null-move pruning: if we pass the turn and the opponent STILL
        // can't get under beta, this node is prunable. Guards: in check
        // (illegal), consecutive nulls (infinite recursion), pawn-only
        // material (zugzwang), eval below beta (no margin to give away).
        if ply > 0
            && !in_check
            && depth >= 3
            && static_eval >= beta
            && self.stack[ply - 1].current_move != Move::NULL
            && self.pos.has_non_pawn_material(self.pos.stm())
        {
            const R: i32 = 3;
            self.stack[ply].current_move = Move::NULL;
            self.stack[ply].moved_piece = PieceType::Pawn; // NULL move: never read
            self.pos.make_null();
            let score = -self.negamax(depth - 1 - R, -beta, -beta + 1, ply + 1);
            self.pos.unmake_null();
            if self.stopped {
                return 0;
            }
            if score >= beta {
                // never return unproven mate scores from a null search
                return if score >= MATE_BOUND { beta } else { score };
            }
        }

        let killers = self.stack[ply].killers;
        let stm = self.pos.stm();
        // Continuation-history parent keys: (piece, to) of the moves made 1 and
        // 2 plies ago. NULL-guarded (a null move has no conthist key).
        let ch1: ContKey =
            (ply >= 1 && self.stack[ply - 1].current_move != Move::NULL).then(|| {
                let e = &self.stack[ply - 1];
                (e.moved_piece, e.current_move.to())
            });
        let ch2: ContKey =
            (ply >= 2 && self.stack[ply - 2].current_move != Move::NULL).then(|| {
                let e = &self.stack[ply - 2];
                (e.moved_piece, e.current_move.to())
            });
        let futile = ply > 0
            && !in_check
            && depth <= 2
            && alpha.abs() < MATE_BOUND
            && static_eval + 90 * depth + 120 <= alpha;
        let mut picker = MovePicker::new(
            &self.pos,
            tt_move,
            killers,
            &self.history,
            &self.cont_hist1,
            &self.cont_hist2,
            ch1,
            ch2,
            stm,
        );
        let mut legal = 0u32;
        let mut quiet_count = 0u32;
        let mut best = -INF;
        let mut best_move = Move::NULL;
        let mut first = true;
        // Tried quiets (and the piece that moved each), for the conthist malus
        // applied to non-cutoff quiets on a beta cutoff.
        let mut tried_quiets: [(Move, PieceType); 64] = [(Move::NULL, PieceType::Pawn); 64];
        let mut tried_quiet_count = 0usize;
        while let Some(mv) = picker.next() {
            // futility: at very shallow depth with a hopeless eval, quiet moves
            // can't recover. depth <= 2 only: deeper skips break sacrificial
            // combinations (WAC canary 268->257, attributed by A/B 2026-06-05)
            if futile && legal > 0 && !mv.is_capture() && !mv.is_promotion() {
                continue;
            }
            // piece type that moves (read before make: from-square empties)
            let moved_piece = self.pos.piece_on(mv.from()).expect("mover").piece_type();
            let is_quiet = !mv.is_capture() && !mv.is_promotion();
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            self.stack[ply].current_move = mv;
            self.stack[ply].moved_piece = moved_piece;
            legal += 1;
            if is_quiet && tried_quiet_count < tried_quiets.len() {
                tried_quiets[tried_quiet_count] = (mv, moved_piece);
                tried_quiet_count += 1;
            }
            let score = if first {
                -self.negamax(depth - 1, -beta, -alpha, ply + 1)
            } else {
                // LMR: late quiets get a reduced-depth scout; surprises get
                // re-searched at full depth before the full-window re-search
                let mut r = 0;
                if !in_check && !mv.is_capture() && !mv.is_promotion() {
                    quiet_count += 1;
                    let is_killer = mv == killers[0] || mv == killers[1];
                    if depth >= 3 && quiet_count >= 3 && !is_killer {
                        // base reduction from the log-formula table
                        r = self.reductions[(depth.min(63)) as usize]
                            [(quiet_count.min(63)) as usize];
                        // history adjustment: hot quiets reduce less, cold reduce
                        // more. Reads the SAME combined score the picker ordered
                        // by (via the shared `quiet_history` helper).
                        let hist = quiet_history(
                            &self.history,
                            &self.cont_hist1,
                            &self.cont_hist2,
                            ch1,
                            ch2,
                            stm,
                            moved_piece,
                            mv.from(),
                            mv.to(),
                        );
                        r -= (hist / 8_000).clamp(-2, 2);
                        // never reduce into qsearch (depth >= 3 here, so the
                        // upper bound is >= 1 and r can't go negative)
                        r = r.clamp(0, depth - 2);
                    }
                }
                let mut zw = -self.negamax(depth - 1 - r, -alpha - 1, -alpha, ply + 1);
                if r > 0 && zw > alpha && !self.stopped {
                    zw = -self.negamax(depth - 1, -alpha - 1, -alpha, ply + 1);
                }
                if zw > alpha && zw < beta && !self.stopped {
                    -self.negamax(depth - 1, -beta, -alpha, ply + 1)
                } else {
                    zw
                }
            };
            first = false;
            self.pos.unmake();
            self.eval.on_unmake(mv, &self.pos);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    best_move = mv;
                    self.pv.update(ply, mv);
                    if alpha >= beta {
                        // killer + history update on quiet beta cutoffs only
                        if !mv.is_capture() {
                            let k = &mut self.stack[ply].killers;
                            if k[0] != mv {
                                k[1] = k[0];
                                k[0] = mv;
                            }
                            let h = &mut self.history[self.pos.stm().index()][mv.from().index()]
                                [mv.to().index()];
                            *h = (*h + depth * depth).min(799_999);
                        }
                        // continuation history: bump the cutoff quiet, malus the
                        // quiets that were tried but failed to cut off (both
                        // 1-ply and 2-ply tables). Butterfly stays bonus-only.
                        if is_quiet {
                            let bonus = (depth * depth).min(400) as i16;
                            let to = mv.to();
                            if let Some(parent) = ch1 {
                                bump_cont_hist(
                                    &mut self.cont_hist1,
                                    parent,
                                    moved_piece,
                                    to,
                                    bonus,
                                );
                            }
                            if let Some(parent) = ch2 {
                                bump_cont_hist(
                                    &mut self.cont_hist2,
                                    parent,
                                    moved_piece,
                                    to,
                                    bonus,
                                );
                            }
                            for &(tq_mv, tq_piece) in &tried_quiets[..tried_quiet_count] {
                                if tq_mv == mv {
                                    continue; // the cutoff move keeps its bonus
                                }
                                if let Some(parent) = ch1 {
                                    bump_cont_hist(
                                        &mut self.cont_hist1,
                                        parent,
                                        tq_piece,
                                        tq_mv.to(),
                                        -bonus,
                                    );
                                }
                                if let Some(parent) = ch2 {
                                    bump_cont_hist(
                                        &mut self.cont_hist2,
                                        parent,
                                        tq_piece,
                                        tq_mv.to(),
                                        -bonus,
                                    );
                                }
                            }
                        }
                        break; // beta cutoff
                    }
                }
            }
        }

        if legal == 0 {
            // legal==0 path does NOT store to the TT (no best move to record).
            return if in_check {
                -(MATE - ply as i32) // checkmated at this ply
            } else {
                self.draw_score() // stalemate
            };
        }

        let bound = if best >= beta {
            tt::Bound::Lower // the stored move is the cutoff move
        } else if best_move != Move::NULL {
            tt::Bound::Exact
        } else {
            tt::Bound::Upper // failed low: no move raised alpha
        };
        self.tt.store(
            self.pos.key(),
            best_move,
            best,
            static_eval,
            depth,
            bound,
            ply,
        );
        best
    }

    fn qsearch(&mut self, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.pv.clear_ply(ply);
        self.nodes += 1;
        if self.should_stop() {
            return 0;
        }
        if ply >= MAX_PLY - 1 {
            return self.eval.evaluate(&self.pos);
        }
        // stm has no king: unreachable through legal make() flows, but GUI
        // FENs can be illegal (enemy king en prise). Score as already-mated
        // (stm-relative) so the capturer prefers it — and never crash.
        if self
            .pos
            .piece_bb(self.pos.stm(), PieceType::King)
            .is_empty()
        {
            return -(MATE - ply as i32);
        }

        // Step 6.1: TT probe + cutoff at qsearch entry (any stored depth >= 0
        // dominates a qsearch node which runs at depth 0).
        let tt_hit = self.tt.probe(self.pos.key(), ply);
        if let Some(ref h) = tt_hit {
            match h.bound {
                tt::Bound::Exact => return h.score,
                tt::Bound::Lower if h.score >= beta => return h.score,
                tt::Bound::Upper if h.score <= alpha => return h.score,
                _ => {}
            }
        }
        let tt_move = tt_hit.as_ref().map_or(Move::NULL, |h| h.mv);

        let orig_alpha = alpha;
        let in_check = self.pos.in_check(self.pos.stm());
        let mut best_move = Move::NULL;
        let mut best = if in_check {
            // in-check: there is no meaningful static eval — store the sentinel
            // so a later negamax probe of this entry recomputes instead of
            // adopting a fake 0 (review hygiene: the eval field means
            // "static eval of this position" or EVAL_NONE, never a placeholder)
            self.stack[ply].static_eval = tt::EVAL_NONE;
            -INF // no stand-pat while in check: must find an evasion
        } else {
            let stand_pat = self.eval.evaluate(&self.pos);
            // Step 6.2: the stand-pat IS the static eval in the not-in-check branch
            self.stack[ply].static_eval = stand_pat;
            if stand_pat >= beta {
                // Beta cutoff at stand-pat: store as Lower bound before returning.
                // The in-check && legal==0 mate return stays un-stored (mirrors negamax).
                self.tt.store(
                    self.pos.key(),
                    Move::NULL,
                    stand_pat,
                    stand_pat,
                    0,
                    tt::Bound::Lower,
                    ply,
                );
                return stand_pat;
            }
            if stand_pat > alpha {
                alpha = stand_pat;
            }
            stand_pat
        };

        let stm = self.pos.stm();
        // Step 6.1: pass tt_move into the picker (a quiet TT move scores 2M
        // but the !in_check && !mv.is_capture() filter below will skip it —
        // harmless; captures and check-evasions get the ordering benefit).
        // qsearch does not score by continuation history (None keys); the
        // tables are still threaded to satisfy the picker signature.
        let mut picker = MovePicker::new(
            &self.pos,
            tt_move,
            [Move::NULL; 2],
            &self.history,
            &self.cont_hist1,
            &self.cont_hist2,
            None,
            None,
            stm,
        );
        let mut legal = 0u32;
        while let Some(mv) = picker.next() {
            // quiet moves only matter when evading check
            if !in_check && !mv.is_capture() {
                continue;
            }
            // SEE pruning: skip captures that lose material outright (not while
            // in check — every evasion must be tried; not promotions — the
            // value swing is too big for the no-promo SEE approximation).
            if !in_check && mv.is_capture() && !mv.is_promotion() && see(&self.pos, mv) < 0 {
                continue;
            }
            // piece type that moves (read before make: from-square empties)
            let moved_piece = self.pos.piece_on(mv.from()).expect("mover").piece_type();
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            self.stack[ply].current_move = mv;
            self.stack[ply].moved_piece = moved_piece;
            legal += 1;
            let score = -self.qsearch(-beta, -alpha, ply + 1);
            self.pos.unmake();
            self.eval.on_unmake(mv, &self.pos);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    best_move = mv;
                    self.pv.update(ply, mv);
                    if alpha >= beta {
                        break;
                    }
                }
            }
        }

        if in_check && legal == 0 {
            // Mate found in qsearch: un-stored (mirrors negamax).
            return -(MATE - ply as i32);
        }

        // Step 6.2: store on exit with bound classification.
        // Exact with NULL move: stand-pat raised alpha (best_move stays NULL).
        let bound = if best >= beta {
            tt::Bound::Lower
        } else if best > orig_alpha {
            tt::Bound::Exact // may carry a NULL move: stand-pat raised alpha
        } else {
            tt::Bound::Upper
        };
        self.tt.store(
            self.pos.key(),
            best_move,
            best,
            self.stack[ply].static_eval,
            0,
            bound,
            ply,
        );
        best
    }

    /// Narrow window around the previous score; widen exponentially on fail.
    fn aspiration(&mut self, depth: i32, guess: i32) -> (Option<Move>, i32) {
        let mut delta = 25;
        let mut alpha = (guess - delta).max(-INF);
        let mut beta = (guess + delta).min(INF);
        loop {
            let (mv, score) = self.search_root(depth, alpha, beta);
            if self.stopped {
                return (mv, score);
            }
            if score <= alpha {
                // fail-low: lower alpha, pull beta toward the fail point
                beta = (alpha + beta) / 2;
                alpha = (score - delta).max(-INF);
            } else if score >= beta {
                beta = (score + delta).min(INF);
            } else {
                return (mv, score);
            }
            delta *= 2; // mate-region fails widen to the full window fast
        }
    }

    /// First root move that survives the legality filter (bestmove fallback).
    fn first_legal(&mut self) -> Option<Move> {
        crate::board::movegen::find_first_legal(&mut self.pos)
    }

    /// Iterative deepening driver. Returns None only when the root has no
    /// legal moves (mate/stalemate already on the board).
    /// `info` is called after every COMPLETED iteration.
    /// NOTE: the caller owns stop-flag hygiene — clear it BEFORE spawning the search thread (a worker-side clear races with an early external stop).
    pub fn iterate(&mut self, limits: &Limits, mut info: impl FnMut(IterInfo)) -> Option<Move> {
        let tm = TimeManager::new(limits, self.pos.stm(), self.overhead_ms);
        self.deadline = tm.hard_deadline();
        self.node_limit = limits.nodes;
        self.nodes = 0;
        self.tt.new_search();

        let mut best = self.first_legal()?;
        let max_depth = limits
            .depth
            .unwrap_or(MAX_PLY as i32 - 1)
            .clamp(1, MAX_PLY as i32 - 1);

        let mut prev_score = 0;
        for depth in 1..=max_depth {
            let (mv, score) = if depth >= 4 {
                self.aspiration(depth, prev_score)
            } else {
                self.search_to_depth(depth)
            };
            if self.was_stopped() {
                // partial iteration: only trust it at depth 1 (first full
                // root move beats the arbitrary fallback)
                if depth == 1 {
                    if let Some(mv) = mv {
                        best = mv;
                    }
                }
                break;
            }
            best = mv.expect("completed iteration always has a move");
            prev_score = score;
            info(IterInfo {
                depth,
                score,
                nodes: self.nodes,
                elapsed_ms: tm.elapsed_ms(),
                pv: self.pv.line(),
            });
            if tm.past_soft() {
                break;
            }
            if score.abs() >= MATE_BOUND {
                break; // forced mate found; deeper search can't change it
            }
        }
        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{movegen::find_uci_move, Color, Position, Square};
    use crate::eval::Evaluator;

    /// Test evaluator: side-to-move-relative score that is −10_000 exactly when
    /// a white pawn stands on e4 with Black to move, else 0. This makes the
    /// white move `e2e4` score +10_000 (a forced beta cutoff against a moderate
    /// beta) while every other quiet scores 0 — deterministic conthist tests.
    struct PawnE4Eval;
    impl Evaluator for PawnE4Eval {
        fn refresh(&mut self, _pos: &Position) {}
        fn on_make(&mut self, _mv: Move, _pos: &Position) {}
        fn on_unmake(&mut self, _mv: Move, _pos: &Position) {}
        fn evaluate(&mut self, pos: &Position) -> i32 {
            let e4 = Square::from_name("e4").unwrap();
            let white_pawn_on_e4 =
                pos.piece_on(e4) == Some(crate::board::Piece::new(Color::White, PieceType::Pawn));
            if white_pawn_on_e4 && pos.stm() == Color::Black {
                -10_000
            } else {
                0
            }
        }
    }

    /// Prime `stack[0]` and `stack[1]` so a node searched at ply 2 sees both
    /// continuation-history parents: grandparent (ch2) key = (Bishop, c4),
    /// parent (ch1) key = (Knight, d4). Returns the two expected keys.
    fn prime_conthist_parents<E: Evaluator>(
        st: &mut SearchThread<E>,
    ) -> ((PieceType, Square), (PieceType, Square)) {
        let c4 = Square::from_name("c4").unwrap();
        let d4 = Square::from_name("d4").unwrap();
        let a1 = Square::A1;
        // grandparent move (ply 0): a Bishop landing on c4
        st.stack[0].current_move = Move::new(a1, c4, Move::QUIET);
        st.stack[0].moved_piece = PieceType::Bishop;
        // parent move (ply 1): a Knight landing on d4
        st.stack[1].current_move = Move::new(a1, d4, Move::QUIET);
        st.stack[1].moved_piece = PieceType::Knight;
        ((PieceType::Bishop, c4), (PieceType::Knight, d4))
    }

    #[test]
    #[allow(unused_assignments)] // captures_done is read in the assert; the final write before break is intentionally dead
    fn picker_yields_ordering_tiers() {
        // kiwipete: captures + plenty of quiets
        let pos = Position::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let tt_move = find_uci_move(&pos, "a2a3").unwrap(); // arbitrary quiet as TT move
        let k0 = find_uci_move(&pos, "a2a4").unwrap();
        let k1 = find_uci_move(&pos, "g2g3").unwrap();
        let history: Box<HistoryTable> = Box::new([[[0; 64]; 64]; 2]);
        let ch1 = zeroed_cont_hist();
        let ch2 = zeroed_cont_hist();
        let mut picker = MovePicker::new(
            &pos,
            tt_move,
            [k0, k1],
            &history,
            &ch1,
            &ch2,
            None,
            None,
            crate::board::Color::White,
        );
        let first = picker.next().unwrap();
        assert_eq!(first, tt_move, "TT move first even though quiet");
        // then all captures, then exactly k0, k1, then the rest
        let mut seen_killer0 = false;
        let mut captures_done = false;
        while let Some(mv) = picker.next() {
            if mv == k0 {
                captures_done = true;
                seen_killer0 = true;
                let next = picker.next().unwrap();
                assert_eq!(next, k1, "killer1 follows killer0");
                break;
            }
            assert!(
                mv.is_capture() && !captures_done,
                "non-capture {mv} before killers"
            );
        }
        assert!(seen_killer0);
    }

    #[test]
    fn history_orders_quiets_below_killers() {
        let pos = Position::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let hot = find_uci_move(&pos, "g2g3").unwrap();
        let killer = find_uci_move(&pos, "a2a4").unwrap();
        let mut history: Box<HistoryTable> = Box::new([[[0; 64]; 64]; 2]);
        history[0][hot.from().index()][hot.to().index()] = 50_000;
        let ch1 = zeroed_cont_hist();
        let ch2 = zeroed_cont_hist();
        let mut picker = MovePicker::new(
            &pos,
            Move::NULL,
            [killer, Move::NULL],
            &history,
            &ch1,
            &ch2,
            None,
            None,
            crate::board::Color::White,
        );
        // order: captures..., killer, hot history quiet, ...rest
        let mut prev_was_killer = false;
        while let Some(mv) = picker.next() {
            if mv == killer {
                prev_was_killer = true;
                continue;
            }
            if prev_was_killer {
                assert_eq!(mv, hot, "hot-history quiet must follow the killer");
                break;
            }
            assert!(mv.is_capture(), "captures precede the killer");
        }
    }

    #[test]
    fn conthist_prefers_hot_quiet_over_cold() {
        // Same shape as `history_orders_quiets_below_killers`, but the boost
        // comes from continuation history (1-ply table) instead of butterfly.
        let pos = Position::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let hot = find_uci_move(&pos, "g2g3").unwrap(); // quiet pawn push
        let cold = find_uci_move(&pos, "a2a3").unwrap(); // another quiet
        let history: Box<HistoryTable> = Box::new([[[0; 64]; 64]; 2]);
        let mut ch1 = zeroed_cont_hist();
        let ch2 = zeroed_cont_hist();
        // parent (1-ply) key the picker will be told about
        let parent = (PieceType::Knight, Square::from_name("d4").unwrap());
        // g2g3 is a pawn push: the moving piece is a Pawn landing on g3
        let hot_piece = pos.piece_on(hot.from()).unwrap().piece_type();
        ch1[parent.0.index()][parent.1.index()][hot_piece.index()][hot.to().index()] = 5_000;
        let mut picker = MovePicker::new(
            &pos,
            Move::NULL,
            [Move::NULL; 2],
            &history,
            &ch1,
            &ch2,
            Some(parent),
            None,
            Color::White,
        );
        // order: captures..., hot conthist quiet, ...other quiets (incl. cold)
        let mut seen_hot = false;
        while let Some(mv) = picker.next() {
            if mv.is_capture() {
                continue;
            }
            // first non-capture must be the conthist-hot quiet
            assert_eq!(mv, hot, "conthist-hot quiet leads the quiets");
            seen_hot = true;
            break;
        }
        assert!(seen_hot, "the hot quiet was yielded");
        assert_ne!(hot, cold);
    }

    #[test]
    fn cutoff_bumps_all_three_history_tables() {
        // KP-vs-K: white has only quiet moves; e2e4 is the unique high-scoring
        // move under PawnE4Eval, so it is a forced quiet beta cutoff.
        let pos = Position::from_fen("7k/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        let mut st = SearchThread::new(pos, PawnE4Eval);
        let (ch2_key, ch1_key) = prime_conthist_parents(&mut st);
        let e2e4 = find_uci_move(&st.pos, "e2e4").unwrap();
        let piece = PieceType::Pawn; // pawn double-push
        let to = e2e4.to();

        // depth 1 so the child is qsearch (stand-pat = PawnE4Eval): score +10000
        // for e2e4 cuts off against beta 5000 on the FIRST quiet tried.
        let score = st.negamax(1, -INF, 5_000, 2);
        assert!(score >= 5_000, "e2e4 produced a beta cutoff (got {score})");

        let bf = st.history[Color::White.index()][e2e4.from().index()][to.index()];
        assert!(bf > 0, "butterfly history bumped (got {bf})");
        let c1 = st.cont_hist1[ch1_key.0.index()][ch1_key.1.index()][piece.index()][to.index()];
        assert!(c1 > 0, "cont_hist1 bumped (got {c1})");
        let c2 = st.cont_hist2[ch2_key.0.index()][ch2_key.1.index()][piece.index()][to.index()];
        assert!(c2 > 0, "cont_hist2 bumped (got {c2})");
    }

    #[test]
    fn cutoff_applies_malus_to_tried_but_failed_quiets() {
        // Same position, but butterfly-order a king move (Ke1f1) BEFORE e2e4 so
        // it is searched first, fails low (score 0 < beta), and then receives
        // the conthist malus when e2e4 cuts off.
        let pos = Position::from_fen("7k/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        let mut st = SearchThread::new(pos, PawnE4Eval);
        let (ch2_key, ch1_key) = prime_conthist_parents(&mut st);
        let kf1 = find_uci_move(&st.pos, "e1f1").unwrap(); // quiet king move
                                                           // sort the king move first via a large butterfly score
        st.history[Color::White.index()][kf1.from().index()][kf1.to().index()] = 1_000_000;
        let king_piece = PieceType::King;
        let kto = kf1.to();

        let score = st.negamax(1, -INF, 5_000, 2);
        assert!(score >= 5_000, "e2e4 still cuts off (got {score})");

        // the king move was tried first, failed low, so both conthist tables
        // record a NEGATIVE malus for it
        let c1 =
            st.cont_hist1[ch1_key.0.index()][ch1_key.1.index()][king_piece.index()][kto.index()];
        assert!(
            c1 < 0,
            "cont_hist1 malus on tried-but-failed quiet (got {c1})"
        );
        let c2 =
            st.cont_hist2[ch2_key.0.index()][ch2_key.1.index()][king_piece.index()][kto.index()];
        assert!(
            c2 < 0,
            "cont_hist2 malus on tried-but-failed quiet (got {c2})"
        );
    }

    #[test]
    fn lmr_reductions_table_is_sane() {
        // The log-formula table is built in `new()`; any evaluator works.
        let pos = Position::from_fen("7k/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        let st = SearchThread::new(pos, PawnE4Eval);
        let r = &st.reductions;

        // explicit floor: a depth-3, 3rd-quiet reduction is at least 1 ply
        assert!(r[3][3] >= 1, "r[3][3] must be >= 1 (got {})", r[3][3]);
        // pinned corner values (log formula, truncated) — for the record
        assert_eq!(r[3][3], 1);
        assert_eq!(r[8][8], 2);
        assert_eq!(r[20][30], 5);
        assert_eq!(r[63][63], 8);

        // monotone non-decreasing along BOTH axes over the live range (1..64):
        // ln is increasing and the coefficient is positive, so deeper / later
        // never reduces less. Truncation to i32 preserves the ordering.
        for d in 1..64 {
            for m in 2..64 {
                assert!(
                    r[d][m] >= r[d][m - 1],
                    "non-monotone in move-index at d={d}: r[{d}][{m}]={} < r[{d}][{}]={}",
                    r[d][m],
                    m - 1,
                    r[d][m - 1],
                );
            }
        }
        for m in 1..64 {
            for d in 2..64 {
                assert!(
                    r[d][m] >= r[d - 1][m],
                    "non-monotone in depth at m={m}: r[{d}][{m}]={} < r[{}][{m}]={}",
                    r[d][m],
                    d - 1,
                    r[d - 1][m],
                );
            }
        }
    }
}
