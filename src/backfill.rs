//! Scores finished runs into the history database. It replaces the old bash
//! backfill: old runs did not write the database, so
//! this command scores their thumbnail and track with the current rubric and
//! inserts them with `below_target`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::{history, score};

pub struct Options {
    pub out: Option<PathBuf>,
    pub replace: bool,
}

fn usage() -> &'static str {
    "usage: melodias backfill [output-dir] [--replace]"
}

fn parse(args: &[String]) -> Result<Options> {
    let mut out = None;
    let mut replace = false;
    for arg in args {
        match arg.as_str() {
            "--replace" => replace = true,
            other if other.starts_with('-') => bail!("unknown option: {other}\n{}", usage()),
            other => {
                if out.is_some() {
                    bail!("unexpected argument: {other}\n{}", usage());
                }
                out = Some(PathBuf::from(other));
            }
        }
    }
    Ok(Options { out, replace })
}

pub fn run(cfg: &Config, args: &[String]) -> Result<()> {
    let options = parse(args)?;
    let out = options.out.clone().unwrap_or_else(|| cfg.out_dir.clone());
    if !out.is_dir() {
        bail!("no output directory: {}", out.display());
    }
    if let Some(parent) = cfg.db_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    history::open(&cfg.db_path)?;

    let mut dirs: Vec<PathBuf> = fs::read_dir(&out)
        .with_context(|| format!("reading {}", out.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("meta.json").is_file())
        .collect();
    dirs.sort();

    println!("database: {}\n", cfg.db_path.display());
    println!(
        "{:<42} {:>7} {:>7} {:>7} {:>4}",
        "slug", "tech", "edit", "total", "ok"
    );
    let mut seen = 0;
    for dir in dirs {
        let meta_path = dir.join("meta.json");
        let meta: serde_json::Value = serde_json::from_slice(
            &fs::read(&meta_path).with_context(|| format!("reading {}", meta_path.display()))?,
        )
        .with_context(|| format!("{} is not valid JSON", meta_path.display()))?;
        let slug = meta["slug"]
            .as_str()
            .map(str::to_string)
            .or_else(|| {
                dir.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "run".to_string());
        let vibe = meta["vibe"].as_str().unwrap_or("");

        if !dir.join("thumbnail.png").is_file() || !dir.join("audio.mp3").is_file() {
            eprintln!("{slug}: skipped, missing thumbnail or audio");
            continue;
        }

        if let Some(existing) = existing_run(&cfg.db_path, &slug)? {
            if !options.replace {
                eprintln!("{slug}: already in the database, use --replace");
                continue;
            }
            delete_run(&cfg.db_path, existing)?;
        }

        let run_score = match score::score_run(&dir) {
            Ok(score) => score,
            Err(error) => {
                eprintln!("{slug}: skipped, {error:#}");
                continue;
            }
        };
        let started_at = started_at(&meta_path);
        let record = history::RunRecord {
            db: &cfg.db_path,
            dir: &dir,
            slug: &slug,
            vibe,
            started_at: &started_at,
            seconds: 0.0,
            rounds: 0,
            patches: &[],
            outcome: "below_target",
            outcome_reason: Some("backfilled without a retry loop"),
        };
        history::insert(&record, &run_score)?;
        println!(
            "{:<42} {:>7.1} {:>7} {:>7.1} {:>4}",
            slug,
            run_score.technical,
            run_score
                .editorial
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "-".to_string()),
            run_score.total,
            if run_score.accepted { 1 } else { 0 }
        );
        seen += 1;
    }

    println!();
    let conn = history::open(&cfg.db_path)?;
    let mut statement = conn.prepare(
        "select code, count(*) as runs, round(sum(delta), 1) as points
         from findings group by code order by sum(delta)",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    println!("{:<16} {:>6} {:>8}", "code", "runs", "points");
    for row in rows {
        let (code, runs, points) = row?;
        println!("{code:<16} {runs:>6} {points:>8.1}");
    }

    let runs: i64 = conn.query_row("select count(*) from runs", [], |row| row.get(0))?;
    let candidates: i64 =
        conn.query_row("select count(*) from candidates", [], |row| row.get(0))?;
    let findings: i64 = conn.query_row("select count(*) from findings", [], |row| row.get(0))?;
    println!();
    println!("runs: {runs}, candidates: {candidates}, findings: {findings}");
    if seen == 0 {
        eprintln!("nothing to backfill: every run is already in the database (use --replace)");
    }
    Ok(())
}

fn existing_run(db: &Path, slug: &str) -> Result<Option<i64>> {
    let conn = history::open(db)?;
    let mut statement = conn.prepare("select id from runs where slug = ?1 limit 1")?;
    let mut rows = statement.query([slug])?;
    Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
}

fn delete_run(db: &Path, run_id: i64) -> Result<()> {
    let mut conn = history::open(db)?;
    let tx = conn.transaction()?;
    tx.execute("delete from findings where run_id = ?1", [run_id])?;
    tx.execute("delete from candidates where run_id = ?1", [run_id])?;
    tx.execute("delete from patches where run_id = ?1", [run_id])?;
    tx.execute("delete from runs where id = ?1", [run_id])?;
    tx.commit()?;
    Ok(())
}

fn started_at(path: &Path) -> String {
    Command::new("date")
        .args(["-r"])
        .arg(path)
        .arg("--iso-8601=seconds")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(history::now_iso8601)
}

#[cfg(test)]
#[path = "tests/backfill.rs"]
mod tests;
