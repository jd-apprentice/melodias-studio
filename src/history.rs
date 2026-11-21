//! Local history: one SQLite file next to the config, written at the end of a
//! run and read by `melodias history`. See RFC 0002, phase 2.
//!
//! The write is best effort: the pipeline logs a failure and keeps the run.
//! `melodias history` only reads, it never creates or migrates the file.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;

use crate::score::{Finding, RunScore};

const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
pragma journal_mode=wal;
create table if not exists runs (
  id integer primary key,
  started_at text not null,
  slug text not null,
  vibe text not null,
  rubric text not null default 'score-v1',
  technical real,
  editorial real,
  total real,
  accepted integer not null,
  outcome text,
  outcome_reason text,
  rounds integer not null,
  cost_usd real,
  seconds real,
  config_json text
);
create table if not exists candidates (
  id integer primary key,
  run_id integer not null references runs(id),
  kind text not null,
  idx integer not null,
  seed integer,
  score real,
  score_json text
);
create table if not exists findings (
  id integer primary key,
  run_id integer not null references runs(id),
  candidate_id integer references candidates(id),
  code text not null,
  source text not null,
  severity text not null,
  metric text,
  value real,
  threshold real,
  delta real not null,
  message text
);
create table if not exists patches (
  id integer primary key,
  run_id integer not null references runs(id),
  code text not null,
  prompt_before text,
  prompt_after text,
  applied_at text not null
);
create index if not exists findings_run on findings(run_id);
create index if not exists findings_code on findings(code);
create index if not exists candidates_run on candidates(run_id);
";

/// Everything the pipeline knows about the run before the score exists.
pub struct RunRecord<'a> {
    pub db: &'a Path,
    pub dir: &'a Path,
    pub slug: &'a str,
    pub vibe: &'a str,
    pub started_at: &'a str,
    pub seconds: f64,
    pub rounds: u32,
    pub patches: &'a [crate::retry::Applied],
    pub outcome: &'a str,
    pub outcome_reason: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Table,
    Json,
    Csv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    Day,
    Week,
    Month,
}

