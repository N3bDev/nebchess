//! `go` parameters and time allocation (spec §5.4).

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

pub struct TimeManager {
    start: Instant,
    soft: Option<Duration>,
    hard: Option<Duration>,
}

impl TimeManager {
    pub fn new(limits: &Limits, stm: Color, overhead_ms: u64) -> TimeManager {
        let start = Instant::now();
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
                    let avail = time.saturating_sub(overhead_ms).max(1);
                    let mtg = u64::from(limits.movestogo.unwrap_or(30).clamp(1, 30));
                    let soft = (avail / mtg + inc / 2).clamp(1, avail);
                    let hard = (soft * 4).min(avail);
                    (Some(soft), Some(hard))
                }
            }
        };
        TimeManager {
            start,
            soft: soft.map(Duration::from_millis),
            hard: hard.map(Duration::from_millis),
        }
    }

    /// Absolute instant for the in-search abort poll (None = no time control).
    pub fn hard_deadline(&self) -> Option<Instant> {
        self.hard.map(|d| self.start + d)
    }

    /// Checked between iterations: don't start another depth past soft.
    pub fn past_soft(&self) -> bool {
        match self.soft {
            Some(s) => self.start.elapsed() >= s,
            None => false,
        }
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    /// (soft, hard) in ms — for tests and debugging.
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
            "soft ~ time/30, got {soft}"
        );
        assert_eq!(hard, soft * 4);
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
            "time/10 + inc/2, got {soft}"
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
}
