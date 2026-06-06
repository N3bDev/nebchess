//! Texel tuner v2: manifest-driven, trace-fed, phase-weighted, parallel (M5 T6).
//!   cargo build --release && ./target/release/tune tools/data/quiet-labeled.epd \
//!     > /tmp/eval_params_new.rs && cp /tmp/eval_params_new.rs src/eval/eval_params.rs
//! NEVER `cargo run ... > src/eval/eval_params.rs`: the shell truncates the params
//! file before cargo compiles the library that includes it (build fails, params lost).
//! Optional args: [epochs=400] [lr=0.05] [limit=0(all)] [mode: fit-k|pin-material]
//!
//! Param vector: [mg_bank | eg_bank], each TOTAL_PAIRS long. A traced
//! feature (idx, sign) at a position with phase ph contributes:
//!   d(eval)/d(mg[idx]) = sign * ph/24,  d(eval)/d(eg[idx]) = sign * (24-ph)/24.
//!
//! Experiment modes (step 6.5 big3 investigation; opt-in via a 5th CLI arg):
//!   fit-k         — re-enable the coarse-to-fine K line search (fit on the
//!                   warm-start params before the epoch loop) and REPORT the
//!                   fitted K. This is the DELIBERATE re-anchoring act per the
//!                   K-freeze law: any shipping candidate from this mode needs
//!                   margin revalidation + a fresh tactics canary. The default
//!                   path stays frozen at K_FROZEN.
//!   pin-material  — K stays frozen; after EVERY Adam step re-pin the ENTIRE
//!                   MATERIAL row (all 6 pairs, BOTH banks) to the compiled-in
//!                   PARAMS values (the T5 material), not just P_mg=100.
//! The two modes are mutually exclusive. With no mode given the behavior is
//! byte-identical to the default path (the determinism guarantee below holds).
//!
//! Parallelism & determinism (std::thread::scope, no external crates):
//! The per-sample gradient and MSE loops — the dominant cost — are parallel;
//! the Adam update stays serial (O(params), trivial). Determinism is REQUIRED:
//! the output must be byte-identical regardless of core count. Floating-point
//! addition is non-associative, so the reduction order must be FIXED. We do that
//! by partitioning `train` into a CONSTANT `NUM_CHUNKS` (16) contiguous chunks
//! — a count independent of the machine — having a pool of worker threads claim
//! chunks via an atomic cursor and return their `(chunk_idx, partial)` results,
//! then placing each partial by its chunk index and reducing `[0..NUM_CHUNKS]` in
//! chunk-index order on the main thread. Within a chunk the sample order is fixed
//! (contiguous slice), and the cross-chunk reduce order is fixed (0,1,2,...), so
//! the total sum is identical for any thread count or scheduling. Verified: two
//! reruns with `<data> 20 0.05 50000` produce byte-identical eval_params output.

use nebchess::board::Position;
use nebchess::eval::hce::{eval_terms, phase};
use nebchess::eval::manifest::{TERMS, TOTAL_PAIRS};
use nebchess::eval::trace::CollectingTracer;
use std::sync::atomic::{AtomicUsize, Ordering};

const N: usize = TOTAL_PAIRS;

/// Fixed partition count for the parallel gradient/MSE reductions. CONSTANT and
/// independent of the core count — that is what makes the float reduction order
/// (and therefore the tuner's output) deterministic across machines. Do NOT tie
/// this to `available_parallelism()`.
const NUM_CHUNKS: usize = 16;

/// Contiguous chunk boundaries that partition `0..len` into `NUM_CHUNKS` pieces
/// (the last chunks absorb the remainder). Order is fixed → reduction is fixed.
fn chunk_bounds(len: usize) -> Vec<(usize, usize)> {
    let base = len / NUM_CHUNKS;
    let rem = len % NUM_CHUNKS;
    let mut bounds = Vec::with_capacity(NUM_CHUNKS);
    let mut start = 0;
    for c in 0..NUM_CHUNKS {
        // Spread the remainder over the first `rem` chunks for balance; the
        // exact split is irrelevant to determinism — only its fixedness is.
        let this = base + usize::from(c < rem);
        bounds.push((start, start + this));
        start += this;
    }
    bounds
}

