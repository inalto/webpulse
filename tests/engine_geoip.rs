//! Port of tests/cases/engine_geoip.php and sec_mmdb.php (with the maxminddb crate:
//! hostile files must fail fast with an error, never exhaust CPU or memory).
#![allow(clippy::type_complexity)]

mod common;

use std::path::PathBuf;
use std::time::Instant;

use common::{M, MmdbWriter, map, s};
use webpulse::engine::classify::geoip::{GeoIp, open_reader};

fn write(name: &str, bin: &[u8]) -> PathBuf {
    let dir = common::repo().join("tests/tmp/rs-geoip");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join(name);
    std::fs::write(&f, bin).unwrap();
    f
}

fn build(ip_version: u32, record_size: u32, filler: usize) -> Vec<u8> {
    let mut w = MmdbWriter::new(ip_version, record_size, &["IT", "Italy", "Europe"], filler);
    let it = map(&[
        (
            "country",
            map(&[
                ("iso_code", s("IT")),
                ("names", map(&[("en", s("Italy")), ("de", s("Italien"))])),
            ]),
        ),
        (
            "continent",
            map(&[("code", s("EU")), ("names", map(&[("en", s("Europe"))]))]),
        ),
    ]);
    w.insert("151.0.0.0/8", &it);
    w.insert("2.32.0.0/11", &it);
    w.insert(
        "8.8.8.0/24",
        &map(&[
            ("country", map(&[("iso_code", s("US"))])),
            (
                "location",
                map(&[
                    ("latitude", M::Double(37.751)),
                    ("longitude", M::Double(-97.822)),
                ]),
            ),
        ]),
    );
    w.insert(
        "81.2.69.0/24",
        &map(&[
            ("city", map(&[("names", map(&[("en", s("London"))]))])),
            ("country", map(&[("iso_code", s("GB"))])),
            (
                "subdivisions",
                M::Arr(vec![map(&[("names", map(&[("en", s("England"))]))])]),
            ),
            (
                "location",
                map(&[
                    ("latitude", M::Double(51.5142)),
                    ("longitude", M::Double(-0.0931)),
                ]),
            ),
        ]),
    );
    w.insert(
        "203.0.113.0/24",
        &map(&[("registered_country", map(&[("iso_code", s("JP"))]))]),
    );
    w.insert(
        "198.51.100.0/24",
        &map(&[
            ("u16", M::U16(65535)),
            ("u32", M::U32(4294967295)),
            ("u64", M::U64(u64::MAX as u128)),
            ("u128", M::U128(u128::MAX)),
            ("i32", M::I32(-5)),
            ("d", M::Double(3.25)),
            ("f", M::Float(1.5)),
            ("b", M::Bytes(vec![0, 1, 255])),
            ("t", M::Bool(true)),
            (
                "arr",
                M::Arr(vec![M::U32(1), s("two"), map(&[("three", M::U32(3))])]),
            ),
            ("long", s(&"x".repeat(300))),
            ("longer", s(&"y".repeat(70000))),
            ("shared", s("Europe")),
            ("country", map(&[("iso_code", s("NL"))])),
        ]),
    );
    if ip_version == 6 {
        w.insert(
            "2001:db8::/32",
            &map(&[("country", map(&[("iso_code", s("DE"))]))]),
        );
        w.insert(
            "2a01:4f8::/29",
            &map(&[("country", map(&[("iso_code", s("DE"))]))]),
        );
    }
    w.build(&[])
}

#[test]
fn mmdb_lookups_every_tree_and_record_size() {
    for (v, rs) in [(6, 24), (6, 28), (6, 32), (4, 24), (4, 28), (4, 32)] {
        let f = write(&format!("t{v}-{rs}.mmdb"), &build(v, rs, 0));
        let r = open_reader(&f).unwrap();
        assert_eq!(r.metadata().record_size as u32, rs);
        assert_eq!(r.metadata().database_type, "WebPulse-Test");
        let g = GeoIp::new(Some(&f), None, None);
        assert_eq!(
            g.country("151.12.34.56").as_deref(),
            Some("it"),
            "v{v} rs{rs}"
        );
        assert_eq!(g.country("2.40.1.1").as_deref(), Some("it"));
        assert_eq!(g.country("8.8.8.8").as_deref(), Some("us"));
        assert_eq!(
            g.country("198.51.100.1").as_deref(),
            Some("nl"),
            "every data type decodes"
        );
        assert!(g.country("8.8.9.8").is_none());
        assert!(g.country("10.0.0.1").is_none());
        assert!(g.country("not an ip").is_none());
        assert_eq!(
            g.country("::ffff:8.8.8.8").as_deref(),
            Some("us"),
            "IPv4-mapped"
        );
        if v == 6 {
            assert_eq!(g.country("2001:db8:1::1").as_deref(), Some("de"));
            assert_eq!(g.country("2a01:4f8:c0c:1::1").as_deref(), Some("de"));
            assert!(g.country("2001:db9::1").is_none());
        } else {
            assert!(g.country("2001:db8:1::1").is_none());
        }
    }
}

