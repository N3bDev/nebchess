//! UCI edge-case gates (spec §7): these failures forfeit real games.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use nebchess::board::{movegen::find_uci_move, Move, Position, Square};
use nebchess::book::polyglot_key;

const T: Duration = Duration::from_secs(5);

struct Engine {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl Engine {
    fn start() -> Engine {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nebchess"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn engine");
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Engine { child, stdin, rx }
    }

    fn send(&mut self, s: &str) {
        writeln!(self.stdin, "{s}").expect("engine stdin");
    }

    /// Lines until (and including) the first one matching `stop`.
    fn collect_until(&mut self, stop: impl Fn(&str) -> bool) -> Vec<String> {
        let deadline = Instant::now() + T;
        let mut out = Vec::new();
        loop {
            let remain = deadline
                .checked_duration_since(Instant::now())
                .expect("timeout waiting for engine output");
            let line = self.rx.recv_timeout(remain).expect("engine output timeout");
            let done = stop(&line);
            out.push(line);
            if done {
                return out;
            }
        }
    }

    fn expect_line(&mut self, pred: impl Fn(&str) -> bool) -> String {
        self.collect_until(pred).pop().unwrap()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        thread::sleep(Duration::from_millis(100));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// bestmove must be legal in the given position (resolved via the lib).
fn assert_legal_bestmove(line: &str, pos: &Position) {
    let mv = line
        .strip_prefix("bestmove ")
        .expect("bestmove line")
        .split_whitespace()
        .next()
        .unwrap();
    assert!(
        find_uci_move(pos, mv).is_some(),
        "illegal bestmove {mv} in {}",
        pos.to_fen()
    );
}

#[test]
fn uci_handshake_lists_identity_and_options() {
    let mut e = Engine::start();
    e.send("uci");
    let lines = e.collect_until(|l| l == "uciok");
    assert!(lines.iter().any(|l| l.starts_with("id name NebChess")));
    assert!(lines
        .iter()
        .any(|l| l.contains("option name Move Overhead")));
    assert!(lines.iter().any(|l| l.contains("option name Hash")));
}

#[test]
fn isready_answers_readyok() {
    let mut e = Engine::start();
    e.send("isready");
    e.expect_line(|l| l == "readyok");
}

#[test]
fn position_replay_matches_library_fen() {
    // full-game replay equivalence: castles both sides + pawn captures
    let moves = "e2e4 e7e5 g1f3 g8f6 f1c4 f8c5 e1g1 e8g8 d2d4 e5d4 c2c3 d4c3 b1c3";
    let mut e = Engine::start();
    e.send(&format!("position startpos moves {moves}"));
    e.send("fen");
    let got = e.expect_line(|l| l.contains(' ') && l.split(' ').count() == 6);
    // compute the same thing through the library
    let mut pos = Position::startpos();
    for m in moves.split(' ') {
        let mv = find_uci_move(&pos, m).expect("test moves are legal");
        assert!(pos.make(mv));
    }
    assert_eq!(got, pos.to_fen(), "UCI replay diverged from library");
}

#[test]
fn go_then_immediate_stop_still_gives_legal_bestmove() {
    let mut e = Engine::start();
    e.send("position startpos moves e2e4 e7e5");
    e.send("go infinite");
    e.send("stop");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    let mut pos = Position::startpos();
    for m in ["e2e4", "e7e5"] {
        let mv = find_uci_move(&pos, m).unwrap();
        pos.make(mv);
    }
    assert_legal_bestmove(&line, &pos);
}

#[test]
fn bestmove_advertises_ponder_move() {
    // UCI pondering is GUI-driven: the engine must name the predicted reply
    // (`bestmove <best> ponder <reply>`) or the GUI never issues `go ponder`.
    // The ponder move is the 2nd PV move and must be legal after the bestmove.
    let mut e = Engine::start();
    e.send("position startpos moves d2d4 g8f6 c2c4 e7e6 g1f3 d7d5");
    e.send("go depth 10");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    let parts: Vec<&str> = line.split(' ').collect();
    assert_eq!(
        parts.len(),
        4,
        "expected `bestmove X ponder Y`, got {line:?}"
    );
    assert_eq!(parts[2], "ponder", "3rd token must be `ponder`: {line:?}");
    // the ponder move must be legal in the position after the bestmove
    let mut pos = Position::startpos();
    for m in ["d2d4", "g8f6", "c2c4", "e7e6", "g1f3", "d7d5", parts[1]] {
        let mv = find_uci_move(&pos, m).expect("legal move");
        pos.make(mv);
    }
    assert!(
        find_uci_move(&pos, parts[3]).is_some(),
        "ponder move {} illegal after bestmove {}",
        parts[3],
        parts[1]
    );
}

#[test]
fn mate_in_one_bestmove_has_no_ponder() {
    // A single-move PV (mate ends the line) → bare bestmove, no ponder token.
    let mut e = Engine::start();
    e.send("position fen 6k1/5ppp/8/8/8/8/8/R6K w - - 0 1");
    e.send("go depth 6");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert!(
        !line.contains(" ponder "),
        "mate-in-1 should emit a bare bestmove, got {line:?}"
    );
}

#[test]
fn isready_during_search_answers_before_bestmove() {
    let mut e = Engine::start();
    e.send("position startpos");
    e.send("go movetime 500");
    e.send("isready");
    let line = e.expect_line(|l| l == "readyok" || l.starts_with("bestmove"));
    assert_eq!(line, "readyok", "isready must not wait for the search");
    e.expect_line(|l| l.starts_with("bestmove"));
}

#[test]
fn ucinewgame_resets_cleanly_between_games() {
    let mut e = Engine::start();
    e.send("ucinewgame");
    e.send("position startpos moves e2e4");
    e.send("go depth 3");
    e.expect_line(|l| l.starts_with("bestmove"));
    e.send("ucinewgame");
    e.send("position startpos");
    e.send("go depth 3");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert_legal_bestmove(&line, &Position::startpos());
}

#[test]
fn go_depth_emits_info_lines_with_pv() {
    let mut e = Engine::start();
    e.send("position startpos");
    e.send("go depth 4");
    let lines = e.collect_until(|l| l.starts_with("bestmove"));
    let infos: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("info depth"))
        .collect();
    assert!(infos.len() >= 4, "one info per completed depth");
    assert!(
        infos.iter().all(|l| l.contains(" pv ")),
        "info lines carry pv"
    );
    assert!(infos
        .iter()
        .all(|l| l.contains(" score cp ") || l.contains(" score mate ")));
}

#[test]
fn illegal_replay_move_is_reported_not_fatal() {
    let mut e = Engine::start();
    e.send("position startpos moves e2e4 e2e4");
    let line = e.expect_line(|l| l.starts_with("info string"));
    assert!(line.contains("illegal"));
    // engine must still be responsive afterwards
    e.send("isready");
    e.expect_line(|l| l == "readyok");
}

#[test]
fn checkmated_position_answers_null_bestmove() {
    // fool's mate delivered: white is mated, no legal moves
    let mut e = Engine::start();
    e.send("position startpos moves f2f3 e7e5 g2g4 d8h4");
    e.send("go depth 3");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert_eq!(line, "bestmove 0000");
}

/// Encodes a from/to/promo into the PolyGlot 16-bit move (to:0..6, from:6..12,
/// promo:12..15) — the layout the book reader decodes.
fn pg_move(from: Square, to: Square, promo: u16) -> u16 {
    (to.file() as u16)
        | ((to.rank() as u16) << 3)
        | ((from.file() as u16) << 6)
        | ((from.rank() as u16) << 9)
        | (promo << 12)
}

/// One 16-byte big-endian PolyGlot entry.
fn pg_entry(key: u64, mv: u16, weight: u16) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&key.to_be_bytes());
    b[8..10].copy_from_slice(&mv.to_be_bytes());
    b[10..12].copy_from_slice(&weight.to_be_bytes());
    // learn (b[12..16]) left zero
    b
}