/// Worker-thread count: take it from the machine but cap at 14 (leave headroom on
/// the 16-core box). Thread count affects ONLY speed, never the result — the
/// reduction is over `NUM_CHUNKS` fixed chunks in fixed order regardless.
fn num_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 14)
}

/// Sigmoid scale, fitted once at the tapered foundation (M5 T1) and frozen.
/// See the comment at its use site before changing this.
const K_FROZEN: f64 = 1.520;

/// Opt-in experiment mode selected by the optional 5th CLI arg. `Default` is
/// the production path (frozen K, P_mg=100 anchor) and is byte-identical to the
/// pre-experiment tuner. The two named modes are mutually exclusive — see the
/// module-level doc-comment for the step-6.5 rationale.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Default,
    /// Re-enable the coarse-to-fine K line search (deliberate re-anchoring).
    FitK,
    /// Frozen K, but re-pin the whole MATERIAL row to compiled-in PARAMS.
    PinMaterial,
}

struct Sample {
    features: Vec<(u16, i8)>,
    phase: i32,
    result: f64,
}

fn extract(pos: &Position, result: f64) -> Sample {
    let mut tr = CollectingTracer::default();
    let _ = eval_terms(pos, &mut tr); // the REAL eval produces the features
    Sample {
        features: tr.features,
        phase: phase(pos),
        result,
    }
}

/// Parse one dataset line into (FEN, white-relative result), sniffing the format
/// per line so a single tuner handles both corpora:
///   - zurichess quiet-labeled: `<placement> <stm> <castling> <ep> c9 "1-0";`
///     (no move counters; result encoded `1-0`/`0-1`/`1/2-1/2`).
///   - lichess-big3-resolved: `<full fen> [<0.0|0.5|1.0>]` (bracketed white-score;
///     the FEN carries its own counters).
///
/// Returns None for blank/uncommented/malformed lines (skipped by the loader).
fn parse_line(line: &str) -> Option<(String, f64)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if let Some((fen4, tail)) = line.split_once(" c9 ") {
        // zurichess: counters absent, append a synthetic halfmove/fullmove.
        let result = if tail.contains("1-0") {
            1.0
        } else if tail.contains("0-1") {
            0.0
        } else {
            0.5
        };
        return Some((format!("{fen4} 0 1"), result));
    }
    if let Some((fen, tail)) = line.split_once('[') {
        // big3: bracketed white-relative score, full FEN already present.
        let score = tail.trim_end_matches(']').trim();
        let result = score.parse::<f64>().ok()?;
        return Some((fen.trim().to_string(), result));
    }
    None
}

fn eval_sample(s: &Sample, p: &[f64]) -> f64 {
    // p layout: [0..N) = mg, [N..2N) = eg
    let (wmg, weg) = (s.phase as f64 / 24.0, (24 - s.phase) as f64 / 24.0);
    let mut e = 0.0;
    for &(idx, sign) in &s.features {
        let i = idx as usize;
        e += sign as f64 * (p[i] * wmg + p[N + i] * weg);
    }
    e
}

fn sigmoid(k: f64, e: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-k * e / 400.0))
}

/// Sum of squared errors over one contiguous slice, summed in slice order.
fn sse_chunk(samples: &[Sample], p: &[f64], k: f64) -> f64 {
    let mut acc = 0.0;
    for s in samples {
        let d = s.result - sigmoid(k, eval_sample(s, p));
        acc += d * d;
    }
    acc
}

