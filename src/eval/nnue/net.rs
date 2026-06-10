//! NNUE net: raw quantised weights (matches the trainer's quantised.bin) + the forward pass.
use std::alloc::{alloc_zeroed, Layout};

pub const HIDDEN: usize = 768;
pub const QA: i16 = 255;
pub const QB: i16 = 64;
pub const SCALE: i32 = 400;

/// One perspective half / one feature-weight column. `align(64)` for AVX2 aligned loads.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [i16; HIDDEN],
}

/// Raw quantised network, laid out to match the trainer's `quantised.bin` byte-for-byte.
#[repr(C)]
pub struct Network {
    pub feature_weights: [Accumulator; 768], // [feature_index] -> column of HIDDEN, QA-scaled
    pub feature_bias: Accumulator,           // QA-scaled
    pub output_weights: [i16; 2 * HIDDEN],   // [0..H]=us, [H..2H]=them, QB-scaled
    pub output_bias: i16,                    // QA*QB-scaled
}

const _: () = assert!(std::mem::size_of::<Network>() == 1_184_320);

impl Network {
    /// Load from raw bytes (works for `include_bytes!` or `std::fs::read`). Returns a
    /// 64-byte-aligned boxed Network (alignment comes from Network's `align(64)` fields).
    pub fn from_bytes(bytes: &[u8]) -> Box<Network> {
        assert_eq!(
            bytes.len(),
            std::mem::size_of::<Network>(),
            "NNUE net size mismatch"
        );
        // SAFETY: Network is repr(C) and all-i16 (plain old data). alloc_zeroed gives a
        // correctly-aligned allocation for Network; we then copy the exact bytes in.
        unsafe {
            let layout = Layout::new::<Network>();
            let ptr = alloc_zeroed(layout) as *mut Network;
            assert!(!ptr.is_null(), "NNUE net allocation failed");
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            Box::from_raw(ptr)
        }
    }

    #[inline]
    fn screlu(x: i16) -> i32 {
        let y = i32::from(x).clamp(0, i32::from(QA));
        y * y
    }

    /// Quantised forward pass. `us`/`them` are the side-to-move / opponent accumulator halves.
    /// Returns side-to-move-relative centipawns.
    pub fn out(&self, us: &Accumulator, them: &Accumulator) -> i32 {
        let sum = self.sum(us, them);
        (sum / i32::from(QA) + i32::from(self.output_bias)) * SCALE
            / (i32::from(QA) * i32::from(QB))
    }

