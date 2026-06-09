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
        assert_eq!(bytes.len(), std::mem::size_of::<Network>(), "NNUE net size mismatch");
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
        let sum = self.out_scalar(us, them);
        (sum / i32::from(QA) + i32::from(self.output_bias)) * SCALE / (i32::from(QA) * i32::from(QB))
    }

    #[inline]
    fn out_scalar(&self, us: &Accumulator, them: &Accumulator) -> i32 {
        let mut sum = 0i32;
        for (&i, &w) in us.vals.iter().zip(&self.output_weights[..HIDDEN]) { sum += Self::screlu(i) * i32::from(w); }
        for (&i, &w) in them.vals.iter().zip(&self.output_weights[HIDDEN..]) { sum += Self::screlu(i) * i32::from(w); }
        sum
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
        fn screlu(x: i16) -> i32 { let y = i32::from(x).clamp(0, i32::from(QA)); y * y }
        let mut output = 0i32;
        for (&i, &w) in us.vals.iter().zip(&net.output_weights[..HIDDEN]) { output += screlu(i) * i32::from(w); }
        for (&i, &w) in them.vals.iter().zip(&net.output_weights[HIDDEN..]) { output += screlu(i) * i32::from(w); }
        output /= i32::from(QA);
        output += i32::from(net.output_bias);
        output *= SCALE;
        output /= i32::from(QA) * i32::from(QB);
        output
    }

    #[test]
    fn out_matches_reference() {
        let Ok(bytes) = std::fs::read(TOY_NET) else { return };
        let net = Network::from_bytes(&bytes);
        let mut s = 0x1234_5678u64;
        let mut rnd = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s % 512) as i16 - 128 };
        for _ in 0..64 {
            let mut us = Accumulator { vals: [0; HIDDEN] };
            let mut them = Accumulator { vals: [0; HIDDEN] };
            for i in 0..HIDDEN { us.vals[i] = rnd(); them.vals[i] = rnd(); }
            assert_eq!(net.out(&us, &them), reference_out(&net, &us, &them));
        }
    }
}
