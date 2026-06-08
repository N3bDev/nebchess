//! UCI protocol (spec §7). Main thread: stdin + master position.
//! Worker thread: one search at a time, aborted via the shared stop flag.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::board::movegen::find_uci_move;
use crate::board::{Color, Position};
use crate::book::Book;
use crate::eval::Hce;
use crate::search::limits::{Limits, TimeManager};
use crate::search::tt::Tt;
use crate::search::{
    IterInfo, PonderArm, PonderHandle, SearchState, SearchThread, MATE, MATE_BOUND,
};

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
    /// Running search worker. It returns the [`SearchState`] via the JoinHandle
    /// so the same allocation can be reused (and, critically, so a ponder search
    /// can hand its state back); reclaimed at join in `stop_and_join`. The
    /// histories do not warm-start — the worker's `iterate` clears them.
    search: Option<JoinHandle<SearchState>>,
    /// Move-ordering histories (butterfly + conthist) owned by the UCI layer.
    /// `Some` while no search runs (it has been reclaimed); `None` while a
    /// search owns it on the worker thread. The allocation is kept alive across
    /// moves for the pondering hand-off, but the tables are cleared at the start
    /// of every search (and on `ucinewgame`) — no cross-move warm-start.
    state: Option<SearchState>,
    overhead_ms: u64,
    tt: Arc<Tt>,
    /// Loaded PolyGlot opening book (`BookFile`); `None` = off.
    book: Option<Book>,
    /// Loaded Syzygy tablebases (`SyzygyPath`); `None` = off. Shared into each
    /// search thread (read-only) via `Arc`.
    tb: Option<Arc<crate::tb::Tb>>,
    /// Plies the book will answer before handing off to search (`BookDepth`).
    book_depth: u32,
    /// Plies played into the current game (from the `position` command's move
    /// list). Used as the book cutoff and to vary the per-game RNG seed.
    game_ply: u32,
    /// A `go ponder` search is running (waiting for `ponderhit` or `stop`).
    /// While set, the running search is infinite and `ponderhit` arms its time
    /// budget. Cleared once the search ends (stop/ponderhit/join).
    pondering: bool,
    /// The ponder arm shared with the running ponder search; `ponderhit` arms
    /// it. `None` when no ponder search is in flight.
    ponder_handle: Option<PonderHandle>,
    /// The REAL clock limits the GUI sent with `go ponder` (wtime/btime/incs/
    /// movestogo per UCI) plus the side to move — stashed so `ponderhit` can
    /// compute the time budget at the moment the opponent's move lands.
    ponder_limits: Option<(Limits, Color)>,
}

