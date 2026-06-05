//! UCI edge-case gates (spec §7): these failures forfeit real games.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use nebchess::board::{movegen::find_uci_move, Position};

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
