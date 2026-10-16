//! `webpulse dev gen-logs` must produce byte-identical logs (and the same summary)
//! as tests/gen_logs.php of 0.1.0 for the same options (checked when php and the
//! PHP generator are available).

mod common;

fn have_php_generator() -> bool {
    std::process::Command::new("php")
        .arg("-v")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && common::repo().join("tests/gen_logs.php").is_file()
}

fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn gunzip(p: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let mut v = Vec::new();
    flate2::read::MultiGzDecoder::new(std::fs::File::open(p).unwrap())
        .read_to_end(&mut v)
        .unwrap();
    v
}

#[test]
fn identical_to_the_php_generator() {
    if !have_php_generator() {
        eprintln!("php or tests/gen_logs.php not available: skipped");
        return;
    }
    let base = common::repo().join("tests/tmp/rs-genlogs");
    common::rm(&base);
    for (seed, days, lines, extra) in [
        ("42", "20", "8000", vec![]),
        (
            "7",
            "3",
            "3000",
            vec!["--no-archive", "--tz", "America/New_York"],
        ),
        ("5", "40", "5000", vec!["--max-ts", "1790000000"]),
    ] {
        let (a, b) = (
            base.join(format!("php-{seed}")),
            base.join(format!("rust-{seed}")),
        );
        let mut args: Vec<String> = [
            "--seed",
            seed,
            "--days",
            days,
            "--lines",
            lines,
            "--end",
            "2026-10-10",
        ]
        .map(String::from)
        .to_vec();
        args.extend(extra.iter().map(|s| s.to_string()));
        let php = std::process::Command::new("php")
            .arg(common::repo().join("tests/gen_logs.php"))
            .args(&args)
            .arg("--out")
            .arg(&a)
            .output()
            .unwrap();
        assert!(php.status.success());
        let mut rargs: Vec<String> = vec!["dev".into(), "gen-logs".into()];
        rargs.extend(args.clone());
        rargs.extend(["--out".to_string(), b.display().to_string()]);
        let rust = common::exec(&rargs, &[]);
        assert_eq!(rust.code, 0, "{}", rust.stderr);
        let pj: serde_json::Value = serde_json::from_slice(&php.stdout).unwrap();
        assert_eq!(pj, rust.json, "summary seed {seed}");
        let mut fa = Vec::new();
        files(&a, &mut fa);
        assert!(!fa.is_empty());
        for f in fa {
            let g = b.join(f.strip_prefix(&a).unwrap());
            if f.extension().map(|e| e == "gz").unwrap_or(false) {
                assert!(gunzip(&f) == gunzip(&g), "{}", f.display());
            } else {
                assert!(
                    std::fs::read(&f).unwrap() == std::fs::read(&g).unwrap(),
                    "{}",
                    f.display()
                );
            }
        }
    }
}