/// Parallel MSE. Deterministic across core counts: `train` is split into a fixed
/// `NUM_CHUNKS` contiguous chunks; a pool of workers claims chunk indices off an
/// atomic cursor, each returning `(chunk_idx, partial_sse)` pairs; the main thread
/// places them by index and reduces in chunk-index order. (See the module-level
/// determinism note.) Returning per-chunk results keeps the code unsafe-free.
fn mse(samples: &[Sample], p: &[f64], k: f64) -> f64 {
    let bounds = chunk_bounds(samples.len());
    let cursor = AtomicUsize::new(0);
    let threads = num_threads().min(NUM_CHUNKS);
    let mut partials = vec![0f64; NUM_CHUNKS];
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let cursor = &cursor;
                let bounds = &bounds;
                scope.spawn(move || {
                    let mut out: Vec<(usize, f64)> = Vec::new();
                    loop {
                        let c = cursor.fetch_add(1, Ordering::Relaxed);
                        if c >= NUM_CHUNKS {
                            break;
                        }
                        let (lo, hi) = bounds[c];
                        out.push((c, sse_chunk(&samples[lo..hi], p, k)));
                    }
                    out
                })
            })
            .collect();
        for h in handles {
            for (c, v) in h.join().unwrap() {
                partials[c] = v;
            }
        }
    });
    // Reduce in chunk-index order — fixed regardless of which worker did which.
    let mut total = 0.0;
    for v in &partials {
        total += *v;
    }
    total / samples.len() as f64
}

/// Accumulate the (unnormalized) `2*N` gradient over one contiguous slice, in
/// slice order. `scale = k*ln(10)/400`. p layout: [0..N)=mg, [N..2N)=eg.
fn grad_chunk(samples: &[Sample], p: &[f64], k: f64, scale: f64) -> Vec<f64> {
    let mut grad = vec![0f64; 2 * N];
    for s in samples {
        let ev = eval_sample(s, p);
        let pr = sigmoid(k, ev);
        // d(MSE)/d(param) = -2 (r - p) p(1-p) scale * feature_gradient
        let common = -2.0 * (s.result - pr) * pr * (1.0 - pr) * scale;
        let wmg = s.phase as f64 / 24.0;
        let weg = (24 - s.phase) as f64 / 24.0;
        for &(idx, sign) in &s.features {
            let i = idx as usize;
            grad[i] += common * sign as f64 * wmg; // mg gradient
            grad[N + i] += common * sign as f64 * weg; // eg gradient
        }
    }
    grad
}

/// Parallel full-batch gradient. Deterministic across core counts: the SAME fixed
/// `NUM_CHUNKS` partition as `mse`; workers claim chunks off an atomic cursor and
/// return `(chunk_idx, partial_grad)`; the main thread sums the per-chunk gradient
/// vectors in chunk-index order. Float addition is non-associative, so a FIXED
/// reduction order is what guarantees identical output regardless of thread count.
fn gradient(samples: &[Sample], p: &[f64], k: f64, scale: f64) -> Vec<f64> {
    let bounds = chunk_bounds(samples.len());
    let cursor = AtomicUsize::new(0);
    let threads = num_threads().min(NUM_CHUNKS);
    let mut chunk_grads: Vec<Option<Vec<f64>>> = (0..NUM_CHUNKS).map(|_| None).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let cursor = &cursor;
                let bounds = &bounds;
                scope.spawn(move || {
                    let mut out: Vec<(usize, Vec<f64>)> = Vec::new();
                    loop {
                        let c = cursor.fetch_add(1, Ordering::Relaxed);
                        if c >= NUM_CHUNKS {
                            break;
                        }
                        let (lo, hi) = bounds[c];
                        out.push((c, grad_chunk(&samples[lo..hi], p, k, scale)));
                    }
                    out
                })
            })
            .collect();
        for h in handles {
            for (c, g) in h.join().unwrap() {
                chunk_grads[c] = Some(g);
            }
        }
    });
    // Reduce per-chunk gradients in chunk-index order.
    let mut grad = vec![0f64; 2 * N];
    for cg in chunk_grads.into_iter().flatten() {
        for i in 0..2 * N {
            grad[i] += cg[i];
        }
    }
    grad
}

/// Warm-start: read PARAMS pairs -> p[i]=mg, p[N+i]=eg.
fn warm_start() -> Vec<f64> {
    use nebchess::eval::eval_params::PARAMS;
    let mut p = vec![0f64; 2 * N];
    for i in 0..N {
        p[i] = PARAMS[i].0 as f64; // mg bank
        p[N + i] = PARAMS[i].1 as f64; // eg bank
    }
    p
}

