use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::{clients, config::Config, history, judge, media, score};

pub const STAGES: [&str; 11] = [
    "Image prompt",
    "Prepare image",
    "Image pool",
    "Download",
    "Reference",
    "Upload",
    "Audio drafts",
    "Review",
    "Final take",
    "Master + save",
    "Video",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Running,
    Ok,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub file: PathBuf,
    pub seed: u32,
}

#[derive(Clone, Debug)]
pub struct Pool {
    pub images: Vec<Candidate>,
    pub audios: Vec<Candidate>,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub image: usize,
    pub audio: usize,
}

pub struct Outcome {
    pub audio: PathBuf,
    pub image: PathBuf,
    pub video: PathBuf,
    pub seed: u32,
    pub image_prompt: String,
}

#[derive(Clone, Debug)]
pub struct ReviewRequest {
    pub pool: Pool,
    /// Indices suggested by the cloud judge, if it ran and answered.
    pub suggestion: Option<Selection>,
    pub note: Option<String>,
}

pub enum Msg {
    Stage(usize, State),
    Log(String),
    Choose(ReviewRequest),
    Done(Result<Outcome, String>),
}

/// Starts the run and returns the channel the UI uses to answer the review.
pub fn spawn(cfg: Config, url: String, vibe: String, tx: Sender<Msg>) -> Sender<Selection> {
    let (decision_tx, decision_rx) = channel();
    thread::spawn(move || {
        let result =
            run(&cfg, &url, &vibe, &tx, &decision_rx).map_err(|error| format!("{error:#}"));
        let _ = tx.send(Msg::Done(result));
    });
    decision_tx
}

fn step<T>(tx: &Sender<Msg>, idx: usize, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _ = tx.send(Msg::Stage(idx, State::Running));
    match f() {
        Ok(value) => {
            let _ = tx.send(Msg::Stage(idx, State::Ok));
            Ok(value)
        }
        Err(err) => {
            let _ = tx.send(Msg::Stage(idx, State::Failed));
            let _ = tx.send(Msg::Log(format!("{}: {err:#}", STAGES[idx])));
            Err(err)
        }
    }
}

fn log(tx: &Sender<Msg>, text: impl Into<String>) {
    let _ = tx.send(Msg::Log(text.into()));
}

/// One line per stage transition, shared by the TUI log and the headless view.
pub fn stage_label(idx: usize, state: State) -> Option<String> {
    let name = STAGES.get(idx)?;
    let number = idx + 1;
    let total = STAGES.len();
    match state {
        State::Pending => None,
        State::Running => Some(format!("▶ {number}/{total} {name}")),
        State::Ok => Some(format!("✔ {number}/{total} {name}")),
        State::Failed => Some(format!("✖ {number}/{total} {name}")),
    }
}

