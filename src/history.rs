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

const SCHEMA_VERSION: i64 = 1;

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
    if version < SCHEMA_VERSION {
        conn.execute_batch(&format!("pragma user_version = {SCHEMA_VERSION}"))?;
    }
    if created {
        restrict(path);
    }
    Ok(conn)
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
                           accepted, rounds, cost_usd, seconds, config_json)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            record.started_at,
            record.slug,
            record.vibe,
            score.rubric,
            score.technical,
            score.editorial,
            score.total,
            score.accepted,
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
    let mut statement = conn.prepare(
        "select id, started_at, slug, technical, editorial, total, accepted, rounds
         from runs order by id desc limit ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        Ok(RunRow {
            id: row.get(0)?,
            started_at: row.get(1)?,
            slug: row.get(2)?,
            technical: row.get(3)?,
            editorial: row.get(4)?,
            total: row.get(5)?,
            accepted: row.get(6)?,
            rounds: row.get(7)?,
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
        let mut out =
            String::from("id,started_at,slug,technical,editorial,total,accepted,rounds\n");
        for row in rows {
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{},{}",
                row.id,
                csv_field(&row.started_at),
                csv_field(&row.slug),
                field(row.technical),
                field(row.editorial),
                field(row.total),
                row.accepted,
                row.rounds
            );
        }
        return Ok(out);
    }
    if rows.is_empty() {
        return Ok("no runs yet\n".to_string());
    }
    let mut out = format!(
        "{:>4}  {:<20}  {:<40}  {:>6}  {:>4}  {:>6}  {}\n",
        "id", "started", "slug", "tech", "edit", "total", "accepted"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:>4}  {:<20}  {:<40}  {:>6}  {:>4}  {:>6}  {}",
            row.id,
            row.started_at,
            row.slug,
            number(row.technical),
            number(row.editorial),
            number(row.total),
            if row.accepted { "yes" } else { "no" }
        );
    }
    Ok(out)
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
mod tests {
    use super::*;
    use crate::score::CandidateScore;
    use std::path::PathBuf;