#[test]
fn book_move_is_played_immediately_without_search() {
    // Hand-build a one-entry book mapping startpos -> e2e4, write it to a temp
    // file, point the engine at it, and confirm `go` returns the book move
    // instantly with the book-move info line and no search info lines.
    let e2 = Square::from_name("e2").unwrap();
    let e4 = Square::from_name("e4").unwrap();
    let key = polyglot_key(&Position::startpos());
    let raw = pg_entry(key, pg_move(e2, e4, 0), 100);

    let mut path = std::env::temp_dir();
    path.push(format!("nebchess_test_book_{}.bin", std::process::id()));
    std::fs::write(&path, raw).expect("write temp book");

    let mut e = Engine::start();
    e.send(&format!("setoption name BookFile value {}", path.display()));
    e.send("position startpos");
    e.send("go depth 30"); // a deep search would take a while; the book short-circuits it
    let lines = e.collect_until(|l| l.starts_with("bestmove"));
    let best = lines.last().unwrap();
    assert_eq!(best, "bestmove e2e4", "book move must be returned");
    assert!(
        lines.iter().any(|l| l.contains("book move")),
        "book-move info line expected, got: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("info depth")),
        "book hit must not run a search, got: {lines:?}"
    );

    // Past BookDepth the book is silent and a normal search runs.
    e.send("setoption name BookDepth value 0");
    e.send("position startpos");
    e.send("go depth 3");
    let lines = e.collect_until(|l| l.starts_with("bestmove"));
    assert!(
        lines.iter().any(|l| l.starts_with("info depth")),
        "with BookDepth 0 a search must run, got: {lines:?}"
    );
    assert_legal_bestmove(lines.last().unwrap(), &Position::startpos());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn book_castling_entry_resolves_to_castle_move() {
    // PolyGlot encodes castling as king-takes-rook (e1h1). The engine must
    // decode it to the legal e1g1 king-castle and play it from the book.
    let fen = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let key = polyglot_key(&pos);
    let raw = pg_entry(key, pg_move(Square::E1, Square::H1, 0), 1);

    let mut path = std::env::temp_dir();
    path.push(format!(
        "nebchess_test_book_castle_{}.bin",
        std::process::id()
    ));
    std::fs::write(&path, raw).expect("write temp book");

    let mut e = Engine::start();
    e.send(&format!("setoption name BookFile value {}", path.display()));
    e.send(&format!("position fen {fen}"));
    e.send("go depth 20");
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert_eq!(line, "bestmove e1g1", "castle decoded from king-takes-rook");
    // the decoded move must be a legal castle in the position
    let mv = find_uci_move(&pos, "e1g1").unwrap();
    assert_eq!(mv.flag(), Move::KING_CASTLE);

    let _ = std::fs::remove_file(&path);
}

impl Engine {
    /// Read the `histsum N` count from the engine's debug command (non-zero
    /// continuation-history entries on the reclaimed state). The command joins
    /// any in-flight search first, so this reflects a settled state.
    fn histsum(&mut self) -> u64 {
        self.send("histsum");
        let line = self.expect_line(|l| l.starts_with("histsum "));
        line.strip_prefix("histsum ")
            .unwrap()
            .trim()
            .parse()
            .expect("histsum count")
    }

    /// Node count from the deepest `info depth …` line of the most recent `go`
    /// (the line just before `bestmove`). Used to observe TT reuse across
    /// moves: a warm TT makes a re-search of the same position far cheaper.
    fn nodes_of_last_go(&mut self) -> u64 {
        let lines = self.collect_until(|l| l.starts_with("bestmove"));
        let info = lines
            .iter()
            .rev()
            .find(|l| l.starts_with("info depth"))
            .expect("an info line before bestmove");
        let toks: Vec<&str> = info.split_whitespace().collect();
        let i = toks
            .iter()
            .position(|&t| t == "nodes")
            .expect("nodes field");
        toks[i + 1].parse().expect("node count")
    }
}

#[test]
fn histories_reset_each_search_while_tt_persists_across_moves() {
    // The carve: the SearchState plumbing stays (the worker returns it via the
    // JoinHandle — pondering needs that), but the move-ordering histories do
    // NOT warm-start across moves (the cross-move warm-start regressed −70
    // elo). They are cleared at the START of every search. The TT is a separate
    // Arc-shared table whose cross-move persistence is correct and unchanged.
    let mut e = Engine::start();
    e.send("ucinewgame");
    e.send("position startpos");
    // before any search the state is zeroed
    assert_eq!(e.histsum(), 0, "histories start empty");

    // go #1: a real search populates the continuation histories
    e.send("position startpos moves e2e4");
    e.send("go depth 10");
    let nodes_go1 = e.nodes_of_last_go();
    let after_go1 = e.histsum();
    assert!(after_go1 > 0, "go #1 populated conthist (got {after_go1})");
    assert!(nodes_go1 > 0, "go #1 searched some nodes");

    // go #2 starts from a FRESH (zeroed) history table — the warm-start was
    // dropped. We prove the reset-at-start with a terminal position: this FEN
    // is a stalemate, so the search returns immediately with no quiet beta
    // cutoff and therefore adds NOTHING to the histories. The reclaimed state
    // is zero ONLY because `iterate` cleared it before bailing on the
    // no-legal-moves root; a warm-start would leave go #1's table intact here.
    e.send("position fen 7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
    e.send("go depth 1");
    e.expect_line(|l| l.starts_with("bestmove"));
    assert_eq!(
        e.histsum(),
        0,
        "histories are FRESH (zeroed) at the start of go #2 — no warm-start"
    );

    // TT persistence (unchanged): re-search the EXACT position from go #1. The
    // persistent TT (never cleared except on ucinewgame / Hash) makes this far
    // cheaper than the cold go #1 — proof the TT survived across the moves
    // above while the histories reset.
    e.send("position startpos moves e2e4");
    e.send("go depth 10");
    let nodes_go3 = e.nodes_of_last_go();
    assert!(
        nodes_go3 < nodes_go1,
        "TT persisted: warm re-search ({nodes_go3}) is cheaper than cold ({nodes_go1})"
    );

    // ucinewgame still zeroes the histories (and clears the TT)
    e.send("ucinewgame");
    assert_eq!(e.histsum(), 0, "ucinewgame cleared the histories");
}

#[test]
fn uci_advertises_ponder_option() {
    let mut e = Engine::start();
    e.send("uci");
    let lines = e.collect_until(|l| l == "uciok");
    assert!(
        lines.iter().any(|l| l.contains("option name Ponder")),
        "Ponder option must be advertised, got: {lines:?}"
    );
}

#[test]
fn ponderhit_arms_the_clock_and_bestmove_arrives_within_budget() {
    // go ponder (infinite) -> ponderhit (arm a real, bounded clock) -> the
    // search must finish (bestmove) on the timed budget, NOT run forever. The
    // clock here (~2s) bounds the search well inside the 5s harness timeout; we
    // additionally assert it lands under 4s, proving the arm actually fired
    // (an un-armed infinite search would hit the 5s timeout and fail).
    let mut e = Engine::start();
    e.send("position startpos moves e2e4 e7e5 g1f3");
    e.send("go ponder wtime 2000 btime 2000");
    // give the ponder search a moment to actually be running before the hit
    thread::sleep(Duration::from_millis(150));
    let hit_at = Instant::now();
    e.send("ponderhit");
    e.expect_line(|l| l.starts_with("bestmove"));
    assert!(
        hit_at.elapsed() < Duration::from_secs(4),
        "bestmove must arrive on the armed budget, took {:?}",
        hit_at.elapsed()
    );
}

#[test]
fn stop_during_ponder_returns_bestmove_promptly() {
    // go ponder -> stop (a ponder MISS / abort): bestmove comes back at once,
    // the same M2 stop discipline as `go infinite` + `stop`.
    let mut e = Engine::start();
    e.send("position startpos moves d2d4 d7d5");
    e.send("go ponder wtime 60000 btime 60000");
    e.send("stop");
    let started = Instant::now();
    let line = e.expect_line(|l| l.starts_with("bestmove"));
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "stop during ponder must return promptly"
    );
    let mut pos = Position::startpos();
    for m in ["d2d4", "d7d5"] {
        pos.make(find_uci_move(&pos, m).unwrap());
    }
    assert_legal_bestmove(&line, &pos);
}

#[test]
fn ponder_storm_never_hangs() {
    // Watchdog mirroring zero_delay_stop_never_hangs but for the ponder states:
    // hammer go-ponder/ponderhit and go-ponder/stop interleavings; any hang or
    // panic fails via the 5s timeout. Preserves the stop-flag-race invariant
    // (cmd_go clears the flag before spawn; ponder adds states, not a race).
    for i in 0..25 {
        let mut e = Engine::start();
        e.send("position startpos moves e2e4");
        if i % 2 == 0 {
            e.send("go ponder wtime 1000 btime 1000");
            e.send("ponderhit");
        } else {
            e.send("go ponder wtime 60000 btime 60000");
            e.send("stop");
        }
        e.expect_line(|l| l.starts_with("bestmove"));
    }
}

#[test]
fn zero_delay_stop_never_hangs() {
    // regression for the stop-flag clear race: repeat the tightest
    // go-infinite/stop interleaving; any hang fails via the 5s timeout
    for _ in 0..25 {
        let mut e = Engine::start();
        e.send("position startpos");
        e.send("go infinite");
        e.send("stop");
        e.expect_line(|l| l.starts_with("bestmove"));
    }
}
