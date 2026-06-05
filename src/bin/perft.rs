//! Perft/divide CLI. Usage:
//!   perft "<fen>|startpos" <depth> [uci_move ...]
//! Prints one "uci: count" line per root move, then "total: N".

use nebchess::board::movegen::find_uci_move;
use nebchess::board::Position;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: perft \"<fen>|startpos\" <depth> [uci_move ...]");
        std::process::exit(2);
    }
    let mut pos = if args[0] == "startpos" {
        Position::startpos()
    } else {
        Position::from_fen(&args[0]).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        })
    };
    let depth: u32 = args[1].parse().unwrap_or_else(|_| {
        eprintln!("bad depth: {}", args[1]);
        std::process::exit(2);
    });
    for uci in &args[2..] {
        match find_uci_move(&pos, uci) {
            Some(mv) if pos.make(mv) => {}
            Some(_) => {
                eprintln!("illegal move: {uci}");
                std::process::exit(2);
            }
            None => {
                eprintln!("unknown move: {uci}");
                std::process::exit(2);
            }
        }
    }
    if depth == 0 {
        // divide(_, 0) would underflow; perft(_, 0) is 1 by convention
        println!("total: 1");
        return;
    }
    let mut total = 0u64;
    let mut parts = nebchess::board::perft::divide(&mut pos, depth);
    parts.sort_by_key(|(mv, _)| mv.to_string());
    for (mv, nodes) in parts {
        println!("{mv}: {nodes}");
        total += nodes;
    }
    println!("total: {total}");
}
