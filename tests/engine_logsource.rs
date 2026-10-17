//! Port of tests/cases/engine_logsource.php.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use webpulse::engine::logsource::{CHUNK, Checkpoint, LogSource, MAX_LINE};

fn dir(name: &str) -> PathBuf {
    let d = common::repo().join("tests/tmp/rs-logsource").join(name);
    common::rm(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn read(s: &mut LogSource) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(l) = s.read_line().unwrap() {
        out.push(String::from_utf8_lossy(&l).into_owned());
    }
    out
}

fn cp(s: &LogSource) -> Checkpoint {
    Checkpoint {
        inode: s.inode() as i64,
        offset: s.offset() as i64,
        size: s.size() as i64,
        first_line: Some(s.fingerprint()),
        last_ts: None,
    }
}

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn open(p: &Path, c: Option<Checkpoint>, gzip: bool) -> LogSource {
    let mut s = LogSource::new(p, false, c, gzip);
    s.open().unwrap();
    s
}

#[test]
fn reads_complete_lines_only_resumes_at_checkpoint() {
    let f = dir("a").join("a.log");
    std::fs::write(&f, "one\ntwo\r\nthree-partial").unwrap();
    let mut s = open(&f, None, false);
    assert_eq!(read(&mut s), ["one", "two"]);
    assert_eq!(s.offset(), 9);
    let c = cp(&s);
    s.close();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&f)
        .unwrap()
        .write_all(b"-done\nfour\n")
        .unwrap();
    let mut s2 = open(&f, Some(c), false);
    assert!(s2.reset_reason().is_none());
    assert_eq!(read(&mut s2), ["three-partial-done", "four"]);
}

#[test]
fn truncation_restarts_from_zero() {
    let f = dir("t").join("t.log");
    std::fs::write(&f, "line\n".repeat(100)).unwrap();
    let mut s = open(&f, None, false);
    read(&mut s);
    let c = cp(&s);
    std::fs::write(&f, "new1\nnew2\n").unwrap();
    let mut s2 = open(&f, Some(c), false);
    assert_eq!(s2.reset_reason(), Some("truncated"));
    assert_eq!(read(&mut s2), ["new1", "new2"]);
}

#[test]
fn inode_change_rotation_restarts_from_zero() {
    let d = dir("r");
    let f = d.join("r.log");
    std::fs::write(&f, "a\nb\n").unwrap();
    let mut s = open(&f, None, false);
    read(&mut s);
    let c = cp(&s);
    s.close();
    std::fs::rename(&f, d.join("r.log.old")).unwrap();
    std::fs::write(d.join("r.log.tmp"), "c\nd\ne\nf\n").unwrap();
    std::fs::rename(d.join("r.log.tmp"), &f).unwrap();
    let mut s2 = open(&f, Some(c), false);
    assert_eq!(s2.reset_reason(), Some("inode"));
    assert_eq!(read(&mut s2), ["c", "d", "e", "f"]);
}

#[test]
fn same_inode_rewritten_content_larger_than_offset_restarts() {
    let f = dir("f").join("f.log");
    std::fs::write(&f, "first line of the old file\nsecond\n").unwrap();
    let mut s = open(&f, None, false);
    read(&mut s);
    let c = cp(&s);
    s.close();
    let mut h = std::fs::OpenOptions::new().write(true).open(&f).unwrap();
    h.set_len(0).unwrap();
    h.write_all("REPLACED content line\n".repeat(10).as_bytes())
        .unwrap();
    drop(h);
    let mut s2 = open(&f, Some(c), false);
    assert_eq!(s2.reset_reason(), Some("replaced"));
    assert_eq!(read(&mut s2).len(), 10);
}

#[test]
fn small_file_growing_past_256_bytes_keeps_its_checkpoint() {
    let f = dir("g").join("g.log");
    std::fs::write(&f, "short\n").unwrap();
    let mut s = open(&f, None, false);
    read(&mut s);
    let c = cp(&s);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&f)
        .unwrap()
        .write_all("more more more more\n".repeat(30).as_bytes())
        .unwrap();
    let mut s2 = open(&f, Some(c), false);
    assert!(s2.reset_reason().is_none());
    assert_eq!(read(&mut s2).len(), 30);
}

#[test]
fn chunk_boundaries_and_huge_lines() {
    let f = dir("big").join("big.log");
    let mut h = std::io::BufWriter::new(std::fs::File::create(&f).unwrap());
    let mut n = 0usize;
    let mut len = 0usize;
    while len < 3 * CHUNK {
        let l = format!("line {} {}", n, "x".repeat((n + 1) % 300));
        n += 1;
        writeln!(h, "{l}").unwrap();
        len += l.len() + 1;
    }
    h.write_all("Z".repeat(3 * CHUNK).as_bytes()).unwrap();
    h.write_all(b"\nafter\n").unwrap();
    drop(h);
    let mut s = open(&f, None, false);
    let lines = read(&mut s);
    assert_eq!(lines.len(), n + 2);
    assert_eq!(lines[0], "line 0 x");
    assert_eq!(lines[n].len(), MAX_LINE);
    assert_eq!(lines[n + 1], "after");
    assert_eq!(s.offset(), std::fs::metadata(&f).unwrap().len());
}

#[test]
fn gzip_archives_multi_member() {
    let f = dir("arch").join("arch.gz");
    let mut data = gz(b"a\nb\n");
    data.extend(gz(b"c\nd-no-newline"));
    std::fs::write(&f, data).unwrap();
    let mut s = LogSource::new(&f, true, None, true);
    s.open().unwrap();
    assert_eq!(read(&mut s), ["a", "b", "c", "d-no-newline"]);
}

