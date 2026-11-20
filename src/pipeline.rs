use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::{clients, config::Config, media};

pub const STAGES: [&str; 9] = [
    "Image prompt",
    "Prepare image",
    "Thumbnail",
    "Download",
    "Reference",
    "Upload",
    "Audio",
    "Master",
    "Save",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Running,
    Ok,
    Failed,
}

pub struct Outcome {
    pub audio: PathBuf,
    pub image: PathBuf,
    pub seed: u32,
    pub image_prompt: String,
}

pub enum Msg {
    Stage(usize, State),
    Log(String),
    Done(Result<Outcome, String>),
}

pub fn spawn(cfg: Config, url: String, vibe: String, tx: Sender<Msg>) {
    thread::spawn(move || {
        let result = run(&cfg, &url, &vibe, &tx).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Done(result));
    });
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

fn run(cfg: &Config, url: &str, vibe: &str, tx: &Sender<Msg>) -> Result<Outcome> {
    let slug = slugify(vibe);
    let work = cfg.out_dir.join(&slug).join("work");
    std::fs::create_dir_all(&work).context("creating the work directory")?;
    let seed = random_seed();

    log(tx, format!("working in {}", work.display()));
    log(tx, format!("seed {seed}"));
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
    let image_bytes = step(tx, 2, || clients::generate_image(cfg, &image_prompt))?;
    let image = cfg.out_dir.join(&slug).join("thumbnail.png");
    std::fs::write(&image, &image_bytes).context("saving the thumbnail")?;
    let (width, height) = media::png_size(&image)?;
    log(
        tx,
        format!(
            "thumbnail: {} ({width}x{height}, {:.1} MB, {:.1} min)",
            image.display(),
            image_bytes.len() as f64 / 1e6,
            image_started.elapsed().as_secs_f64() / 60.0
        ),
    );
    if (width, height) != (cfg.width, cfg.height) {
        log(
            tx,
            format!(
                "warning: the server returned {width}x{height} instead of {}x{}",
                cfg.width, cfg.height
            ),
        );
    }

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

    // The vibe is the only audio prompt. The word "instrumental" plus the
    // negative prompt keep voices out of every delivery.
    let audio_prompt = format!(
        "{vibe}, instrumental, soft intro that builds gradually, full arrangement in the middle, \
         gentle outro that resolves and fades out"
    );
    log(tx, format!("prompt audio: {audio_prompt}"));

    let audio_result = step(tx, 6, || {
        // The timbre reference pushes ACE-Step past what fits next to the 4B,
        // so both the image model and the LLM go away before the audio stage.
        media::ssh_run(cfg, &cfg.sd_stop_cmd)?;
        media::ssh_run(cfg, &cfg.llm_stop_cmd)?;
        media::ssh_run(cfg, &cfg.audio_start_cmd)?;
        clients::wait_audio(cfg, Duration::from_secs(180))?;
        clients::generate_audio(cfg, &audio_prompt, &remote, seed)
    });
    // Release the P40 even when the take failed, so a later image stage can load.
    let stopped = media::ssh_run(cfg, &cfg.audio_stop_cmd);
    if let Err(err) = &stopped {
        log(
            tx,
            format!("warning: the audio server is still up: {err:#}"),
        );
    }
    let audio_bytes = audio_result?;
    let take = work.join("take.wav");
    std::fs::write(&take, &audio_bytes).context("saving the take")?;
    log(
        tx,
        format!(
            "take: {} ({:.1} MB)",
            take.display(),
            audio_bytes.len() as f64 / 1e6
        ),
    );

    let mastered = step(tx, 7, || media::master_take(cfg, &take, &work))?;
    let mp3 = media::encode_mp3(&mastered, &work, 320)?;
    log(
        tx,
        format!(
            "master: {} ({:.0} s)",
            mp3.display(),
            media::duration_secs(&mp3)?
        ),
    );

    let master = format!(
        "fade-in {} s, fade-out {} s, {} LUFS",
        cfg.fade_in_sec, cfg.fade_out_sec, cfg.loudness_lufs
    );

    step(tx, 8, || {
        let final_audio = cfg.out_dir.join(&slug).join("audio.mp3");
        std::fs::copy(&mp3, &final_audio)?;
        let meta = serde_json::json!({
            "slug": slug,
            "source_url": url,
            "vibe": vibe,
            "image_prompt": image_prompt,
            "seed": seed,
            "audio": {
                "model": cfg.audio_model,
                "prompt": audio_prompt,
                "negative_prompt": cfg.audio_negative_prompt,
                "steps": cfg.steps,
                "guidance": cfg.guidance,
                "sampler": cfg.sampler,
                "duration_sec": cfg.duration_sec,
                "master": master,
                "reference": remote,
                "file": final_audio,
            },
            "image": {
                "size": format!("{}x{}", cfg.width, cfg.height),
                "file": image,
            }
        });
        std::fs::write(
            cfg.out_dir.join(&slug).join("meta.json"),
            serde_json::to_vec_pretty(&meta)?,
        )?;
        Ok(())
    })?;

    Ok(Outcome {
        audio: cfg.out_dir.join(&slug).join("audio.mp3"),
        image,
        seed,
        image_prompt,
    })
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