impl Uci {
    fn new() -> Uci {
        Uci {
            pos: Position::startpos(),
            stop: Arc::new(AtomicBool::new(false)),
            search: None,
            state: Some(SearchState::new()),
            overhead_ms: 50,
            tt: Arc::new(Tt::new(16)),
            book: None,
            tb: None,
            book_depth: DEFAULT_BOOK_DEPTH,
            game_ply: 0,
            pondering: false,
            ponder_handle: None,
            ponder_limits: None,
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
                    // Fresh game: drop the prior game's move-ordering histories
                    // so they don't bias the new one (the TT is cleared above).
                    if let Some(state) = self.state.as_mut() {
                        state.clear();
                    }
                }
                "position" => {
                    self.stop_and_join();
                    self.cmd_position(&line);
                }
                "go" => {
                    self.stop_and_join();
                    self.cmd_go(&line);
                }
                "ponderhit" => self.cmd_ponderhit(),
                "stop" => self.stop_and_join(),
                "setoption" => self.cmd_setoption(&line),
                // debug extension (not UCI): print the current FEN
                "fen" => println!("{}", self.pos.to_fen()),
                // debug extension (not UCI): print the count of non-zero
                // continuation-history entries on the reclaimed state. Used by
                // the histories-reset test (populated within a `go`, but zero
                // again at the start of the next `go` — no warm-start). Joins
                // any finished/in-flight search first so the state has been
                // reclaimed before we read it.
                "histsum" => {
                    self.stop_and_join();
                    let n = self.state.as_ref().map_or(0, SearchState::conthist_nonzero);
                    println!("histsum {n}");
                }
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

    /// Abort any running search and wait for its bestmove to be printed. The
    /// worker returns its [`SearchState`]; we reclaim it so the allocation is
    /// reused (and so a ponder search can hand its state back). The histories
    /// were cleared at the start of that search — no warm-start. On a worker
    /// panic the state is lost — we install a fresh one (a panic is
    /// exceptional).
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.search.take() {
            match h.join() {
                Ok(state) => self.state = Some(state),
                Err(_) => {
                    // worker panicked before printing bestmove: emit a legal
                    // fallback so the GUI doesn't timeout-forfeit the game
                    let mv = crate::board::movegen::find_first_legal(&mut self.pos);
                    match mv {
                        Some(mv) => println!("bestmove {mv}"),
                        None => println!("bestmove 0000"),
                    }
                    io::stdout().flush().ok();
                    // histories went down with the worker: start fresh.
                    self.state = Some(SearchState::new());
                }
            }
        }
        // The search (ponder or normal) is over: drop the ponder bookkeeping so
        // a stray later `ponderhit` is a no-op (a ponder MISS is exactly this
        // path — the GUI sends `stop` then a fresh `position`/`go`).
        self.pondering = false;
        self.ponder_handle = None;
        self.ponder_limits = None;
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
        // Syzygy tablebases: a directory path (empty = off).
        println!("option name SyzygyPath type string default <empty>");
        // Pondering: think on the opponent's clock (the GUI drives it via
        // `go ponder` / `ponderhit`). The option is advertised so a GUI will
        // enable pondering; the engine ponders whenever a `go ponder` arrives
        // regardless, so the flag is informational on our side.
        println!("option name Ponder type check default false");
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
            ("SyzygyPath", Some(v)) => {
                self.set_tb(&v);
            }
            // Ponder is accepted and inert: we ponder whenever a `go ponder`
            // arrives (GUI-driven), so the toggle needs no engine-side state.
            ("Ponder", Some(_)) => {}
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

    /// Loads (or clears) Syzygy tablebases from a `SyzygyPath` value. An empty
    /// value or the UCI sentinel `<empty>` turns them off; a path with no usable
    /// tables is reported and leaves them off (never fatal). See `Tb::init` for
    /// the pyrrhic-rs process-singleton caveat (set the path once per process).
    fn set_tb(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() || path == "<empty>" {
            self.tb = None;
            return;
        }
        match crate::tb::Tb::init(path) {
            Some(tb) => {
                println!("info string syzygy loaded: up to {}-men", tb.max_men());
                self.tb = Some(Arc::new(tb));
            }
            None => {
                println!("info string syzygy load failed or no tables at: {path}");
                self.tb = None;
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
        // `go ponder`: search on the opponent's clock. We must NOT short-circuit
        // to a book move (the GUI expects no bestmove until ponderhit/stop), and
        // the search runs at infinite until `ponderhit` arms the clock.
        let is_ponder = line.split_whitespace().any(|t| t == "ponder");

        // Book short-circuit: a pre-search root move source. When it hits we
        // emit bestmove immediately with no search thread (and no info/time
        // telemetry — there was no search). `go` parameters are ignored, which
        // is correct: a book reply costs ~no clock. Skipped while pondering.
        if !is_ponder {
            if let Some(mv) = self.book_move() {
                println!("info string book move {mv}");
                println!("bestmove {mv}");
                io::stdout().flush().ok();
                return;
            }
        }

        // `parse_go` maps `ponder` to `infinite` already (the search runs with
        // no deadline). For a ponder search we ALSO keep the real clock limits
        // (wtime/btime/incs/movestogo — parsed into the same Limits) so
        // `ponderhit` can budget the time at the moment the opponent replies.
        let limits = parse_go(line);
        let mut st = SearchThread::new(self.pos.clone(), Hce::new());
        st.set_stop_flag(Arc::clone(&self.stop));
        st.set_overhead_ms(self.overhead_ms);
        st.set_tt(Arc::clone(&self.tt));
        st.set_tb(self.tb.clone());

        if is_ponder {
            // Build the shared arm and hand a clone to the worker; stash the
            // real limits + side-to-move for `ponderhit`.
            let handle: PonderHandle = Arc::new(Mutex::new(PonderArm::default()));
            st.set_ponder(Some(Arc::clone(&handle)));
            self.ponder_handle = Some(handle);
            // The stashed limits drive the budget at ponderhit: drop `infinite`
            // so TimeManager computes real soft/hard from the clock fields.
            let mut real = limits.clone();
            real.infinite = false;
            self.ponder_limits = Some((real, self.pos.stm()));
            self.pondering = true;
        }

        // Thread the SearchState into the worker. This plumbing (state owned by
        // Uci, handed in here and returned via the JoinHandle) is what pondering
        // depends on; the histories themselves do NOT warm-start — `iterate`
        // clears them at the start of the search (the cross-move warm-start
        // regressed −70 elo). `state` is always Some here: stop_and_join ran
        // before this `go` and reclaimed it; the unwrap_or guard is defensive.
        st.set_state(self.state.take().unwrap_or_default());
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
            // Return the state so the UCI loop reclaims it at join (reused next
            // `go`, and required for the ponder hand-off). Its histories were
            // cleared at the start of this search — no cross-move warm-start.
            st.take_state()
        }));
    }

    /// `ponderhit`: the opponent played the predicted move, so the still-running
    /// ponder search must switch from infinite to timed. We do NOT stop and
    /// restart — the search CONTINUES (its work so far cost us nothing: the
    /// opponent's think time was free). We compute the real time budget now
    /// (`start = Instant::now()` — the ponderhit moment) and ARM the shared
    /// handle; the running search then honours the hard deadline in its node
    /// poll and the soft target between iterations, exactly like a normal move.
    ///
    /// If no ponder search is in flight (a stray `ponderhit`), this is a no-op.
    fn cmd_ponderhit(&mut self) {
        if !self.pondering {
            return; // nothing pondering: ignore (UCI custom)
        }
        if let (Some(handle), Some((limits, stm))) =
            (self.ponder_handle.as_ref(), self.ponder_limits.as_ref())
        {
            let now = Instant::now();
            let tm = TimeManager::new(limits, *stm, self.overhead_ms);
            // effective soft (stability-scaled base; un-scaled = base at the
            // start) and the hard ceiling, both as durations from `now`.
            let soft = tm.effective_soft_ms();
            let hard = tm.budgets_ms().1;
            handle
                .lock()
                .expect("ponder arm poisoned")
                .arm(now, soft, hard);
        }
        // The clock is armed; the search owns the rest and will print bestmove
        // when it stops. We are no longer "waiting to ponder" — but the search
        // is still running, so keep the handle/limits until it joins (a `stop`
        // or the next command runs stop_and_join, which clears them).
        self.pondering = false;
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
            // `ponder` searches at infinite (no deadline) until `ponderhit`
            // arms the clock — the same no-deadline path as `infinite`. The
            // real wtime/btime carried alongside are parsed above and stashed by
            // `cmd_go` for the ponderhit budget.
            "infinite" | "ponder" => limits.infinite = true,
            _ => {} // searchmoves etc: ignored
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
