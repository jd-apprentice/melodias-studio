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
        rounds: 1,
        patches: &[],
        outcome: "accepted",
        outcome_reason: None,
    };
    let mut run = score();
    run.accepted = true;
    insert(&record, &run).unwrap();
    let limited = RunRecord {
        outcome: "accepted_by_limit",
        outcome_reason: Some("the image budget is exhausted at 84.2"),
        started_at: "2026-11-20T14:00:00Z",
        seconds: 1000.0,
        ..record
    };
    insert(&limited, &score()).unwrap();

    let summary = summary(&db, 20).unwrap();
    assert!(summary.exists);
    assert_eq!(summary.totals.runs, 2);
    assert_eq!(summary.totals.accepted, 1);
    assert_eq!(summary.totals.by_limit, 1);
    assert_eq!(summary.runs.len(), 2);
    assert_eq!(
        summary.runs[0].outcome.as_deref(),
        Some("accepted_by_limit")
    );
    assert_eq!(summary.runs[1].outcome.as_deref(), Some("accepted"));
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