/// Re-pin anchors after an Adam step.
///   - `Mode::Default` / `Mode::FitK`: pin only pawn mg = 100 (immovable) —
///     byte-identical to the production tuner.
///   - `Mode::PinMaterial`: additionally re-pin the ENTIRE MATERIAL row (all 6
///     pairs, BOTH mg and eg banks) to the compiled-in PARAMS values (the T5
///     material). This holds the material scale fixed so the big3 corpus tunes
///     only the non-material knowledge (step 6.5 hypothesis 2). P_mg=100 falls
///     out of this since PARAMS[MATERIAL].0 == 100.
fn repin(p: &mut [f64], mode: Mode) {
    use nebchess::eval::eval_params::PARAMS;
    use nebchess::eval::manifest;
    match mode {
        Mode::Default | Mode::FitK => {
            p[manifest::MATERIAL] = 100.0; // P mg anchor
        }
        Mode::PinMaterial => {
            // MATERIAL is the first term: 6 pairs at flat offset `MATERIAL`.
            for i in 0..6 {
                p[manifest::MATERIAL + i] = PARAMS[manifest::MATERIAL + i].0 as f64; // mg bank
                p[N + manifest::MATERIAL + i] = PARAMS[manifest::MATERIAL + i].1 as f64;
                // eg bank
            }
        }
    }
}