    #[inline]
    fn sum(&self, us: &Accumulator, them: &Accumulator) -> i32 {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                // SAFETY: guarded by the runtime feature check.
                return unsafe { Self::out_avx2(us, them, &self.output_weights) };
            }
        }
        self.out_scalar(us, them)
    }

    // The dot-product sum is i32 deliberately: it must match the AVX2 path (which accumulates
    // in i32 lanes via _mm256_madd_epi16) and bullet's reference (simple.rs, also i32). bullet's
    // AdamW weight-clipping (+-1.98) plus sparse, sign-cancelling activations keep real-net sums
    // far inside i32; the degenerate worst case cannot occur for a trained net. This is why the
    // whole ecosystem (simple.rs/akimbo/Carp) uses i32 here.
    #[inline]
    fn out_scalar(&self, us: &Accumulator, them: &Accumulator) -> i32 {
        let mut sum = 0i32;
        for (&i, &w) in us.vals.iter().zip(&self.output_weights[..HIDDEN]) {
            sum += Self::screlu(i) * i32::from(w);
        }
        for (&i, &w) in them.vals.iter().zip(&self.output_weights[HIDDEN..]) {
            sum += Self::screlu(i) * i32::from(w);
        }
        sum
    }

    /// AVX2 SCReLU dot-product, returning the same raw i32 sum as `out_scalar`.
    /// Overflow-safe SCReLU: compute v*(v*w) via mullo+madd so the i16 intermediate (v*w)
    /// stays in range (v<=QA=255, |w| small), then madd widens to i32.
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn out_avx2(us: &Accumulator, them: &Accumulator, weights: &[i16; 2 * HIDDEN]) -> i32 {
        use std::arch::x86_64::*;
        let min = _mm256_setzero_si256();
        let max = _mm256_set1_epi16(QA);
        let mut acc = _mm256_setzero_si256();
        let mut i = 0;
        while i < HIDDEN {
            // us half (aligned load: Accumulator is align(64))
            let v = _mm256_min_epi16(
                _mm256_max_epi16(
                    _mm256_load_si256(us.vals.as_ptr().add(i) as *const __m256i),
                    min,
                ),
                max,
            );
            let w = _mm256_loadu_si256(weights.as_ptr().add(i) as *const __m256i);
            acc = _mm256_add_epi32(acc, _mm256_madd_epi16(v, _mm256_mullo_epi16(v, w)));
            // them half
            let v2 = _mm256_min_epi16(
                _mm256_max_epi16(
                    _mm256_load_si256(them.vals.as_ptr().add(i) as *const __m256i),
                    min,
                ),
                max,
            );
            let w2 = _mm256_loadu_si256(weights.as_ptr().add(HIDDEN + i) as *const __m256i);
            acc = _mm256_add_epi32(acc, _mm256_madd_epi16(v2, _mm256_mullo_epi16(v2, w2)));
            i += 16;
        }
        // horizontal sum of the 8 i32 lanes
        let hi = _mm256_extracti128_si256(acc, 1);
        let lo = _mm256_castsi256_si128(acc);
        let s = _mm_add_epi32(hi, lo);
        let s = _mm_add_epi32(s, _mm_shuffle_epi32(s, 0b01_00_11_10));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32(s, 0b10_11_00_01));
        _mm_cvtsi128_si32(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOY_NET: &str = "tools/trainer/checkpoints/toy-5/quantised.bin";

    #[test]
    fn loads_toy_net_if_present() {
        let Ok(bytes) = std::fs::read(TOY_NET) else {
            eprintln!("skipping: {TOY_NET} not present");
            return;
        };
        assert_eq!(bytes.len(), 1_184_320, "toy net must be the contract size");
        let net = Network::from_bytes(&bytes);
        let _ = net.feature_bias.vals[0];
    }

    fn reference_out(net: &Network, us: &Accumulator, them: &Accumulator) -> i32 {
        // Verbatim port of bullet examples/simple.rs Network::evaluate — the canonical reference.
        fn screlu(x: i16) -> i32 {
            let y = i32::from(x).clamp(0, i32::from(QA));
            y * y
        }
        let mut output = 0i32;
        for (&i, &w) in us.vals.iter().zip(&net.output_weights[..HIDDEN]) {
            output += screlu(i) * i32::from(w);
        }
        for (&i, &w) in them.vals.iter().zip(&net.output_weights[HIDDEN..]) {
            output += screlu(i) * i32::from(w);
        }
        output /= i32::from(QA);
        output += i32::from(net.output_bias);
        output *= SCALE;
        output /= i32::from(QA) * i32::from(QB);
        output
    }

    #[test]
    fn out_matches_reference() {
        let Ok(bytes) = std::fs::read(TOY_NET) else {
            return;
        };
        let net = Network::from_bytes(&bytes);
        let mut s = 0x1234_5678u64;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 512) as i16 - 128
        };
        for _ in 0..64 {
            let mut us = Accumulator { vals: [0; HIDDEN] };
            let mut them = Accumulator { vals: [0; HIDDEN] };
            for i in 0..HIDDEN {
                us.vals[i] = rnd();
                them.vals[i] = rnd();
            }
            assert_eq!(net.out(&us, &them), reference_out(&net, &us, &them));
        }
    }

    #[test]
    fn scalar_and_avx2_agree() {
        let Ok(bytes) = std::fs::read(TOY_NET) else {
            return;
        };
        #[cfg(target_arch = "x86_64")]
        {
            if !std::arch::is_x86_feature_detected!("avx2") {
                eprintln!("skip: no avx2");
                return;
            }
            let net = Network::from_bytes(&bytes);
            let mut s = 0xABCD_1234u64;
            let mut rnd = || {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s % 600) as i16 - 200
            };
            for _ in 0..64 {
                let mut us = Accumulator { vals: [0; HIDDEN] };
                let mut them = Accumulator { vals: [0; HIDDEN] };
                for i in 0..HIDDEN {
                    us.vals[i] = rnd();
                    them.vals[i] = rnd();
                }
                let scalar = net.out_scalar(&us, &them);
                let avx2 = unsafe { Network::out_avx2(&us, &them, &net.output_weights) };
                assert_eq!(scalar, avx2, "scalar and AVX2 sums disagree");
            }
        }
    }

    #[test]
    #[ignore]
    fn eval_throughput() {
        use std::time::Instant;

        let Ok(bytes) = std::fs::read(TOY_NET) else {
            eprintln!("skipping eval_throughput: {TOY_NET} not present");
            return;
        };
        let net = Network::from_bytes(&bytes);

        // Build a fixed accumulator pair
        let mut us = Accumulator {
            vals: [0i16; HIDDEN],
        };
        let mut them = Accumulator {
            vals: [0i16; HIDDEN],
        };
        for i in 0..HIDDEN {
            us.vals[i] = (i % 255) as i16;
            them.vals[i] = ((i + 128) % 255) as i16;
        }

        const ITERS: u32 = 200_000;
        let t0 = Instant::now();
        let mut sink = 0i32;
        for _ in 0..ITERS {
            sink = sink.wrapping_add(net.out(&us, &them));
        }
        let elapsed = t0.elapsed();
        let per_sec = ITERS as f64 / elapsed.as_secs_f64();
        eprintln!(
            "NNUE out() throughput: {:.0} evals/sec (sink={})",
            per_sec, sink
        );
    }
}
