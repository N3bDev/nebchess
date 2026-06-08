//! Throwaway PGN replay helper for Plan-7 Task-1 field analysis.
//!
//! Reads a lichess-style PGN (movetext may carry `{ [%clk ...] }` comments and
//! move-number tokens), replays each game through the trusted library
//! (`Position` + `find_san_move`), and emits a flat TSV stream that the Python
//! analysis driver (`tools/analyze-field.py`) consumes. Keeping SAN resolution
//! and terminal-mechanism detection inside the engine library means the Python
//! side never reimplements chess rules.
//!
//! Usage: `cargo run --release --example pgn_replay -- <file.pgn>`
//!
//! Output records (tab-separated, one per line):
//!   GAME <idx> <neb_color W|B> <result> <opponent> <opp_elo> <tc> <site> <plies>
//!   POS  <idx> <ply> <fullmove> <neb_to_move 0|1> <played_uci> <played_san> <fen> <halfmove> <piececount>
//!   TERM <idx> <final_fen> <rep 0|1> <fifty 0|1> <insuff 0|1> <result>
//!
//! `ply` is 0-based half-move index of the position BEFORE the listed move.
//!
//! Second mode — move annotation (for the sacrifice-entrance scan):
//!   `cargo run --release --example pgn_replay -- annotate <FEN> <uci> [<uci>...]`
//! emits one ANNOT line per move:
//!   ANNOT <uci> <is_capture 0|1> <gives_check 0|1> <see_swing_cp>
//! where see_swing_cp is the FULL static-exchange-evaluation of the move
//! (`see_swing` below — a faithful port of the engine's private `see`, so the
//! tool's "loses material by SEE" judgment matches engine semantics exactly).
//! Negative = the move loses material on the exchange = the sacrifice signal.
//! `gives_check` is from a real make() + in_check (catches discovered checks).
//!
//! Third mode — SAN→UCI resolution (for padding the suite from WAC):
//!   `cargo run --release --example pgn_replay -- san2uci <FEN> <san> [<san>...]`
//! emits `SAN2UCI <san> <uci|?>` per token via the library's `find_san_move`.

use std::fs;

use nebchess::board::movegen::{find_san_move, find_uci_move, generate_moves};
use nebchess::board::{Bitboard, Color, Move, MoveList, PieceType, Position, Square};

struct Header {
    white: String,
    black: String,
    result: String,
    white_elo: String,
    black_elo: String,
    tc: String,
    site: String,
}

impl Header {
    fn new() -> Header {
        Header {
            white: String::new(),
            black: String::new(),
            result: String::new(),
            white_elo: String::new(),
            black_elo: String::new(),
            tc: String::new(),
            site: String::new(),
        }
    }
}

fn tag_value(line: &str) -> String {
    // [Key "value"] -> value
    match (line.find('"'), line.rfind('"')) {
        (Some(a), Some(b)) if b > a => line[a + 1..b].to_string(),
        _ => String::new(),
    }
}

/// Strip lichess movetext down to a clean SAN token stream: drop `{...}`
/// comments, move-number tokens (`12.` / `12...`), NAGs (`$3`), result tokens,
/// and variation parens (the corpus has none, but be safe).
fn tokenize_movetext(movetext: &str) -> Vec<String> {
    // Remove brace comments first (they can contain spaces / digits).
    let mut cleaned = String::with_capacity(movetext.len());
    let mut depth_brace = 0i32;
    let mut depth_paren = 0i32;
    for c in movetext.chars() {
        match c {
            '{' => depth_brace += 1,
            '}' => {
                if depth_brace > 0 {
                    depth_brace -= 1;
                }
            }
            '(' => depth_paren += 1,
            ')' => {
                if depth_paren > 0 {
                    depth_paren -= 1;
                }
            }
            _ if depth_brace == 0 && depth_paren == 0 => cleaned.push(c),
            _ => {}
        }
    }

    let mut out = Vec::new();
    for raw in cleaned.split_whitespace() {
        let tok = raw.trim();
        if tok.is_empty() {
            continue;
        }
        // result tokens
        if matches!(tok, "1-0" | "0-1" | "1/2-1/2" | "*") {
            continue;
        }
        // NAGs
        if tok.starts_with('$') {
            continue;
        }
        // move-number tokens: leading digit and contains a '.'
        if tok.as_bytes()[0].is_ascii_digit() && tok.contains('.') {
            // could be "12." or "12...Nf6" (lichess separates, but be safe)
            let after = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
            if after.is_empty() {
                continue;
            }
            out.push(after.to_string());
            continue;
        }
        out.push(tok.to_string());
    }
    out
}

fn piece_count(pos: &Position) -> u32 {
    pos.occ_all().count()
}