fn run(
    cfg: &Config,
    url: &str,
    vibe: &str,
    tx: &Sender<Msg>,
    decisions: &Receiver<Selection>,
) -> Result<Outcome> {
    let count = cfg.candidates.max(1);
    let slug = slugify(vibe);
    let root = cfg.out_dir.join(&slug);
    let work = root.join("work");
    let candidates_dir = root.join("candidates");
    std::fs::create_dir_all(&work).context("creating the work directory")?;
    std::fs::create_dir_all(&candidates_dir).context("creating the candidates directory")?;
    let seed = random_seed();
    let image_seed = cfg.image_seed.unwrap_or_else(random_seed);
    let started_at = history::now_iso8601();
    let clock = Instant::now();

    log(tx, format!("working in {}", root.display()));
    log(
        tx,
        format!("audio seed {seed} · image seed {image_seed} · {count} candidates"),
    );
    media::preflight()?;

    // The LLM writes the thumbnail prompt while it still owns the GPU.
    let image_prompt = step(tx, 0, || {
        media::ssh_run(cfg, &cfg.llm_start_cmd)?;
        clients::wait_llm(cfg, Duration::from_secs(180))?;
        clients::ask_image_prompt(cfg, vibe)
    })?;
    log(tx, format!("image prompt: {image_prompt}"));

    // Free the LLM, then bring up the image server.
    step(tx, 1, || {
        media::ssh_run(cfg, &cfg.llm_stop_cmd)?;
        media::ssh_run(cfg, &cfg.sd_start_cmd)?;
        clients::wait_sd(cfg, Duration::from_secs(180))
    })?;

    let image_started = std::time::Instant::now();
    let images = step(tx, 2, || {
        let mut candidates = Vec::new();
        for index in 0..count {
            let candidate_seed = image_seed.wrapping_add(index);
            let bytes = clients::generate_image(cfg, &image_prompt, candidate_seed)?;
            let path = candidates_dir.join(format!("image-{:02}.png", index + 1));
            std::fs::write(&path, &bytes).context("saving an image candidate")?;
            let (width, height) = media::png_size(&path)?;
            if (width, height) != (cfg.width, cfg.height) {
                log(
                    tx,
                    format!(
                        "warning: candidate {} returned {width}x{height} instead of {}x{}",
                        index + 1,
                        cfg.width,
                        cfg.height
                    ),
                );
            }
            log(
                tx,
                format!(
                    "image candidate {}/{}: {} (seed {candidate_seed}, {:.1} MB)",
                    index + 1,
                    count,
                    path.display(),
                    bytes.len() as f64 / 1e6
                ),
            );
            candidates.push(Candidate {
                file: path,
                seed: candidate_seed,
            });
        }
        Ok(candidates)
    })?;
    log(
        tx,
        format!(
            "image pool: {} candidates in {:.1} min",
            images.len(),
            image_started.elapsed().as_secs_f64() / 60.0
        ),
    );

    let source = step(tx, 3, || media::download_audio(url, &work))?;
    log(tx, format!("source: {}", source.display()));

    let reference = step(tx, 4, || media::pick_reference(cfg, &source, &work))?;
    log(
        tx,
        format!(
            "reference: {} ({:.0} s)",
            reference.display(),
            media::duration_secs(&reference)?
        ),
    );

    let remote = step(tx, 5, || media::push_reference(cfg, &reference, &slug))?;
    log(tx, format!("remote reference: {remote}"));

    let audio_prompt = compose_audio_prompt(vibe, &cfg.audio_style);
    log(tx, format!("prompt audio: {audio_prompt}"));

    let drafts_result = step(tx, 6, || {
        // The timbre reference pushes ACE-Step past what fits next to the 4B,
        // so both the image model and the LLM go away before the audio stage.
        media::ssh_run(cfg, &cfg.sd_stop_cmd)?;
        media::ssh_run(cfg, &cfg.llm_stop_cmd)?;
        media::ssh_run(cfg, &cfg.audio_start_cmd)?;
        clients::wait_audio(cfg, Duration::from_secs(180))?;
        let mut candidates = Vec::new();
        for index in 0..count {
            let candidate_seed = seed.wrapping_add(index);
            let bytes = clients::generate_audio(
                cfg,
                clients::AudioRequest {
                    prompt: &audio_prompt,
                    reference: &remote,
                    seed: candidate_seed,
                    duration_sec: cfg.audio_draft_sec.max(10),
                    steps: cfg.audio_draft_steps.max(1),
                    sampler: &cfg.audio_draft_sampler,
                },
            )?;
            let path = candidates_dir.join(format!("audio-{:02}.wav", index + 1));
            std::fs::write(&path, &bytes).context("saving an audio draft")?;
            log(
                tx,
                format!(
                    "audio draft {}/{}: {} (seed {candidate_seed}, {:.0} s)",
                    index + 1,
                    count,
                    path.display(),
                    media::duration_secs(&path)?
                ),
            );
            candidates.push(Candidate {
                file: path,
                seed: candidate_seed,
            });
        }
        Ok(candidates)
    });
    let audios = match drafts_result {
        Ok(audios) => audios,
        Err(err) => {
            let _ = media::ssh_run(cfg, &cfg.audio_stop_cmd);
            return Err(err);
        }
    };

    // The audio server stays loaded while the review is open and the final
    // take runs, then it is released even if something fails.
    let pool = Pool {
        images: images.clone(),
        audios: audios.clone(),
    };
    let (selection, judge_summary) = step(tx, 7, || {
        let mut suggestion = None;
        let mut summary = None;
        if judge::enabled(cfg) {
            log(
                tx,
                format!(
                    "judge: reviewing thumbnails and clips with {}",
                    judge::describe(cfg)
                ),
            );
            match judge::decide(cfg, vibe, &images, &audios, &work) {
                Ok(verdict) => {
                    let note = format!(
                        "judge: image {} · audio {} · score {} · {}",
                        verdict.image + 1,
                        verdict.audio + 1,
                        verdict.score,
                        verdict.reason
                    );
                    log(tx, note.clone());
                    suggestion = Some(Selection {
                        image: verdict.image,
                        audio: verdict.audio,
                    });
                    summary = Some(verdict.summary());
                }
                Err(error) => log(
                    tx,
                    format!("judge: failed, keeping the manual pick: {error:#}"),
                ),
            }
        } else if cfg.judge_enabled {
            log(
                tx,
                format!("judge: enabled but no API key for {}", judge::describe(cfg)),
            );
        }
        let request = ReviewRequest {
            pool: pool.clone(),
            suggestion,
            note: summary
                .as_ref()
                .and_then(|value| value["reason"].as_str())
                .map(str::to_string),
        };
        let _ = tx.send(Msg::Choose(request));
        let selection = decisions.recv().context("the review was cancelled")?;
        Ok((selection, summary))
    })?;
    let selected_image = images
        .get(selection.image)
        .context("the selected image does not exist")?;
    let selected_audio = audios
        .get(selection.audio)
        .context("the selected audio draft does not exist")?;
    log(
        tx,
        format!(
            "selected image {} and audio draft {}",
            selection.image + 1,
            selection.audio + 1
        ),
    );

    let final_result = step(tx, 8, || {
        let bytes = clients::generate_audio(
            cfg,
            clients::AudioRequest {
                prompt: &audio_prompt,
                reference: &remote,
                seed: selected_audio.seed,
                duration_sec: cfg.duration_sec,
                steps: cfg.steps,
                sampler: &cfg.sampler,
            },
        )?;
        let take = work.join("take.wav");
        std::fs::write(&take, &bytes).context("saving the take")?;
        log(
            tx,
            format!(
                "final take: {} ({:.1} MB)",
                take.display(),
                bytes.len() as f64 / 1e6
            ),
        );
        Ok(take)
    });
    let stopped = media::ssh_run(cfg, &cfg.audio_stop_cmd);
    if let Err(err) = &stopped {
        log(
            tx,
            format!("warning: the audio server is still up: {err:#}"),
        );
    }
    let take = final_result?;

    let master = format!(
        "fade-in {} s, fade-out {} s, {} LUFS",
        cfg.fade_in_sec, cfg.fade_out_sec, cfg.loudness_lufs
    );

    let (final_audio, final_image) = step(tx, 9, || {
        let mastered = media::master_take(cfg, &take, &work)?;
        let mp3 = media::encode_mp3(&mastered, &work, 320)?;
        log(
            tx,
            format!(
                "master: {} ({:.0} s)",
                mp3.display(),
                media::duration_secs(&mp3)?
            ),
        );

        let final_audio = root.join("audio.mp3");
        std::fs::copy(&mp3, &final_audio)?;
        let final_image = root.join("thumbnail.png");
        std::fs::copy(&selected_image.file, &final_image)?;
        Ok((final_audio, final_image))
    })?;

    let video = step(tx, 10, || {
        let video = media::assemble_video(cfg, &final_image, &final_audio, &root)?;
        let megabytes = std::fs::metadata(&video)
            .map(|meta| meta.len())
            .unwrap_or(0) as f64
            / 1e6;
        log(
            tx,
            format!("video: {} ({megabytes:.1} MB)", video.display()),
        );

        let image_candidates: Vec<_> = images
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "file": candidate.file.display().to_string(),
                    "seed": candidate.seed,
                })
            })
            .collect();
        let audio_candidates: Vec<_> = audios
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "file": candidate.file.display().to_string(),
                    "seed": candidate.seed,
                })
            })
            .collect();

        let meta = serde_json::json!({
            "slug": slug,
            "source_url": url,
            "vibe": vibe,
            "image_prompt": image_prompt,
            "seed": selected_audio.seed,
            "image": {
                "size": format!("{}x{}", cfg.width, cfg.height),
                "steps": cfg.image_steps,
                "candidates": image_candidates,
                "selected": selection.image + 1,
                "file": final_image.display().to_string(),
            },
            "audio": {
                "model": cfg.audio_model,
                "prompt": audio_prompt,
                "negative_prompt": cfg.audio_negative_prompt,
                "draft": {
                    "duration_sec": cfg.audio_draft_sec,
                    "steps": cfg.audio_draft_steps,
                    "sampler": cfg.audio_draft_sampler,
                    "candidates": audio_candidates,
                    "selected": selection.audio + 1,
                },
                "final": {
                    "duration_sec": cfg.duration_sec,
                    "steps": cfg.steps,
                    "guidance": cfg.guidance,
                    "sampler": cfg.sampler,
                    "seed": selected_audio.seed,
                },
                "master": master,
                "reference": remote,
                "file": final_audio.display().to_string(),
            },
            "video": {
                "file": video.display().to_string(),
                "size": format!("{}x{}", cfg.video_width, cfg.video_height),
            },
            "judge": judge_summary,
        });
        std::fs::write(root.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;
        Ok(video)
    })?;

    record_history(cfg, tx, &root, &slug, vibe, &started_at, clock);

    Ok(Outcome {
        audio: final_audio,
        image: final_image,
        video,
        seed: selected_audio.seed,
        image_prompt,
    })
}

