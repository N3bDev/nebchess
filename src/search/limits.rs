//! `go` parameters and time allocation (spec §5.4).
//!
//! TimeBrain Gate 1 (Plan 7 T2): a stateful per-search controller consulted
//! between iterations. Allocation reserves an emergency clock buffer (flag
//! protection), scales the soft deadline by best-move stability (stop early
//! when the PV is settled, spend longer right after it changes), and caps any
//! single move at a third of the usable clock. The search tree is unchanged —
//! only WHEN we stop between iterations moves.
//!
//! The Gate 2 adaptive layer (panic extension + clock catch-up; won-fast was
//! carved out earlier) SPRT'd negative twice and was reverted: this is Gate 1
//! plus the movetime-exact correctness fix (a `go movetime N` budget is a UCI
//! EXACT contract, exempt from stability scaling).

use std::time::{Duration, Instant};

use crate::board::Color;

#[derive(Default, Clone, Debug)]
pub struct Limits {
    pub depth: Option<i32>,
    pub nodes: Option<u64>,
    /// Stop at the NEXT depth-iteration boundary once `self.nodes` exceeds this.
    /// Unlike `nodes` (a hard mid-search cutoff), this lets the current iteration finish.
    pub soft_nodes: Option<u64>,
    pub movetime: Option<u64>, // ms
    pub wtime: Option<u64>,
    pub btime: Option<u64>,
    pub winc: Option<u64>,
    pub binc: Option<u64>,
    pub movestogo: Option<u32>,
    pub infinite: bool,
}

/// Smallest emergency reserve we will ever hold back (ms) — even on a near-flag
/// clock, leave this so a move can physically be transmitted.
pub const RESERVE_MIN_MS: u64 = 50;

pub struct TimeManager {
    start: Instant,
    soft: Option<Duration>, // base soft (pre-stability-scaling)
    hard: Option<Duration>,
    // `go movetime N` is a UCI EXACT-budget contract: the search must spend ~N
    // ms regardless of stability. True ONLY for the movetime branch (where
    // soft==hard); when set, `effective_soft_ms` returns the base soft UNSCALED
    // so stability scaling cannot shrink (or grow) the budget.
    exact: bool,
    // Gate-1 stability state:
    last_best: u16,      // caller-supplied move key (Move's raw bits)
    stable_iters: u32,   // consecutive iterations with the same best move
    soft_scale_pct: u32, // 100 = base; recomputed by report_iteration
}

impl TimeManager {
    pub fn new(limits: &Limits, stm: Color, overhead_ms: u64) -> TimeManager {
        let start = Instant::now();
        // movetime is the only EXACT-budget path (soft==hard, no scaling).
        let exact = !limits.infinite && limits.movetime.is_some();
        let (soft, hard) = if limits.infinite {
            (None, None)
        } else if let Some(mt) = limits.movetime {
            let t = mt.saturating_sub(overhead_ms).max(1);
            (Some(t), Some(t))
        } else {
            let (time, inc) = match stm {
                Color::White => (limits.wtime, limits.winc.unwrap_or(0)),
                Color::Black => (limits.btime, limits.binc.unwrap_or(0)),
            };
            match time {
                None => (None, None), // depth/nodes-only searches
                Some(time) => {
                    // avail   = time left after subtracting transmission overhead
                    // reserve = emergency buffer (flag protection)
                    // usable  = what this whole game-phase may consume
                    // soft    = the per-move base target (movestogo + a slice of inc)
                    // hard    = the absolute one-move ceiling (a third of usable)
                    let avail = time.saturating_sub(overhead_ms).max(1);
                    let reserve = (avail / 16).clamp(RESERVE_MIN_MS, 2_000);
                    let usable = avail.saturating_sub(reserve).max(1);
                    // `usable / 3` is the hard cap; floor it at 1 so the clamp
                    // bounds never invert (clamp panics if min > max) on a
                    // near-flag clock where usable is 1 or 2.
                    let third = (usable / 3).max(1);
                    let mtg = u64::from(limits.movestogo.unwrap_or(30).clamp(1, 40));
                    let soft = (usable / mtg + inc * 3 / 4).clamp(1, third);
                    let hard = (soft * 5).min(third).max(soft);
                    (Some(soft), Some(hard))
                }
            }
        };
        TimeManager {
            start,
            soft: soft.map(Duration::from_millis),
            hard: hard.map(Duration::from_millis),
            exact,
            last_best: 0,
            stable_iters: 0,
            soft_scale_pct: 100,
        }
    }

