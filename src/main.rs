//! WebPulse — lightweight, log-based web analytics for cPanel & WHM.
//! One static binary: collector, registry, rollup, reports, JSON API, dashboard, CGI.
//! The modules live in the library target (`src/lib.rs`).

fn main() {
    let code = webpulse::cli::main(std::env::args_os().collect());
    std::process::exit(code);
}
