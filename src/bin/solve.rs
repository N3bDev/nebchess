//! EPD tactical-suite runner. Usage:
//!   solve <suite.epd> [movetime_ms=1000]
//! EPD line: <fen4> bm <san...>; id "name"; ...
//! Scores a position when the searched bestmove matches ANY listed bm.

use nebchess::board::movegen::find_san_move;
use nebchess::board::Position;
use nebchess::eval::Hce;
use nebchess::search::limits::Limits;
use nebchess::search::SearchThread;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .expect("usage: solve <suite.epd> [movetime_ms]");
    let movetime: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let data = std::fs::read_to_string(path).expect("read suite");

    let (mut solved, mut total) = (0u32, 0u32);
    let mut misses = Vec::new();
    for line in data.lines() {
        let Some(bm_at) = line.find(" bm ") else {
            continue;
        };
        let fen4 = &line[..bm_at];
        let rest = &line[bm_at + 4..];
        let bms: Vec<&str> = rest
            .split(';')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let id = line
            .split("id \"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("?");
        let Ok(pos) = Position::from_fen(&format!("{fen4} 0 1")) else {
            eprintln!("bad fen: {id}");
            continue;
        };
        let targets: Vec<String> = bms
            .iter()
            .filter_map(|san| find_san_move(&pos, san))
            .map(|m| m.to_string())
            .collect();
        if targets.is_empty() {
            eprintln!("unresolvable bm in {id}: {bms:?}");
            continue;
        }
        total += 1;
        let mut st = SearchThread::new(pos, Hce::new());
        let limits = Limits {
            movetime: Some(movetime),
            ..Limits::default()
        };
        let best = st.iterate(&limits, |_| {});
        match best {
            Some(mv) if targets.contains(&mv.to_string()) => solved += 1,
            best => misses.push(format!(
                "{id}: played {} wanted {targets:?}",
                best.map_or("none".into(), |m| m.to_string())
            )),
        }
    }
    for m in &misses {
        println!("MISS {m}");
    }
    println!("Solved: {solved}/{total}");
}