    /// Report the result of a completed iteration. Gate 1 uses only `best_key`
    /// (the raw bits of the iteration's best move): a repeat of the previous
    /// best grows `stable_iters`; a change resets it. The resulting scale (vs
    /// the base soft, in percent) shrinks the soft deadline for a settled PV
    /// and extends it on the iteration right after the best move changes.
    /// `depth`/`score_cp` are unused (Gate-2 scaffolding that consumed them was
    /// reverted; `score_cp` is kept on the signature for call-site stability).
    pub fn report_iteration(&mut self, depth: i32, _score_cp: i32, best_key: u16) {
        if best_key == self.last_best {
            self.stable_iters += 1;
        } else {
            self.stable_iters = 0;
            self.last_best = best_key;
        }
        // 140 takes precedence: the iteration immediately AFTER a change (we
        // just reset, and this isn't the very first iteration) means the search
        // is still discovering — spend longer. Otherwise scale by how settled
        // the PV is.
        self.soft_scale_pct = if self.stable_iters == 0 && depth > 1 {
            140
        } else if self.stable_iters >= 3 {
            60
        } else if self.stable_iters == 2 {
            80
        } else {
            100
        };
    }

    /// Stability-scaled soft deadline (ms), clamped to never exceed `hard`.
    /// None when there is no clock (infinite / depth / nodes search).
    ///
    /// EXACT budgets (`go movetime N`) are exempt: stability scaling is the
    /// +26-elo wtime/btime feature, but movetime is a fixed UCI contract —
    /// return the base soft UNSCALED (the cap is moot since soft==hard). This is
    /// the single choke point; `report_iteration` may still run harmlessly.
    pub fn effective_soft_ms(&self) -> Option<u64> {
        let soft = self.soft?.as_millis() as u64;
        if self.exact {
            return Some(soft);
        }
        let scaled = soft * u64::from(self.soft_scale_pct) / 100;
        // never let the (possibly extended) soft outrun the hard ceiling
        let capped = match self.hard {
            Some(h) => scaled.min(h.as_millis() as u64),
            None => scaled,
        };
        Some(capped)
    }

    /// Absolute instant for the in-search abort poll (None = no time control).
    pub fn hard_deadline(&self) -> Option<Instant> {
        self.hard.map(|d| self.start + d)
    }

