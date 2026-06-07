//! UCI protocol (spec §7). Main thread: stdin + master position.
//! Worker thread: one search at a time, aborted via the shared stop flag.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::board::movegen::find_uci_move;
use crate::board::Position;
use crate::book::Book;
use crate::eval::Hce;
use crate::search::limits::Limits;
use crate::search::tt::Tt;
use crate::search::{IterInfo, SearchThread, MATE, MATE_BOUND};

pub const NAME: &str = concat!("NebChess ", env!("CARGO_PKG_VERSION"));

/// Default book cutoff in plies (`BookDepth`): book moves are consulted only
/// while the game ply is below this.
const DEFAULT_BOOK_DEPTH: u32 = 16;

pub fn run() {
    Uci::new().main_loop();
}

struct Uci {
    pos: Position,
    stop: Arc<AtomicBool>,
    search: Option<JoinHandle<()>>,
    overhead_ms: u64,
    tt: Arc<Tt>,
    /// Loaded PolyGlot opening book (`BookFile`); `None` = off.
    book: Option<Book>,
    /// Plies the book will answer before handing off to search (`BookDepth`).
    book_depth: u32,
    /// Plies played into the current game (from the `position` command's move
    /// list). Used as the book cutoff and to vary the per-game RNG seed.
    game_ply: u32,
}

impl Uci {
    fn new() -> Uci {
        Uci {
            pos: Position::startpos(),
            stop: Arc::new(AtomicBool::new(false)),
            search: None,
            overhead_ms: 50,
            tt: Arc::new(Tt::new(16)),
            book: None,
            book_depth: DEFAULT_BOOK_DEPTH,
            game_ply: 0,
        }
    }

    fn main_loop(&mut self) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let cmd = line.split_whitespace().next().unwrap_or("");
            match cmd {
                "uci" => self.cmd_uci(),
                "isready" => println!("readyok"),
                "ucinewgame" => {
                    self.stop_and_join();
                    self.pos = Position::startpos();
                    self.game_ply = 0;
                    self.tt.clear();
                }
                "position" => {
                    self.stop_and_join();
                    self.cmd_position(&line);
                }
                "go" => {
                    self.stop_and_join();
                    self.cmd_go(&line);
                }
                "stop" => self.stop_and_join(),
                "setoption" => self.cmd_setoption(&line),
                // debug extension (not UCI): print the current FEN
                "fen" => println!("{}", self.pos.to_fen()),
                "quit" => {
                    self.stop_and_join();
                    return;
                }
                _ => {} // unknown commands are ignored per UCI custom
            }
            io::stdout().flush().ok();
        }
        self.stop_and_join(); // EOF
    }

    /// Abort any running search and wait for its bestmove to be printed.
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.search.take() {
            if h.join().is_err() {
                // worker panicked before printing bestmove: emit a legal
                // fallback so the GUI doesn't timeout-forfeit the game
                let mv = crate::board::movegen::find_first_legal(&mut self.pos);
                match mv {
                    Some(mv) => println!("bestmove {mv}"),
                    None => println!("bestmove 0000"),
                }
                io::stdout().flush().ok();
            }
        }
    }

    fn cmd_uci(&self) {
        println!("id name {NAME}");
        println!("id author N3bDev");
        // Hash/Threads/MultiPV: accepted but inert in M2 (TT=M3, SMP=M9b);
        // advertised so GUIs can set them without erroring.
        println!("option name Hash type spin default 16 min 1 max 4096");
        println!("option name Threads type spin default 1 min 1 max 1");
        println!("option name MultiPV type spin default 1 min 1 max 1");
        println!("option name Move Overhead type spin default 50 min 0 max 5000");
        // PolyGlot opening book: a file path (empty = off) and a ply cutoff.
        println!("option name BookFile type string default <empty>");
        println!("option name BookDepth type spin default {DEFAULT_BOOK_DEPTH} min 0 max 40");
        println!("uciok");
    }

    fn cmd_setoption(&mut self, line: &str) {
        // setoption name <name words...> value <v...>
        // The value may contain spaces (e.g. a BookFile path), so everything
        // after "value" is taken verbatim as a single string.
        let mut name = Vec::new();
        let mut value: Option<String> = None;
        let mut tok = line.split_whitespace().skip(1); // skip "setoption"
        if tok.next() != Some("name") {
            return;
        }
        let mut value_words: Vec<&str> = Vec::new();
        let mut in_value = false;
        for t in tok {
            if !in_value && t == "value" {
                in_value = true;
            } else if in_value {
                value_words.push(t);
            } else {
                name.push(t);
            }
        }
        if in_value {
            value = Some(value_words.join(" "));
        }
        let name = name.join(" ");
        match (name.as_str(), value) {
            ("Move Overhead", Some(v)) => {
                if let Ok(ms) = v.parse::<u64>() {
                    self.overhead_ms = ms.min(5000);
                }
            }
            ("Hash", Some(v)) => {
                if let Ok(mb) = v.parse::<usize>() {
                    self.stop_and_join();
                    self.tt = Arc::new(Tt::new(mb.clamp(1, 4096)));
                }
            }
            ("BookFile", Some(v)) => {
                self.set_book(&v);
            }
            ("BookDepth", Some(v)) => {
                if let Ok(d) = v.parse::<u32>() {
                    self.book_depth = d.min(40);
                }
            }
            _ => {}
        }
    }

    fn cmd_position(&mut self, line: &str) {
        let mut tok = line.split_whitespace().skip(1); // skip "position"
        let mut saw_moves = false;
        match tok.next() {
            Some("startpos") => {
                self.pos = Position::startpos();
                saw_moves = tok.next() == Some("moves");
            }
            Some("fen") => {
                let mut fen_parts = Vec::new();
                for t in tok.by_ref() {
                    if t == "moves" {
                        saw_moves = true;
                        break;
                    }
                    fen_parts.push(t);
                }
                match Position::from_fen(&fen_parts.join(" ")) {
                    Ok(p) => self.pos = p,
                    Err(e) => {
                        println!("info string {e}");
                        return;
                    }
                }
            }
            _ => return,
        }
        // game_ply = moves applied from the command's base position. The GUI
        // re-sends the whole move list every move, so this stays accurate; it
        // drives the book cutoff and varies the per-game RNG seed.
        self.game_ply = 0;
        if saw_moves {
            for uci in tok {
                match find_uci_move(&self.pos, uci) {
                    Some(mv) if self.pos.make(mv) => self.game_ply += 1,
                    _ => {
                        println!("info string ignoring illegal move {uci}");
                        return;
                    }
                }
            }
        }
    }

    /// Loads (or clears) the opening book from a `BookFile` value. An empty
    /// value or the UCI sentinel `<empty>` turns the book off; an unreadable
    /// path is reported and leaves the book off (never fatal).
    fn set_book(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() || path == "<empty>" {
            self.book = None;
            return;
        }
        match Book::open(path) {
            Ok(b) => {
                println!("info string book loaded: {} entries", b.len());
                self.book = Some(b);
            }
            Err(e) => {
                println!("info string book load failed ({path}): {e}");
                self.book = None;
            }
        }
    }

    /// If the book is loaded and the game is still inside the book window,
    /// returns a book move for the current position (or `None`). The RNG seed
    /// is deterministic per position-per-game but varies across games.
    fn book_move(&self) -> Option<crate::board::Move> {
        let book = self.book.as_ref()?;
        if self.game_ply >= self.book_depth {
            return None;
        }
        book.pick(&self.pos, self.pos.key() ^ self.game_ply as u64)
    }

    fn cmd_go(&mut self, line: &str) {
        // Book short-circuit: a pre-search root move source. When it hits we
        // emit bestmove immediately with no search thread (and no info/time
        // telemetry — there was no search). `go` parameters are ignored, which
        // is correct: a book reply costs ~no clock.
        if let Some(mv) = self.book_move() {
            println!("info string book move {mv}");
            println!("bestmove {mv}");
            io::stdout().flush().ok();
            return;
        }

        let limits = parse_go(line);
        let mut st = SearchThread::new(self.pos.clone(), Hce::new());
        st.set_stop_flag(Arc::clone(&self.stop));
        st.set_overhead_ms(self.overhead_ms);
        st.set_tt(Arc::clone(&self.tt));
        // clear the stop flag on THIS thread before spawn: a worker-side
        // clear races with a GUI 'stop' arriving right after 'go'
        self.stop.store(false, Ordering::Relaxed);
        self.search = Some(std::thread::spawn(move || {
            let best = st.iterate(&limits, print_info);
            // Move-time telemetry (ungated, one line per move): budgeted soft/
            // hard vs ms actually spent — the clock-collapse field instrument.
            let (soft, hard, used) = st.last_move_time();
            let fmt = |o: Option<u64>| o.map_or_else(|| "-".to_string(), |v| v.to_string());
            println!(
                "info string time soft={} hard={} used={used}",
                fmt(soft),
                fmt(hard)
            );
            match best {
                Some(mv) => println!("bestmove {mv}"),
                None => println!("bestmove 0000"), // no legal moves on board
            }
            io::stdout().flush().ok();
        }));
    }
}