#[test]
fn gzip_archive_larger_than_one_chunk() {
    let f = dir("large").join("large.gz");
    let mut data = String::new();
    for i in 0..60000 {
        data.push_str(&format!("line {i} {:x}\n", i * 7919));
    }
    std::fs::write(&f, gz(data.as_bytes())).unwrap();
    let mut s = open(&f, None, true);
    let lines = read(&mut s);
    assert_eq!(lines.len(), 60000);
    assert_eq!(lines[59999], format!("line 59999 {:x}", 59999 * 7919));
}

#[test]
fn corrupt_gzip_raises() {
    let f = dir("bad").join("bad.gz");
    let mut d = b"\x1f\x8b".to_vec();
    d.extend("garbage".repeat(50).as_bytes());
    std::fs::write(&f, d).unwrap();
    let mut s = open(&f, None, true);
    let mut err = false;
    loop {
        match s.read_line() {
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(e) => {
                assert!(e.to_string().contains("corrupt gzip data"));
                err = true;
                break;
            }
        }
    }
    assert!(err);
}

#[test]
fn zero_padding_after_the_last_member_ends_the_stream() {
    let f = dir("pad").join("pad.gz");
    let mut d = gz(b"x\ny\n");
    d.extend([0u8; 1000]);
    std::fs::write(&f, d).unwrap();
    let mut s = open(&f, None, true);
    assert_eq!(read(&mut s), ["x", "y"]);
}

#[test]
fn symlinked_archive_is_refused() {
    let d = dir("link");
    let t = d.join("target.gz");
    std::fs::write(&t, gz(b"x\n")).unwrap();
    std::os::unix::fs::symlink(&t, d.join("link.gz")).unwrap();
    let mut s = LogSource::new(&d.join("link.gz"), false, None, true);
    let e = s.open().unwrap_err().to_string();
    assert!(e.contains("regular file"), "{e}");
}

#[test]
fn rotation_finishes_the_previous_file_before_the_new_one() {
    use std::os::unix::fs::MetadataExt;
    for rotated in ["r.log-20261017", "r.log.1"] {
        let d = dir(&format!("rot-{rotated}"));
        let f = d.join("r.log");
        std::fs::write(&f, "a\nb\n").unwrap();
        let mut s = open(&f, None, false);
        assert_eq!(read(&mut s), ["a", "b"]);
        let c = cp(&s);
        let old_fp = s.fingerprint();
        s.close();
        // written after the last run, then logrotate renames and the server reopens
        std::fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(b"c\npartial-without-newline")
            .unwrap();
        let old_inode = std::fs::metadata(&f).unwrap().ino();
        std::fs::rename(&f, d.join(rotated)).unwrap();
        std::fs::write(&f, "d\ne\n").unwrap();
        let mut s2 = open(&f, Some(c), false);
        assert_eq!(s2.reset_reason(), None, "{rotated}");
        let (from, off) = s2.rotated_from().expect("rotated file found");
        assert_eq!(from, d.join(rotated));
        assert_eq!(off, 4);
        assert_eq!(s2.current_inode(), old_inode);
        assert_eq!(s2.read_line().unwrap().unwrap(), b"c");
        // a checkpoint taken now belongs to the old file
        assert_eq!(s2.current_inode(), old_inode);
        assert_eq!(s2.offset(), 6);
        // size and fingerprint of the old file (29 bytes now), not of the new one
        let (size, fp) = s2.checkpoint_of(old_inode);
        assert_eq!(size, 29);
        assert!(fp.starts_with("29:") && fp != old_fp, "{fp}");
        assert!(s2.checkpoint_of(s2.inode()).1.starts_with("4:"));
        // then the new file from 0 (the unterminated tail of the old one is dropped)
        assert_eq!(read(&mut s2), ["d", "e"], "{rotated}");
        assert_eq!(s2.current_inode(), s2.inode());
        assert_eq!(s2.offset(), 4);
    }
}

#[test]
fn compressed_or_foreign_rotations_are_not_finished() {
    let d = dir("rot-gz");
    let f = d.join("r.log");
    std::fs::write(&f, "a\nb\n").unwrap();
    let mut s = open(&f, None, false);
    read(&mut s);
    let mut c = cp(&s);
    s.close();
    // same inode under a .gz name: a compressed rotation is an archive, not a live tail
    std::fs::rename(&f, d.join("r.log.1.gz")).unwrap();
    std::fs::write(&f, "x\n").unwrap();
    let mut s2 = open(&f, Some(c.clone()), false);
    assert_eq!(s2.reset_reason(), Some("inode"));
    assert_eq!(read(&mut s2), ["x"]);
    // a rotated name whose first bytes differ (another file with a recycled inode)
    std::fs::rename(d.join("r.log.1.gz"), d.join("r.log.1")).unwrap();
    c.first_line = Some("4:0000000000000000000000000000000000000000".into());
    let s3 = open(&f, Some(c), false);
    assert_eq!(s3.reset_reason(), Some("inode"));
}

#[test]
fn plain_archives_for_backfill() {
    let d = dir("plain-archive");
    std::fs::write(d.join("x.log.1"), "p1\np2").unwrap();
    std::fs::write(d.join("x.log.2.gz"), gz(b"g1\ng2\n")).unwrap();
    let mut a = LogSource::archive(&d.join("x.log.1"), false);
    a.open().unwrap();
    assert_eq!(read(&mut a), ["p1", "p2"], "last line without newline kept");
    let mut b = LogSource::archive(&d.join("x.log.2.gz"), false);
    b.open().unwrap();
    assert_eq!(read(&mut b), ["g1", "g2"]);
}
