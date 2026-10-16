//! The Rust port of `tests/support/seed_db.php` must produce exactly the data
//! the PHP fixture produces (same `mt_rand` stream, rounding and float
//! arithmetic), so that expectations can be cross-checked with the PHP tests.
//! The checksums below were computed from the PHP-built fixture.

mod http_common;

use http_common::*;
use rusqlite::Connection;
use serde_json::{Value, json};

fn checksums(path: &std::path::Path) -> Value {
    let c = Connection::open(path).unwrap();
    let q = |sql: &str| -> Value {
        let mut st = c.prepare(sql).unwrap();
        let n = st.column_count();
        st.query_row([], |r| {
            Ok((0..n)
                .map(|i| r.get::<_, Option<i64>>(i).unwrap().unwrap_or(0))
                .collect::<Vec<i64>>())
        })
        .map(|v| json!(v))
        .unwrap()
    };
    json!([
        q(
            "SELECT SUM(visits),SUM(visitors),SUM(pageviews),SUM(hits),SUM(bytes),SUM(bounces),SUM(duration),SUM(downloads),SUM(errors_4xx),SUM(errors_5xx),SUM(bot_hits),SUM(bot_bytes) FROM daily"
        ),
        q(
            "SELECT COUNT(*),SUM(visits),SUM(pageviews),SUM(hits),SUM(bytes),SUM(bounces),SUM(duration),SUM(length(key)) FROM dim_daily"
        ),
        q(
            "SELECT COUNT(*),SUM(start_ts),SUM(end_ts),SUM(duration),SUM(bytes),SUM(length(entry_page)),SUM(length(country)) FROM visits"
        ),
        q(
            "SELECT COUNT(*),SUM(ts),SUM(bytes),SUM(status),SUM(length(path)),SUM(COALESCE(visit_id,0)) FROM events"
        ),
        q("SELECT COUNT(*),SUM(visit_id),SUM(start_ts),SUM(pageviews) FROM open_visits"),
        q("SELECT SUM(visits),SUM(pageviews),SUM(hits),SUM(bytes),SUM(bot_hits) FROM hourly"),
    ])
}

#[test]
fn php_mt_rand_stream() {
    let mut mt = Mt::new((42 + crc32("example.com") as i64) as u32);
    let mut got: Vec<i64> = (0..8).map(|_| mt.rand(0, 60)).collect();
    got.push(mt.rand(8000, 60000));
    got.push(mt.rand(0, 9));
    assert_eq!(got, vec![59, 49, 6, 59, 41, 34, 1, 12, 58034, 7]);
    assert_eq!(crc32("example.com"), 3069857465);
    let mut mt = Mt::new(5);
    assert_eq!((mt.rand(0, 4294967295), mt.rand(0, 1)), (953453411, 0));
}

#[test]
fn seed_matches_php_fixture() {
    let env = shared("http-seed");
    let data = env.root.join("var/webpulse/data");
    let expect = [
        (
            "alice/example.com",
            json!([
                [
                    11741,
                    9750,
                    31596,
                    94788,
                    2154622743_i64,
                    5330,
                    1238607,
                    474,
                    5704,
                    571,
                    41897,
                    377073000
                ],
                [
                    14850,
                    169893,
                    369370,
                    601754,
                    60685391572_i64,
                    40505,
                    8846704,
                    142875
                ],
                [
                    243,
                    435459133911_i64,
                    435459192991_i64,
                    58218,
                    25552814,
                    2715,
                    486
                ],
                [823, 1474831708832_i64, 97266974, 168680, 10184, 904826],
                [6, 7473, 10752862800_i64, 13],
                [11741, 31596, 94788, 2154622743_i64, 41897]
            ]),
        ),
        (
            "alice/shop.example.com",
            json!([
                [
                    4176, 3466, 11184, 33552, 766889100, 1874, 437509, 163, 2015, 203, 14755,
                    132795000
                ],
                [
                    14684,
                    60399,
                    130845,
                    212899,
                    21562908200_i64,
                    14429,
                    3148824,
                    140047
                ],
                [
                    246,
                    440835358658_i64,
                    440835421655_i64,
                    62477,
                    27786734,
                    2986,
                    492
                ],
                [892, 1598478873503_i64, 91100894, 184724, 11163, 981247],
                [6, 7491, 10752862800_i64, 13],
                [4176, 11184, 33552, 766889100, 14755]
            ]),
        ),
        (
            "bob/blog.net",
            json!([
                [
                    6997, 5811, 18873, 56619, 1278441360, 3095, 721873, 280, 3408, 342, 25261,
                    227349000
                ],
                [
                    14825,
                    101314,
                    220690,
                    359679,
                    36388495840_i64,
                    24084,
                    5269468,
                    142331
                ],
                [
                    246,
                    440835163051_i64,
                    440835219335_i64,
                    54605,
                    24098208,
                    2995,
                    492
                ],
                [786, 1408526637532_i64, 112612368, 162504, 9932, 864034],
                [6, 7491, 10752862800_i64, 13],
                [6997, 18873, 56619, 1278441360, 25261]
            ]),
        ),
    ];
    for (db, exp) in expect {
        assert_eq!(checksums(&data.join(format!("{db}.db"))), exp, "{db}");
    }
    assert_eq!(env.totals["example.com"]["visits"], 11741);
}
