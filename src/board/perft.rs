//! Perft: exhaustive legal-move-tree leaf counting. The movegen correctness
//! oracle. Divide splits by root move for bug bisection.

use crate::board::{generate_moves, Move, MoveList, Position};

pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut list = MoveList::new();
    generate_moves(pos, &mut list);
    let mut nodes = 0;
    for &mv in list.iter() {
        if pos.make(mv) {
            nodes += perft(pos, depth - 1);
            pos.unmake();
            // incremental-key corruption shows up here across millions of nodes
            debug_assert_eq!(pos.key(), pos.compute_key());
        }
    }
    nodes
}

pub fn divide(pos: &mut Position, depth: u32) -> Vec<(Move, u64)> {
    let mut list = MoveList::new();
    generate_moves(pos, &mut list);
    let mut out = Vec::new();
    for &mv in list.iter() {
        if pos.make(mv) {
            out.push((mv, perft(pos, depth - 1)));
            pos.unmake();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Position;

    #[test]
    fn perft_startpos_shallow() {
        let mut pos = Position::startpos();
        assert_eq!(perft(&mut pos, 1), 20);
        assert_eq!(perft(&mut pos, 2), 400);
        assert_eq!(perft(&mut pos, 3), 8_902);
        assert_eq!(perft(&mut pos, 4), 197_281);
        // depth-0 convention
        assert_eq!(perft(&mut pos, 0), 1);
    }

    #[test]
    fn divide_sums_to_perft() {
        let mut pos = Position::startpos();
        let parts = divide(&mut pos, 3);
        assert_eq!(parts.len(), 20);
        let total: u64 = parts.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 8_902);
    }
}