impl By {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    fn expression(self) -> &'static str {
        match self {
            Self::Day => "substr(started_at, 1, 10)",
            Self::Week => "date(started_at, 'weekday 0', '-6 days')",
            Self::Month => "substr(started_at, 1, 7)",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RunRow {
    pub id: i64,
    pub started_at: String,
    pub slug: String,
    pub technical: Option<f64>,
    pub editorial: Option<f64>,
    pub total: Option<f64>,
    pub accepted: bool,
    pub outcome: Option<String>,
    pub outcome_reason: Option<String>,
    pub rounds: i64,
}

#[derive(Debug, Serialize)]
pub struct FindingRow {
    pub code: String,
    pub hits: i64,
    pub lost: Option<f64>,
    pub points: Option<f64>,
    pub accepted: i64,
}

#[derive(Debug, Serialize)]
pub struct TrendRow {
    pub bucket: String,
    pub runs: i64,
    pub technical: Option<f64>,
    pub total: Option<f64>,
    pub accepted: i64,
}

/// Opens, creates or migrates the database. Only the writer uses this.
pub fn open(path: &Path) -> Result<Connection> {
    let created = !path.exists();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = conn.query_row("pragma user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        bail!(
            "{} uses schema version {version}, newer than {SCHEMA_VERSION}",
            path.display()
        );
    }
    conn.execute_batch(SCHEMA)
        .with_context(|| format!("creating the schema in {}", path.display()))?;
    migrate(&conn, version)?;
    if created {
        restrict(path);
    }
    Ok(conn)
}

/// Adds the columns that predate the current version. `create table if not
/// exists` cannot add a column to an existing table, so old databases need an
/// explicit `alter`. The backfilled outcome mirrors the accepted flag.
fn migrate(conn: &Connection, version: i64) -> Result<()> {
    if version < 2 {
        if !has_column(conn, "runs", "outcome")? {
            conn.execute_batch("alter table runs add column outcome text")?;
        }
        if !has_column(conn, "runs", "outcome_reason")? {
            conn.execute_batch("alter table runs add column outcome_reason text")?;
        }
        conn.execute_batch(
            "update runs set outcome = case when accepted = 1 then 'accepted' else 'below_target' end
             where outcome is null",
        )?;
    }
    if version < SCHEMA_VERSION {
        conn.execute_batch(&format!("pragma user_version = {SCHEMA_VERSION}"))?;
    }
    Ok(())
}

/// Readers use it to tolerate a database written before a column existed. The
/// table name is a literal from this module, never input.
pub(crate) fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = conn.prepare(&format!("pragma table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

/// Reads only: no schema creation, no migration, no writes.
fn reader(path: &Path) -> Result<Option<Connection>> {
    if !path.is_file() {
        return Ok(None);
    }
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(Some(conn))
}

/// Writes one run, its pool candidates and the findings of the final pair.
/// A failed write is the caller's problem to log; this function never retries.
pub fn insert(record: &RunRecord<'_>, score: &RunScore) -> Result<i64> {
    let mut conn = open(record.db)?;
    let tx = conn.transaction()?;
    let meta = read_json(&record.dir.join("meta.json"));
    tx.execute(
        "insert into runs (started_at, slug, vibe, rubric, technical, editorial, total,
                           accepted, outcome, outcome_reason, rounds, cost_usd, seconds,
                           config_json)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            record.started_at,
            record.slug,
            record.vibe,
            score.rubric,
            score.technical,
            score.editorial,
            score.total,
            score.accepted,
            record.outcome,
            record.outcome_reason,
            record.rounds,
            cost_usd(record.dir),
            record.seconds,
            meta.as_ref().map(Value::to_string),
        ],
    )?;
    let run_id = tx.last_insert_rowid();

    for (kind, candidates) in [("image", &score.images), ("audio", &score.audios)] {
        for candidate in candidates {
            tx.execute(
                "insert into candidates (run_id, kind, idx, seed, score, score_json)
                 values (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    run_id,
                    kind,
                    candidate.index as i64,
                    candidate.seed,
                    candidate.score,
                    serde_json::to_string(candidate)?,
                ],
            )?;
        }
    }

    // Findings describe the final pair, which has no candidate row, so the
    // candidate stays null. The per-candidate findings live in score_json.
    let findings = score
        .final_image
        .findings
        .iter()
        .chain(score.final_audio.findings.iter())
        .chain(score.gates.iter());
    for finding in findings {
        insert_finding(&tx, run_id, finding)?;
    }

    for patch in record.patches {
        tx.execute(
            "insert into patches (run_id, code, prompt_before, prompt_after, applied_at)
             values (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id,
                patch.code,
                patch.prompt_before,
                patch.prompt_after,
                patch.applied_at,
            ],
        )?;
    }

    tx.commit()?;
    Ok(run_id)
}

fn insert_finding(tx: &rusqlite::Transaction<'_>, run_id: i64, finding: &Finding) -> Result<()> {
    tx.execute(
        "insert into findings (run_id, candidate_id, code, source, severity, metric, value,
                               threshold, delta, message)
         values (?1, null, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            run_id,
            finding.code,
            finding.source,
            finding.severity,
            finding.metric,
            finding.value,
            finding.threshold,
            finding.delta,
            finding.message,
        ],
    )?;
    Ok(())
}

fn read_json(path: &Path) -> Option<Value> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// The gateway reports the debit in `usage.opencode.allowance`; older
/// verdicts only carry the list cost.
fn cost_usd(dir: &Path) -> Option<f64> {
    let verdict = read_json(&dir.join("work/judge/verdict.json"))?;
    verdict
        .pointer("/usage/opencode/allowance")
        .and_then(Value::as_f64)
        .or_else(|| verdict.pointer("/usage/list_cost").and_then(Value::as_f64))
}

