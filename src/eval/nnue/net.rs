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
}
