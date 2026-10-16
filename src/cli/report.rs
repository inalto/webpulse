//! `webpulse report --user U --domain D [--range 30d | --from X --to Y]
//! [--name DIM] [--limit N] [--sort COL] [--compare] [--json]`
//! (port of `Cli\Command\ReportCommand`).

use crate::app::App;
use crate::error::{Result, WpError};
use crate::http::ctx::Ctx;
use crate::http::json as pjson;
use crate::reports::range::Range;
use crate::reports::{Invalid, Query, dimension, overview, php, pretty, report};
use serde_json::Value;

/// Options of `webpulse report`.
#[derive(Debug, clap::Args)]
#[command(
    after_help = "Without --name prints the KPIs of the period (with the change versus the previous period). \
With --name prints the ranked table of a dimension: page entry_page exit_page referrer_host referrer_url channel \
search_engine keyword campaign utm_source utm_medium source_medium country city browser browser_version os os_version \
device status method protocol http_version file_type static_file download not_found not_found_referrer server_error bot \
visit_duration pages_per_visit.\nRanges: today yesterday 7d 30d 90d 12m month lastmonth year all"
)]
pub struct Args {
    /// cPanel account
    #[arg(long)]
    pub user: Option<String>,
    /// Domain of the account
    #[arg(long)]
    pub domain: Option<String>,
    /// Range shortcut (default 30d)
    #[arg(long)]
    pub range: Option<String>,
    /// First day (YYYY-MM-DD, with --to)
    #[arg(long)]
    pub from: Option<String>,
    /// Last day (YYYY-MM-DD, with --from)
    #[arg(long)]
    pub to: Option<String>,
    /// Dimension to print instead of the KPIs
    #[arg(long)]
    pub name: Option<String>,
    /// Rows (default 10)
    #[arg(long)]
    pub limit: Option<String>,
    /// Sort column, `-` prefix for descending
    #[arg(long)]
    pub sort: Option<String>,
    /// Include the previous period
    #[arg(long)]
    pub compare: bool,
}

fn usage(e: anyhow::Error) -> anyhow::Error {
    match e.downcast::<Invalid>() {
        Ok(i) => WpError::Usage(i.0).into(),
        Err(e) => e,
    }
}

/// Runs the command.
pub fn run(app: &App, a: &Args) -> Result<i32> {
    let user = a.user.clone().unwrap_or_default();
    let domain = a.domain.clone().unwrap_or_default().to_ascii_lowercase();
    if !crate::paths::is_valid_user(&user) || !crate::http::resolver::valid_domain(&domain) {
        return Err(WpError::Usage(
            "--user and --domain are required and must be valid names".into(),
        )
        .into());
    }
    let ctx = Ctx::new(app);
    let d = ctx.domain_data(&user, &domain, ctx.cfg_i64("raw_retention_days", 30))?;
    let mut q = Query::new();
    for (k, v) in [("range", &a.range), ("from", &a.from), ("to", &a.to)] {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
            q.set(k, v.clone());
        }
    }
    let first = d.first_day();
    let range = Range::from_query(&q, d.tz, d.now(), first.as_deref()).map_err(usage)?;

    let (data, human) = match a.name.as_ref().filter(|n| !n.is_empty()) {
        None => {
            let ov = overview::build(&d, &range, a.compare)?;
            let h = render_overview(&domain, &range, &ov);
            (ov, h)
        }
        Some(name) => {
            let mut pq = Query::new();
            pq.set("name", name.clone());
            if let Some(l) = &a.limit {
                pq.set("limit", l.clone());
            }
            if let Some(s) = &a.sort {
                pq.set("sort", s.clone());
            }
            if a.compare {
                pq.set("compare", "1");
            }
            let p = report::params(&pq, false).map_err(usage)?;
            let rep = report::build(&d, &range, &p)?;
            let h = render_report(&domain, &range, &rep);
            (rep, h)
        }
    };
    if app.global.json {
        println!("{}", pjson::encode(&data, pjson::CLI));
    } else if !app.global.quiet {
        println!("{human}");
    }
    Ok(0)
}

