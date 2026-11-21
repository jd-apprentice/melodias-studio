//! Experimental NES cover: source audio -> Demucs stems -> Basic Pitch -> a
//! FamiStudio text project -> FamiStudio render -> stereo mix and master.
//!
//! This command is optional and does not touch the main pipeline. It reuses
//! the local Demucs and Basic Pitch virtualenv, the FamiStudio CLI and the
//! arrangement module in `src/chip/`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::media;
use crate::pipeline::slugify;

mod arrange;
mod audio;
mod dmc;
mod midi;

const USAGE: &str = "usage: melodias chip <url|file> [--title TITLE] [--out DIR] [--keep-work]

  Renders an experimental NES-style cover of a track. It downloads the audio,
  separates it with Demucs, transcribes melody and bass with Basic Pitch,
  writes a FamiStudio project, renders it and mixes a stereo master.

  Tools (override with env vars):
    CHIP_VENV       virtualenv with demucs and basic-pitch
                    (default: ~/.local/share/melodias-studio/venvs/audio8bit)
    FAMISTUDIO_BIN  FamiStudio executable
                    (default: ~/.local/share/melodias-studio/tools/famistudio/FamiStudio)
    DOTNET_ROOT     .NET runtime for FamiStudio
                    (default: ~/.local/share/melodias-studio/tools/dotnet)

  Output: <out>/<slug>/ with audio.wav, audio.mp3, preview.mp3, project.txt
  and meta.json. Work files live in <out>/<slug>/work and are removed unless
  --keep-work is passed.";

pub struct Options {
    pub input: String,
    pub title: Option<String>,
    pub out: Option<PathBuf>,
    pub keep_work: bool,
}

fn parse(args: &[String]) -> Result<Options> {
    let mut input = None;
    let mut title = None;
    let mut out = None;
    let mut keep_work = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--title" => {
                index += 1;
                title = Some(
                    args.get(index)
                        .cloned()
                        .context("missing value for --title")?,
                );
            }
            "--out" => {
                index += 1;
                out = Some(PathBuf::from(
                    args.get(index)
                        .cloned()
                        .context("missing value for --out")?,
                ));
            }
            "--keep-work" => keep_work = true,
            other if other.starts_with('-') => bail!("unknown option: {other}"),
            other => {
                if input.is_some() {
                    bail!("unexpected argument: {other}");
                }
                input = Some(other.to_string());
            }
        }
        index += 1;
    }
    Ok(Options {
        input: input.context(USAGE)?,
        title,
        out,
        keep_work,
    })
}

fn data_dir() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("melodias-studio")
}

fn from_env(name: &str, default: PathBuf) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or(default)
}

fn run(mut command: Command, label: &str) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("running {label}"))?;
    if !status.success() {
        bail!("{label} failed");
    }
    Ok(())
}

