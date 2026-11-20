mod app;
mod banner;
mod clients;
mod config;
mod media;
mod pipeline;

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

use crossterm::style::Stylize;

use pipeline::{Msg, Selection, State};

const USAGE: &str = "usage: melodias                 (TUI)\n       melodias <url> <vibe>     (headless)\n       melodias image <dir>      (regenerate the thumbnail of a run)\n       melodias render <dir>     (produced video with Remotion)\n       melodias video <dir>      (fast video with ffmpeg)";

fn main() -> io::Result<()> {
    let cfg = config::Config::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "-h" || arg == "--help")
    {
        println!("{USAGE}");
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "image") {
        return run_image(cfg, &args);
    }
    if args.first().is_some_and(|arg| arg == "render") {
        return run_render(cfg, &args);
    }
    if args.first().is_some_and(|arg| arg == "video") {
        return run_video(cfg, &args);
    }
    if !args.is_empty() {
        return run_headless(cfg, &args);
    }

    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, cfg);
    ratatui::restore();
    result
}

/// Runs the same pipeline the TUI drives, with a compact terminal view. Stages
/// are printed as groups with icons and color, and the plain text goes to the
/// file named by MELODIAS_LOG when it is set.
fn run_headless(cfg: config::Config, args: &[String]) -> io::Result<()> {
    let args: Vec<&String> = args.iter().filter(|arg| *arg != "--headless").collect();
    let [url, vibe] = args.as_slice() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };

    let host = cfg.ssh_host.clone();
    let gpu = cfg.gpu_label.clone();
    let log_path = std::env::var_os("MELODIAS_LOG").map(PathBuf::from);
    if let Some(path) = &log_path {
        println!("log: {}", path.display());
    }
    let mut out = Reporter::new(log_path)?;

    banner::print(out.pretty, &gpu);

    let title = format!("♪ melodias-studio · {host}");
    let colored = out.pretty.then(|| title.as_str().cyan().bold().to_string());
    out.line(&title, colored);
    let separator = "────────────────────────────────────────";
    let colored = out.pretty.then(|| separator.dark_grey().to_string());
    out.line(separator, colored);

    let (tx, rx) = channel();
    let decisions = pipeline::spawn(cfg, url.trim().to_string(), vibe.trim().to_string(), tx);

    for msg in rx {
        match msg {
            Msg::Stage(idx, state) => out.stage(idx, state),
            Msg::Log(text) => out.log_line(&text),
            Msg::Choose(pool) => {
                out.line("", None);
                let header = format!(
                    "Review: {} images and {} audio drafts",
                    pool.images.len(),
                    pool.audios.len()
                );
                let colored = out
                    .pretty
                    .then(|| header.as_str().cyan().bold().to_string());
                out.line(&header, colored);
                for (index, candidate) in pool.images.iter().enumerate() {
                    out.log_line(&format!(
                        "image {}: {} (seed {})",
                        index + 1,
                        candidate.file.display(),
                        candidate.seed
                    ));
                }
                for (index, candidate) in pool.audios.iter().enumerate() {
                    out.log_line(&format!(
                        "audio {}: {} (seed {})",
                        index + 1,
                        candidate.file.display(),
                        candidate.seed
                    ));
                }
                let selection = Selection { image: 0, audio: 0 };
                let _ = decisions.send(selection);
                out.log_line(
                    "auto-selected image 1 and audio draft 1 (headless; use the TUI to choose)",
                );
            }
            Msg::Done(Ok(outcome)) => {
                out.line("", None);
                let line = format!(
                    "✔ done · seed {} · {}",
                    outcome.seed,
                    outcome.audio.display()
                );
                let colored = out.pretty.then(|| line.as_str().green().bold().to_string());
                out.line(&line, colored);
                let image = format!("✔ thumbnail · {}", outcome.image.display());
                let colored = out.pretty.then(|| image.as_str().green().to_string());
                out.line(&image, colored);
                let video = format!("✔ video · {}", outcome.video.display());
                let colored = out.pretty.then(|| video.as_str().green().to_string());
                out.line(&video, colored);
                return Ok(());
            }
            Msg::Done(Err(err)) => {
                out.error(&format!("✖ failed: {err}"));
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// Renders the produced video with the Remotion project: overlay clips, an
/// audio-reactive waveform and a slow zoom. Slower than ffmpeg.
fn run_render(cfg: config::Config, args: &[String]) -> io::Result<()> {
    let [_, target] = args else {
        eprintln!("usage: melodias render <output-dir>");
        std::process::exit(2);
    };
    let requested = PathBuf::from(target);
    let dir = if requested.is_dir() {
        requested
    } else {
        cfg.out_dir.join(target)
    };
    let dir = std::fs::canonicalize(&dir)?;
    let image = dir.join("thumbnail.png");
    let audio = dir.join("audio.mp3");
    for path in [&image, &audio] {
        if !path.is_file() {
            eprintln!("missing {}", path.display());
            std::process::exit(2);
        }
    }
    let project = std::env::current_dir()?.join("video");
    if !project.join("node_modules").is_dir() {
        eprintln!("the Remotion project is not installed; run: cd video && pnpm install");
        std::process::exit(2);
    }
    let current = project.join("public").join("current");
    std::fs::create_dir_all(&current)?;
    std::fs::copy(&image, current.join("thumbnail.png"))?;
    std::fs::copy(&audio, current.join("audio.mp3"))?;
    let duration =
        media::duration_secs(&audio).map_err(|error| io::Error::other(format!("{error:#}")))?;
    let props = project.join("props.json");
    std::fs::write(
        &props,
        serde_json::to_vec(&serde_json::json!({
            "durationSec": duration,
            "channel": "melodias8bit",
        }))?,
    )?;
    let output = dir.join("video.mp4");
    if output.is_file() {
        let _ = std::fs::copy(&output, dir.join("video-ffmpeg.mp4"));
    }
    let status = std::process::Command::new("pnpm")
        .current_dir(&project)
        .args([
            "exec",
            "remotion",
            "render",
            "src/index.ts",
            "MelodiasVideo",
        ])
        .arg(&output)
        .arg(format!("--props={}", props.display()))
        .arg("--concurrency=4")
        .status()?;
    if !status.success() {
        return Err(io::Error::other("remotion render failed"));
    }
    println!("video: {}", output.display());
    Ok(())
}

/// Regenerates the thumbnail of a finished run from its stored prompt, keeping
/// the chosen seed, then rebuilds the video. It only uses sd-server.
fn run_image(cfg: config::Config, args: &[String]) -> io::Result<()> {
    let [_, target] = args else {
        eprintln!("usage: melodias image <output-dir>");
        std::process::exit(2);
    };
    let requested = PathBuf::from(target);
    let dir = if requested.is_dir() {
        requested
    } else {
        cfg.out_dir.join(target)
    };
    let meta_path = dir.join("meta.json");
    let text = std::fs::read_to_string(&meta_path)?;
    let mut meta: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| io::Error::other(format!("invalid {}: {error}", meta_path.display())))?;
    let prompt = meta["image_prompt"]
        .as_str()
        .ok_or_else(|| io::Error::other("meta.json has no image_prompt"))
        .map(clients::sanitize_eye_prompt)?;
    let selected = meta["image"]["selected"].as_u64().unwrap_or(1) as usize;
    let count = meta["image"]["candidates"]
        .as_array()
        .map(|candidates| candidates.len() + 1)
        .unwrap_or(1);
    let seed = meta["image"]["candidates"]
        .as_array()
        .and_then(|candidates| candidates.get(selected.saturating_sub(1)))
        .and_then(|candidate| candidate["seed"].as_u64())
        .unwrap_or(1) as u32;
    let candidate = dir.join("candidates").join(format!("image-{count:02}.png"));

    media::ssh_run(&cfg, &cfg.audio_stop_cmd)
        .map_err(|error| io::Error::other(format!("{error:#}")))?;
    media::ssh_run(&cfg, &cfg.sd_start_cmd)
        .map_err(|error| io::Error::other(format!("{error:#}")))?;
    let generated = (|| -> anyhow::Result<()> {
        clients::wait_sd(&cfg, Duration::from_secs(180))?;
        let bytes = clients::generate_image(&cfg, &prompt, seed)?;
        if let Some(parent) = candidate.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&candidate, &bytes)?;
        media::png_size(&candidate)?;
        Ok(())
    })();
    let stopped = media::ssh_run(&cfg, &cfg.sd_stop_cmd);
    generated.map_err(|error| io::Error::other(format!("{error:#}")))?;
    if let Err(error) = stopped {
        eprintln!("warning: sd-server: {error:#}");
    }

    let thumbnail = dir.join("thumbnail.png");
    let backup = dir.join("thumbnail-old.png");
    if thumbnail.is_file() {
        std::fs::copy(&thumbnail, &backup)?;
    }
    std::fs::copy(&candidate, &thumbnail)?;

    if let Some(candidates) = meta["image"]["candidates"].as_array_mut() {
        candidates.push(serde_json::json!({
            "file": candidate.display().to_string(),
            "seed": seed,
        }));
    }
    meta["image"]["selected"] = serde_json::json!(count);
    meta["image"]["file"] = serde_json::json!(thumbnail.display().to_string());
    meta["image_prompt"] = serde_json::json!(prompt);
    std::fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?)?;

    println!("image: {}", thumbnail.display());
    let audio = dir.join("audio.mp3");
    let video = media::assemble_video(&cfg, &thumbnail, &audio, &dir)
        .map_err(|error| io::Error::other(format!("{error:#}")))?;
    println!("video: {}", video.display());
    Ok(())
}