/// `Cli::table()`: aligned text table (byte widths, two-space gutters).
pub fn table(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut w: Vec<usize> = headers.iter().map(String::len).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i >= w.len() {
                w.push(0);
            }
            w[i] = w[i].max(c.len());
        }
    }
    let fmt = |line: &[String]| -> String {
        let cells: Vec<String> = line
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{s}{}", " ".repeat(w[i].saturating_sub(s.len()))))
            .collect();
        cells.join("  ").trim_end().to_string()
    };
    let mut out = vec![
        fmt(headers),
        fmt(&w.iter().map(|x| "-".repeat(*x)).collect::<Vec<_>>()),
    ];
    for r in rows {
        out.push(fmt(r));
    }
    out.join("\n")
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// Human rendering of the overview KPIs.
pub fn render_overview(domain: &str, r: &Range, ov: &Value) -> String {
    let units = [
        ("visits", 'n'),
        ("visitors", 'n'),
        ("pageviews", 'n'),
        ("pages_per_visit", 'd'),
        ("avg_duration", 't'),
        ("bounce_rate", '%'),
        ("bytes", 'b'),
        ("bot_share", '%'),
        ("downloads", 'n'),
        ("errors_4xx", 'n'),
        ("errors_5xx", 'n'),
    ];
    let mut rows = Vec::new();
    for (k, u) in units {
        let kv = &ov["kpi"][k];
        let f = |v: &Value| match u {
            'n' => php::number_format(num(v), 0),
            'd' => php::number_format(num(v), 2),
            't' => pretty::duration(num(v)),
            '%' => format!("{}%", php::number_format(num(v) * 100.0, 1)),
            _ => pretty::bytes(num(v)),
        };
        let chg = if kv["change"].is_null() {
            "-".to_string()
        } else {
            format!("{:+.1}%", num(&kv["change"]) * 100.0)
        };
        rows.push(vec![k.to_string(), f(&kv["value"]), f(&kv["prev"]), chg]);
    }
    format!(
        "{domain}  {} → {} ({} days, previous {} → {})\n\n{}",
        r.from,
        r.to,
        r.days,
        r.prev_from,
        r.prev_to,
        table(
            &["metric", "value", "previous", "change"].map(String::from),
            &rows
        )
    )
}

fn strimwidth(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Human rendering of a report table.
pub fn render_report(domain: &str, r: &Range, rep: &Value) -> String {
    let dim = rep["name"].as_str().unwrap_or("");
    let metrics = dimension::metrics(dim);
    let mut rows = Vec::new();
    for row in rep["rows"].as_array().cloned().unwrap_or_default() {
        let label = row
            .get("label")
            .and_then(Value::as_str)
            .or(row["key"].as_str())
            .unwrap_or("");
        let mut line = vec![strimwidth(label, 60)];
        for m in &metrics {
            let v = &row[*m];
            line.push(if v.is_null() {
                "-".into()
            } else if *m == "bytes" {
                pretty::bytes(num(v))
            } else if *m == "avg_duration" {
                pretty::duration(num(v))
            } else if m.ends_with("_rate") {
                format!("{}%", php::number_format(num(v) * 100.0, 1))
            } else {
                php::number_format(num(v), 0)
            });
        }
        line.push(format!(
            "{}%",
            php::number_format(num(&row["share"]) * 100.0, 1)
        ));
        rows.push(line);
    }
    let head = format!(
        "{domain}  {}  {} → {}\n\n",
        dimension::label(dim),
        r.from,
        r.to
    );
    if rows.is_empty() {
        return format!("{head}(no rows)");
    }
    let mut headers = vec![dim.to_string()];
    headers.extend(metrics.iter().map(|m| m.to_string()));
    headers.push("share".into());
    let more = if rep["has_more"].as_bool().unwrap_or(false) {
        format!(
            "\n… {} more rows (use --limit)",
            rep["total"]["rows"].as_i64().unwrap_or(0) - rows.len() as i64
        )
    } else {
        String::new()
    };
    format!("{head}{}{more}", table(&headers, &rows))
}
