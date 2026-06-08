//! PolyGlot book builder: turns PGN game collections into a sorted `*.bin`.
//!
//! Usage:
//!   bookgen <out.bin> <pgn> [<pgn>...]
//!           [--min-elo 2300] [--min-plies 40]
//!           [--max-book-plies 16] [--min-count 2]
//!
//! For every game that passes the filters, the first `--max-book-plies` plies
//! are walked. At each position we accumulate, keyed by
//! `(polyglot_key, polyglot_move)`: a visit count and a quality score
//! (`+2` for a win by the side to move, `+1` for a draw, `0` for a loss). The
//! per-entry `weight` written to disk is the score, saturated to `u16`.
//! Entries with `count < --min-count` are dropped. Output is sorted by key and
//! written as 16-byte big-endian PolyGlot entries.
//!
//! Filters (drop the whole game when any fails):
//!   * either side's Elo `< --min-elo` (a missing/unparseable Elo fails)
//!   * total plies `< --min-plies`
//!   * `WhiteTitle`/`BlackTitle` == "BOT" (online engine accounts — junk)
//!   * `TimeControl` base seconds `< 180` (bullet — junk; absent = OTB, kept)

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};

use nebchess::board::movegen::find_san_move;
use nebchess::board::{Color, Move, PieceType, Position, Square};
use nebchess::book::polyglot_key;

#[derive(Clone, Copy)]
struct Args {
    min_elo: u32,
    min_plies: usize,
    max_book_plies: usize,
    min_count: u32,
}

impl Default for Args {
    fn default() -> Args {
        Args {
            min_elo: 2300,
            min_plies: 40,
            max_book_plies: 16,
            min_count: 2,
        }
    }
}

/// Accumulator for one (key, move) cell.
#[derive(Default, Clone, Copy)]
struct Stat {
    count: u32,
    score: u64,
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut out_path: Option<String> = None;
    let mut pgns: Vec<String> = Vec::new();
    let mut args = Args::default();

    let mut it = raw.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--min-elo" => args.min_elo = parse_flag(&mut it, "--min-elo"),
            "--min-plies" => args.min_plies = parse_flag(&mut it, "--min-plies"),
            "--max-book-plies" => args.max_book_plies = parse_flag(&mut it, "--max-book-plies"),
            "--min-count" => args.min_count = parse_flag(&mut it, "--min-count"),
            other if other.starts_with("--") => {
                eprintln!("unknown flag: {other}");
                std::process::exit(2);
            }
            other => {
                if out_path.is_none() {
                    out_path = Some(other.to_string());
                } else {
                    pgns.push(other.to_string());
                }
            }
        }
    }

    let out_path = out_path.unwrap_or_else(|| {
        eprintln!("usage: bookgen <out.bin> <pgn> [<pgn>...] [--min-elo N] [--min-plies N] [--max-book-plies N] [--min-count N]");
        std::process::exit(2);
    });
    if pgns.is_empty() {
        eprintln!("error: no input PGN files given");
        std::process::exit(2);
    }

    let mut table: HashMap<(u64, u16), Stat> = HashMap::new();
    let mut games_read = 0u64;
    let mut games_kept = 0u64;
    let mut positions = 0u64;

    for path in &pgns {
        let mut text = String::new();
        match File::open(path).and_then(|mut f| f.read_to_string(&mut text)) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("error reading {path}: {e}");
                std::process::exit(1);
            }
        }
        eprintln!("parsing {path} ({} bytes)...", text.len());
        for game in GameIter::new(&text) {
            games_read += 1;
            if let Some(p) = process_game(&game, &args, &mut table) {
                games_kept += 1;
                positions += p;
            }
            if games_read.is_multiple_of(100_000) {
                eprintln!(
                    "  ...{games_read} read, {games_kept} kept, {} cells",
                    table.len()
                );
            }
        }
    }

    // materialise entries, drop low-count cells, saturate the score to u16
    let mut entries: Vec<(u64, u16, u16)> = table
        .into_iter()
        .filter(|(_, s)| s.count >= args.min_count)
        .map(|((key, mv), s)| (key, mv, s.score.min(u16::MAX as u64) as u16))
        .collect();
    // PolyGlot books are sorted by key; ties broken by descending weight so a
    // reader scanning a key-run sees the strongest move first (cosmetic).
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)));

    match write_book(&out_path, &entries) {
        Ok(bytes) => {
            eprintln!("wrote {out_path}: {} entries, {bytes} bytes", entries.len());
        }
        Err(e) => {
            eprintln!("error writing {out_path}: {e}");
            std::process::exit(1);
        }
    }

    println!("games read:   {games_read}");
    println!("games kept:   {games_kept}");
    println!("positions:    {positions}");
    println!("entries:      {}", entries.len());
}

