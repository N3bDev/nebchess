//! `go` parameters and time allocation (spec §5.4).
//!
//! TimeBrain (Plan 7 T2+T3): a stateful per-search controller consulted between
//! iterations. The search tree is unchanged — only WHEN we stop between
//! iterations moves.
//!
//! Gate 1 (T2): allocation reserves an emergency clock buffer (flag
//! protection), scales the soft deadline by best-move stability (stop early
//! when the PV is settled, spend longer right after it changes), and caps any
//! single move at a third of the usable clock.
//!
//! Gate 2 (T3) layers two field-motivated adaptive behaviors (won-fast, a
//! third Gate-2 behavior, was carved out: a +500 eval at this strength is
//! often a sharp position still needing accuracy, not a trivial win, so
//! halving time there lost games):
//! - **panic extension** — a >=50cp eval drop at depth >=8 extends the soft
//!   deadline (the "I'm getting mated" late realization from the corpus);
//! - **clock catch-up** — being well behind on the clock trims the base soft
//!   at allocation (stay in time).
//!
//! Resolution order between iterations: panic-extend beats stability (see
//! [`TimeManager::report_iteration`]); catch-up is allocation-time only (see
//! [`TimeManager::new`]).

use std::time::{Duration, Instant};

use crate::board::Color;

#[derive(Default, Clone, Debug)]
pub struct Limits {
    pub depth: Option<i32>,
    pub nodes: Option<u64>,
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

// ---- TimeBrain Gate 2 thresholds (field-motivated; see Plan 7 T3) ----
/// Panic extension fires when the score falls at least this many cp between
/// iterations — the late "I'm getting mated" realization from the field corpus.
const PANIC_DROP_CP: i32 = 50;
/// ...but only once the search is deep enough to trust the drop (shallow
/// score swings are noise, not danger).
const PANIC_MIN_DEPTH: i32 = 8;

pub struct TimeManager {
    start: Instant,
    soft: Option<Duration>, // base soft (post-catch-up, pre-trend-scaling)
    hard: Option<Duration>,
    // `go movetime N` is a UCI EXACT-budget contract: the search must spend ~N
    // ms regardless of stability/panic. True ONLY for the movetime branch
    // (where soft==hard); when set, `effective_soft_ms` returns the base soft
    // UNSCALED so trend-scaling cannot shrink (or grow) the budget.
    exact: bool,
    // Gate-1 stability state:
    last_best: u16,      // caller-supplied move key (Move's raw bits)
    stable_iters: u32,   // consecutive iterations with the same best move
    soft_scale_pct: u32, // 100 = base; recomputed by report_iteration
    // Gate-2 score-trend state:
    prev_score: Option<i32>, // last iteration's score (None before iter 1)
}

impl TimeManager {
    /// `opp_time` is the OPPONENT's remaining clock (ms) when known — threaded
    /// from the `go` command (stm white → btime, stm black → wtime). Gate 2's
    /// clock catch-up consults it: when we are well behind (`my_time <
    /// opp_time * 6 / 10`) the base soft is trimmed ×0.8 AT ALLOCATION so we
    /// stop falling further behind. `None` (no opponent clock / movetime /
    /// infinite) leaves allocation at the Gate-1 value.
    pub fn new(
        limits: &Limits,
        stm: Color,
        overhead_ms: u64,
        opp_time: Option<u64>,
    ) -> TimeManager {
        let start = Instant::now();
        // movetime is the only EXACT-budget path (soft==hard, no trend-scaling).
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
                    let mut soft = (usable / mtg + inc * 3 / 4).clamp(1, third);
                    // Gate-2 clock catch-up: if we are well behind on the clock
                    // (our remaining `time` < opp * 6/10), trim the base soft
                    // ×0.8 so we stop falling further behind. Applied at
                    // allocation — NOT per iteration. It runs BEFORE the hard
                    // ceiling is derived, so the proportional `hard = soft*5`
                    // relationship is preserved (a genuine panic can still
                    // re-expand the effective soft up to that contracted hard).
                    if let Some(opp) = opp_time {
                        if time < opp * 6 / 10 {
                            soft = (soft * 8 / 10).max(1);
                        }
                    }
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
            prev_score: None,
        }
    }