/// Scores the finished run and appends it to the local database. Failures are
/// logged and never abort the run, the same way the judge fails open.
fn record_history(
    cfg: &Config,
    tx: &Sender<Msg>,
    root: &Path,
    slug: &str,
    vibe: &str,
    started_at: &str,
    clock: Instant,
) {
    let record = history::RunRecord {
        db: &cfg.db_path,
        dir: root,
        slug,
        vibe,
        started_at,
        seconds: clock.elapsed().as_secs_f64(),
        rounds: 0,
    };
    log(tx, "history: scoring the run for the local database");
    let result = score::score_run(root).and_then(|score| {
        score::write_json(root, &score)?;
        let id = history::insert(&record, &score)?;
        Ok((id, score))
    });
    match result {
        Ok((id, score)) => {
            log(
                tx,
                format!(
                    "history: run {id} · technical {:.1} · total {:.1} · {}",
                    score.technical,
                    score.total,
                    if score.accepted {
                        "accepted"
                    } else {
                        "below target"
                    }
                ),
            );
            let worst = score
                .final_image
                .findings
                .iter()
                .chain(score.final_audio.findings.iter())
                .min_by(|a, b| a.delta.total_cmp(&b.delta));
            if let Some(finding) = worst {
                log(
                    tx,
                    format!(
                        "history: worst finding {} {:+.1} ({})",
                        finding.code, finding.delta, finding.message
                    ),
                );
            }
        }
        Err(error) => log(
            tx,
            format!("warning: the history was not written: {error:#}"),
        ),
    }
}

fn random_seed() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1);
    nanos.max(1)
}

pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "take".to_string()
    } else {
        trimmed
    }
}

/// The channel medium is a rule: the vibe adds mood, the style is always there.
pub fn compose_audio_prompt(vibe: &str, style: &str) -> String {
    format!(
        "{vibe}, {style}, instrumental, soft intro that builds gradually, full arrangement in \
         the middle, gentle outro that resolves and fades out"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_medium_is_always_in_the_audio_prompt() {
        let prompt = compose_audio_prompt("epic orchestral", "8-bit chiptune");
        assert!(prompt.starts_with("epic orchestral, 8-bit chiptune"));
        assert!(prompt.contains("instrumental"));
        assert!(prompt.contains("fades out"));
    }
}