fn parse_flag<'a, T: std::str::FromStr>(
    it: &mut impl Iterator<Item = &'a String>,
    name: &str,
) -> T {
    match it.next().and_then(|v| v.parse::<T>().ok()) {
        Some(v) => v,
        None => {
            eprintln!("error: {name} needs a numeric value");
            std::process::exit(2);
        }
    }
}

/// One game's headers + raw movetext.
struct Game<'a> {
    headers: HashMap<&'a str, &'a str>,
    movetext: String,
}

impl Game<'_> {
    fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key).copied()
    }
}

/// Streaming PGN splitter: yields one [`Game`] per `[Event ...]`-rooted block.
struct GameIter<'a> {
    lines: std::iter::Peekable<std::str::Lines<'a>>,
}

impl<'a> GameIter<'a> {
    fn new(text: &'a str) -> GameIter<'a> {
        GameIter {
            lines: text.lines().peekable(),
        }
    }
}

impl<'a> Iterator for GameIter<'a> {
    type Item = Game<'a>;

    fn next(&mut self) -> Option<Game<'a>> {
        // skip blank lines between games
        while matches!(self.lines.peek(), Some(l) if l.trim().is_empty()) {
            self.lines.next();
        }
        self.lines.peek()?; // EOF

        let mut headers = HashMap::new();
        // header block: lines starting with '['
        while let Some(line) = self.lines.peek() {
            let line = line.trim_start();
            if !line.starts_with('[') {
                break;
            }
            let line = self.lines.next().unwrap().trim();
            if let Some((k, v)) = parse_header(line) {
                headers.insert(k, v);
            }
        }
        // skip the blank separator line(s) between headers and movetext
        while matches!(self.lines.peek(), Some(l) if l.trim().is_empty()) {
            self.lines.next();
        }
        // movetext: accumulate non-blank lines until the blank line that ends
        // the game, the next game's header block, or EOF. (Movetext can span
        // multiple physical lines.)
        let mut movetext = String::new();
        while let Some(line) = self.lines.peek() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                self.lines.next(); // consume the game-terminating blank
                break;
            }
            if trimmed.starts_with('[') {
                break; // next game's headers (no blank separator)
            }
            movetext.push_str(line);
            movetext.push(' ');
            self.lines.next();
        }

        Some(Game { headers, movetext })
    }
}

/// Parses a `[Key "value"]` header line.
fn parse_header(line: &str) -> Option<(&str, &str)> {
    let line = line.strip_prefix('[')?.strip_suffix(']')?;
    let (key, rest) = line.split_once(' ')?;
    let value = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some((key.trim(), value))
}

/// Applies filters, replays up to `max_book_plies`, accumulates stats.
/// Returns `Some(positions_added)` if the game was kept, else `None`.
fn process_game(game: &Game, args: &Args, table: &mut HashMap<(u64, u16), Stat>) -> Option<u64> {
    // --- header filters ---
    let welo = parse_elo(game.header("WhiteElo"))?;
    let belo = parse_elo(game.header("BlackElo"))?;
    if welo < args.min_elo || belo < args.min_elo {
        return None;
    }
    if game.header("WhiteTitle") == Some("BOT") || game.header("BlackTitle") == Some("BOT") {
        return None;
    }
    if let Some(tc) = game.header("TimeControl") {
        if let Some(base) = time_control_base(tc) {
            if base < 180 {
                return None;
            }
        }
    }

    // --- result -> per-mover score ---
    let result = game.header("Result").unwrap_or("*");
    let (white_pts, black_pts): (u64, u64) = match result {
        "1-0" => (2, 0),
        "0-1" => (0, 2),
        "1/2-1/2" => (1, 1),
        _ => return None, // unfinished/unknown: not useful for a book
    };

    // --- tokenize movetext, replay, count plies ---
    let tokens = tokenize_moves(&game.movetext);
    if tokens.len() < args.min_plies {
        return None;
    }

    let mut pos = Position::startpos();
    let mut added = 0u64;
    for (ply, san) in tokens.iter().enumerate() {
        if ply >= args.max_book_plies {
            break;
        }
        let Some(mv) = find_san_move(&pos, san) else {
            // unresolvable SAN (malformed token / variation leak): stop this
            // game here — the position chain is no longer trustworthy.
            break;
        };
        let key = polyglot_key(&pos);
        let pg_move = encode_polyglot_move(&pos, mv);
        let mover = pos.stm();
        let score = if mover == Color::White {
            white_pts
        } else {
            black_pts
        };
        let cell = table.entry((key, pg_move)).or_default();
        cell.count += 1;
        cell.score += score;
        added += 1;

        if !pos.make(mv) {
            break; // SAN resolved but illegal in context (should not happen)
        }
    }
    Some(added)
}

