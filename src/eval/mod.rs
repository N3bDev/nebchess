//! Evaluation behind the NNUE-ready seam (spec §6.1). The search calls
//! refresh/on_make/on_unmake unconditionally from M2 onward; HCE no-ops
//! them, a future NNUE updates its accumulator there.

pub mod hce;
pub mod psqt;

use crate::board::{Move, Position};

pub trait Evaluator {
    /// Full rebuild from the position (search root, ucinewgame).
    fn refresh(&mut self, pos: &Position);
    /// Incremental update; called immediately AFTER pos.make(mv).
    fn on_make(&mut self, mv: Move, pos: &Position);
    /// Incremental downdate; called immediately AFTER pos.unmake().
    fn on_unmake(&mut self, mv: Move, pos: &Position);
    /// Static evaluation in centipawns, side-to-move relative.
    /// (&mut: the HCE pawn hash (M5) and NNUE both mutate caches.)
    fn evaluate(&mut self, pos: &Position) -> i32;
}

pub use hce::Hce;