    fn temp_db(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("melodias-history-{}-{name}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", path.display()));
        }
        path
    }

    fn finding(code: &'static str, delta: f64) -> Finding {
        Finding {
            code,
            source: "deterministic",
            severity: if delta <= -10.0 { "high" } else { "medium" },
            metric: "unique_colors",
            value: 216_853.0,
            threshold: 16_384.0,
            delta,
            message: "216853 colors: full-color render, not pixel art".to_string(),
        }
    }

    fn candidate(
        index: usize,
        seed: u32,
        selected: bool,
        findings: Vec<Finding>,
    ) -> CandidateScore {
        CandidateScore {
            index,
            file: format!("/tmp/image-{index:02}.png"),
            seed: Some(seed),
            selected,
            mode: "image",
            score: 58.25,
            points: 29.13,
            evaluated: 50.0,
            findings,
            skipped: Vec::new(),
        }
    }

    fn score_run(accepted: bool) -> RunScore {
        let lost = finding("IMG_PALETTE", -20.0);
        let build = finding("AUD_BUILD", -3.5);
        RunScore {
            rubric: crate::score::RUBRIC,
            dir: "/tmp/run".to_string(),
            technical: 72.35,
            editorial: Some(67.0),
            total: 70.21,
            accepted,
            gates: Vec::new(),
            images: vec![candidate(1, 11, true, vec![lost.clone()])],
            audios: Vec::new(),
            final_image: candidate(1, 11, true, vec![lost]),
            final_audio: candidate(0, 0, true, vec![build.clone()]),
        }
    }

    #[test]
    fn formats_utc_timestamps() {
        let epoch = UNIX_EPOCH;
        assert_eq!(iso8601(epoch), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso8601(epoch + Duration::from_secs(1_795_182_044)),
            "2026-11-20T13:40:44Z"
        );
        assert_eq!(
            iso8601(epoch + Duration::from_secs(1_709_251_199)),
            "2024-02-29T23:59:59Z"
        );
        assert_eq!(
            iso8601(epoch + Duration::from_secs(951_868_800)),
            "2000-03-01T00:00:00Z"
        );
    }

    #[test]
    fn round_trips_a_run() {
        let db = temp_db("round-trip");
        let dir = std::env::temp_dir().join(format!("melodias-history-{}", std::process::id()));
        let record = RunRecord {
            db: &db,
            dir: &dir,
            slug: "epic-8-bit",
            vibe: "epic 8-bit",
            started_at: "2026-11-20T13:40:44Z",
            seconds: 2123.0,
            rounds: 0,
        };
        let id = insert(&record, &score_run(false)).unwrap();
        assert_eq!(id, 1);
        assert_eq!(insert(&record, &score_run(true)).unwrap(), 2);

        let runs = recent(&db, 20).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].slug, "epic-8-bit");
        assert_eq!(runs[0].technical, Some(72.35));
        assert_eq!(runs[0].editorial, Some(67.0));
        assert!(runs[0].accepted);
        assert!(!runs[1].accepted);

        let findings = findings(&db).unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].code, "IMG_PALETTE");
        assert_eq!(findings[0].hits, 2);
        assert_eq!(findings[0].points, Some(-40.0));
        assert_eq!(findings[0].accepted, 1);

        let trend = trend(&db, By::Day).unwrap();
        assert_eq!(trend.len(), 1);
        assert_eq!(trend[0].bucket, "2026-11-20");
        assert_eq!(trend[0].runs, 2);
        assert_eq!(trend[0].accepted, 1);

        let _ = fs::remove_file(&db);
        let _ = fs::remove_file(format!("{}-wal", db.display()));
        let _ = fs::remove_file(format!("{}-shm", db.display()));
    }

    #[test]
    fn a_missing_database_reads_as_empty() {
        let db = temp_db("missing");
        assert!(recent(&db, 20).unwrap().is_empty());
        assert!(findings(&db).unwrap().is_empty());
        assert!(trend(&db, By::Week).unwrap().is_empty());
        assert!(!db.exists(), "reading must not create the file");
    }

    #[test]
    fn renders_tables_and_csv() {
        let rows = vec![RunRow {
            id: 7,
            started_at: "2026-11-20T13:40:44Z".to_string(),
            slug: "epic, 8-bit".to_string(),
            technical: Some(72.35),
            editorial: None,
            total: Some(72.35),
            accepted: false,
            rounds: 0,
        }];
        let table = render_runs(&rows, Format::Table).unwrap();
        assert!(table.contains("epic, 8-bit"));
        assert!(table.contains("72.3"));
        let csv = render_runs(&rows, Format::Csv).unwrap();
        assert!(csv.starts_with("id,started_at,slug"));
        assert!(csv.contains("\"epic, 8-bit\""));
        assert!(csv.contains(",72.35,-,72.35,"));
        let json = render_runs(&rows, Format::Json).unwrap();
        assert!(json.contains("\"accepted\": false"));
        assert_eq!(render_runs(&[], Format::Json).unwrap().trim(), "[]");
    }

    #[test]
    #[ignore = "needs a finished run in output/ and ffmpeg"]
    fn writes_a_real_run_end_to_end() {
        let dir = Path::new("output/world-s-most-epic-emotional-violin");
        if !dir.join("meta.json").is_file() {
            eprintln!("skipping: {} is not here", dir.display());
            return;
        }
        let score = crate::score::score_run(dir).unwrap();
        let db = temp_db("real");
        let started = now_iso8601();
        let record = RunRecord {
            db: &db,
            dir,
            slug: "real-run",
            vibe: "real run",
            started_at: &started,
            seconds: 1.0,
            rounds: 0,
        };
        let id = insert(&record, &score).unwrap();
        let runs = recent(&db, 5).unwrap();
        assert_eq!(runs[0].id, id);
        assert_eq!(runs[0].slug, "real-run");
        assert_eq!(runs[0].technical, Some(score.technical));
        assert!(!findings(&db).unwrap().is_empty());

        let _ = fs::remove_file(&db);
        let _ = fs::remove_file(format!("{}-wal", db.display()));
        let _ = fs::remove_file(format!("{}-shm", db.display()));
    }
}