fn parse_go(line: &str) -> Limits {
    let mut limits = Limits::default();
    let mut tok = line.split_whitespace().skip(1).peekable();
    while let Some(t) = tok.next() {
        let mut num = |dst: &mut Option<u64>| {
            if let Some(v) = tok.peek().and_then(|s| s.parse::<u64>().ok()) {
                *dst = Some(v);
                tok.next();
            }
        };
        match t {
            "wtime" => num(&mut limits.wtime),
            "btime" => num(&mut limits.btime),
            "winc" => num(&mut limits.winc),
            "binc" => num(&mut limits.binc),
            "movetime" => num(&mut limits.movetime),
            "nodes" => num(&mut limits.nodes),
            "depth" => {
                if let Some(v) = tok.peek().and_then(|s| s.parse::<i32>().ok()) {
                    limits.depth = Some(v);
                    tok.next();
                }
            }
            "movestogo" => {
                if let Some(v) = tok.peek().and_then(|s| s.parse::<u32>().ok()) {
                    limits.movestogo = Some(v);
                    tok.next();
                }
            }
            // ponder isn't advertised; if a GUI sends it anyway, treating the
            // search as infinite is the safe interpretation (stop/quit ends it)
            "infinite" | "ponder" => limits.infinite = true,
            _ => {} // searchmoves etc: ignored in M2
        }
    }
    limits
}

fn print_info(i: IterInfo) {
    let nps = (i.nodes as u128 * 1000)
        .checked_div(i.elapsed_ms)
        .unwrap_or(0) as u64;
    let score = if i.score.abs() >= MATE_BOUND {
        let plies = MATE - i.score.abs();
        let moves = (plies + 1) / 2;
        if i.score > 0 {
            format!("mate {moves}")
        } else {
            format!("mate -{moves}")
        }
    } else {
        format!("cp {}", i.score)
    };
    let mut line = format!(
        "info depth {} score {} nodes {} nps {} time {} pv",
        i.depth, score, i.nodes, nps, i.elapsed_ms
    );
    for mv in i.pv {
        line.push_str(&format!(" {mv}"));
    }
    println!("{line}");
    io::stdout().flush().ok();
}
