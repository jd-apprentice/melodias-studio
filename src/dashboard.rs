//! Local, read-only dashboard over the history database.
//!
//! It serves one page plus a small JSON summary from `127.0.0.1`. The page
//! polls the summary once a minute, so a finished run shows up without a
//! reload. The server never writes to the database and never creates it.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::history;

pub const DEFAULT_PORT: u16 = 7373;
pub const DEFAULT_LIMIT: usize = 200;

const INDEX: &str = include_str!("../dashboard/index.html");
const APP_JS: &str = include_str!("../dashboard/app.js");
const STYLE_CSS: &str = include_str!("../dashboard/style.css");

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Totals {
    pub runs: i64,
    pub accepted: i64,
    pub by_limit: i64,
    pub cost_usd: f64,
    pub seconds: f64,
    pub technical: Option<f64>,
    pub editorial: Option<f64>,
    pub total: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunPoint {
    pub id: i64,
    pub started_at: String,
    pub slug: String,
    pub vibe: String,
    pub rubric: String,
    pub technical: Option<f64>,
    pub editorial: Option<f64>,
    pub total: Option<f64>,
    pub accepted: bool,
    pub outcome: Option<String>,
    pub outcome_reason: Option<String>,
    pub cost_usd: Option<f64>,
    pub seconds: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FindingPoint {
    pub run_id: i64,
    pub code: String,
    pub source: String,
    pub severity: String,
    pub delta: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub db: String,
    pub exists: bool,
    pub generated_at: String,
    pub limit: usize,
    pub totals: Totals,
    pub rubrics: Vec<String>,
    pub runs: Vec<RunPoint>,
    pub findings: Vec<FindingPoint>,
}

/// Serves the dashboard until the process is killed. Each request opens its
/// own read-only connection, so a run in progress is never blocked.
pub fn serve(db: PathBuf, port: u16, limit: usize) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).with_context(|| {
        format!("binding 127.0.0.1:{port}, is another dashboard already running?")
    })?;
    println!("dashboard: http://127.0.0.1:{port}/");
    println!("database: {}", db.display());
    println!("read-only · polling the file · Ctrl+C stops it");
    let db = Arc::new(db);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let db = Arc::clone(&db);
        thread::spawn(move || {
            if let Err(error) = handle(stream, &db, limit) {
                eprintln!("dashboard: {error:#}");
            }
        });
    }
    Ok(())
}

fn handle(mut stream: TcpStream, db: &Path, default_limit: usize) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let Some((method, target)) = request_target(&line) else {
        return write_response(&mut stream, Response::error(400, "bad request"), false);
    };
    // Drain the headers so closing the socket does not reset the connection.
    let mut header = String::new();
    loop {
        header.clear();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }
    if method != "GET" && method != "HEAD" {
        return write_response(
            &mut stream,
            Response::error(405, "only GET and HEAD are allowed"),
            method == "HEAD",
        );
    }
    let response = route(target, db, default_limit);
    write_response(&mut stream, response, method == "HEAD")
}

fn request_target(line: &str) -> Option<(&str, &str)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    Some((method, target))
}

fn route(target: &str, db: &Path, default_limit: usize) -> Response {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/" | "/index.html" => Response::ok("text/html; charset=utf-8", INDEX),
        "/app.js" => Response::ok("text/javascript; charset=utf-8", APP_JS),
        "/style.css" => Response::ok("text/css; charset=utf-8", STYLE_CSS),
        "/api/summary" => {
            let limit = query_limit(query).unwrap_or(default_limit).clamp(1, 1000);
            match summary(db, limit).and_then(|summary| Ok(serde_json::to_vec(&summary)?)) {
                Ok(body) => Response {
                    status: 200,
                    content_type: "application/json",
                    body,
                },
                Err(error) => {
                    eprintln!("dashboard: {error:#}");
                    Response::error(
                        500,
                        "the history database could not be read; check the database path",
                    )
                }
            }
        }
        _ => Response::error(404, "not found"),
    }
}

fn query_limit(query: &str) -> Option<usize> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key == "limit" {
            value.parse().ok()
        } else {
            None
        }
    })
}