    /// Checked between iterations: don't start another depth past the
    /// stability-scaled (effective) soft deadline.
    pub fn past_soft(&self) -> bool {
        match self.effective_soft_ms() {
            Some(s) => self.start.elapsed() >= Duration::from_millis(s),
            None => false,
        }
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    /// (base soft, hard) in ms — for tests and debugging. Reflects the
    /// allocation, NOT the stability scaling (see [`effective_soft_ms`]).
    pub fn budgets_ms(&self) -> (Option<u64>, Option<u64>) {
        (
            self.soft.map(|d| d.as_millis() as u64),
            self.hard.map(|d| d.as_millis() as u64),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Color;

    #[test]
    fn movetime_sets_equal_soft_and_hard() {
        let l = Limits {
            movetime: Some(500),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, hard) = tm.budgets_ms();
        assert_eq!(soft, Some(490));
        assert_eq!(hard, Some(490));
    }

    #[test]
    fn movetime_is_exact_under_stable_pv() {
        // movetime is a UCI EXACT-budget contract: trend-scaling must NOT touch
        // the movetime budget. A settled PV would scale a wtime search to 60%
        // (Gate-1 stability); movetime stays at the full budget.
        let l = Limits {
            movetime: Some(2_000),
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&l, Color::White, 0);
        let soft = tm.budgets_ms().0.unwrap();
        // Settled PV across 4 iterations (stable_iters >= 3 would scale to 60%
        // on a wtime search).
        for d in 7..=10 {
            tm.report_iteration(d, 800, 1 /*same move key*/);
        }
        assert_eq!(
            tm.effective_soft_ms(),
            Some(soft),
            "movetime is exact: full budget, NOT scaled by stability"
        );
    }

    #[test]
    fn clock_allocation_is_sane() {
        let l = Limits {
            wtime: Some(60_000),
            ..Limits::default()
        }; // one minute, no increment
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, hard) = tm.budgets_ms();
        let (soft, hard) = (soft.unwrap(), hard.unwrap());
        assert!(
            (1_500..=2_500).contains(&soft),
            "soft ~ usable/30, got {soft}"
        );
        // TimeBrain Gate 1: hard is soft*5 capped at usable/3 (was soft*4).
        // 60s no-inc => usable/3 ~ 19_330, so the soft*5 term (9_665) wins.
        assert_eq!(hard, soft * 5);
        // black's clock must be read for black
        let l = Limits {
            btime: Some(30_000),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::Black, 10);
        assert!(tm.budgets_ms().0.unwrap() <= 1_100);
    }

    #[test]
    fn movestogo_and_increment_raise_budget() {
        let l = Limits {
            wtime: Some(60_000),
            movestogo: Some(10),
            winc: Some(2_000),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 10);
        let soft = tm.budgets_ms().0.unwrap();
        assert!(
            (6_500..=7_500).contains(&soft),
            "usable/10 + inc*3/4, got {soft}"
        );
    }

    #[test]
    fn low_time_never_overspends() {
        let l = Limits {
            wtime: Some(50),
            ..Limits::default()
        }; // 50ms on the clock!
        let tm = TimeManager::new(&l, Color::White, 10);
        let (soft, hard) = tm.budgets_ms();
        let hard = hard.unwrap();
        assert!(
            hard <= 40,
            "hard must stay under remaining-overhead, got {hard}"
        );
        assert!(hard >= 1);
        assert!(soft.unwrap() <= hard);
    }

    #[test]
    fn infinite_and_depth_only_have_no_deadlines() {
        let l = Limits {
            infinite: true,
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 10);
        assert_eq!(tm.budgets_ms(), (None, None));
        let l = Limits {
            depth: Some(6),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 10);
        assert_eq!(tm.budgets_ms(), (None, None));
    }

    // ---- TimeBrain Gate 1: reserve + stability scaling + hard cap ----

    #[test]
    fn emergency_reserve_is_never_allocated() {
        // 1000ms left, no inc: hard must leave >= RESERVE_MIN_MS (50) + overhead untouched
        let l = Limits {
            wtime: Some(1_000),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 50);
        let (_, hard) = tm.budgets_ms();
        assert!(
            hard.unwrap() <= 1_000 - 50 - 50,
            "hard {} must respect reserve",
            hard.unwrap()
        );
    }

    #[test]
    fn stability_scales_soft_down() {
        let l = Limits {
            wtime: Some(60_000),
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        // 3 iterations, same best move, steady score -> effective soft shrinks
        for d in 1..=3 {
            tm.report_iteration(d, 25, 1 /*same move key*/);
        }
        assert!(
            tm.effective_soft_ms().unwrap() < base,
            "stable PV must shrink soft"
        );
    }

    #[test]
    fn best_move_change_extends_soft() {
        let l = Limits {
            wtime: Some(60_000),
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        tm.report_iteration(1, 25, 1);
        tm.report_iteration(2, 25, 2); // best move CHANGED
        assert!(
            tm.effective_soft_ms().unwrap() > base,
            "instability must extend soft"
        );
    }

    #[test]
    fn hard_cap_is_a_third_of_usable() {
        let l = Limits {
            wtime: Some(30_000),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 10);
        let (_, hard) = tm.budgets_ms();
        assert!(hard.unwrap() <= 10_000, "never >1/3 of usable on one move");
    }

    #[test]
    fn won_position_uses_gate1_stability() {
        // won-fast was CARVED OUT and the Gate-2 adaptive layer reverted: a
        // score >= +500 with a settled PV does NOT halve the budget (pct 50).
        // It uses Gate-1 stability — a settled PV (stable_iters >= 3) scales to
        // 60, exactly as any other settled position does, regardless of how
        // winning the score is.
        let l = Limits {
            wtime: Some(60_000),
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&l, Color::White, 10);
        let base = tm.budgets_ms().0.unwrap();
        // 4 iterations, same move, comfortably winning, no drops.
        for d in 1..=4 {
            tm.report_iteration(d, 600, 1 /*same move key*/);
        }
        assert_eq!(
            tm.effective_soft_ms().unwrap(),
            base * 60 / 100,
            "won + stable uses Gate-1 stability (pct 60), NOT won-fast (50)"
        );
    }
}
