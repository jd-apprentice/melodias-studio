//! The video stage. Remotion is the default and there is no silent fallback:
//! if the project or pnpm is missing, the stage fails with the command to fix
//! it. ffmpeg is used only when `VIDEO_RENDERER=ffmpeg` or through
//! `melodias video`, which is the explicit fast path.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::media;

pub struct Rendered {
    pub file: PathBuf,
    pub renderer: &'static str,
    pub particles: String,
}

/// Checks the renderer before the GPU work starts, so a missing Remotion
/// project does not waste a whole run.
pub fn preflight(cfg: &Config) -> Result<()> {
    if renderer(cfg)? == "ffmpeg" {
        return Ok(());
    }
    check_remotion(cfg, particles_value(&cfg.video_particles)?)
}

pub fn render(
    cfg: &Config,
    image: &Path,
    audio: &Path,
    dir: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<Rendered> {
    if renderer(cfg)? == "ffmpeg" {
        let file = media::assemble_video(cfg, image, audio, dir)?;
        return Ok(Rendered {
            file,
            renderer: "ffmpeg",
            particles: String::new(),
        });
    }

    let particles = particles_value(&cfg.video_particles)?.to_string();
    let output = dir.join("video.mp4");
    if output.is_file() {
        std::fs::copy(&output, dir.join("video-ffmpeg.mp4"))
            .context("keeping the previous video")?;
    }
    let temp = dir.join("work").join("video-remotion.mp4");
    if let Some(parent) = temp.parent() {
        std::fs::create_dir_all(parent)?;
    }
    log(&format!(
        "remotion: rendering with particles={particles}, this takes about 12 minutes"
    ));
    if let Err(error) = render_remotion(cfg, image, audio, &temp, log) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    std::fs::rename(&temp, &output).with_context(|| {
        format!(
            "moving the Remotion video from {} to {}",
            temp.display(),
            output.display()
        )
    })?;
    Ok(Rendered {
        file: output,
        renderer: "remotion",
        particles,
    })
}

/// Explicit Remotion render for `melodias render`, regardless of
/// `VIDEO_RENDERER`.
pub fn render_remotion(
    cfg: &Config,
    image: &Path,
    audio: &Path,
    output: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<()> {
    // Remotion runs with the project as its working directory, so a relative
    // output path would land inside the project instead of the run directory.
    let output = absolute(output)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let particles = particles_value(&cfg.video_particles)?;
    check_remotion(cfg, particles)?;
    let project = project_dir(cfg);
    let current = project.join("public").join("current");
    std::fs::create_dir_all(&current)?;
    std::fs::copy(image, current.join("thumbnail.png")).context("copying the thumbnail")?;
    std::fs::copy(audio, current.join("audio.mp3")).context("copying the audio")?;
    media::waveform_overlay(
        &current.join("audio.mp3"),
        &current.join("waveform.mp4"),
        cfg.video_width,
        cfg.video_height,
    )
    .context("rendering the waveform overlay")?;
    let duration = media::duration_secs(audio)?;
    let props = project.join("props.json");
    std::fs::write(
        &props,
        serde_json::to_vec(&serde_json::json!({
            "durationSec": duration,
            "channel": "melodias8bit",
            "particles": particles,
        }))?,
    )?;
    log(&format!(
        "remotion: rendering {} with particles={particles}",
        output.display()
    ));
    let mut cmd = Command::new("pnpm");
    cmd.current_dir(&project)
        .args([
            "exec",
            "remotion",
            "render",
            "src/index.ts",
            "MelodiasVideo",
        ])
        .arg(output)
        .arg(format!("--props={}", props.display()))
        .arg("--concurrency=4");
    media::attach_output(&mut cmd);
    let status = cmd.status().context("running the Remotion renderer")?;
    if !status.success() {
        bail!("the Remotion renderer failed");
    }
    Ok(())
}

fn check_remotion(cfg: &Config, particles: &str) -> Result<()> {
    let project = project_dir(cfg);
    if !project.join("node_modules").is_dir() {
        bail!(
            "the Remotion project is not installed: run cd {} && pnpm install",
            project.display()
        );
    }
    if !pnpm_available() {
        bail!("pnpm is not on PATH; install it or set VIDEO_RENDERER=ffmpeg");
    }
    for name in required_overlays(particles) {
        let path = project.join("public").join("overlays").join(name);
        if !path.is_file() {
            bail!("missing Remotion overlay: {}", path.display());
        }
    }
    Ok(())
}

fn renderer(cfg: &Config) -> Result<&str> {
    let value = cfg.video_renderer.trim();
    if value.eq_ignore_ascii_case("ffmpeg") {
        Ok("ffmpeg")
    } else if value.eq_ignore_ascii_case("remotion") {
        Ok("remotion")
    } else {
        bail!("VIDEO_RENDERER must be remotion or ffmpeg, got {value}");
    }
}

pub fn particles_value(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "both" => Ok("both"),
        "bokeh" => Ok("bokeh"),
        "sparks" => Ok("sparks"),
        other => bail!("VIDEO_PARTICLES must be both, bokeh or sparks, got {other}"),
    }
}

fn required_overlays(particles: &str) -> &'static [&'static str] {
    match particles {
        "bokeh" => &["particles-bokeh.mp4", "birds.mp4"],
        "sparks" => &["particles-sparks.mp4", "birds.mp4"],
        _ => &["particles-bokeh.mp4", "particles-sparks.mp4", "birds.mp4"],
    }
}

fn project_dir(cfg: &Config) -> PathBuf {
    if cfg.video_project.is_absolute() {
        cfg.video_project.clone()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&cfg.video_project)
    }
}

/// Resolves a path against the working directory without requiring the file to
/// exist, so relative run directories keep working next to the Remotion project.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("resolving the working directory")?
        .join(path))
}

fn pnpm_available() -> bool {
    Command::new("pnpm")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
#[path = "tests/video.rs"]
mod tests;