pub fn recent(db: &Path, limit: usize) -> Result<Vec<RunRow>> {
    let Some(conn) = reader(db)? else {
        return Ok(Vec::new());
    };
    let outcome = if has_column(&conn, "runs", "outcome")? {
        "outcome"
    } else {
        "null"
    };
    let reason = if has_column(&conn, "runs", "outcome_reason")? {
        "outcome_reason"
    } else {
        "null"
    };
    let sql = format!(
        "select id, started_at, slug, technical, editorial, total, accepted, {outcome}, {reason},
                rounds
         from runs order by id desc limit ?1"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([limit as i64], |row| {
        Ok(RunRow {
            id: row.get(0)?,
            started_at: row.get(1)?,
            slug: row.get(2)?,
            technical: row.get(3)?,
            editorial: row.get(4)?,
            total: row.get(5)?,
            accepted: row.get(6)?,
            outcome: row.get(7)?,
            outcome_reason: row.get(8)?,
            rounds: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn findings(db: &Path) -> Result<Vec<FindingRow>> {
    let Some(conn) = reader(db)? else {
        return Ok(Vec::new());
    };
    let mut statement = conn.prepare(
        "select f.code,
                count(*) as hits,
                round(avg(f.delta), 1) as lost,
                round(sum(f.delta), 1) as points,
                sum(case when r.accepted = 1 then 1 else 0 end) as accepted
         from findings f join runs r on r.id = f.run_id
         group by f.code
         order by lost asc",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(FindingRow {
            code: row.get(0)?,
            hits: row.get(1)?,
            lost: row.get(2)?,
            points: row.get(3)?,
            accepted: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn trend(db: &Path, by: By) -> Result<Vec<TrendRow>> {
    let Some(conn) = reader(db)? else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "select {} as bucket,
                count(*) as runs,
                round(avg(technical), 1),
                round(avg(total), 1),
                coalesce(sum(accepted), 0)
         from runs group by bucket order by bucket",
        by.expression()
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        Ok(TrendRow {
            bucket: row.get(0)?,
            runs: row.get(1)?,
            technical: row.get(2)?,
            total: row.get(3)?,
            accepted: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn render_runs(rows: &[RunRow], format: Format) -> Result<String> {
    if format == Format::Json {
        return Ok(serde_json::to_string_pretty(rows)?);
    }
    if format == Format::Csv {
        let mut out = String::from(
            "id,started_at,slug,technical,editorial,total,accepted,outcome,outcome_reason,rounds\n",
        );
        for row in rows {
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{},{},{},{}",
                row.id,
                csv_field(&row.started_at),
                csv_field(&row.slug),
                field(row.technical),
                field(row.editorial),
                field(row.total),
                row.accepted,
                csv_field(row.outcome.as_deref().unwrap_or("")),
                csv_field(row.outcome_reason.as_deref().unwrap_or("")),
                row.rounds
            );
        }
        return Ok(out);
    }
    if rows.is_empty() {
        return Ok("no runs yet\n".to_string());
    }
    let mut out = format!(
        "{:>4}  {:<25}  {:<40}  {:>6}  {:>4}  {:>6}  {:<14}\n",
        "id", "started", "slug", "tech", "edit", "total", "outcome"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:>4}  {:<25}  {:<40}  {:>6}  {:>4}  {:>6}  {:<14}",
            row.id,
            row.started_at,
            row.slug,
            number(row.technical),
            number(row.editorial),
            number(row.total),
            outcome_label(row)
        );
    }
    Ok(out)
}

fn outcome_label(row: &RunRow) -> &str {
    match row.outcome.as_deref() {
        Some("accepted") => "accepted",
        Some("accepted_by_limit") => "by limit",
        Some("below_target") => "below target",
        Some(other) => other,
        None if row.accepted => "accepted",
        None => "below target",
    }
}

pub fn render_findings(rows: &[FindingRow], format: Format) -> Result<String> {
    if format == Format::Json {
        return Ok(serde_json::to_string_pretty(rows)?);
    }
    if format == Format::Csv {
        let mut out = String::from("code,hits,lost,points,accepted\n");
        for row in rows {
            let _ = writeln!(
                out,
                "{},{},{},{},{}",
                csv_field(&row.code),
                row.hits,
                field(row.lost),
                field(row.points),
                row.accepted
            );
        }
        return Ok(out);
    }
    if rows.is_empty() {
        return Ok("no findings yet\n".to_string());
    }
    let mut out = format!(
        "{:<16}  {:>5}  {:>7}  {:>7}  {:>8}\n",
        "code", "hits", "lost", "points", "accepted"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<16}  {:>5}  {:>7}  {:>7}  {:>8}",
            row.code,
            row.hits,
            number(row.lost),
            number(row.points),
            row.accepted
        );
    }
    Ok(out)
}

pub fn render_trend(rows: &[TrendRow], format: Format) -> Result<String> {
    if format == Format::Json {
        return Ok(serde_json::to_string_pretty(rows)?);
    }
    if format == Format::Csv {
        let mut out = String::from("bucket,runs,technical,total,accepted\n");
        for row in rows {
            let _ = writeln!(
                out,
                "{},{},{},{},{}",
                csv_field(&row.bucket),
                row.runs,
                field(row.technical),
                field(row.total),
                row.accepted
            );
        }
        return Ok(out);
    }
    if rows.is_empty() {
        return Ok("no runs yet\n".to_string());
    }
    let mut out = format!(
        "{:<10}  {:>4}  {:>6}  {:>6}  {:>8}\n",
        "bucket", "runs", "tech", "total", "accepted"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<10}  {:>4}  {:>6}  {:>6}  {:>8}",
            row.bucket,
            row.runs,
            number(row.technical),
            number(row.total),
            row.accepted
        );
    }
    Ok(out)
}

fn number(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "-".to_string())
}

fn field(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "-".to_string())
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// The timestamp the `runs` table keeps: UTC, second precision, sortable.
pub fn now_iso8601() -> String {
    iso8601(SystemTime::now())
}

fn iso8601(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Howard Hinnant's civil_from_days: days since the Unix epoch to a date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
#[path = "tests/history.rs"]
mod tests;
