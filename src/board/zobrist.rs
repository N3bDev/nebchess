//! Engine Zobrist keys. NOT the Polyglot book keys (those are a fixed public
//! array, implemented separately in the book module at M7).

pub struct ZobristKeys {
    /// [piece.index()][square.index()]
    pub pieces: [[u64; 64]; 12],
    /// Indexed by CastlingRights::bits(); precombined XOR of 4 base keys.
    pub castling: [u64; 16],
    /// Indexed by en-passant file.
    pub ep_file: [u64; 8],
    pub black_to_move: u64,
}

const fn splitmix64(state: u64) -> (u64, u64) {
    let state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    (state, z ^ (z >> 31))
}

const fn build() -> ZobristKeys {
    let mut state = 0x4E45_4243_4845_5353u64; // "NEBCHESS"
    let mut pieces = [[0u64; 64]; 12];
    let mut p = 0;
    while p < 12 {
        let mut s = 0;
        while s < 64 {
            let (st, k) = splitmix64(state);
            state = st;
            pieces[p][s] = k;
            s += 1;
        }
        p += 1;
    }
    let mut base = [0u64; 4];
    let mut i = 0;
    while i < 4 {
        let (st, k) = splitmix64(state);
        state = st;
        base[i] = k;
        i += 1;
    }
    let mut castling = [0u64; 16];
    let mut bits = 0;
    while bits < 16 {
        let mut k = 0u64;
        let mut j = 0;
        while j < 4 {
            if bits & (1 << j) != 0 {
                k ^= base[j];
            }
            j += 1;
        }
        castling[bits] = k;
        bits += 1;
    }
    let mut ep_file = [0u64; 8];
    let mut f = 0;
    while f < 8 {
        let (st, k) = splitmix64(state);
        state = st;
        ep_file[f] = k;
        f += 1;
    }
    let (_, black_to_move) = splitmix64(state);
    ZobristKeys {
        pieces,
        castling,
        ep_file,
        black_to_move,
    }
}

pub static KEYS: ZobristKeys = build();

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_keys_distinct() {
        let mut seen = HashSet::new();
        for sq_keys in KEYS.pieces.iter() {
            for &k in sq_keys.iter() {
                assert!(seen.insert(k), "duplicate piece key");
            }
        }
        for &k in &KEYS.ep_file {
            assert!(seen.insert(k), "duplicate ep key");
        }
        assert!(seen.insert(KEYS.black_to_move));
        // castling[0] must be 0 (no rights = nothing hashed)
        assert_eq!(KEYS.castling[0], 0);
        // the four single-right entries are the base keys and distinct
        for bits in [1usize, 2, 4, 8] {
            assert!(seen.insert(KEYS.castling[bits]), "duplicate castling key");
        }
    }

    #[test]
    fn castling_table_is_xor_composed() {
        for bits in 0..16usize {
            let mut expect = 0u64;
            for base_bit in [1usize, 2, 4, 8] {
                if bits & base_bit != 0 {
                    expect ^= KEYS.castling[base_bit];
                }
            }
            assert_eq!(KEYS.castling[bits], expect, "castling[{bits}]");
        }
    }
}
