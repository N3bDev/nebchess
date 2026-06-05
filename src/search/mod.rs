//! M2 search: iterative-deepening driver lives in Task 5; this module is
//! fixed-depth negamax + alpha-beta + quiescence with MVV-LVA ordering.
//! All mutable search state lives in SearchThread (spec §5.1).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::board::{generate_moves, Move, MoveList, PieceType, Position};
use crate::eval::psqt::MATERIAL;
use crate::eval::Evaluator;

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

/// Scores generated moves once, then yields them best-first by selection.
/// M2 ordering: captures by MVV-LVA (above all quiets), quiets unordered.
struct MovePicker {
    moves: MoveList,
    scores: [i32; 256],
    cur: usize,
}

impl MovePicker {
    fn new(pos: &Position) -> MovePicker {
        let mut moves = MoveList::new();
        generate_moves(pos, &mut moves);
        let mut scores = [0i32; 256];
        for (i, &mv) in moves.iter().enumerate() {
            if mv.is_capture() {
                let victim = if mv.flag() == Move::EN_PASSANT {
                    PieceType::Pawn
                } else {
                    pos.piece_on(mv.to()).expect("capture target").piece_type()
                };
                let attacker = pos.piece_on(mv.from()).expect("mover").piece_type();
                scores[i] = 1_000_000 + 10 * MATERIAL[victim.index()] - MATERIAL[attacker.index()];
            }
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

pub struct SearchThread<E: Evaluator> {
    pub pos: Position,
    pub eval: E,
    pub nodes: u64,
    stop: Arc<AtomicBool>,
    node_limit: Option<u64>,
    stopped: bool,
    pv: PvTable,
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
            pv: PvTable::new(),
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

    /// Best line from the last completed search call.
    pub fn pv_line(&self) -> &[Move] {
        self.pv.line()
    }
    pub fn was_stopped(&self) -> bool {
        self.stopped
    }

    /// Fixed-depth, full-window search. Returns (best move, score).
    /// Task 5's iterative deepening calls this once per depth.
    pub fn search_to_depth(&mut self, depth: i32) -> (Option<Move>, i32) {
        self.eval.refresh(&self.pos);
        self.stopped = false;
        let score = self.negamax(depth, -INF, INF, 0);
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
        }
        self.stopped
    }

    /// Small jitter (±1cp) instead of flat 0: avoids threefold blindness in
    /// self-play pools (spec §3).
    #[inline]
    fn draw_score(&self) -> i32 {
        1 - (self.nodes as i32 & 2)
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

        let mut picker = MovePicker::new(&self.pos);
        let mut legal = 0u32;
        let mut best = -INF;
        while let Some(mv) = picker.next() {
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            legal += 1;
            let score = -self.negamax(depth - 1, -beta, -alpha, ply + 1);
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
                        break; // beta cutoff
                    }
                }
            }
        }

        if legal == 0 {
            return if self.pos.in_check(self.pos.stm()) {
                -(MATE - ply as i32) // checkmated at this ply
            } else {
                self.draw_score() // stalemate
            };
        }
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

        let mut picker = MovePicker::new(&self.pos);
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
}
