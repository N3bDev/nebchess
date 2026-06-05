//! Transposition table (spec §5.3): 32-byte clusters of three 10-byte
//! entries (AtomicU64 data + AtomicU16 key fragment), depth-preferred
//! replacement with 6-bit generation aging, mate-score ply adjustment.
//! All atomics Relaxed: the engine is single-threaded until M9b; the types
//! are SMP-shaped so Lazy SMP doesn't re-layout (XOR validation lands then).
//!
//! Layout note: `[Entry; 3]` would pad each Entry to 16 bytes (AtomicU64
//! alignment pulls the struct size from 10 to 16), making the cluster 64B.
//! We use parallel arrays — `data: [AtomicU64; 3]`, `keys: [AtomicU16; 3]`
//! — to keep the cluster exactly 32B at align(32): 24 + 6 + 2(_pad) = 32.

use std::sync::atomic::{AtomicU16, AtomicU64, AtomicU8, Ordering};

use crate::board::Move;
use crate::search::MATE_BOUND;

/// Sentinel for "no static eval stored" (the field is reserved for M4).
pub const EVAL_NONE: i32 = i16::MIN as i32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    Exact = 1,
    Lower = 2, // fail-high: real score >= stored
    Upper = 3, // fail-low:  real score <= stored
}

/// data word layout: [0..16) move | [16..32) score(i16) | [32..48) eval(i16)
///                   | [48..56) depth(u8) | [56..62) generation | [62..64) bound
///
/// Parallel-array layout: 3×AtomicU64 (24B) + 3×AtomicU16 key (6B) + 1×AtomicU16 pad (2B) = 32B.
/// This avoids the 16B-per-entry padding that a `[Entry; 3]` with AtomicU64 fields would incur.
#[repr(C, align(32))]
pub(crate) struct Cluster {
    data: [AtomicU64; 3],
    keys: [AtomicU16; 3],
    _pad: AtomicU16,
}

pub struct TtHit {
    pub mv: Move,
    pub score: i32,
    pub eval: i32,
    pub depth: i32,
    pub bound: Bound,
}

pub struct Tt {
    pub(crate) clusters: Vec<Cluster>,
    generation: AtomicU8, // 6 bits used
}

fn pack(mv: Move, score: i16, eval: i16, depth: u8, generation: u8, bound: Bound) -> u64 {
    (mv.raw() as u64)
        | ((score as u16 as u64) << 16)
        | ((eval as u16 as u64) << 32)
        | ((depth as u64) << 48)
        | (((generation & 0x3F) as u64) << 56)
        | ((bound as u64) << 62)
}

