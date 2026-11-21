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

fn candidate(index: usize, seed: u32, selected: bool, findings: Vec<Finding>) -> CandidateScore {
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

fn patch() -> crate::retry::Applied {
    crate::retry::Applied {
        code: "IMG_PALETTE",
        prompt_before: "{\"positive\":\"before\",\"negative\":\"\"}".to_string(),
        prompt_after: "{\"positive\":\"after\",\"negative\":\"\"}".to_string(),
        applied_at: "2026-11-20T13:40:44Z".to_string(),
    }
}

fn cleanup(db: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", db.display()));
    }
}

#[test]
fn round_trips_a_run() {
    let db = temp_db("round-trip");
    let dir = std::env::temp_dir().join(format!("melodias-history-{}", std::process::id()));
    let patches = vec![patch()];
    let record = RunRecord {
        db: &db,
        dir: &dir,
        slug: "epic-8-bit",
        vibe: "epic 8-bit",
        started_at: "2026-11-20T13:40:44Z",
        seconds: 2123.0,
        rounds: 1,
        patches: &patches,
        outcome: "accepted_by_limit",
        outcome_reason: Some("the image budget is exhausted at 84.2"),
    };
    let id = insert(&record, &score_run(false)).unwrap();
    assert_eq!(id, 1);
    assert_eq!(insert(&record, &score_run(true)).unwrap(), 2);

    let runs = recent(&db, 20).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].slug, "epic-8-bit");
    assert_eq!(runs[0].technical, Some(72.35));
    assert_eq!(runs[0].editorial, Some(67.0));
    assert_eq!(runs[0].outcome.as_deref(), Some("accepted_by_limit"));
    assert!(runs[0].outcome_reason.is_some());
    assert!(runs[0].accepted);
    assert!(!runs[1].accepted);

    let findings = findings(&db).unwrap();
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].code, "IMG_PALETTE");
    assert_eq!(findings[0].hits, 2);
    assert_eq!(findings[0].points, Some(-40.0));
    assert_eq!(findings[0].accepted, 1);

    let patches: i64 = open(&db)
        .unwrap()
        .query_row("select count(*) from patches", [], |row| row.get(0))
        .unwrap();
    assert_eq!(patches, 2, "one patch row per inserted run");

    let trend = trend(&db, By::Day).unwrap();
    assert_eq!(trend.len(), 1);
    assert_eq!(trend[0].bucket, "2026-11-20");
    assert_eq!(trend[0].runs, 2);
    assert_eq!(trend[0].accepted, 1);

    cleanup(&db);
}

#[test]
fn migrates_a_v1_database() {
    let db = temp_db("migrate");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "create table runs (
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
         insert into runs (started_at, slug, vibe, accepted, rounds)
         values ('2026-11-19T23:00:00Z', 'old-run', 'old', 1, 0);
         pragma user_version = 1;",
    )
    .unwrap();
    drop(conn);

    // A reader on the old file must not fail on the missing column.
    let rows = recent(&db, 5).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, None);

    let conn = open(&db).unwrap();
    let outcome: String = conn
        .query_row(
            "select outcome from runs where slug = 'old-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "accepted");
    let version: i64 = conn
        .query_row("pragma user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    drop(conn);
    cleanup(&db);
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
        outcome: Some("accepted_by_limit".to_string()),
        outcome_reason: Some("the audio budget is exhausted at 84.2".to_string()),
        rounds: 1,
    }];
    let table = render_runs(&rows, Format::Table).unwrap();
    assert!(table.contains("epic, 8-bit"));
    assert!(table.contains("72.3"));
    assert!(table.contains("by limit"));
    let csv = render_runs(&rows, Format::Csv).unwrap();
    assert!(csv.starts_with("id,started_at,slug"));
    assert!(csv.contains("\"epic, 8-bit\""));
    assert!(csv.contains(",72.35,-,72.35,"));
    assert!(csv.contains("accepted_by_limit"));
    let json = render_runs(&rows, Format::Json).unwrap();
    assert!(json.contains("\"accepted\": false"));
    assert!(json.contains("\"outcome\": \"accepted_by_limit\""));
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
        patches: &[],
        outcome: "accepted_by_limit",
        outcome_reason: Some("test run"),
    };
    let id = insert(&record, &score).unwrap();
    let runs = recent(&db, 5).unwrap();
    assert_eq!(runs[0].id, id);
    assert_eq!(runs[0].slug, "real-run");
    assert_eq!(runs[0].technical, Some(score.technical));
    assert!(!findings(&db).unwrap().is_empty());

    cleanup(&db);
}