/// True for K-vs-K, K+minor-vs-K, K-vs-K+minor (the lichess "insufficient
/// material" draw class). Not exhaustive (ignores KNNvK etc.), enough to label.
fn insufficient_material(pos: &Position) -> bool {
    let total = pos.occ_all().count();
    if total == 2 {
        return true; // bare kings
    }
    if total == 3 {
        // one side has a single minor, the other only the king
        let minors = (pos.piece_bb(Color::White, PieceType::Knight)
            | pos.piece_bb(Color::Black, PieceType::Knight)
            | pos.piece_bb(Color::White, PieceType::Bishop)
            | pos.piece_bb(Color::Black, PieceType::Bishop))
        .count();
        return minors == 1;
    }
    false
}

fn process_game(idx: usize, header: &Header, movetext: &str) {
    let neb_color = if header.white.to_lowercase().contains("neb") {
        Color::White
    } else if header.black.to_lowercase().contains("neb") {
        Color::Black
    } else {
        eprintln!("game {idx}: no Neb player found, skipping");
        return;
    };
    let (opponent, opp_elo) = match neb_color {
        Color::White => (header.black.clone(), header.black_elo.clone()),
        Color::Black => (header.white.clone(), header.white_elo.clone()),
    };

    let tokens = tokenize_movetext(movetext);
    let mut pos = Position::startpos();

    // Buffer POS lines; emit GAME first with the final ply count.
    let mut pos_lines: Vec<String> = Vec::new();
    let mut ply = 0usize;
    for san in &tokens {
        let neb_to_move = pos.stm() == neb_color;
        match find_san_move(&pos, san) {
            Some(mv) => {
                let fen = pos.to_fen();
                let fullmove = pos.fullmove();
                let hm = pos.halfmove();
                let pc = piece_count(&pos);
                pos_lines.push(format!(
                    "POS\t{idx}\t{ply}\t{fullmove}\t{}\t{}\t{san}\t{fen}\t{hm}\t{pc}",
                    if neb_to_move { 1 } else { 0 },
                    mv,
                ));
                let ok = pos.make(mv);
                if !ok {
                    eprintln!("game {idx} ply {ply}: SAN '{san}' resolved to illegal {mv}");
                    return;
                }
                ply += 1;
            }
            None => {
                eprintln!("game {idx} ply {ply}: could not resolve SAN '{san}'");
                return;
            }
        }
    }

    println!(
        "GAME\t{idx}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        if neb_color == Color::White { "W" } else { "B" },
        header.result,
        opponent,
        opp_elo,
        header.tc,
        header.site,
        ply,
    );
    for l in &pos_lines {
        println!("{l}");
    }

    // Terminal mechanism from the library. is_repetition() inspects the
    // make()-built history stack, so it must run on the final replayed pos.
    let final_fen = pos.to_fen();
    let rep = pos.is_repetition();
    let fifty = pos.is_fifty_move_draw();
    let insuff = insufficient_material(&pos);
    println!(
        "TERM\t{idx}\t{final_fen}\t{}\t{}\t{}\t{}",
        if rep { 1 } else { 0 },
        if fifty { 1 } else { 0 },
        if insuff { 1 } else { 0 },
        header.result,
    );
}

// SEE-local piece values (centipawns), matching the engine's SEE table so the
// material-swing judgment is consistent with how the engine prunes captures.
const PVAL: [i32; 6] = [100, 320, 330, 500, 900, 20_000];

fn val_of(pt: PieceType) -> i32 {
    PVAL[pt.index()]
}

/// All attackers (both colors) of `to` under `occ` (sliders resolved vs occ).
fn attackers_to(pos: &Position, to: Square, occ: Bitboard) -> Bitboard {
    use nebchess::board::attacks;
    use PieceType::*;
    (attacks::pawn_attacks(Color::Black, to) & pos.piece_bb(Color::White, Pawn))
        | (attacks::pawn_attacks(Color::White, to) & pos.piece_bb(Color::Black, Pawn))
        | (attacks::knight_attacks(to)
            & (pos.piece_bb(Color::White, Knight) | pos.piece_bb(Color::Black, Knight)))
        | (attacks::king_attacks(to)
            & (pos.piece_bb(Color::White, King) | pos.piece_bb(Color::Black, King)))
        | (attacks::bishop_attacks(to, occ)
            & (pos.piece_bb(Color::White, Bishop)
                | pos.piece_bb(Color::Black, Bishop)
                | pos.piece_bb(Color::White, Queen)
                | pos.piece_bb(Color::Black, Queen)))
        | (attacks::rook_attacks(to, occ)
            & (pos.piece_bb(Color::White, Rook)
                | pos.piece_bb(Color::Black, Rook)
                | pos.piece_bb(Color::White, Queen)
                | pos.piece_bb(Color::Black, Queen)))
}

