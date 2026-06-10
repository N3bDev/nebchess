use super::net::{Accumulator, Network, HIDDEN};
use crate::board::types::{Color, Piece, Square};

/// (white-view, black-view) feature indices for a piece on `sq`. Source-verified against
/// bullet Chess768 (chess768.rs): own/opp split at 384, type*64, black-view flips sq.
#[inline]
pub fn feature_indices(piece: Piece, sq: Square) -> (usize, usize) {
    let pt = piece.piece_type() as usize; // 0..=5
    let s = sq.index();
    let (w_off, b_off) = match piece.color() {
        Color::White => (0usize, 384usize),
        Color::Black => (384usize, 0usize),
    };
    (w_off + 64 * pt + s, b_off + 64 * pt + (s ^ 56))
}

/// Both perspective accumulators for the current position.
#[derive(Clone, Copy)]
pub struct AccPair {
    pub white: Accumulator, // white's view
    pub black: Accumulator, // black's view
}

impl AccPair {
    /// Initialised to the feature bias (so we can add/sub piece features afterwards).
    pub fn fresh(net: &Network) -> AccPair {
        AccPair {
            white: net.feature_bias,
            black: net.feature_bias,
        }
    }

    #[inline]
    pub fn add(&mut self, net: &Network, piece: Piece, sq: Square) {
        let (w, b) = feature_indices(piece, sq);
        let cw = &net.feature_weights[w].vals;
        let cb = &net.feature_weights[b].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] += cw[i];
            self.black.vals[i] += cb[i];
        }
    }

    #[inline]
    pub fn sub(&mut self, net: &Network, piece: Piece, sq: Square) {
        let (w, b) = feature_indices(piece, sq);
        let cw = &net.feature_weights[w].vals;
        let cb = &net.feature_weights[b].vals;
        for i in 0..HIDDEN {
            self.white.vals[i] -= cw[i];
            self.black.vals[i] -= cb[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::types::PieceType;

    #[test]
    fn feature_index_matches_convention() {
        // White knight (type 1) on a1 (sq 0): white-view = 0 + 64*1 + 0 = 64; black-view = 384 + 64 + (0^56)=504.
        let (w, b) = feature_indices(Piece::new(Color::White, PieceType::Knight), Square::new(0));
        assert_eq!((w, b), (64, 504));
        // Black pawn (type 0) on a8 (sq 56): white-view = 384 + 0 + 56 = 440; black-view = 0 + 0 + (56^56)=0.
        let (w, b) = feature_indices(Piece::new(Color::Black, PieceType::Pawn), Square::new(56));
        assert_eq!((w, b), (440, 0));
    }

    #[test]
    fn add_then_sub_is_identity() {
        let Ok(bytes) = std::fs::read("tools/trainer/checkpoints/toy-5/quantised.bin") else {
            return;
        };
        let net = Network::from_bytes(&bytes);
        let mut acc = AccPair::fresh(&net);
        let before = acc;
        let p = Piece::new(Color::White, PieceType::Queen);
        acc.add(&net, p, Square::new(27));
        assert_ne!(
            acc.white.vals, before.white.vals,
            "add changed the accumulator"
        );
        acc.sub(&net, p, Square::new(27));
        assert_eq!(acc.white.vals, before.white.vals);
        assert_eq!(acc.black.vals, before.black.vals);
    }
}