fn emit(p: &[f64], k: f64, train_mse: f64, val_mse: f64) {
    let today = "2026-06-06";
    println!("//! GENERATED by `cargo run --release --bin tune`. Do not edit.");
    println!(
        "//! (retuned {today}; K = {k:.3}, train MSE = {train_mse:.6}, val MSE = {val_mse:.6})"
    );
    println!("//! Layout: manifest order, one `(mg, eg)` pair per parameter.");
    println!("//! PST layout: rank-8 row first; white reads PST[sq ^ 56], black PST[sq].");
    println!();
    println!("pub static PARAMS: [(i32, i32); crate::eval::manifest::TOTAL_PAIRS] = [");

    // One pair per line, exactly as rustfmt formats it — keeps the generated
    // file `cargo fmt --check`-clean across retunes.
    let r = |v: f64| v.round() as i32;
    let mut off = 0usize;
    for term in TERMS {
        match term.name {
            "MATERIAL" => println!("    // MATERIAL: P N B R Q K"),
            name => println!("    // {name} ({} pairs)", term.len),
        }
        for i in 0..term.len {
            let (mg, eg) = (r(p[off + i]), r(p[N + off + i]));
            println!("    ({mg}, {eg}),");
        }
        off += term.len;
    }
    println!("];");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .expect("usage: tune <epd> [epochs] [lr] [limit] [fit-k|pin-material]");
    let epochs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(400);
    let lr: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.05);
    let limit: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);

    // Optional 5th arg selects an opt-in experiment mode. Absent => Default
    // (byte-identical to the production tuner). The single-arg encoding makes
    // fit-k and pin-material mutually exclusive by construction; we still reject
    // any unrecognized value rather than silently falling back.
    let mode = match args.get(4).map(String::as_str) {
        None => Mode::Default,
        Some("fit-k") => Mode::FitK,
        Some("pin-material") => Mode::PinMaterial,
        Some(other) => panic!("unknown mode {other:?} (expected fit-k or pin-material)"),
    };

    let data = std::fs::read_to_string(path).expect("read epd");
    let mut samples = Vec::new();
    for line in data.lines() {
        // Sniff zurichess `c9 "result"` vs big3 `[white-score]` per line.
        let Some((fen, result)) = parse_line(line) else {
            continue;
        };
        let Ok(pos) = Position::from_fen(&fen) else {
            continue;
        };
        samples.push(extract(&pos, result));
        if limit > 0 && samples.len() >= limit {
            break;
        }
    }
    eprintln!("loaded {} samples", samples.len());
    let split = samples.len() * 9 / 10;
    let (train, val) = samples.split_at(split);

    let mut p = warm_start();
    repin(&mut p, mode);

    // K is FROZEN at the scale fitted for the tapered foundation (M5 T1).
    // Do NOT refit per run: MSE only sees the product K*eval, so refitting K
    // lets Adam slide the whole param vector along that degeneracy — T2's
    // refit (1.520 -> 1.377) inflated every piece value ~10% against the
    // search's fixed-centipawn margins (futility/RFP/aspiration) and the
    // P_mg=100 anchor, and WAC dropped 267 -> 258 (tactics-log 2026-06-06).
    // Re-anchoring K is a deliberate act: it requires re-validating search
    // margins and a fresh tactics canary.
    //
    // `fit-k` (step 6.5 hypothesis 1) DELIBERATELY re-anchors K via the
    // pre-c58d05d coarse-to-fine line search on the warm-start params, to test
    // whether a K matched to the big3 corpus dissolves the mg-deflate/eg-inflate
    // phase distortion. The fitted K is reported prominently below. Per the
    // K-freeze law any shipping candidate from this mode still requires margin
    // revalidation + a fresh tactics canary; the default path stays K_FROZEN.
    let k = if mode == Mode::FitK {
        // Coarse-to-fine line search (fit ONCE on warm-start params).
        let mut k = 1.0;
        let mut step = 0.5;
        for _ in 0..12 {
            let (lo, hi) = (mse(train, &p, k - step), mse(train, &p, k + step));
            let mid = mse(train, &p, k);
            if lo < mid && lo <= hi {
                k -= step;
            } else if hi < mid {
                k += step;
            } else {
                step /= 2.0;
            }
        }
        eprintln!("*** DELIBERATE K RE-ANCHORING (fit-k mode) ***");
        eprintln!("*** fitted K_big3 = {k:.4} (default frozen K = {K_FROZEN}) ***");
        eprintln!("*** shipping requires margin revalidation + fresh canary ***");
        k
    } else {
        K_FROZEN
    };
    let warm_mse = mse(train, &p, k);
    let k_label = if mode == Mode::FitK {
        "fitted"
    } else {
        "frozen"
    };
    eprintln!("{k_label} K = {k:.4}, warm train MSE = {warm_mse:.6}");

    // Adam, full batch — two separate moment banks (mg and eg)
    let mut m_mom = vec![0f64; 2 * N];
    let mut v_mom = vec![0f64; 2 * N];
    let (b1, b2, eps) = (0.9, 0.999, 1e-8);
    let scale = k * f64::ln(10.0) / 400.0; // d/dx sigmoid10(kx/400) factor

    // Overfit watchdog state (reported at the 50-epoch val checks, not enforced).
    let mut prev_val = warm_mse; // seed with the warm-start train MSE proxy
    let mut val_rises = 0usize;

    for epoch in 1..=epochs {
        // Parallel full-batch gradient (deterministic chunked reduce); the Adam
        // step below stays serial — it's O(params), trivial.
        let grad = gradient(train, &p, k, scale);
        let n = train.len() as f64;
        for i in 0..2 * N {
            let g = grad[i] / n;
            m_mom[i] = b1 * m_mom[i] + (1.0 - b1) * g;
            v_mom[i] = b2 * v_mom[i] + (1.0 - b2) * g * g;
            let mh = m_mom[i] / (1.0 - b1.powi(epoch as i32));
            let vh = v_mom[i] / (1.0 - b2.powi(epoch as i32));
            p[i] -= lr * 100.0 * mh / (vh.sqrt() + eps); // cp-scale lr
        }
        // AFTER EVERY STEP: re-pin the anchor(s) per mode — P mg = 100 by
        // default, or the entire MATERIAL row under pin-material.
        repin(&mut p, mode);

        if epoch % 50 == 0 {
            let tr = mse(train, &p, k);
            let va = mse(val, &p, k);
            eprintln!("epoch {epoch}: train {tr:.6} val {va:.6}");
            // Overfit watchdog: REPORT (don't abort — the controller's SPRT is the
            // real gate, and a truncated emit would surprise) if val MSE rises for
            // 3 consecutive checks.
            if va > prev_val {
                val_rises += 1;
                if val_rises >= 3 {
                    eprintln!(
                        "  [early-stop signal] val MSE rose {val_rises} checks running \
                         (last {prev_val:.6} -> {va:.6}); possible overfit"
                    );
                }
            } else {
                val_rises = 0;
            }
            prev_val = va;
        }
    }
    let final_train = mse(train, &p, k);
    let final_val = mse(val, &p, k);
    eprintln!("final: train {final_train:.6} val {final_val:.6}");
    emit(&p, k, final_train, final_val);
}
