# NebChess Plan 3 (M3): Transposition Table, Move Ordering, PVS — SPRT-Gated

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The first measured ELO climb: transposition table (cutoffs, then TT-move ordering), killer moves, history heuristic, and PVS — each feature accepted only after passing its own SPRT self-play test (expected cumulative gain ~+200-330; landing zone ~1800-1900 CCRL-ish).

**Architecture:** A shared `Arc<Tt>` (atomics-backed 32-byte clusters, spec §5.3) owned by the UCI layer and persisting across moves within a game; a per-ply `StackEntry` array in `SearchThread` (spec §5.1 — killers now, `static_eval`/`excluded_move` slots reserved for M4/M6); MovePicker grows ordering tiers (TT move > captures > killers > history). PVS converts negamax's move loop to null-window re-search.

**Tech Stack:** Rust std-only (`AtomicU64`/`AtomicU16`). fastchess SPRT via the frozen `tools/sprt.sh` protocol (first real use).

**Spec:** `docs/superpowers/specs/2026-06-04-nebchess-engine-design.md` §5.1 (SearchStack), §5.3 (TT layout/replacement/mate-ply), §10.3 (SPRT protocol), §12 M3 row ("TT cuts → TT-move ordering → killers → history → PVS, each its own SPRT").

**Executed-review amendments (code as landed differs from blocks below):**
- T2: TT cluster landed as parallel arrays (`data: [AtomicU64; 3]` + `keys: [AtomicU16; 3]` + pad) — the plan's `[Entry; 3]` form pads to 64B via AtomicU64 alignment; the parallel form hits exactly 32B (size/align test enforces). Documented in tt.rs.
- Forfeit-scan gate reinterpretation: SPRT logs under concurrency-15 host load showed 1-2 tiny timeouts (1-82ms overruns, symmetric across old/new in gate #4) in 4 of 5 runs; these were classified as scheduler-load artifacts. The BLOCKER gate is the dedicated `forfeit-gauntlet.sh` run on an idle system, which passed 0/200. Tracked: backlog item "WSL2 timing robustness" (revisit at desktop migration; options: concurrency cap, adaptive overhead).

**Plan deviations from spec (recorded deliberately):**
- §5.3 parallel TT clear/resize: deferred to M9b (SMP) — M3 clears with a simple loop (single-threaded engine; a 4096MB table would stall, but Hash max stays modest until SMP).
- §5.3 TT prefetch hook: deferred to the NPS milestone (no measurable value pre-SMP at M3 node rates).
- §5.3 16-bit static-eval field: the entry RESERVES the field but M3 stores a sentinel (`i16::MIN`) — the static-eval stack that feeds it lands in M4 (RFP/improving). Recorded so M4 doesn't re-layout the entry.
- §10.5 "TT-on vs TT-off comparison": literal score equality at fixed depth is NOT a sound invariant (depth-preferred grafting legitimately changes scores). The validation gate is instead: identical mate verdicts on the mate suite, zero crashes/illegal moves under a 1MB-TT collision stress, and the SPRT gauntlets themselves.
- History tables reset per `go` (SearchThread is constructed per search; persistence across moves needs UCI-held search state — an M4 refactor, noted in §SPRT-log).
- qsearch TT probing: M4.

**THE SPRT GATE PROTOCOL (controller-executed; this is the heart of the plan):**
1. Task T1 snapshots the current M2 binary as `tools/bin/baseline-m2`.
2. Each gated task (T3-T7) ends with: implementer commits (with `Bench:` line), reviews pass, then the CONTROLLER (not the implementer) launches:
   `tools/sprt.sh target/release/nebchess tools/bin/baseline-<prev> 10 2>&1 | tee tools/sprt-<feature>.log`
   in a background shell (these run 15 min - 2 hrs; fastchess stops itself at the LLR bound).
3. Verdict handling: fastchess prints `H1 was accepted` (gain confirmed) or `H0 was accepted` (no gain / regression). **H1** → append a row to `docs/sprt-log.md`, run `tools/baseline.sh <feature>` to promote the new binary as the next baseline, proceed. **H0** → STOP the pipeline; this is the spec's stop-and-debug signal — investigate before any further feature lands.
4. Bounds: elo0=0, elo1=10 (early-project gains are large); the log records bounds per run so future tightening is visible.
5. A quick `grep -Ei "loses on time|disconnect|illegal" tools/sprt-<feature>.log` after each run — any hit is a blocker bug regardless of the ELO verdict.

**Environment facts:** 16-core WSL2 laptop (SPRT concurrency 15 ≈ 30-40 games/min at 8+0.08); fastchess 1.8.1 at `tools/bin/fastchess` (sprt.sh flags already fixed); Move Overhead 50ms; current bench 5133563. Prefix cargo commands with `source "$HOME/.cargo/env" && `.

---

## File structure (end state)

```
src/
  search/
    mod.rs        # + StackEntry array, TT probe/store in negamax, killers/history
                  #   updates, PVS move loop, ATTACKER_VALS king fix in MovePicker
    tt.rs         # NEW: Tt, Entry (AtomicU64+AtomicU16), Cluster (align 32),
                  #   probe/store with mate-ply adjust, generation, replacement
  board/moves.rs  # + Move::raw()/from_raw() (TT move packing)
  uci/mod.rs      # Arc<Tt> ownership, Hash option functional, ucinewgame clears
docs/sprt-log.md  # NEW: committed per-feature SPRT results table
tools/baseline.sh # NEW: snapshot release binary as tools/bin/baseline-<name>
tests/search.rs   # + TT validation tests, killer/history behavior tests
```

**Glossary for this plan:**
- *TT (transposition table):* hash table keyed by Zobrist; an entry from an earlier (often deeper) search of the same position short-circuits work. *Bound types:* Exact (searched full window), Lower (failed high — real score ≥ stored), Upper (failed low — real score ≤ stored).
- *Generation/age:* a counter bumped per `go`; replacement prefers stale entries.
- *Mate-ply adjustment:* mate scores are stored relative to the NODE (`score+ply` on store, `-ply` on probe) so "mate in 3 from here" stays true wherever it's probed from.
- *Killers:* the 2 most recent quiet moves that caused a beta cutoff at this ply — cheap, position-independent ordering. *History:* a `[side][from][to]` score bumped by `depth²` on quiet cutoffs.
- *PVS:* search move 1 with the full window; prove the rest inferior with a cheap null-window (`alpha, alpha+1`) search, re-searching only on surprise.
- *SPRT:* sequential test that stops itself once the data proves H1 (gain ≥ elo1) or H0 (gain ≤ elo0) at α=β=0.05.

---

### Task 1: SPRT infrastructure + SearchStack scaffold (no behavior change)

**Files:**
- Create: `tools/baseline.sh`, `docs/sprt-log.md`
- Modify: `src/search/mod.rs`

- [ ] **Step 1.1: `tools/baseline.sh`**

```bash
#!/usr/bin/env bash
# Snapshot the current release binary as an SPRT baseline.
# Usage: tools/baseline.sh <name>   ->  tools/bin/baseline-<name>
set -euo pipefail
cd "$(dirname "$0")"
[ $# -eq 1 ] || { echo "usage: baseline.sh <name>" >&2; exit 2; }
cp ../target/release/nebchess "bin/baseline-$1"
echo "saved baseline-$1: $(bin/baseline-$1 bench 2>/dev/null | tail -1)"
```

- [ ] **Step 1.2: `docs/sprt-log.md`**

```markdown
# SPRT Log

Per-feature self-play results (protocol: tools/sprt.sh — STC 8+0.08, Hash 16,
8moves_v3 book, reversed pairs, alpha=beta=0.05, model=normalized).
H1 accepted = gain >= elo1 confirmed; the feature binary becomes the next baseline.

| date | feature | vs baseline | bounds | games | W-L-D | result | bench |
|------|---------|-------------|--------|-------|-------|--------|-------|
```

- [ ] **Step 1.3: Snapshot the M2 baseline**

```bash
chmod +x tools/baseline.sh
cargo build --release
tools/baseline.sh m2
```

Expected: `saved baseline-m2: Bench: 5133563`.

- [ ] **Step 1.4: SearchStack scaffold.** In `src/search/mod.rs`, add above `SearchThread`:

```rust
/// Per-ply search state (spec §5.1). M3 uses `killers` and `current_move`;
/// `static_eval` (M4: RFP/improving) and `excluded_move` (M6: singular
/// extensions) are reserved so their features don't re-layout the stack.
#[derive(Clone, Copy)]
struct StackEntry {
    static_eval: i32,
    current_move: Move,
    killers: [Move; 2],
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
```

Add the field to `SearchThread` (after `pv: PvTable,`):

```rust
    stack: Box<[StackEntry; MAX_PLY]>,
```

initialize in `new()` (after the `pv:` line):

```rust
            stack: Box::new([StackEntry::EMPTY; MAX_PLY]),
```

and record the move being searched: in BOTH `negamax` and `qsearch`, immediately after each successful `self.pos.make(mv)` / `self.eval.on_make(...)` pair, add:

```rust
            self.stack[ply].current_move = mv;
```

(`current_move` is read by nothing yet — M4's continuation history will; killers are wired in Task 5. `#[allow(dead_code)]` is NOT needed: the fields are written/readable via the struct. If clippy complains about any individual field, add a targeted `#[allow(dead_code)]` ON THE FIELD with a `// M4`/`// M6` comment and report it.)

- [ ] **Step 1.5: Verify zero behavior change**

```bash
cargo test && cargo build --release
./target/release/nebchess bench | tail -1
```

Expected: 93 tests green; bench **exactly 5133563** (the scaffold must not alter search behavior — if the number moved, something is reading the stack).

- [ ] **Step 1.6: Lint + commit (bench unchanged → include the line anyway, it's an engine-code commit)**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add tools/baseline.sh docs/sprt-log.md src/search/mod.rs
git commit -m "feat(search): per-ply SearchStack scaffold + SPRT baseline tooling" -m "Bench: 5133563"
```

---

### Task 2: The transposition table (data structure + wiring; still no search use)

**Files:**
- Create: `src/search/tt.rs`
- Modify: `src/board/moves.rs`, `src/search/mod.rs`, `src/uci/mod.rs`

Spec §5.3 layout: 10 logical bytes per entry (`AtomicU64` data + `AtomicU16` key fragment), 3 entries per 32-byte cluster, depth-preferred replacement with 6-bit generation aging, mate-score ply adjustment on store/probe. Atomics with `Relaxed` ordering everywhere (single-threaded engine; SMP revisits the memory model in M9b — the *types* are SMP-shaped now so M9b doesn't re-layout).

- [ ] **Step 2.1: Move raw-encoding escape hatch.** In `src/board/moves.rs`, inside `impl Move` (after `from`):

```rust
    /// Raw 16-bit encoding (engine-internal: TT storage). Round-trips with
    /// `from_raw`; a raw value from a corrupted/collided TT entry decodes to
    /// SOME move — consumers must validate against generated moves.
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub const fn from_raw(raw: u16) -> Move {
        Move(raw)
    }
```

and one test inside its `mod tests`:

```rust
    #[test]
    fn raw_roundtrip() {
        let mv = Move::new(Square::E1, Square::G1, Move::KING_CASTLE);
        assert_eq!(Move::from_raw(mv.raw()), mv);
        assert_eq!(Move::from_raw(0), Move::NULL);
    }
```

- [ ] **Step 2.2: Write failing TT unit tests** (bottom of new `src/search/tt.rs`)

```rust
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
        tt.store(0xDEAD_BEEF_CAFE_F00D, mv(), 123, EVAL_NONE, 7, Bound::Exact, 0);
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
        assert!(tt.probe(k(4), 0).is_some(), "stale depth lost to fresh entry");
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
            tt.store(key, mv(), (i % 2000) as i32 - 1000, EVAL_NONE, (i % 32) as i32, Bound::Lower, (i % 64) as usize);
            let _ = tt.probe(next(), (i % 64) as usize); // mostly misses
        }
    }
}
```

- [ ] **Step 2.3: Implement** (top of `tt.rs`)

```rust
//! Transposition table (spec §5.3): 32-byte clusters of three 10-byte
//! entries (AtomicU64 data + AtomicU16 key fragment), depth-preferred
//! replacement with 6-bit generation aging, mate-score ply adjustment.
//! All atomics Relaxed: the engine is single-threaded until M9b; the types
//! are SMP-shaped so Lazy SMP doesn't re-layout (XOR validation lands then).

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
struct Entry {
    data: AtomicU64,
    key16: AtomicU16,
}