#[test]
fn mmdb_pointer_sizes() {
    for filler in [0, 3000, 600000] {
        let f = write(&format!("types-{filler}.mmdb"), &build(6, 28, filler));
        let g = GeoIp::new(Some(&f), None, None);
        assert_eq!(
            g.country("151.1.1.1").as_deref(),
            Some("it"),
            "pointer with filler {filler}"
        );
        assert_eq!(g.country("198.51.100.1").as_deref(), Some("nl"));
    }
}

#[test]
fn geoip_facade_country_city_registered_country_fallback_no_db() {
    let f = write("facade.mmdb", &build(6, 24, 0));
    let g = GeoIp::new(Some(&f), Some(&f), None);
    assert!(g.available());
    assert_eq!(g.country("151.1.1.1").as_deref(), Some("it"));
    assert_eq!(g.country("203.0.113.5").as_deref(), Some("jp"));
    assert!(g.country("10.1.1.1").is_none());
    let c = g.city("81.2.69.160").unwrap();
    assert_eq!(
        (c.name.as_str(), c.country.as_deref(), c.region.as_deref()),
        ("London", Some("gb"), Some("England"))
    );
    assert!(g.city("8.8.8.8").is_none(), "no city name");
    let none = GeoIp::none();
    assert!(!none.available());
    assert!(none.country("8.8.8.8").is_none());
    assert!(none.city("8.8.8.8").is_none());
    let broken = GeoIp::new(Some(&write("broken.mmdb", b"xx")), None, None);
    assert!(!broken.available());
    assert!(open_reader(&write("bad.mmdb", "nope".repeat(100).as_bytes())).is_err());
    assert!(open_reader(&common::repo().join("tests/tmp/rs-geoip/missing.mmdb")).is_err());
}

#[test]
fn real_dbip_country_lite_database() {
    let f = common::repo().join("tests/fixtures/geoip/dbip-country-lite.mmdb");
    if !f.is_file() {
        eprintln!("no DB-IP fixture: skipped");
        return;
    }
    let g = GeoIp::new(Some(&f), None, None);
    assert_eq!(g.country("8.8.8.8").as_deref(), Some("us"));
    assert_eq!(g.country("151.1.1.1").as_deref(), Some("it"));
    assert_eq!(g.country("81.2.69.160").as_deref(), Some("gb"));
    assert!(g.country("127.0.0.1").is_none());
    assert!(g.country("::1").is_none());
    assert!(g.country("2001:4860:4860::8888").is_some());
    let meta = g.metadata().unwrap();
    assert_eq!(meta["ip_version"], 6);
    assert!(meta["database_type"].as_str().unwrap().contains("Country"));
}

// ---- hostile files (sec_mmdb.php) -------------------------------------------------------------

fn ctrl(t: u8, size: usize) -> Vec<u8> {
    MmdbWriter::ctrl(t, size)
}
fn str_(v: &str) -> Vec<u8> {
    let mut o = ctrl(2, v.len());
    o.extend(v.as_bytes());
    o
}
fn u16_(v: u16) -> Vec<u8> {
    let mut o = ctrl(5, 2);
    o.extend(v.to_be_bytes());
    o
}
fn u32_(v: u32) -> Vec<u8> {
    let mut o = ctrl(6, 4);
    o.extend(v.to_be_bytes());
    o
}
fn u64_(v: u64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let start = b.iter().position(|&x| x != 0).unwrap_or(8);
    let mut o = ctrl(9, 8 - start);
    o.extend(&b[start..]);
    o
}
fn ptr(p: usize) -> Vec<u8> {
    vec![0x20 | (p >> 8) as u8, (p & 0xff) as u8]
}
fn map_(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut o = ctrl(7, entries.len());
    for (k, v) in entries {
        o.extend(str_(k));
        o.extend(v);
    }
    o
}
fn file(data: &[u8], root: usize, meta: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let rec = (1 + 16 + root) as u32;
    let mut out = rec.to_be_bytes()[1..].to_vec();
    out.extend(&rec.to_be_bytes()[1..]);
    out.extend([0u8; 16]);
    out.extend(data);
    out.extend(b"\xAB\xCD\xEFMaxMind.com");
    let mut m: Vec<(String, Vec<u8>)> = vec![
        ("node_count".into(), u32_(1)),
        ("record_size".into(), u16_(24)),
        ("ip_version".into(), u16_(4)),
        ("database_type".into(), str_("Hostile")),
        ("binary_format_major_version".into(), u16_(2)),
        ("binary_format_minor_version".into(), u16_(0)),
        ("build_epoch".into(), u64_(1)),
        ("languages".into(), ctrl(11, 0)),
        ("description".into(), map_(&[])),
    ];
    for (k, v) in meta {
        match m.iter_mut().find(|(x, _)| x == k) {
            Some(e) => e.1 = v.clone(),
            None => m.push((k.to_string(), v.clone())),
        }
    }
    out.extend(map_(&m));
    out
}