/// Parses an Elo header; `None` if missing or non-numeric.
fn parse_elo(v: Option<&str>) -> Option<u32> {
    v.and_then(|s| s.trim().parse::<u32>().ok())
}

/// Extracts the base seconds from a PGN `TimeControl` value ("180+2",
/// "300", "40/9000", "-", "?"). `None` when there is no usable base.
fn time_control_base(tc: &str) -> Option<u64> {
    // take the first field (multi-period TCs are 'a/b:c/d')
    let first = tc.split(':').next().unwrap_or(tc);
    // "moves/seconds" -> take the seconds part; plain "seconds+inc" -> base
    let secs = first.rsplit('/').next().unwrap_or(first);
    let base = secs.split('+').next().unwrap_or(secs).trim();
    base.parse::<u64>().ok()
}

/// Splits movetext into SAN tokens, discarding move numbers, results,
/// comments `{...}`, NAGs `$n`, and parenthesised variations `(...)`.
fn tokenize_moves(movetext: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = movetext.as_bytes();
    let mut i = 0;
    let mut depth_paren = 0i32; // variation nesting
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'{' => {
                // comment to matching '}'
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                i += 1;
            }
            b'(' => {
                depth_paren += 1;
                i += 1;
            }
            b')' => {
                depth_paren -= 1;
                i += 1;
            }
            b'$' => {
                // NAG: skip '$' and following digits
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'{'
                    && bytes[i] != b'('
                    && bytes[i] != b')'
                {
                    i += 1;
                }
                if depth_paren > 0 {
                    continue; // inside a variation: ignore the token
                }
                let tok = &movetext[start..i];
                if let Some(san) = clean_token(tok) {
                    out.push(san.to_string());
                }
            }
        }
    }
    out
}

/// Filters a raw movetext token down to a SAN move, or `None` for move
/// numbers / results / decorations. Strips trailing `!?+#` and check glyphs.
fn clean_token(tok: &str) -> Option<&str> {
    // results
    if matches!(tok, "1-0" | "0-1" | "1/2-1/2" | "*") {
        return None;
    }
    // move number like "12." or "12..." or "1...": digits then dots
    let head = tok.trim_end_matches('.');
    if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // a token that still ends in '.' after the digit check is a numbered move
    // glued to nothing useful — drop the dotted prefix if present
    let tok = match tok.split_once('.') {
        // "12.e4" style (no space after number) -> keep the move part
        Some((num, rest)) if !rest.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => tok,
    };
    let tok = tok.trim_end_matches(['!', '?', '+', '#']);
    if tok.is_empty() {
        return None;
    }
    Some(tok)
}

/// Encodes our `Move` into a PolyGlot 16-bit move (the same layout
/// `decode_move` reads): to(0..6), from(6..12), promo(12..15). Castling uses
/// the king-takes-rook convention (e1h1 / e1a1 / e8h8 / e8a8).
fn encode_polyglot_move(pos: &Position, mv: Move) -> u16 {
    let (from, to) = (mv.from(), mv.to());
    // castling: emit the rook square as the destination
    let to = match mv.flag() {
        Move::KING_CASTLE => match pos.stm() {
            Color::White => Square::H1,
            Color::Black => Square::H8,
        },
        Move::QUEEN_CASTLE => match pos.stm() {
            Color::White => Square::A1,
            Color::Black => Square::A8,
        },
        _ => to,
    };
    let promo: u16 = if mv.is_promotion() {
        match mv.promotion_piece_type() {
            PieceType::Knight => 1,
            PieceType::Bishop => 2,
            PieceType::Rook => 3,
            PieceType::Queen => 4,
            _ => 0,
        }
    } else {
        0
    };
    (to.file() as u16)
        | ((to.rank() as u16) << 3)
        | ((from.file() as u16) << 6)
        | ((from.rank() as u16) << 9)
        | (promo << 12)
}

/// Writes sorted `(key, move, weight)` entries as 16-byte big-endian PolyGlot
/// records (learn field zeroed). Returns the byte count written.
fn write_book(path: &str, entries: &[(u64, u16, u16)]) -> std::io::Result<usize> {
    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    let mut buf = [0u8; 16];
    for &(key, mv, weight) in entries {
        buf[0..8].copy_from_slice(&key.to_be_bytes());
        buf[8..10].copy_from_slice(&mv.to_be_bytes());
        buf[10..12].copy_from_slice(&weight.to_be_bytes());
        buf[12..16].copy_from_slice(&0u32.to_be_bytes());
        w.write_all(&buf)?;
    }
    w.flush()?;
    Ok(entries.len() * 16)
}