#[repr(C, align(32))]
pub(crate) struct Cluster {
    entries: [Entry; 3],
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
                entries: [
                    Entry { data: AtomicU64::new(0), key16: AtomicU16::new(0) },
                    Entry { data: AtomicU64::new(0), key16: AtomicU16::new(0) },
                    Entry { data: AtomicU64::new(0), key16: AtomicU16::new(0) },
                ],
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
            for e in &c.entries {
                e.data.store(0, Ordering::Relaxed);
                e.key16.store(0, Ordering::Relaxed);
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
        for e in &cluster.entries {
            if e.key16.load(Ordering::Relaxed) == frag {
                let d = e.data.load(Ordering::Relaxed);
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
        for (i, e) in cluster.entries.iter().enumerate() {
            let d = e.data.load(Ordering::Relaxed);
            if e.key16.load(Ordering::Relaxed) == frag && d >> 62 != 0 {
                victim = i;
                victim_quality = i32::MIN; // always replace same-key
                break;
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

        let e = &cluster.entries[victim];
        e.key16.store(frag, Ordering::Relaxed);
        e.data.store(
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
```

- [ ] **Step 2.4: Wire it (still probed nowhere).**

`src/search/mod.rs`: add `pub mod tt;` at the top (alongside `pub mod bench;`/`pub mod limits;`), then:

```rust
use crate::search::tt::Tt;
use std::sync::Arc;
```

(`Arc` is already imported — extend the existing use.) Add the field to `SearchThread` (after `stack:`):

```rust
    tt: Arc<Tt>,
```

initialize in `new()` — default 16 MB so bench/tests are deterministic at the advertised default:

```rust
            tt: Arc::new(Tt::new(16)),
```

add the setter:

```rust
    pub fn set_tt(&mut self, tt: Arc<Tt>) {
        self.tt = tt;
    }
```

and in `iterate()`, right after the stop-flag/limits setup at entry:

```rust
        self.tt.new_search();
```

`src/uci/mod.rs`: add `use crate::search::tt::Tt;`, give `Uci` a `tt: Arc<Tt>` field initialized `Arc::new(Tt::new(16))`, then:
- `"ucinewgame"` arm: replace the `// M3: clear...` comment with `self.tt.clear();`
- `cmd_setoption`: extend the match with a functional Hash arm:

```rust
            ("Hash", Some(v)) => {
                if let Ok(mb) = v.parse::<usize>() {
                    self.tt = Arc::new(Tt::new(mb.clamp(1, 4096)));
                }
            }
```

(GUIs set options while idle; `setoption` already runs on the main thread between searches — no join needed beyond what the protocol guarantees, but add `self.stop_and_join();` as the first line of the Hash arm for safety.)
- `cmd_go`: after `st.set_overhead_ms(...)`, add `st.set_tt(Arc::clone(&self.tt));`

- [ ] **Step 2.5: Run everything — bench MUST be unchanged**

```bash
cargo test && cargo build --release && ./target/release/nebchess bench | tail -1
```

Expected: all tests green (93 + 1 raw_roundtrip + 8 tt = 102); bench **exactly 5133563** (the table exists but nothing probes it).

- [ ] **Step 2.6: Lint + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/board/moves.rs src/search/ src/uci/mod.rs
git commit -m "feat(search): transposition table data structure and wiring" -m "Bench: 5133563"
```

---

### Task 3: TT cutoffs + store — SPRT GATE #1

**Files:**
- Modify: `src/search/mod.rs`
- Test: `tests/search.rs`

- [ ] **Step 3.1: Probe + cutoff in negamax.** In `negamax`, immediately after the `if ply >= MAX_PLY - 1 { ... }` guard and before the MovePicker construction, insert:

```rust
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
```

with `use crate::search::tt::{self, Tt};` adjusted at the top (the module is already imported; extend to bring `tt::` paths in as needed — match the file's existing import style).

KNOWN, ACCEPTED CAVEAT (note in a code comment): TT grafting can interact with path-dependent draw scores (repetition/50-move). The draw checks run BEFORE the probe, which bounds the damage; engines at this level universally accept the residue.

- [ ] **Step 3.2: Track the best move + store on exit.** In the move loop, add `let mut best_move = Move::NULL;` next to `let mut best = -INF;`, set it where alpha is raised:

```rust
                if score > alpha {
                    alpha = score;
                    best_move = mv;
                    self.pv.update(ply, mv);
```

and replace the final `best` return (AFTER the `legal == 0` early-return block, which stays un-stored) with:

```rust
        let bound = if best >= beta {
            tt::Bound::Lower // the stored move is the cutoff move
        } else if best_move != Move::NULL {
            tt::Bound::Exact
        } else {
            tt::Bound::Upper // failed low: no move raised alpha
        };
        self.tt
            .store(self.pos.key(), best_move, best, tt::EVAL_NONE, depth, bound, ply);
        best
```

(All stopped-paths return 0 from INSIDE the loop, so a truncated node never stores.)

- [ ] **Step 3.3: TT behavior tests** (append to `tests/search.rs`)

```rust
#[test]
fn tt_makes_research_cheap_and_stable() {
    let mut st = searcher("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1");
    let (best1, score1) = st.search_to_depth(6);
    let nodes_first = st.nodes;
    let (best2, score2) = st.search_to_depth(6); // same thread: warm TT
    let nodes_second = st.nodes - nodes_first;
    assert_eq!(best1, best2, "warm-TT re-search must agree");
    assert_eq!(score1, score2);
    assert!(
        nodes_second * 4 < nodes_first,
        "warm TT should slash nodes: {nodes_first} then {nodes_second}"
    );
}

#[test]
fn mate_scores_survive_tt_round_trips() {
    let mut st = searcher("k7/8/2K5/8/8/8/8/7R w - - 0 1");
    let (_b1, s1) = st.search_to_depth(4);
    assert_eq!(s1, MATE - 3);
    let (b2, s2) = st.search_to_depth(4); // warm TT: ply-adjust must hold
    assert_eq!(s2, MATE - 3, "mate distance corrupted through the TT");
    assert_eq!(b2.unwrap().to_string(), "c6b6");
}

#[test]
fn tiny_tt_collision_storm_is_sound() {
    // 1MB table + deep-ish search = heavy collisions; the gate is soundness
    // (no panics, legal move, sane score), not strength
    let mut st = searcher("r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10");
    st.set_tt(std::sync::Arc::new(nebchess::search::tt::Tt::new(1)));
    let (best, score) = st.search_to_depth(7);
    assert!(best.is_some());
    assert!(score.abs() < 1000, "quiet position, sane score, got {score}");
}
```

- [ ] **Step 3.4: Run + measure the new bench**

```bash
cargo test
cargo build --release && ./target/release/nebchess bench | tail -1   # run twice, must match
```

Expected: all green; bench DROPS substantially (TT cutoffs prune re-searched subtrees) and is reproducible. Record the new number.

- [ ] **Step 3.5: Lint + commit (new bench line!)**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/mod.rs tests/search.rs
git commit -m "feat(search): transposition table cutoffs and stores" -m "Bench: <new number>"
```

- [ ] **Step 3.6 (CONTROLLER): SPRT GATE #1**

```bash
tools/sprt.sh "$PWD/target/release/nebchess" "$PWD/tools/bin/baseline-m2" 10 2>&1 | tee tools/sprt-tt-cuts.log
```

Run in a background shell (expect ~15-60 min; Rustic measured +42 for TT cuts alone — bounds [0,10] should accept H1 quickly). On `H1 was accepted`: scan the log for forfeits (`grep -Ei "loses on time|disconnect|illegal" tools/sprt-tt-cuts.log` → must be empty), append the result row to `docs/sprt-log.md`, commit it, and run `tools/baseline.sh tt-cuts`. On `H0 was accepted`: STOP THE PIPELINE — debug before Task 4.

---

### Task 4: TT-move ordering + MVV-LVA king fix — SPRT GATE #2

**Files:**
- Modify: `src/search/mod.rs`
- Test: `tests/search.rs`

- [ ] **Step 4.1: Extend MovePicker.** Change its constructor signature and scoring:

```rust
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
    fn new(pos: &Position, tt_move: Move) -> MovePicker {
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
            } else {
                0
            };
        }
        MovePicker { moves, scores, cur: 0 }
    }
    // next() unchanged
}
```

Call sites: in `negamax`, `let tt_move = tt_hit.as_ref().map_or(Move::NULL, |h| h.mv);` then `MovePicker::new(&self.pos, tt_move)`. In `qsearch` (no TT probe in M3): `MovePicker::new(&self.pos, Move::NULL)`. In `fifty_move_score` and `find_first_legal` paths nothing changes (they don't use the picker).

- [ ] **Step 4.2: Validation tests** (append to `tests/search.rs`)

```rust
#[test]
fn junk_tt_move_is_ignored_not_played() {
    // poison the TT entry for the root position with a junk move encoding,
    // then search: the engine must neither panic nor emit an illegal move
    let fen = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    let mut st = searcher(fen);
    let tt = std::sync::Arc::new(nebchess::search::tt::Tt::new(1));
    let key = nebchess::board::Position::from_fen(fen).unwrap().key();
    // raw 0xFFFF decodes to h8->h8 promo-capture nonsense: never generated
    tt.store(key, nebchess::board::Move::from_raw(0xFFFF), 500, nebchess::search::tt::EVAL_NONE, 12, nebchess::search::tt::Bound::Lower, 0);
    st.set_tt(tt);
    let (best, _) = st.search_to_depth(4);
    let pos = nebchess::board::Position::from_fen(fen).unwrap();
    assert!(
        nebchess::board::movegen::find_uci_move(&pos, &best.unwrap().to_string()).is_some(),
        "junk TT move leaked into play"
    );
}

#[test]
fn tt_ordering_reduces_nodes() {
    // search depth 6 cold, then depth 7: the depth-6 TT moves should steer
    // depth 7 well below a cold depth-7 search
    let fen = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";
    let mut warm = searcher(fen);
    warm.search_to_depth(6);
    let nodes_before_7 = warm.nodes;
    warm.search_to_depth(7);
    let warm_7 = warm.nodes - nodes_before_7;
    let mut cold = searcher(fen);
    cold.search_to_depth(7);
    assert!(
        warm_7 < cold.nodes,
        "TT-move ordering should beat cold search: warm {warm_7} vs cold {}",
        cold.nodes
    );
}
```

- [ ] **Step 4.3: Run + bench (twice, identical) + commit**

```bash
cargo test
cargo build --release && ./target/release/nebchess bench | tail -1
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/mod.rs tests/search.rs
git commit -m "feat(search): TT-move ordering and MVV-LVA king attacker fix" -m "Bench: <new number>"
```

- [ ] **Step 4.4 (CONTROLLER): SPRT GATE #2**

```bash
tools/sprt.sh "$PWD/target/release/nebchess" "$PWD/tools/bin/baseline-tt-cuts" 10 2>&1 | tee tools/sprt-tt-ordering.log
```

(Rustic measured +103 for TT-move ordering — this should be the fastest acceptance of the milestone.) H1 → forfeit-scan, log row, `tools/baseline.sh tt-ordering`. H0 → STOP.

---

### Task 5: Killer moves — SPRT GATE #3

**Files:**
- Modify: `src/search/mod.rs`

- [ ] **Step 5.1: Picker tier.** Extend `MovePicker::new(pos, tt_move, killers: [Move; 2])`; scoring chain gains, between the capture arm and the final `0`:

```rust
            } else if mv == killers[0] {
                900_000
            } else if mv == killers[1] {
                899_999
```

(Killers are quiets by construction — stored only on quiet cutoffs — and move equality includes the flag, so they can never shadow a capture.) Call sites: `negamax` reads `let killers = self.stack[ply].killers;` BEFORE constructing the picker and passes it; `qsearch` passes `[Move::NULL; 2]`.

- [ ] **Step 5.2: Update on quiet beta cutoff.** Inside negamax's `if alpha >= beta {` block, before `break`:

```rust
                        if !mv.is_capture() {
                            let k = &mut self.stack[ply].killers;
                            if k[0] != mv {
                                k[1] = k[0];
                                k[0] = mv;
                            }
                        }
```

- [ ] **Step 5.3: Picker-order unit test.** MovePicker is private — test from inside the module. Add to `src/search/mod.rs` (a new `#[cfg(test)] mod tests` at the bottom if none exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{movegen::find_uci_move, Position};

    #[test]
    fn picker_yields_ordering_tiers() {
        // kiwipete: captures + plenty of quiets
        let pos = Position::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let tt_move = find_uci_move(&pos, "a2a3").unwrap(); // arbitrary quiet as TT move
        let k0 = find_uci_move(&pos, "a2a4").unwrap();
        let k1 = find_uci_move(&pos, "g2g3").unwrap();
        let mut picker = MovePicker::new(&pos, tt_move, [k0, k1]);
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
}
```

- [ ] **Step 5.4: Run + bench (twice) + commit**

```bash
cargo test && cargo build --release && ./target/release/nebchess bench | tail -1
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/mod.rs
git commit -m "feat(search): killer move ordering" -m "Bench: <new number>"
```

- [ ] **Step 5.5 (CONTROLLER): SPRT GATE #3** — `tools/sprt.sh "$PWD/target/release/nebchess" "$PWD/tools/bin/baseline-tt-ordering" 10 2>&1 | tee tools/sprt-killers.log` → H1: forfeit-scan, log row, `tools/baseline.sh killers`. H0: STOP.

---

### Task 6: History heuristic — SPRT GATE #4

**Files:**
- Modify: `src/search/mod.rs`

- [ ] **Step 6.1: The table.** Add to `SearchThread`:

```rust
    history: Box<HistoryTable>,
```

with, near `StackEntry`:

```rust
/// Butterfly history: [side][from][to], bumped depth^2 on quiet beta cutoffs.
/// Fresh per `go` (SearchThread is per-search; cross-move persistence is an
/// M4 refactor — recorded in the plan header).
type HistoryTable = [[[i32; 64]; 64]; 2];
```

init in `new()`: `history: Box::new([[[0; 64]; 64]; 2]),`

- [ ] **Step 6.2: Update + scoring.** In the quiet-beta-cutoff block (next to the killer update):

```rust
                            let h = &mut self.history[self.pos.stm().index()]
                                [mv.from().index()][mv.to().index()];
                            *h = (*h + depth * depth).min(799_999);
```

`MovePicker::new(pos, tt_move, killers, history: &HistoryTable, stm: Color)` — the final quiet arm becomes:

```rust
            } else {
                history[stm.index()][mv.from().index()][mv.to().index()]
            }
```

Call sites: negamax passes `&self.history, self.pos.stm()` (read the stm before the picker borrow if the borrow checker objects — `let stm = self.pos.stm();`); qsearch passes its own `&self.history, self.pos.stm()` (harmless: quiets are skipped there anyway except evasions, where history ordering is a free bonus).

NOTE the borrow shape: `MovePicker::new(&self.pos, tt_move, killers, &self.history, stm)` takes two immutable borrows of `self` fields — legal. The picker must NOT hold them: it copies scores at construction (it already does — scores array), so the loop body's `&mut self` calls stay legal. If the compiler objects anyway, construct the picker in a narrow scope `let mut picker = { ... }` and report.

- [ ] **Step 6.3: Unit test** (append inside the mod.rs tests module)

```rust
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
```

(Adjust the Step 5.3 test's `MovePicker::new` calls to the new 5-arg signature with a zeroed history — keep both tests compiling.)

- [ ] **Step 6.4: Run + bench (twice) + commit**

```bash
cargo test && cargo build --release && ./target/release/nebchess bench | tail -1
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/mod.rs
git commit -m "feat(search): butterfly history heuristic for quiet ordering" -m "Bench: <new number>"
```

- [ ] **Step 6.5 (CONTROLLER): SPRT GATE #4** — vs `baseline-killers`, log `tools/sprt-history.log`, promote `tools/baseline.sh history`. H0: STOP.

---

### Task 7: Principal Variation Search — SPRT GATE #5

**Files:**
- Modify: `src/search/mod.rs`
- Test: `tests/search.rs`

- [ ] **Step 7.1: Convert the negamax move loop.** Replace the plain recursion with first-move-full-window + null-window scout:

```rust
        let mut first = true;
        while let Some(mv) = picker.next() {
            if !self.pos.make(mv) {
                continue;
            }
            self.eval.on_make(mv, &self.pos);
            self.stack[ply].current_move = mv;
            legal += 1;
            let score = if first {
                -self.negamax(depth - 1, -beta, -alpha, ply + 1)
            } else {
                // scout: prove the move can't beat alpha with a null window
                let zw = -self.negamax(depth - 1, -alpha - 1, -alpha, ply + 1);
                if zw > alpha && zw < beta {
                    // surprise: re-search with the real window
                    -self.negamax(depth - 1, -beta, -alpha, ply + 1)
                } else {
                    zw
                }
            };
            first = false;
            self.pos.unmake();
            self.eval.on_unmake(mv, &self.pos);
            // ... (stopped check, best/alpha/beta updates unchanged)
```

(When `beta == alpha + 1` the parent window already IS null — the `zw < beta` condition then prevents a pointless identical re-search. Fail-soft semantics are preserved: a scout fail-high returns `zw >= beta`, a valid lower bound.)

- [ ] **Step 7.2: Equivalence tests** (append to `tests/search.rs` — PVS must change EFFORT, never RESULTS)

```rust
#[test]
fn pvs_preserves_mate_distances_and_pv() {
    let mut st = searcher("k7/8/2K5/8/8/8/8/7R w - - 0 1");
    let (best, score) = st.search_to_depth(6); // deeper than the mate
    assert_eq!(score, MATE - 3, "PVS altered a proven mate score");
    assert_eq!(best.unwrap().to_string(), "c6b6");
}

#[test]
fn pvs_scores_match_across_warm_research() {
    let mut st = searcher("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    let (b1, s1) = st.search_to_depth(7);
    let (b2, s2) = st.search_to_depth(7);
    assert_eq!((b1, s1), (b2, s2), "PVS+TT re-search instability");
}
```

- [ ] **Step 7.3: Run full suite + bench (twice) + commit**

```bash
cargo test && cargo build --release && ./target/release/nebchess bench | tail -1
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/search/mod.rs tests/search.rs
git commit -m "feat(search): principal variation search" -m "Bench: <new number>"
```

- [ ] **Step 7.4 (CONTROLLER): SPRT GATE #5** — vs `baseline-history`, log `tools/sprt-pvs.log`, promote `tools/baseline.sh pvs`. H0: STOP.

---

### Task 8: M3 wrap-up

**Files:**
- Modify: `README.md`, `Cargo.toml`, `docs/sprt-log.md` (final check)

- [ ] **Step 8.1: Forfeit gauntlet rerun** (TT changed the timing profile): `tools/forfeit-gauntlet.sh 100` → 200 games, ZERO forfeits (blocker otherwise).
- [ ] **Step 8.2 (CONTROLLER, informational): strength check** — 10 games vs Stockfish 18 `UCI_LimitStrength=true UCI_Elo=1800` at 8+0.08 via fastchess; record the score in the final report (NOT a gate).
- [ ] **Step 8.3: Docs + version.** README: tick M3, add `- [ ] M4: search pruning (null move, LMR, aspiration) + PST tuning pipeline`; Cargo.toml 0.3.0; verify docs/sprt-log.md has all 5 rows.
- [ ] **Step 8.4: Full local gate** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo test --release --test perft -- --ignored`.
- [ ] **Step 8.5: Commit, push, CI green** — `git commit -m "docs: mark M3 complete, bump to 0.3.0"` (no Bench line: docs/version only), push, `gh run watch --exit-status ...`.

---

## Plan self-review notes

- **Spec coverage:** §5.3 TT (T2-T3; deviations: parallel clear→M9b, prefetch→NPS milestone, eval field sentinel→M4, low-16 fragment NOTE: the spec's "16-bit key fragment" is satisfied; the low-vs-high choice is forced by mulhi indexing); §5.1 stack (T1; static_eval/excluded reserved); §12 M3 row order exactly (cuts→ordering→killers→history→PVS, per-feature SPRT); §10.3 protocol (frozen sprt.sh, [0,10] bounds logged); TT validation gates adapted per deviations header.
- **Type consistency:** MovePicker signature evolves T4(+tt_move) → T5(+killers) → T6(+history,+stm) — each task updates ALL call sites and the prior tests; `tt::Bound`/`EVAL_NONE`/`TtHit` names consistent T2-T4; `HistoryTable` defined T6 before use; baseline names chain m2→tt-cuts→tt-ordering→killers→history→pvs.
- **Placeholder scan:** `<new number>` bench placeholders are measured-at-execution values with explicit instructions (established convention); no TBDs.
- **Process note:** five controller-run SPRT gates make this plan WALL-CLOCK HEAVY (~2-6 hours of match time total). Tasks are strictly sequential (each baseline chains). The controller runs gates in background shells and proceeds with reviews/log-keeping between.

## Execution Handoff

Plan complete. Execute with superpowers:subagent-driven-development (established pipeline) — implementer per task, two-stage review, CONTROLLER owns every SPRT gate step.



