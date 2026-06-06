//! Parameter registry: every tunable term declares (name, length) here.
//! Flat pair-index space: offsets are cumulative. The tuner sizes its
//! vector from TOTAL_PAIRS; eval_params.rs is emitted in manifest order.

pub struct TermDef {
    pub name: &'static str,
    pub len: usize,
}

/// ORDER IS ABI: eval_params.rs and the tuner both index by these offsets.
/// Append-only within a task; never reorder existing entries mid-plan.
pub const TERMS: &[TermDef] = &[
    TermDef {
        name: "MATERIAL",
        len: 6,
    }, // P N B R Q K (K pair stays 0; P mg pinned 100)
    TermDef {
        name: "PST_PAWN",
        len: 64,
    },
    TermDef {
        name: "PST_KNIGHT",
        len: 64,
    },
    TermDef {
        name: "PST_BISHOP",
        len: 64,
    },
    TermDef {
        name: "PST_ROOK",
        len: 64,
    },
    TermDef {
        name: "PST_QUEEN",
        len: 64,
    },
    TermDef {
        name: "PST_KING",
        len: 64,
    },
    // T2: pawn structure
    TermDef {
        name: "PASSED",
        len: 6,
    }, // by relative rank 2..7
    TermDef {
        name: "PASSED_CONNECTED",
        len: 1,
    },
    TermDef {
        name: "ISOLATED",
        len: 1,
    },
    TermDef {
        name: "DOUBLED",
        len: 1,
    },
    // T3 appends: MOB_KNIGHT(9) MOB_BISHOP(14) MOB_ROOK(15) MOB_QUEEN(28)
    // T4 appends: KS_ATTACKER(4) KS_SHIELD(3) KS_OPEN_FILE(1) KS_SEMI_FILE(1)
    // T5 appends: THREAT_BY_PAWN(4) THREAT_BY_MINOR(4) HANGING(1)
    //             BISHOP_PAIR(1) ROOK_OPEN(1) ROOK_SEMI(1) TEMPO(1)
];

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub const fn offset_of(name: &str) -> usize {
    let mut off = 0;
    let mut i = 0;
    while i < TERMS.len() {
        // const-compatible string compare
        if str_eq(TERMS[i].name, name) {
            return off;
        }
        off += TERMS[i].len;
        i += 1;
    }
    panic!("unknown term");
}

pub const fn total_pairs() -> usize {
    let mut off = 0;
    let mut i = 0;
    while i < TERMS.len() {
        off += TERMS[i].len;
        i += 1;
    }
    off
}

// Named offsets (computed once, used by hce term code as const indices)
pub const MATERIAL: usize = offset_of("MATERIAL");
pub const PST_PAWN: usize = offset_of("PST_PAWN");
pub const PST_KNIGHT: usize = offset_of("PST_KNIGHT");
pub const PST_BISHOP: usize = offset_of("PST_BISHOP");
pub const PST_ROOK: usize = offset_of("PST_ROOK");
pub const PST_QUEEN: usize = offset_of("PST_QUEEN");
pub const PST_KING: usize = offset_of("PST_KING");
// T2 pawn structure offsets
pub const PASSED: usize = offset_of("PASSED");
pub const PASSED_CONNECTED: usize = offset_of("PASSED_CONNECTED");
pub const ISOLATED: usize = offset_of("ISOLATED");
pub const DOUBLED: usize = offset_of("DOUBLED");
pub const TOTAL_PAIRS: usize = total_pairs();