/// Full static-exchange-evaluation swing (mover-relative cp) of capture/quiet
/// move `mv` — a faithful port of the engine's `see` (which is private), so the
/// tool's "loses material by SEE" judgment matches engine semantics exactly.
/// Negative = the move loses material on the exchange (the sacrifice signal).
fn see_swing(pos: &Position, mv: Move) -> i32 {
    let to = mv.to();
    let from = mv.from();
    let is_ep = mv.flag() == Move::EN_PASSANT;
    let mover = pos.piece_on(from).expect("mover").piece_type();

    let mut gain = [0i32; 32];
    gain[0] = if is_ep {
        val_of(PieceType::Pawn)
    } else {
        match pos.piece_on(to) {
            Some(p) => val_of(p.piece_type()),
            None => 0,
        }
    };

    let mut occ = pos.occ_all() ^ from.bb();
    if is_ep {
        let cap_sq = Square::new(if pos.stm() == Color::White {
            to.index() as u8 - 8
        } else {
            to.index() as u8 + 8
        });
        occ ^= cap_sq.bb();
    }
    let mut attackers = attackers_to(pos, to, occ) & occ;
    let mut stm = pos.stm().flip();
    let mut victim_val = val_of(mover);
    let mut depth = 0usize;

    loop {
        let my_attackers = attackers & pos.occ(stm) & occ;
        if my_attackers.is_empty() {
            break;
        }
        let mut lva = None;
        for pt in [
            PieceType::Pawn,
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
            PieceType::King,
        ] {
            let s = my_attackers & pos.piece_bb(stm, pt);
            if s.any() {
                lva = Some((s.lsb(), pt));
                break;
            }
        }
        let (sq, pt) = lva.expect("non-empty attackers => an LVA exists");
        depth += 1;
        gain[depth] = victim_val - gain[depth - 1];
        if gain[depth].max(-gain[depth - 1]) < 0 {
            break;
        }
        victim_val = val_of(pt);
        occ ^= sq.bb();
        attackers |= attackers_to(pos, to, occ);
        attackers &= occ;
        stm = stm.flip();
    }
    while depth > 0 {
        gain[depth - 1] = -((-gain[depth - 1]).max(gain[depth]));
        depth -= 1;
    }
    gain[0]
}

fn annotate(fen: &str, ucis: &[String]) {
    let pos = Position::from_fen(fen).expect("annotate: bad FEN");
    let mut list = MoveList::new();
    generate_moves(&pos, &mut list);
    for uci in ucis {
        match find_uci_move(&pos, uci) {
            Some(mv) => {
                let is_cap = mv.is_capture();
                // gives_check: make it (must be legal), test opp king in check.
                let mut p2 = pos.clone();
                let legal = p2.make(mv);
                let gives_check = legal && p2.in_check(pos.stm().flip());
                let swing = see_swing(&pos, mv);
                println!(
                    "ANNOT\t{uci}\t{}\t{}\t{}",
                    if is_cap { 1 } else { 0 },
                    if gives_check { 1 } else { 0 },
                    swing,
                );
            }
            None => println!("ANNOT\t{uci}\t-1\t-1\t0"), // unresolved
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .expect("usage: pgn_replay <file.pgn> | annotate <FEN> <uci>...");
    if first == "annotate" {
        let fen = args.next().expect("annotate: need a FEN");
        let ucis: Vec<String> = args.collect();
        annotate(&fen, &ucis);
        return;
    }
    if first == "san2uci" {
        // san2uci <FEN> <san>...  -> one "SAN2UCI <san> <uci|?>" line each
        let fen = args.next().expect("san2uci: need a FEN");
        let pos = Position::from_fen(&fen).expect("san2uci: bad FEN");
        for san in args {
            match find_san_move(&pos, &san) {
                Some(mv) => println!("SAN2UCI\t{san}\t{mv}"),
                None => println!("SAN2UCI\t{san}\t?"),
            }
        }
        return;
    }
    let path = first;
    let text = fs::read_to_string(&path).expect("read pgn");

    let mut idx = 0usize;
    let mut header = Header::new();
    let mut movetext = String::new();
    let mut in_moves = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if in_moves {
                // a new header block started: flush the previous game
                idx += 1;
                process_game(idx, &header, &movetext);
                header = Header::new();
                movetext.clear();
                in_moves = false;
            }
            let key = line
                .trim_start_matches('[')
                .split_whitespace()
                .next()
                .unwrap_or("");
            match key {
                "White" => header.white = tag_value(line),
                "Black" => header.black = tag_value(line),
                "Result" => header.result = tag_value(line),
                "WhiteElo" => header.white_elo = tag_value(line),
                "BlackElo" => header.black_elo = tag_value(line),
                "TimeControl" => header.tc = tag_value(line),
                "Site" => header.site = tag_value(line),
                _ => {}
            }
        } else if line.is_empty() {
            // blank line: header->moves separator, or moves->next-game gap
            if !movetext.is_empty() {
                in_moves = true;
            }
        } else {
            // movetext line
            in_moves = true;
            movetext.push_str(line);
            movetext.push(' ');
        }
    }
    if in_moves || !movetext.is_empty() {
        idx += 1;
        process_game(idx, &header, &movetext);
    }
}
