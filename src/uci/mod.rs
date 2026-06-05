//! UCI protocol (spec §7). Main thread: stdin + master position.
//! Worker thread: one search at a time, aborted via the shared stop flag.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::board::movegen::find_uci_move;
use crate::board::Position;
use crate::eval::Hce;
use crate::search::limits::Limits;
use crate::search::{IterInfo, SearchThread, MATE, MATE_BOUND};

pub const NAME: &str = concat!("NebChess ", env!("CARGO_PKG_VERSION"));

pub fn run() {
    Uci::new().main_loop();
}

struct Uci {
    pos: Position,
    stop: Arc<AtomicBool>,
    search: Option<JoinHandle<()>>,
    overhead_ms: u64,
}

impl Uci {
    fn new() -> Uci {
        Uci {
            pos: Position::startpos(),
            stop: Arc::new(AtomicBool::new(false)),
            search: None,
            overhead_ms: 50,
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
                    // M3: clear the transposition table here
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
        println!("uciok");
    }

    fn cmd_setoption(&mut self, line: &str) {
        // setoption name <name words...> value <v>
        let mut name = Vec::new();
        let mut value = None;
        let mut tok = line.split_whitespace().skip(1); // skip "setoption"
        if tok.next() != Some("name") {
            return;
        }
        let mut in_value = false;
        for t in tok {
            if t == "value" {
                in_value = true;
            } else if in_value {
                value = Some(t.to_string());
                break;
            } else {
                name.push(t);
            }
        }
        let name = name.join(" ");
        // Hash / Threads / MultiPV: accepted, inert until M3/M9b
        if let ("Move Overhead", Some(v)) = (name.as_str(), value) {
            if let Ok(ms) = v.parse::<u64>() {
                self.overhead_ms = ms.min(5000);
            }
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
        if saw_moves {
            for uci in tok {
                match find_uci_move(&self.pos, uci) {
                    Some(mv) if self.pos.make(mv) => {}
                    _ => {
                        println!("info string ignoring illegal move {uci}");
                        return;
                    }
                }
            }
        }
    }

    fn cmd_go(&mut self, line: &str) {
        let limits = parse_go(line);
        let mut st = SearchThread::new(self.pos.clone(), Hce::new());
        st.set_stop_flag(Arc::clone(&self.stop));
        st.set_overhead_ms(self.overhead_ms);
        // clear the stop flag on THIS thread before spawn: a worker-side
        // clear races with a GUI 'stop' arriving right after 'go'
        self.stop.store(false, Ordering::Relaxed);
        self.search = Some(std::thread::spawn(move || {
            let best = st.iterate(&limits, print_info);
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
