//! `a=export&format=csv`: the rows of a report as CSV (PHP `fputcsv` with no
//! escape character). Cells that a spreadsheet would interpret as formulas are
//! prefixed with `'` (port of `Reports\Export`).

use serde_json::Value;

use super::dimension;
use super::pretty;
use super::range::Range;

/// Renders the output of `report::build()` as CSV.
pub fn csv(report: &Value) -> String {
    let dim = report["name"].as_str().unwrap_or("");
    let mut cols: Vec<&str> = vec!["key"];
    if pretty::has_labels(dim) {
        cols.push("label");
    }
    let mut metrics = dimension::metrics(dim);
    if dimension::has_bounces(dim) {
        for extra in ["bounces", "duration"] {
            if !metrics.contains(&extra) {
                metrics.push(extra);
            }
        }
    }
    cols.extend(metrics);
    cols.push("share");

    let mut out = String::new();
    put_line(&mut out, cols.iter().map(|c| c.to_string()));
    if let Some(rows) = report["rows"].as_array() {
        for row in rows {
            let line = cols.iter().map(|c| match row.get(*c) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => safe(s),
                Some(Value::Bool(b)) => {
                    if *b {
                        "1".into()
                    } else {
                        String::new()
                    }
                }
                Some(Value::Number(n)) if n.is_f64() => {
                    let s = format!("{:.4}", n.as_f64().unwrap_or(0.0));
                    s.trim_end_matches('0').trim_end_matches('.').to_string()
                }
                Some(Value::Number(n)) => n.to_string(),
                Some(other) => other.to_string(),
            });
            put_line(&mut out, line);
        }
    }
    out
}

/// One `fputcsv($fh, $fields, ',', '"', '')` line.
fn put_line(out: &mut String, fields: impl Iterator<Item = String>) {
    let mut first = true;
    for f in fields {
        if !first {
            out.push(',');
        }
        first = false;
        if f.contains([',', '"', '\n', '\r', '\t', ' ']) {
            out.push('"');
            out.push_str(&f.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(&f);
        }
    }
    out.push('\n');
}

/// `example.com-page-2026-09-17_2026-10-16.csv`
pub fn filename(domain: &str, dim: &str, r: &Range) -> String {
    let safe: String = domain
        .to_ascii_lowercase()
        .bytes()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-' {
                c as char
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}-{dim}-{}_{}.csv", r.from, r.to)
}

/// Neutralises spreadsheet formula injection.
pub fn safe(v: &str) -> String {
    match v.as_bytes().first() {
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r') => format!("'{v}"),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn csv_like_fputcsv() {
        assert_eq!(safe("=cmd()"), "'=cmd()");
        assert_eq!(safe("/ok"), "/ok");
        let rep = json!({"name": "country", "rows": [
            {"key": "us", "label": "United States", "visits": 3, "pageviews": 4, "bounce_rate": 0.5, "avg_duration": 2,
             "bounces": 1, "duration": 6, "share": 1.0},
            {"key": "=x", "label": "say \"hi\"", "visits": 0, "pageviews": 0, "bounce_rate": 0, "avg_duration": 0,
             "bounces": 0, "duration": 0, "share": 0.1218}
        ]});
        assert_eq!(
            csv(&rep),
            "key,label,visits,pageviews,bounce_rate,avg_duration,bounces,duration,share\n\
             us,\"United States\",3,4,0.5,2,1,6,1\n\
             '=x,\"say \"\"hi\"\"\",0,0,0,0,0,0,0.1218\n"
        );
    }
}