impl Tt {
    pub fn new(mb: usize) -> Tt {
        let clusters = ((mb.max(1)) << 20) / std::mem::size_of::<Cluster>();
        let mut v = Vec::with_capacity(clusters);
        for _ in 0..clusters {
            v.push(Cluster {
                data: [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)],
                keys: [AtomicU16::new(0), AtomicU16::new(0), AtomicU16::new(0)],
                _pad: AtomicU16::new(0),
            });
        }
        Tt {
            clusters: v,
            generation: AtomicU8::new(0),
        }
    }

    /// Bump the search generation (call once per `go`).
    pub fn new_search(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        for c in &self.clusters {
            for i in 0..3 {
                c.data[i].store(0, Ordering::Relaxed);
                c.keys[i].store(0, Ordering::Relaxed);
            }
        }
        self.generation.store(0, Ordering::Relaxed);
    }

    #[inline]
    fn index(&self, key: u64) -> usize {
        // multiply-high maps the full key range uniformly onto clusters
        ((key as u128 * self.clusters.len() as u128) >> 64) as usize
    }

    #[inline]
    fn fragment(key: u64) -> u16 {
        // LOW 16 bits: the mulhi index consumes the HIGH bits, so a high-bit
        // fragment would overlap the index and validate nothing.
        key as u16
    }

    /// Score stored relative to the node so mate distances stay correct
    /// wherever they're probed from (spec §5.3).
    #[inline]
    fn score_to_tt(score: i32, ply: usize) -> i16 {
        let s = if score >= MATE_BOUND {
            score + ply as i32
        } else if score <= -MATE_BOUND {
            score - ply as i32
        } else {
            score
        };
        s.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16
    }

    #[inline]
    fn score_from_tt(score: i16, ply: usize) -> i32 {
        let s = score as i32;
        if s >= MATE_BOUND {
            s - ply as i32
        } else if s <= -MATE_BOUND {
            s + ply as i32
        } else {
            s
        }
    }

    pub fn probe(&self, key: u64, ply: usize) -> Option<TtHit> {
        let cluster = &self.clusters[self.index(key)];
        let frag = Self::fragment(key);
        for i in 0..3 {
            if cluster.keys[i].load(Ordering::Relaxed) == frag {
                let d = cluster.data[i].load(Ordering::Relaxed);
                let bound = match d >> 62 {
                    1 => Bound::Exact,
                    2 => Bound::Lower,
                    3 => Bound::Upper,
                    _ => continue, // empty slot whose frag coincidentally matched
                };
                return Some(TtHit {
                    mv: Move::from_raw(d as u16),
                    score: Self::score_from_tt((d >> 16) as u16 as i16, ply),
                    eval: ((d >> 32) as u16 as i16) as i32,
                    depth: ((d >> 48) & 0xFF) as i32,
                    bound,
                });
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)] // store mirrors the TT entry fields: no natural grouping
    pub fn store(
        &self,
        key: u64,
        mv: Move,
        score: i32,
        eval: i32,
        depth: i32,
        bound: Bound,
        ply: usize,
    ) {
        let cluster = &self.clusters[self.index(key)];
        let frag = Self::fragment(key);
        let generation = self.generation.load(Ordering::Relaxed) & 0x3F;

        // pick a slot: same key > empty > lowest quality (depth - 4*age)
        let mut victim = 0usize;
        let mut victim_quality = i32::MAX;
        for i in 0..3 {
            let d = cluster.data[i].load(Ordering::Relaxed);
            if cluster.keys[i].load(Ordering::Relaxed) == frag && d >> 62 != 0 {
                victim = i;
                break; // same-key: always replace, quality irrelevant
            }
            if d >> 62 == 0 {
                // empty slot: best possible victim short of same-key
                if victim_quality > i32::MIN + 1 {
                    victim = i;
                    victim_quality = i32::MIN + 1;
                }
                continue;
            }
            let e_depth = ((d >> 48) & 0xFF) as i32;
            let e_gen = ((d >> 56) & 0x3F) as u8;
            let age = (generation.wrapping_sub(e_gen)) & 0x3F;
            let quality = e_depth - 4 * age as i32;
            if quality < victim_quality {
                victim = i;
                victim_quality = quality;
            }
        }

        cluster.keys[victim].store(frag, Ordering::Relaxed);
        cluster.data[victim].store(
            pack(
                mv,
                Self::score_to_tt(score, ply),
                eval.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                depth.clamp(0, 255) as u8,
                generation,
                bound,
            ),
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Move, Square};
    use crate::search::{MATE, MATE_BOUND};

    fn mv() -> Move {
        Move::new(Square::E1, Square::G1, Move::KING_CASTLE)
    }

    #[test]
    fn size_and_alignment() {
        assert_eq!(std::mem::size_of::<Cluster>(), 32);
        assert_eq!(std::mem::align_of::<Cluster>(), 32);
        let tt = Tt::new(1); // 1 MB
        assert_eq!(tt.clusters.len(), (1 << 20) / 32);
    }

    #[test]
    fn store_probe_roundtrip() {
        let tt = Tt::new(1);
        tt.store(
            0xDEAD_BEEF_CAFE_F00D,
            mv(),
            123,
            EVAL_NONE,
            7,
            Bound::Exact,
            0,
        );
        let hit = tt.probe(0xDEAD_BEEF_CAFE_F00D, 0).expect("hit");
        assert_eq!(hit.mv, mv());
        assert_eq!(hit.score, 123);
        assert_eq!(hit.depth, 7);
        assert_eq!(hit.bound, Bound::Exact);
        assert_eq!(hit.eval, EVAL_NONE);
        // same cluster (identical high bits -> same mulhi index), different
        // low-16 fragment: must MISS, proving the fragment discriminates
        assert!(tt.probe(0xDEAD_BEEF_CAFE_F00E, 0).is_none());
    }

    #[test]
    fn mate_scores_adjust_by_ply() {
        let tt = Tt::new(1);
        // at ply 2 we found "mate in 3 plies from root" = MATE - 5
        tt.store(42, mv(), MATE - 5, EVAL_NONE, 9, Bound::Exact, 2);
        // probed from ply 4, the same line is "mate 1 ply nearer root-wise":
        // stored node-relative MATE-3, returned MATE-3-4 = MATE-7
        let hit = tt.probe(42, 4).expect("hit");
        assert_eq!(hit.score, MATE - 7);
        assert!(hit.score > MATE_BOUND);
        // negative mates mirror
        tt.store(43, mv(), -(MATE - 5), EVAL_NONE, 9, Bound::Exact, 2);
        assert_eq!(tt.probe(43, 4).unwrap().score, -(MATE - 7));
    }

    #[test]
    fn same_key_updates_in_place() {
        let tt = Tt::new(1);
        tt.store(7, mv(), 10, EVAL_NONE, 3, Bound::Upper, 0);
        tt.store(7, mv(), 99, EVAL_NONE, 5, Bound::Exact, 0);
        let hit = tt.probe(7, 0).unwrap();
        assert_eq!(hit.score, 99);
        assert_eq!(hit.depth, 5);
        // and only one slot was consumed: low-bit-adjacent keys share the
        // mulhi cluster (high bits identical) but carry distinct fragments
        tt.store(8, mv(), 1, EVAL_NONE, 1, Bound::Exact, 0);
        tt.store(9, mv(), 2, EVAL_NONE, 1, Bound::Exact, 0);
        assert!(tt.probe(7, 0).is_some(), "original survived cluster fill");
        assert!(tt.probe(8, 0).is_some());
        assert!(tt.probe(9, 0).is_some());
    }

    #[test]
    fn replacement_prefers_shallow_and_stale() {
        let tt = Tt::new(1);
        // low-bit-adjacent keys: same mulhi cluster, distinct fragments
        let k = |i: u64| 1000 + i;
        // fill the 3 slots: depths 12, 3, 12
        tt.store(k(1), mv(), 0, EVAL_NONE, 12, Bound::Exact, 0);
        tt.store(k(2), mv(), 0, EVAL_NONE, 3, Bound::Exact, 0);
        tt.store(k(3), mv(), 0, EVAL_NONE, 12, Bound::Exact, 0);
        // a 4th key must evict the depth-3 entry, keeping both depth-12s
        tt.store(k(4), mv(), 0, EVAL_NONE, 8, Bound::Exact, 0);
        assert!(tt.probe(k(2), 0).is_none(), "shallow entry evicted");
        assert!(tt.probe(k(1), 0).is_some());
        assert!(tt.probe(k(3), 0).is_some());
        assert!(tt.probe(k(4), 0).is_some());
    }

    #[test]
    fn generation_ages_old_entries_out() {
        let tt = Tt::new(1);
        let k = |i: u64| 2000 + i; // same cluster, distinct fragments
        tt.store(k(1), mv(), 0, EVAL_NONE, 12, Bound::Exact, 0);
        tt.store(k(2), mv(), 0, EVAL_NONE, 12, Bound::Exact, 0);
        tt.store(k(3), mv(), 0, EVAL_NONE, 12, Bound::Exact, 0);
        // several searches later, a shallower NEW entry still gets a slot
        for _ in 0..4 {
            tt.new_search();
        }
        tt.store(k(4), mv(), 0, EVAL_NONE, 5, Bound::Exact, 0);
        assert!(
            tt.probe(k(4), 0).is_some(),
            "stale depth lost to fresh entry"
        );
    }

    #[test]
    fn clear_empties_everything() {
        let tt = Tt::new(1);
        tt.store(7, mv(), 10, EVAL_NONE, 3, Bound::Exact, 0);
        tt.clear();
        assert!(tt.probe(7, 0).is_none());
    }

    #[test]
    fn collision_stress_no_panics() {
        let tt = Tt::new(1); // tiny: heavy collisions on purpose
        let mut state = 0x4E45u64;
        let mut next = || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        for i in 0..100_000u64 {
            let key = next();
            tt.store(
                key,
                mv(),
                (i % 2000) as i32 - 1000,
                EVAL_NONE,
                (i % 32) as i32,
                Bound::Lower,
                (i % 64) as usize,
            );
            let _ = tt.probe(next(), (i % 64) as usize); // mostly misses
        }
    }
}