    /// Report the result of a completed iteration. `best_key` is the raw bits
    /// of the iteration's best move; `score_cp` its score (side-to-move cp).
    ///
    /// Stability (Gate 1): a repeat of the previous best grows `stable_iters`;
    /// a change resets it. Gate 2 layers one score-trend behavior on top via a
    /// single priority chain — the resolution ORDER is fixed:
    ///
    ///   1. PANIC-EXTEND (beats everything): a >= [`PANIC_DROP_CP`] eval drop
    ///      from the previous iteration at depth >= [`PANIC_MIN_DEPTH`] — the
    ///      "I'm getting mated" late realization. Spend MORE (pct 150).
    ///   2. STABILITY (Gate 1, unchanged): change-extend 140 / settled 60 /
    ///      almost-settled 80 / base 100.
    ///
    /// `score_cp` is consumed only by panic now (won-fast, which also read it,
    /// was carved out). With no eval drop and an even clock the resolved scale
    /// is IDENTICAL to Gate 1 (the chain falls through to branch 2). The
    /// catch-up behavior is allocation-time only (see [`new`]), not here.
    pub fn report_iteration(&mut self, depth: i32, score_cp: i32, best_key: u16) {
        if best_key == self.last_best {
            self.stable_iters += 1;
        } else {
            self.stable_iters = 0;
            self.last_best = best_key;
        }
        // Eval drop vs the PREVIOUS iteration (read before we overwrite it).
        let panic = depth >= PANIC_MIN_DEPTH
            && self
                .prev_score
                .is_some_and(|prev| prev - score_cp >= PANIC_DROP_CP);
        // Priority chain: panic-extend > stability.
        self.soft_scale_pct = if panic {
            150
        } else if self.stable_iters == 0 && depth > 1 {
            // change-extend: still discovering right after a best-move change.
            140
        } else if self.stable_iters >= 3 {
            60
        } else if self.stable_iters == 2 {
            80
        } else {
            100
        };
        self.prev_score = Some(score_cp);
    }

    /// Trend-scaled soft deadline (ms): base soft × `soft_scale_pct`, floored
    /// at `soft / 4` and capped at `hard`. None when there is no clock
    /// (infinite / depth / nodes search).
    ///
    /// The `soft / 4` floor (Gate 2) keeps one fluky low-scaling iteration from
    /// blitzing the move out instantly; the `hard` cap keeps an extension
    /// (change-extend or panic) from outrunning the absolute one-move ceiling.
    /// Floor is applied before the cap so the result is ALWAYS `<= hard`.
    ///
    /// EXACT budgets (`go movetime N`) are exempt: trend-scaling is the +26-elo
    /// wtime/btime feature, but movetime is a fixed UCI contract — return the
    /// base soft UNSCALED (the floor/cap are moot since soft==hard). This is the
    /// single choke point; `report_iteration` may still run harmlessly.
    pub fn effective_soft_ms(&self) -> Option<u64> {
        let soft = self.soft?.as_millis() as u64;
        if self.exact {
            return Some(soft);
        }
        let scaled = (soft * u64::from(self.soft_scale_pct) / 100).max(soft / 4);
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
        let tm = TimeManager::new(&l, Color::White, 10, None);
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
        let mut tm = TimeManager::new(&l, Color::White, 0, None);
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
        let tm = TimeManager::new(&l, Color::White, 10, None);
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
        let tm = TimeManager::new(&l, Color::Black, 10, None);
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
        let tm = TimeManager::new(&l, Color::White, 10, None);
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
        let tm = TimeManager::new(&l, Color::White, 10, None);
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
        let tm = TimeManager::new(&l, Color::White, 10, None);
        assert_eq!(tm.budgets_ms(), (None, None));
        let l = Limits {
            depth: Some(6),
            ..Limits::default()
        };
        let tm = TimeManager::new(&l, Color::White, 10, None);
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
        let tm = TimeManager::new(&l, Color::White, 50, None);
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
        let mut tm = TimeManager::new(&l, Color::White, 10, None);
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
        let mut tm = TimeManager::new(&l, Color::White, 10, None);
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
        let tm = TimeManager::new(&l, Color::White, 10, None);
        let (_, hard) = tm.budgets_ms();
        assert!(hard.unwrap() <= 10_000, "never >1/3 of usable on one move");
    }