#[test]
fn hostile_pointer_to_a_pointer_is_rejected() {
    let mut data = Vec::new();
    for _ in 0..10 {
        let next = data.len() + 2;
        data.extend(ptr(next));
    }
    data.extend(str_("end"));
    let root = data.len();
    let entries: Vec<(String, Vec<u8>)> = (0..28).map(|i| (format!("k{i}"), ptr(0))).collect();
    data.extend(map_(&entries));
    let f = write("chain.mmdb", &file(&data, root, &[]));
    let t = Instant::now();
    let g = GeoIp::new(Some(&f), None, None);
    assert!(g.country("1.2.3.4").is_none());
    if let Ok(r) = open_reader(&f) {
        // the crate either rejects the chain or resolves it boundedly; never hangs
        let _ = r
            .lookup("1.2.3.4".parse().unwrap())
            .and_then(|x| x.decode::<serde_json::Value>());
    }
    assert!(t.elapsed().as_secs_f64() < 1.0);
}

#[test]
fn hostile_pointer_fan_out_cannot_expand_exponentially() {
    let mut data = str_("leaf");
    let mut prev = 0;
    for _ in 0..8 {
        let off = data.len();
        data.extend(ctrl(11, 28));
        for _ in 0..28 {
            data.extend(ptr(prev));
        }
        prev = off;
    }
    let root = data.len();
    let entries: Vec<(String, Vec<u8>)> = (0..28).map(|i| (format!("k{i}"), ptr(prev))).collect();
    data.extend(map_(&entries));
    let f = write("fanout.mmdb", &file(&data, root, &[]));
    let t = Instant::now();
    let g = GeoIp::new(Some(&f), None, None);
    assert!(g.country("1.2.3.4").is_none());
    let r = open_reader(&f).unwrap();
    assert!(
        r.lookup("1.2.3.4".parse().unwrap())
            .unwrap()
            .decode::<serde_json::Value>()
            .is_err(),
        "fan-out is bounded"
    );
    assert!(t.elapsed().as_secs_f64() < 1.0, "fails fast");
    // a moderate fan-out (shared strings, the normal use of pointers) still decodes
    let small = str_("IT");
    let m = map_(&[
        ("a".into(), ptr(0)),
        ("b".into(), ptr(0)),
        ("country".into(), map_(&[("iso_code".into(), ptr(0))])),
    ]);
    let mut d = small.clone();
    d.extend(m);
    let f = write("shared.mmdb", &file(&d, small.len(), &[]));
    assert_eq!(
        GeoIp::new(Some(&f), None, None)
            .country("1.2.3.4")
            .as_deref(),
        Some("it")
    );
}

#[test]
fn hostile_metadata_is_validated_before_the_search_tree_is_used() {
    let data = str_("x");
    let bad: Vec<(&str, Vec<(&str, Vec<u8>)>)> = vec![
        ("huge node count", vec![("node_count", u32_(0x7FFF_FFFF))]),
        ("zero node count", vec![("node_count", u32_(0))]),
        ("bad ip version", vec![("ip_version", u16_(5))]),
        ("bad record size", vec![("record_size", u16_(20))]),
        ("string node count", vec![("node_count", str_("1"))]),
    ];
    for (what, meta) in bad {
        let f = write(
            &format!("meta-{}.mmdb", what.replace(' ', "-")),
            &file(&data, 0, &meta),
        );
        let t = Instant::now();
        let ok = match open_reader(&f) {
            Err(_) => true,
            Ok(r) => {
                r.lookup("1.2.3.4".parse().unwrap())
                    .and_then(|x| x.decode::<serde_json::Value>())
                    .is_err()
                    || GeoIp::new(Some(&f), None, None)
                        .country("1.2.3.4")
                        .is_none()
            }
        };
        assert!(ok, "{what}");
        assert!(t.elapsed().as_secs_f64() < 1.0, "{what}");
    }
    let mut big = ctrl(11, 28);
    for _ in 0..28 {
        big.extend(ctrl(11, 28));
        for _ in 0..28 {
            big.extend(ctrl(11, 28));
            for _ in 0..28 {
                big.extend(str_("z"));
            }
        }
    }
    let f = write("meta-big.mmdb", &file(&data, 0, &[("languages", big)]));
    let t = Instant::now();
    let _ = open_reader(&f);
    assert!(t.elapsed().as_secs_f64() < 2.0);
}
