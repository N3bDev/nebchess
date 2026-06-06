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
    // T3: mobility (safe-square counts per piece; 0..max inclusive)
    TermDef {
        name: "MOB_KNIGHT",
        len: 9,
    }, // 0..=8 safe squares
    TermDef {
        name: "MOB_BISHOP",
        len: 14,
    }, // 0..=13 safe squares
    TermDef {
        name: "MOB_ROOK",
        len: 15,
    }, // 0..=14 safe squares
    TermDef {
        name: "MOB_QUEEN",
        len: 28,
    }, // 0..=27 safe squares
    // T4: king safety
    TermDef {
        name: "KS_ATTACKER",
        len: 4,
    }, // N/B/R/Q touching the king zone
    TermDef {
        name: "KS_SHIELD",
        len: 3,
    }, // shield pawn at rel-rank 2 / rel-rank 3 / missing, per file
    TermDef {
        name: "KS_OPEN_FILE",
        len: 1,
    }, // no pawns on a file near the king
    TermDef {
        name: "KS_SEMI_FILE",
        len: 1,
    }, // no own pawns on a file near the king
    // T5: threats, coordination, tempo
    TermDef {
        name: "THREAT_BY_PAWN",
        len: 4,
    }, // pawn attacks an enemy N/B/R/Q
    TermDef {
        name: "THREAT_BY_MINOR",
        len: 4,
    }, // minor (N/B) attacks an enemy N/B/R/Q
    TermDef {
        name: "HANGING",
        len: 1,
    }, // enemy piece attacked by us, undefended
    TermDef {
        name: "BISHOP_PAIR",
        len: 1,
    }, // two or more bishops
    TermDef {
        name: "ROOK_OPEN",
        len: 1,
    }, // rook on a fully open file
    TermDef {
        name: "ROOK_SEMI",
        len: 1,
    }, // rook on a semi-open file (no own pawns)
    TermDef {
        name: "TEMPO",
        len: 1,
    }, // side-to-move initiative bonus
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
// T3 mobility offsets
pub const MOB_KNIGHT: usize = offset_of("MOB_KNIGHT");
pub const MOB_BISHOP: usize = offset_of("MOB_BISHOP");
pub const MOB_ROOK: usize = offset_of("MOB_ROOK");
pub const MOB_QUEEN: usize = offset_of("MOB_QUEEN");
// T4 king safety offsets
pub const KS_ATTACKER: usize = offset_of("KS_ATTACKER");
pub const KS_SHIELD: usize = offset_of("KS_SHIELD");
pub const KS_OPEN_FILE: usize = offset_of("KS_OPEN_FILE");
pub const KS_SEMI_FILE: usize = offset_of("KS_SEMI_FILE");
// T5 threats / coordination / tempo offsets
pub const THREAT_BY_PAWN: usize = offset_of("THREAT_BY_PAWN");
pub const THREAT_BY_MINOR: usize = offset_of("THREAT_BY_MINOR");
pub const HANGING: usize = offset_of("HANGING");
pub const BISHOP_PAIR: usize = offset_of("BISHOP_PAIR");
pub const ROOK_OPEN: usize = offset_of("ROOK_OPEN");
pub const ROOK_SEMI: usize = offset_of("ROOK_SEMI");
pub const TEMPO: usize = offset_of("TEMPO");
pub const TOTAL_PAIRS: usize = total_pairs();
