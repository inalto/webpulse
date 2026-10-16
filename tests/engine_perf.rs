//! Port of tests/cases/engine_perf.php: 200 000 generated lines collected; prints the
//! rate. Expects > 300 000 lines/s with a release build, asserts > 50 000 lines/s
//! (debug builds); `WEBPULSE_SKIP_PERF=1` skips it.

mod common;

use common::{FakeRoot, ts};
use serde_json::json;

#[test]
fn performance_200k_lines() {
    if std::env::var("WEBPULSE_SKIP_PERF").as_deref() == Ok("1") {
        eprintln!("WEBPULSE_SKIP_PERF=1: skipped");
        return;
    }
    let mut fr = FakeRoot::create("rs-perf", json!({}));
    let file = fr.domlog("alice", "example.com-ssl_log");
    let o = common::exec(
        &[
            "dev",
            "gen-logs",
            "--seed",
            "11",
            "--days",
            "14",
            "--lines",
            "200000",
            "--end",
            "2026-10-16",
            "--no-archive",
            "--single",
            &file.display().to_string(),
        ]
        .map(String::from),
        &[],
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    let data = std::fs::read(&file).unwrap();
    let mut n = 0;
    let mut end = 0;
    for (i, b) in data.iter().enumerate() {
        if *b == b'\n' {
            n += 1;
            end = i + 1;
            if n == 200000 {
                break;
            }
        }
    }
    std::fs::write(&file, &data[..end]).unwrap();
    assert_eq!(n, 200000, "generator produced enough lines");
    fr.set_now(ts("2026-10-17 03:00:00"));
    let t = std::time::Instant::now();
    let cpu0 = webpulse::util::time::children_cpu_seconds();
    let r = fr.run("collect --json").ok("collect");
    let cpu = webpulse::util::time::children_cpu_seconds() - cpu0;
    let wall = t.elapsed().as_secs_f64();
    assert_eq!(r.i("lines"), 200000);
    // CPU time of the collect processes: independent of the load of the machine
    let rate = 200000.0 / cpu.max(0.001);
    eprintln!(
        "  # perf: 200000 lines in {wall:.2}s wall ({:.2}s in collect, {cpu:.2}s CPU) = {:.0} lines/s (CPU), peak memory {:.1} MiB",
        r.json["elapsed"].as_f64().unwrap(),
        rate,
        r.i("peak_memory") as f64 / 1048576.0
    );
    assert!(rate > 50_000.0, "throughput {rate:.0} lines/s");
    assert!(r.i("peak_memory") < 128 * 1048576, "peak memory");
    let t = std::time::Instant::now();
    assert_eq!(fr.run("collect --json").ok("collect").i("lines"), 0);
    assert!(t.elapsed().as_secs_f64() < 2.0, "idle run");
}