pub fn run_chip(cfg: &Config, args: &[String]) -> Result<()> {
    let options = parse(args)?;
    let base = data_dir();
    let venv = from_env("CHIP_VENV", base.join("venvs/audio8bit"));
    let famistudio = from_env("FAMISTUDIO_BIN", base.join("tools/famistudio/FamiStudio"));
    let dotnet = from_env("DOTNET_ROOT", base.join("tools/dotnet"));

    for tool in ["python", "demucs", "basic-pitch"] {
        let path = venv.join("bin").join(tool);
        if !path.is_file() {
            bail!(
                "missing {}; set CHIP_VENV to a virtualenv with demucs and basic-pitch (see docs/usage.md)",
                path.display()
            );
        }
    }
    if !famistudio.is_file() {
        bail!(
            "missing {}; set FAMISTUDIO_BIN to the FamiStudio executable",
            famistudio.display()
        );
    }
    if !dotnet.is_dir() {
        bail!(
            "missing {}; set DOTNET_ROOT to a .NET runtime",
            dotnet.display()
        );
    }

    let is_url = options.input.starts_with("http://") || options.input.starts_with("https://");
    let title = match &options.title {
        Some(title) => title.clone(),
        None if is_url => yt_title(&options.input).unwrap_or_else(|| "chip".to_string()),
        None => Path::new(&options.input)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "chip".to_string()),
    };
    let slug = {
        let slug = slugify(&title);
        if slug.is_empty() {
            "chip".to_string()
        } else {
            slug
        }
    };
    let root = options
        .out
        .clone()
        .unwrap_or_else(|| cfg.out_dir.join("chip"));
    let out_dir = root.join(&slug);
    let work = out_dir.join("work");
    fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;

    println!("chip: {title}");
    println!("output: {}", out_dir.display());

    let source = work.join("source.wav");
    if is_url {
        println!("[1/6] downloading audio");
        run(
            {
                let mut command = Command::new("yt-dlp");
                command
                    .args(["--no-warnings", "-x", "--audio-format", "wav", "-o"])
                    .arg(work.join("source.%(ext)s"))
                    .arg(&options.input);
                command
            },
            "yt-dlp",
        )?;
    } else {
        println!("[1/6] preparing source audio");
        if !Path::new(&options.input).is_file() {
            bail!("no such file: {}", options.input);
        }
        run(
            {
                let mut command = Command::new("ffmpeg");
                command
                    .args(["-v", "error", "-y", "-i"])
                    .arg(&options.input)
                    .args(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"])
                    .arg(&source);
                command
            },
            "ffmpeg",
        )?;
    }
    if !source.is_file() {
        bail!("the source audio was not written to {}", source.display());
    }

    println!("[2/6] separating stems with Demucs");
    run(
        {
            let mut command = Command::new(venv.join("bin/demucs"));
            command
                .args(["-n", "htdemucs", "-o"])
                .arg(work.join("stems"))
                .arg(&source);
            command
        },
        "demucs",
    )?;
    let stems = work.join("stems/htdemucs/source");
    let other = stems.join("other.wav");
    let bass = stems.join("bass.wav");
    let drums = stems.join("drums.wav");
    for stem in [&other, &bass, &drums] {
        if !stem.is_file() {
            bail!("Demucs did not write {}", stem.display());
        }
    }

    println!("[3/6] transcribing melody and bass with Basic Pitch");
    for (name, input) in [("other", &other), ("bass", &bass)] {
        let target = work.join(format!("bp-{name}"));
        fs::create_dir_all(&target)?;
        run(
            {
                let mut command = Command::new(venv.join("bin/basic-pitch"));
                command.arg(&target).arg(input);
                command
            },
            &format!("basic-pitch ({name})"),
        )?;
    }
    let other_mid = work.join("bp-other/other_basic_pitch.mid");
    let bass_mid = work.join("bp-bass/bass_basic_pitch.mid");
    for midi in [&other_mid, &bass_mid] {
        if !midi.is_file() {
            bail!("Basic Pitch did not write {}", midi.display());
        }
    }

    println!("[4/6] building the FamiStudio project");
    let project = work.join("project.txt");
    let melody = midi::read(&other_mid)?;
    let bass_notes = midi::read(&bass_mid)?;
    let summary = arrange::build(arrange::Request {
        melody: &melody,
        bass: &bass_notes,
        source: &source,
        drums_stem: &drums,
        title: &title,
        out: &project,
    })?;
    println!(
        "tempo {:.1} BPM, note length {} frames",
        summary.tempo, summary.note_length
    );
    println!(
        "project: {} patterns, {} lead, {} harmony, {} bass, {} dpcm, {} noise",
        summary.patterns, summary.lead, summary.harmony, summary.bass, summary.dpcm, summary.noise
    );
    println!(
        "textures: {} sparse, {} normal, {} full",
        summary.textures[0], summary.textures[1], summary.textures[2]
    );

    println!("[5/6] rendering with FamiStudio");
    let rendered = work.join("stems.wav");
    run(
        {
            let mut command = Command::new(&famistudio);
            command
                .env("DOTNET_ROOT", &dotnet)
                .env("SDL_VIDEODRIVER", "dummy")
                .arg(&project)
                .args(["wav-export"])
                .arg(&rendered)
                .args(["-wav-export-rate:48000", "-wav-export-separate-channels"]);
            command
        },
        "FamiStudio",
    )?;

    println!("[6/6] mixing and mastering");
    let audio = out_dir.join("audio.wav");
    let filter = concat!(
        "[0:a]aecho=0.8:0.5:60:0.08,pan=stereo|c0=0.95*c0|c1=0.3*c0[p1];",
        "[1:a]volume=1.7,adelay=0|14,pan=stereo|c0=0.3*c0|c1=0.95*c0[p2];",
        "[2:a]volume=0.62,pan=stereo|c0=0.6*c0|c1=0.6*c0[tri];",
        "[3:a]volume=5.6,pan=stereo|c0=0.55*c0|c1=0.55*c0[noi];",
        "[4:a]volume=1.05,pan=stereo|c0=0.6*c0|c1=0.6*c0[dmc];",
        "[p1][p2][tri][noi][dmc]amix=inputs=5:normalize=0,alimiter=limit=0.95,",
        "highpass=f=45,equalizer=f=160:t=q:w=0.8:g=-3,lowpass=f=16000,",
        "loudnorm=I=-16:TP=-2:LRA=11"
    );
    run(
        {
            let mut command = Command::new("ffmpeg");
            command.args(["-v", "error", "-y"]);
            for channel in ["Square1", "Square2", "Triangle", "Noise", "DPCM"] {
                command
                    .arg("-i")
                    .arg(work.join(format!("stems_{channel}.wav")));
            }
            command
                .args(["-filter_complex", filter, "-ar", "48000"])
                .arg(&audio);
            command
        },
        "ffmpeg mix",
    )?;

    let mp3 = out_dir.join("audio.mp3");
    run(
        {
            let mut command = Command::new("ffmpeg");
            command
                .args(["-v", "error", "-y", "-i"])
                .arg(&audio)
                .args(["-codec:a", "libmp3lame", "-b:a", "192k"])
                .arg(&mp3);
            command
        },
        "ffmpeg mp3",
    )?;

    let preview = out_dir.join("preview.mp3");
    if media::duration_secs(&audio).unwrap_or(0.0) > 45.0 {
        run(
            {
                let mut command = Command::new("ffmpeg");
                command
                    .args(["-v", "error", "-y", "-ss", "20", "-t", "25", "-i"])
                    .arg(&audio)
                    .args(["-codec:a", "libmp3lame", "-b:a", "192k"])
                    .arg(&preview);
                command
            },
            "ffmpeg preview",
        )?;
    }

    fs::copy(&project, out_dir.join("project.txt"))
        .with_context(|| "copying the FamiStudio project")?;
    let meta = serde_json::json!({
        "input": options.input,
        "title": title,
        "slug": slug,
        "created_at": crate::refs::util::now_iso(),
        "files": {
            "audio": audio.display().to_string(),
            "mp3": mp3.display().to_string(),
            "preview": preview.display().to_string(),
            "project": out_dir.join("project.txt").display().to_string(),
        },
        "tools": {
            "venv": venv.display().to_string(),
            "famistudio": famistudio.display().to_string(),
            "dotnet": dotnet.display().to_string(),
        }
    });
    fs::write(
        out_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta)?,
    )?;

    if !options.keep_work {
        let _ = fs::remove_dir_all(&work);
    }

    println!("done:");
    println!("  audio:   {}", mp3.display());
    println!("  wav:     {}", audio.display());
    if preview.is_file() {
        println!("  preview: {}", preview.display());
    }
    println!("  project: {}", out_dir.join("project.txt").display());
    Ok(())
}

fn yt_title(url: &str) -> Option<String> {
    let output = Command::new("yt-dlp")
        .args(["--no-warnings", "--print", "%(title)s", url])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
#[path = "tests/chip.rs"]
mod tests;