pub fn summary(db: &Path, limit: usize) -> Result<Summary> {
    let exists = db.is_file();
    let mut summary = Summary {
        db: db.display().to_string(),
        exists,
        generated_at: history::now_iso8601(),
        limit,
        totals: Totals::default(),
        rubrics: Vec::new(),
        runs: Vec::new(),
        findings: Vec::new(),
    };
    if !exists {
        return Ok(summary);
    }

    let conn = connect(db)?;
    let has_outcome = history::has_column(&conn, "runs", "outcome")?;
    let has_reason = history::has_column(&conn, "runs", "outcome_reason")?;
    let outcome_expr = if has_outcome { "outcome" } else { "null" };
    let reason_expr = if has_reason { "outcome_reason" } else { "null" };
    let by_limit = if has_outcome {
        "coalesce(sum(case when outcome = 'accepted_by_limit' then 1 else 0 end), 0)"
    } else {
        "0"
    };
    let totals_sql = format!(
        "select count(*), coalesce(sum(accepted), 0), {by_limit}, coalesce(sum(cost_usd), 0.0),
                coalesce(sum(seconds), 0.0), avg(technical), avg(editorial), avg(total)
         from runs"
    );
    summary.totals = conn.query_row(&totals_sql, [], |row| {
        Ok(Totals {
            runs: row.get(0)?,
            accepted: row.get(1)?,
            by_limit: row.get(2)?,
            cost_usd: row.get(3)?,
            seconds: row.get(4)?,
            technical: row.get(5)?,
            editorial: row.get(6)?,
            total: row.get(7)?,
        })
    })?;

    let mut rubrics = conn.prepare("select distinct rubric from runs order by rubric")?;
    summary.rubrics = rubrics
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    let runs_sql = format!(
        "select id, started_at, slug, vibe, rubric, technical, editorial, total,
                accepted, cost_usd, seconds, {outcome_expr} as outcome,
                {reason_expr} as outcome_reason
         from runs order by id desc limit ?1"
    );
    let mut runs = conn.prepare(&runs_sql)?;
    summary.runs = runs
        .query_map([limit as i64], |row| {
            Ok(RunPoint {
                id: row.get(0)?,
                started_at: row.get(1)?,
                slug: row.get(2)?,
                vibe: row.get(3)?,
                rubric: row.get(4)?,
                technical: row.get(5)?,
                editorial: row.get(6)?,
                total: row.get(7)?,
                accepted: row.get(8)?,
                cost_usd: row.get(9)?,
                seconds: row.get(10)?,
                outcome: row.get(11)?,
                outcome_reason: row.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut findings = conn.prepare(
        "select run_id, code, source, severity, delta
         from findings
         where run_id in (select id from runs order by id desc limit ?1)
         order by run_id desc, code",
    )?;
    summary.findings = findings
        .query_map([limit as i64], |row| {
            Ok(FindingPoint {
                run_id: row.get(0)?,
                code: row.get(1)?,
                source: row.get(2)?,
                severity: row.get(3)?,
                delta: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(summary)
}

/// Read-only first. The fallback exists only for the WAL edge case where a
/// reader cannot map the shared memory file; it still runs SELECTs only, and
/// it never creates a missing database.
fn connect(db: &Path) -> Result<Connection> {
    let conn = match Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(primary) => {
            if !db.is_file() {
                return Err(primary).with_context(|| format!("opening {}", db.display()));
            }
            Connection::open(db).with_context(|| format!("opening {}", db.display()))?
        }
    };
    conn.busy_timeout(Duration::from_secs(2))?;
    Ok(conn)
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn ok(content_type: &'static str, body: &str) -> Self {
        Self {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
        }
    }

    fn error(status: u16, message: &str) -> Self {
        let body = serde_json::to_vec(&serde_json::json!({ "error": message }))
            .unwrap_or_else(|_| b"{\"error\":\"internal error\"}".to_vec());
        Self {
            status,
            content_type: "application/json",
            body,
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

fn write_response(
    stream: &mut TcpStream,
    response: Response,
    head_only: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nContent-Security-Policy: default-src 'self'\r\n\
         Connection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes())?;
    if !head_only {
        stream.write_all(&response.body)?;
    }
    stream.flush()
}

#[cfg(test)]
#[path = "tests/dashboard.rs"]
mod tests;
