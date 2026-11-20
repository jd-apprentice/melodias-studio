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
    summary.totals = conn.query_row(
        "select count(*), coalesce(sum(accepted), 0), coalesce(sum(cost_usd), 0.0),
                coalesce(sum(seconds), 0.0), avg(technical), avg(editorial), avg(total)
         from runs",
        [],
        |row| {
            Ok(Totals {
                runs: row.get(0)?,
                accepted: row.get(1)?,
                cost_usd: row.get(2)?,
                seconds: row.get(3)?,
                technical: row.get(4)?,
                editorial: row.get(5)?,
                total: row.get(6)?,
            })
        },
    )?;

    let mut rubrics = conn.prepare("select distinct rubric from runs order by rubric")?;
    summary.rubrics = rubrics
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    let mut runs = conn.prepare(
        "select id, started_at, slug, vibe, rubric, technical, editorial, total,
                accepted, cost_usd, seconds
         from runs order by id desc limit ?1",
    )?;
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
mod tests {
    use super::*;
    use crate::history::{RunRecord, insert};
    use crate::score::{CandidateScore, RunScore};

    fn temp_db(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "melodias-dashboard-{}-{name}.db",
            std::process::id()
        ));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        path
    }

    fn score() -> RunScore {
        let image = CandidateScore {
            index: 1,
            file: "/tmp/image-01.png".to_string(),
            seed: Some(7),
            selected: true,
            mode: "image",
            score: 58.25,
            points: 29.13,
            evaluated: 50.0,
            findings: Vec::new(),
            skipped: Vec::new(),
        };
        RunScore {
            rubric: crate::score::RUBRIC,
            dir: "/tmp/run".to_string(),
            technical: 72.35,
            editorial: Some(67.0),
            total: 70.21,
            accepted: false,
            gates: Vec::new(),
            images: vec![image.clone()],
            audios: Vec::new(),
            final_image: image,
            final_audio: CandidateScore {
                index: 0,
                file: "/tmp/audio.mp3".to_string(),
                seed: None,
                selected: true,
                mode: "master",
                score: 86.48,
                points: 43.24,
                evaluated: 50.0,
                findings: Vec::new(),
                skipped: Vec::new(),
            },
        }
    }

    #[test]
    fn parses_the_request_line() {
        assert_eq!(
            request_target("GET /api/summary?limit=5 HTTP/1.1\r\n"),
            Some(("GET", "/api/summary?limit=5"))
        );
        assert_eq!(request_target("nonsense"), None);
        assert_eq!(query_limit("limit=5"), Some(5));
        assert_eq!(query_limit("x=1&limit=12"), Some(12));
        assert_eq!(query_limit("limit=abc"), None);
    }

    #[test]
    fn routes_assets_and_errors() {
        let db = temp_db("routes");
        let page = route("/", &db, 10);
        assert_eq!(page.status, 200);
        assert!(String::from_utf8_lossy(&page.body).contains("melodias"));

        let missing = route("/nope", &db, 10);
        assert_eq!(missing.status, 404);

        let json = route("/api/summary", &db, 10);
        assert_eq!(json.status, 200);
        let parsed: Summary = serde_json::from_slice(&json.body).unwrap();
        assert!(!parsed.exists);
        assert!(parsed.runs.is_empty());
    }

    #[test]
    fn summarizes_a_database() {
        let db = temp_db("summary");
        let dir = std::env::temp_dir().join(format!("melodias-dashboard-{}", std::process::id()));
        let record = RunRecord {
            db: &db,
            dir: &dir,
            slug: "epic-8-bit",
            vibe: "epic 8-bit",
            started_at: "2026-11-20T13:40:44Z",
            seconds: 2123.0,
            rounds: 0,
        };
        let mut run = score();
        run.accepted = true;
        insert(&record, &run).unwrap();
        let summary = summary(&db, 20).unwrap();
        assert!(summary.exists);
        assert_eq!(summary.totals.runs, 1);
        assert_eq!(summary.totals.accepted, 1);
        assert_eq!(summary.runs.len(), 1);
        assert_eq!(summary.runs[0].slug, "epic-8-bit");
        assert_eq!(summary.rubrics, vec![crate::score::RUBRIC]);

        let body = route("/api/summary?limit=1", &db, 20).body;
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["limit"], 1);
        assert_eq!(parsed["runs"][0]["technical"], 72.35);

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
        }
    }
}
