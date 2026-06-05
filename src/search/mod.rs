//! M3 search: iterative-deepening + TT cutoffs/stores + alpha-beta + qsearch.
//! All mutable search state lives in SearchThread (spec §5.1).

pub mod bench;
pub mod limits;
pub mod tt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::search::tt::Tt;

use crate::board::{generate_moves, Move, MoveList, PieceType, Position};
use crate::eval::psqt::MATERIAL;
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
    killers: [Move; 2],
    #[allow(dead_code)] // M6
    excluded_move: Move,
}

impl StackEntry {
    const EMPTY: StackEntry = StackEntry {
        static_eval: 0,
        current_move: Move::NULL,
        killers: [Move::NULL; 2],
        excluded_move: Move::NULL,
    };
}

/// Butterfly history: [side][from][to], bumped depth^2 on quiet beta cutoffs.
/// Fresh per `go` (SearchThread is per-search; cross-move persistence is an
/// M4 refactor — recorded in the plan header).
type HistoryTable = [[[i32; 64]; 64]; 2];

/// Ordering tiers: TT move (2M) > captures by MVV-LVA (1M+) > quiets (0).
struct MovePicker {
    moves: MoveList,
    scores: [i32; 256],
    cur: usize,
}

/// LVA values: unlike eval MATERIAL, the king must rank as the MOST
/// expensive attacker (it was 0 there, sorting king-captures first).
const ATTACKER_VALS: [i32; 6] = [100, 320, 330, 500, 900, 10_000];

impl MovePicker {
    fn new(
        pos: &Position,
        tt_move: Move,
        killers: [Move; 2],
        history: &HistoryTable,
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
                1_000_000 + 10 * MATERIAL[victim.index()] - ATTACKER_VALS[attacker.index()]
            } else if mv == killers[0] {
                900_000
            } else if mv == killers[1] {
                899_999
            } else {
                history[stm.index()][mv.from().index()][mv.to().index()]
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
}

impl<E: Evaluator> SearchThread<E> {
    pub fn new(pos: Position, eval: E) -> SearchThread<E> {
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
        let futile = ply > 0
            && !in_check
            && depth <= 2
            && alpha.abs() < MATE_BOUND
            && static_eval + 90 * depth + 120 <= alpha;
        let mut picker = MovePicker::new(&self.pos, tt_move, killers, &self.history, stm);
        let mut legal = 0u32;
        let mut quiet_count = 0u32;
        let mut best = -INF;
        let mut best_move = Move::NULL;
        let mut first = true;
        while let Some(mv) = picker.next() {
            // futility: at very shallow depth with a hopeless eval, quiet moves
            // can't recover. depth <= 2 only: deeper skips break sacrificial
            // combinations (WAC canary 268->257, attributed by A/B 2026-06-05)
            if futile && legal > 0 && !mv.is_capture() && !mv.is_promotion() {
                continue;
            }
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            self.stack[ply].current_move = mv;
            legal += 1;
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
                        r = 1 + i32::from(quiet_count >= 8) + i32::from(depth >= 8);
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
        let in_check = self.pos.in_check(self.pos.stm());
        let mut best = if in_check {
            -INF // no stand-pat while in check: must find an evasion
        } else {
            let stand_pat = self.eval.evaluate(&self.pos);
            if stand_pat >= beta {
                return stand_pat;
            }
            if stand_pat > alpha {
                alpha = stand_pat;
            }
            stand_pat
        };

        let stm = self.pos.stm();
        let mut picker =
            MovePicker::new(&self.pos, Move::NULL, [Move::NULL; 2], &self.history, stm);
        let mut legal = 0u32;
        while let Some(mv) = picker.next() {
            // quiet moves only matter when evading check
            if !in_check && !mv.is_capture() {
                continue;
            }
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            self.stack[ply].current_move = mv;
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
                    self.pv.update(ply, mv);
                    if alpha >= beta {
                        break;
                    }
                }
            }
        }

        if in_check && legal == 0 {
            return -(MATE - ply as i32); // mate found inside qsearch
        }
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
    use crate::board::{movegen::find_uci_move, Position};

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
        let mut picker = MovePicker::new(
            &pos,
            tt_move,
            [k0, k1],
            &history,
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
        let mut picker = MovePicker::new(
            &pos,
            Move::NULL,
            [killer, Move::NULL],
            &history,
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
}