/// Assembles the video of a finished run without touching the GPU.
fn run_video(cfg: config::Config, args: &[String]) -> io::Result<()> {
    let [_, target] = args else {
        eprintln!("usage: melodias video <output-dir>");
        std::process::exit(2);
    };
    let requested = std::path::PathBuf::from(target);
    let dir = if requested.is_dir() {
        requested
    } else {
        cfg.out_dir.join(target)
    };
    let image = dir.join("thumbnail.png");
    let audio = dir.join("audio.mp3");
    for path in [&image, &audio] {
        if !path.is_file() {
            eprintln!("missing {}", path.display());
            std::process::exit(2);
        }
    }
    let video = media::assemble_video(&cfg, &image, &audio, &dir)
        .map_err(|error| io::Error::other(format!("{error:#}")))?;
    println!("video: {}", video.display());
    Ok(())
}

struct Reporter {
    log: Option<File>,
    pretty: bool,
}

impl Reporter {
    fn new(path: Option<PathBuf>) -> io::Result<Self> {
        let log = match path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                media::set_child_log(path.clone());
                Some(OpenOptions::new().create(true).append(true).open(path)?)
            }
            None => None,
        };
        Ok(Self {
            log,
            pretty: io::stdout().is_terminal(),
        })
    }

    fn line(&mut self, plain: &str, colored: Option<String>) {
        if let Some(file) = self.log.as_mut() {
            let _ = writeln!(file, "{plain}");
        }
        match colored {
            Some(text) => println!("{text}"),
            None => println!("{plain}"),
        }
    }

    fn error(&mut self, plain: &str) {
        if let Some(file) = self.log.as_mut() {
            let _ = writeln!(file, "{plain}");
        }
        if self.pretty {
            eprintln!("{}", plain.red().bold());
        } else {
            eprintln!("{plain}");
        }
    }

    fn stage(&mut self, idx: usize, state: State) {
        let Some(plain) = pipeline::stage_label(idx, state) else {
            return;
        };
        if state == State::Running {
            self.line("", None);
        }
        let colored = match state {
            State::Running => self
                .pretty
                .then(|| plain.as_str().yellow().bold().to_string()),
            State::Ok => self.pretty.then(|| plain.as_str().green().to_string()),
            State::Failed => self.pretty.then(|| plain.as_str().red().bold().to_string()),
            State::Pending => None,
        };
        self.line(&plain, colored);
    }

    fn log_line(&mut self, text: &str) {
        let plain = format!("   · {text}");
        let colored = self.pretty.then(|| plain.as_str().dark_grey().to_string());
        self.line(&plain, colored);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_clean_log() {
        let path = std::env::temp_dir().join(format!("melodias-studio-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut out = Reporter::new(Some(path.clone())).unwrap();
        out.stage(0, State::Running);
        out.stage(0, State::Ok);
        out.log_line("image prompt: pixel art castle");

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("▶ 1/11 Image prompt"));
        assert!(text.contains("✔ 1/11 Image prompt"));
        assert!(text.contains("   · image prompt: pixel art castle"));
        let _ = std::fs::remove_file(path);
    }
}
