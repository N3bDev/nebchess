fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("bench") => nebchess::search::bench::run(),
        _ => nebchess::uci::run(),
    }
}
