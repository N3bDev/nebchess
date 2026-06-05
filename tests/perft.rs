//! Enumerated perft suite (spec §10.1). Counts are canonical (CPW + the
//! standard edge-case set). Deep tier: cargo test --release -- --ignored

use nebchess::board::perft::perft;
use nebchess::board::Position;

struct Case {
    name: &'static str,
    fen: &'static str,
    depth: u32,
    nodes: u64,
}

fn run(cases: &[Case]) {
    for c in cases {
        let mut pos = Position::from_fen(c.fen).expect(c.name);
        assert_eq!(
            perft(&mut pos, c.depth),
            c.nodes,
            "{} depth {} (fen: {})",
            c.name,
            c.depth,
            c.fen
        );
    }
}

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
const POS3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
const POS4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
const POS4_MIRROR: &str = "r2q1rk1/pP1p2pp/Q4n2/bbp1p3/Np6/1B3NBn/pPPP1PPP/R3K2R b KQ - 0 1";
const POS5: &str = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";
const POS6: &str = "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10";

#[test]
fn perft_fast_suite() {
    run(&[
        Case {
            name: "startpos",
            fen: STARTPOS,
            depth: 5,
            nodes: 4_865_609,
        },
        Case {
            name: "kiwipete",
            fen: KIWIPETE,
            depth: 4,
            nodes: 4_085_603,
        },
        Case {
            name: "pos3",
            fen: POS3,
            depth: 5,
            nodes: 674_624,
        },
        Case {
            name: "pos4",
            fen: POS4,
            depth: 4,
            nodes: 422_333,
        },
        Case {
            name: "pos4-mirror",
            fen: POS4_MIRROR,
            depth: 4,
            nodes: 422_333,
        },
        Case {
            name: "pos5",
            fen: POS5,
            depth: 4,
            nodes: 2_103_487,
        },
        Case {
            name: "pos6",
            fen: POS6,
            depth: 4,
            nodes: 3_894_594,
        },
    ]);
}

#[test]
fn perft_edge_cases() {
    run(&[
        Case {
            name: "illegal-ep-1",
            fen: "3k4/3p4/8/K1P4r/8/8/8/8 b - - 0 1",
            depth: 6,
            nodes: 1_134_888,
        },
        Case {
            name: "ep-into-check",
            fen: "8/8/4k3/8/2p5/8/B2P2K1/8 w - - 0 1",
            depth: 6,
            nodes: 1_015_133,
        },
        Case {
            name: "ep-pinned",
            fen: "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1",
            depth: 6,
            nodes: 1_440_467,
        },
        Case {
            name: "castle-gives-check",
            fen: "5k2/8/8/8/8/8/8/4K2R w K - 0 1",
            depth: 6,
            nodes: 661_072,
        },
        Case {
            name: "castle-rights",
            fen: "3k4/8/8/8/8/8/8/R3K3 w Q - 0 1",
            depth: 6,
            nodes: 803_711,
        },
        Case {
            name: "castle-prevented",
            fen: "r3k2r/1b4bq/8/8/8/8/7B/R3K2R w KQkq - 0 1",
            depth: 4,
            nodes: 1_274_206,
        },
        Case {
            name: "castle-through-check",
            fen: "r3k2r/8/3Q4/8/8/5q2/8/R3K2R b KQkq - 0 1",
            depth: 4,
            nodes: 1_720_476,
        },
        Case {
            name: "promote-out-of-check",
            fen: "2K2r2/4P3/8/8/8/8/8/3k4 w - - 0 1",
            depth: 6,
            nodes: 3_821_001,
        },
        Case {
            name: "discovered-check",
            fen: "8/8/1P2K3/8/2n5/1q6/8/5k2 b - - 0 1",
            depth: 5,
            nodes: 1_004_658,
        },
        Case {
            name: "promote-gives-check",
            fen: "4k3/1P6/8/8/8/8/K7/8 w - - 0 1",
            depth: 6,
            nodes: 217_342,
        },
        Case {
            name: "underpromote",
            fen: "8/P1k5/K7/8/8/8/8/8 w - - 0 1",
            depth: 6,
            nodes: 92_683,
        },
        Case {
            name: "self-stalemate",
            fen: "K1k5/8/P7/8/8/8/8/8 w - - 0 1",
            depth: 6,
            nodes: 2_217,
        },
        Case {
            name: "stale-and-checkmate",
            fen: "8/k1P5/8/1K6/8/8/8/8 w - - 0 1",
            depth: 7,
            nodes: 567_584,
        },
        Case {
            name: "double-check",
            fen: "8/8/2k5/5q2/5n2/8/5K2/8 b - - 0 1",
            depth: 4,
            nodes: 23_527,
        },
    ]);
}

#[test]
#[ignore = "deep perft (~600M nodes): cargo test --release -- --ignored"]
fn perft_deep_suite() {
    run(&[
        Case {
            name: "startpos",
            fen: STARTPOS,
            depth: 6,
            nodes: 119_060_324,
        },
        Case {
            name: "kiwipete",
            fen: KIWIPETE,
            depth: 5,
            nodes: 193_690_690,
        },
        Case {
            name: "pos3",
            fen: POS3,
            depth: 6,
            nodes: 11_030_083,
        },
        Case {
            name: "pos4",
            fen: POS4,
            depth: 5,
            nodes: 15_833_292,
        },
        Case {
            name: "pos5",
            fen: POS5,
            depth: 5,
            nodes: 89_941_194,
        },
        Case {
            name: "pos6",
            fen: POS6,
            depth: 5,
            nodes: 164_075_551,
        },
    ]);
}