    // ---- TimeBrain Gate 2: panic extension + clock catch-up (won-fast carved out) ----

    #[test]
    fn score_drop_at_depth_extends_soft() {
        // (a) A >=50cp eval drop at depth >= 8 is the "I'm getting mated" late
        // realization (the M4-deferred PV-instability item) — extend ×150.
        let l = Limits {
            wtime: Some(60_000),
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&l, Color::White, 10, None);
        let base = tm.budgets_ms().0.unwrap();
        // Settle the PV first so the change-extend (140) is NOT in play and the
        // baseline scale is stability's 60 — the panic must override it to 150.
        for d in 1..=8 {
            tm.report_iteration(d, 30, 1 /*same move key*/);
        }
        // depth 9: same move, but the score collapsed 30 -> -30 (a 60cp drop).
        tm.report_iteration(9, -30, 1);
        // 150% of base soft, and base*1.5 is well under hard (base*5) so uncapped.
        assert_eq!(
            tm.effective_soft_ms().unwrap(),
            base * 150 / 100,
            "panic extension beats stability: pct 150"
        );
    }

    #[test]
    fn won_position_uses_gate1_stability() {
        // won-fast was CARVED OUT: a score >= +500 with a settled PV no longer
        // halves the budget (pct 50). It must fall through to Gate-1 stability
        // — a settled PV (stable_iters >= 3) scales to 60, exactly as any other
        // settled position does, regardless of how winning the score is.
        let l = Limits {
            wtime: Some(60_000),
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&l, Color::White, 10, None);
        let base = tm.budgets_ms().0.unwrap();
        // 4 iterations, same move, comfortably winning, no drops.
        for d in 1..=4 {
            tm.report_iteration(d, 600, 1 /*same move key*/);
        }
        assert_eq!(
            tm.effective_soft_ms().unwrap(),
            base * 60 / 100,
            "won + stable now uses Gate-1 stability (pct 60), NOT won-fast (50)"
        );
    }

    #[test]
    fn behind_on_clock_allocates_less() {
        // (c) my_time (20s) < opp_time (60s) * 6/10 = 36s -> behind: base soft
        // is scaled ×0.8 AT ALLOCATION (not per-iteration).
        let l = Limits {
            wtime: Some(20_000),
            ..Limits::default()
        };
        let even = TimeManager::new(&l, Color::White, 10, None);
        let behind = TimeManager::new(&l, Color::White, 10, Some(60_000));
        let even_soft = even.budgets_ms().0.unwrap();
        let behind_soft = behind.budgets_ms().0.unwrap();
        assert_eq!(
            behind_soft,
            even_soft * 8 / 10,
            "behind on clock: base soft ×0.8"
        );
        // Not behind when clocks are close: my 20s vs opp 33s, threshold
        // 33*6/10 = 19 (int), and 20 >= 19 -> unscaled.
        let close = TimeManager::new(&l, Color::White, 10, Some(33_000));
        assert_eq!(
            close.budgets_ms().0.unwrap(),
            even_soft,
            "close clocks (my 20s vs opp 33s) are not behind"
        );
    }
}
