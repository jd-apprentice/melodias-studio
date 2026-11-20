use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::{clients, config::Config, media};

pub const STAGES: [&str; 9] = [
    "Prompt imagen",
    "Preparar imagen",
    "Miniatura",
    "Descarga",
    "Referencia",
    "Subida",
    "Audio",
    "Loop + master",
    "Guardado",
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

fn run(cfg: &Config, url: &str, vibe: &str, tx: &Sender<Msg>) -> Result<Outcome> {
    let slug = slugify(vibe);
    let work = cfg.out_dir.join(&slug).join("work");
    std::fs::create_dir_all(&work).context("creando el directorio de trabajo")?;
    let seed = random_seed();

    log(tx, format!("trabajando en {}", work.display()));
    log(tx, format!("seed {seed}"));
    media::preflight()?;

    // The LLM writes the thumbnail prompt while it still owns the GPU.
    let image_prompt = step(tx, 0, || {
        media::ssh_run(cfg, &cfg.llm_start_cmd)?;
        clients::wait_llm(cfg, Duration::from_secs(180))?;
        clients::ask_image_prompt(cfg, vibe)
    })?;
    log(tx, format!("prompt imagen: {image_prompt}"));

    // Free the LLM, then bring up the image server.
    step(tx, 1, || {
        media::ssh_run(cfg, &cfg.llm_stop_cmd)?;
        media::ssh_run(cfg, &cfg.sd_start_cmd)?;
        clients::wait_sd(cfg, Duration::from_secs(180))
    })?;

    let image_started = std::time::Instant::now();
    let image_bytes = step(tx, 2, || clients::generate_image(cfg, &image_prompt))?;
    let image = cfg.out_dir.join(&slug).join("thumbnail.png");
    std::fs::write(&image, &image_bytes).context("guardando la miniatura")?;
    let (width, height) = media::png_size(&image)?;
    log(
        tx,
        format!(
            "miniatura: {} ({width}x{height}, {:.1} MB, {:.1} min)",
            image.display(),
            image_bytes.len() as f64 / 1e6,
            image_started.elapsed().as_secs_f64() / 60.0
        ),
    );
    if (width, height) != (cfg.width, cfg.height) {
        log(
            tx,
            format!(
                "aviso: el servidor devolvió {width}x{height} en vez de {}x{}",
                cfg.width, cfg.height
            ),
        );
    }

    let source = step(tx, 3, || media::download_audio(url, &work))?;
    log(tx, format!("fuente: {}", source.display()));

    let reference = step(tx, 4, || media::pick_reference(&source, &work))?;
    log(
        tx,
        format!(
            "referencia: {} ({:.0} s)",
            reference.display(),
            media::duration_secs(&reference)?
        ),
    );

    let remote = step(tx, 5, || media::push_reference(cfg, &reference, &slug))?;
    log(tx, format!("referencia remota: {remote}"));

    let audio_bytes = step(tx, 6, || {
        media::ssh_run(cfg, &cfg.sd_stop_cmd)?;
        media::ssh_run(cfg, &cfg.audio_start_cmd)?;
        clients::wait_audio(cfg, Duration::from_secs(180))?;
        clients::generate_audio(cfg, vibe, &remote, seed)
    })?;
    let take = work.join("take.wav");
    std::fs::write(&take, &audio_bytes).context("guardando la toma")?;
    log(
        tx,
        format!(
            "toma: {} ({:.1} MB)",
            take.display(),
            audio_bytes.len() as f64 / 1e6
        ),
    );

    let looped = step(tx, 7, || media::make_loop(&take, &work))?;
    let mp3 = media::encode_mp3(&looped, &work, 320)?;
    log(
        tx,
        format!(
            "loop: {} ({:.0} s)",
            mp3.display(),
            media::duration_secs(&mp3)?
        ),
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
                "steps": cfg.steps,
                "guidance": cfg.guidance,
                "sampler": cfg.sampler,
                "duration_sec": cfg.duration_sec,
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
        "toma".to_string()
    } else {
        trimmed
    }
}
