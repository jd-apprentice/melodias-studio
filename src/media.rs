use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};

static CHILD_LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Sends child process output to the run log instead of the terminal, so a long
/// pipeline run and its tools do not fight for the screen.
pub fn set_child_log(path: PathBuf) {
    if let Ok(mut slot) = CHILD_LOG.lock() {
        *slot = Some(path);
    }
}

fn log_file() -> Option<std::fs::File> {
    let path = CHILD_LOG.lock().ok()?.clone()?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

pub(crate) fn attach_output(cmd: &mut Command) {
    if let Some(file) = log_file()
        && let Ok(err) = file.try_clone()
    {
        cmd.stdout(file).stderr(err);
    }
}

fn attach_stderr(cmd: &mut Command) {
    if let Some(file) = log_file() {
        cmd.stderr(file);
    }
}

pub fn preflight() -> Result<()> {
    for bin in ["yt-dlp", "ffmpeg", "ffprobe", "scp"] {
        if which(bin).is_none() {
            bail!("{bin} is not on PATH");
        }
    }
    Ok(())
}

fn which(bin: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
}

/// Reads the PNG signature and IHDR, so a broken or resized thumbnail is caught
/// before it ends up in the output folder. Does not decode the pixels.
pub fn png_size(path: &Path) -> Result<(u32, u32)> {
    let mut header = [0u8; 24];
    std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .read_exact(&mut header)
        .with_context(|| format!("reading the header of {}", path.display()))?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        bail!("{} is not a valid PNG", path.display());
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    Ok((width, height))
}

pub fn download_audio(url: &str, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let template = dir.join("source.%(ext)s");
    let mut cmd = Command::new("yt-dlp");
    cmd.args(["--no-playlist", "-x", "--audio-format", "wav"])
        .args(["--postprocessor-args", "-ar 48000 -ac 2"])
        .arg("-o")
        .arg(&template)
        .arg(url);
    attach_output(&mut cmd);
    let status = cmd.status().context("running yt-dlp")?;
    if !status.success() {
        bail!("yt-dlp failed with {status}");
    }
    let wav = dir.join("source.wav");
    if !wav.is_file() {
        bail!("yt-dlp did not leave {}", wav.display());
    }
    Ok(wav)
}

/// Picks the window with the highest average energy, which tends to skip
/// intros and breakdowns, and writes it as a 48 kHz stereo reference WAV.
pub fn pick_reference(cfg: &crate::config::Config, src: &Path, dir: &Path) -> Result<PathBuf> {
    let total = duration_secs(src)?;
    let reference_sec = cfg.reference_sec.clamp(5, 120);
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-i"])
        .arg(src)
        .args(["-f", "f32le", "-ac", "1", "-ar", "8000", "-"])
        .stdout(Stdio::piped());
    attach_stderr(&mut cmd);
    let mut child = cmd.spawn().context("decoding to measure energy")?;

    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .context("ffmpeg stdout")?
        .read_to_end(&mut raw)?;
    let _ = child.wait();

    let samples: Vec<f32> = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let win = 5 * 8000usize;
    let len = reference_sec as usize * 8000usize;

    let mut start = 0usize;
    if samples.len() > len {
        let mut best = f64::MIN;
        let mut cursor = 0usize;
        while cursor + len <= samples.len() {
            let mut sum = 0.0f64;
            let mut i = cursor;
            while i < cursor + len {
                let end = (i + win).min(cursor + len);
                let slice = &samples[i..end];
                let rms: f64 = slice.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
                    / slice.len() as f64;
                sum += rms;
                i = end;
            }
            if sum > best {
                best = sum;
                start = cursor;
            }
            cursor += win;
        }
    }

    let begin = (start / 8000) as f64;
    let seconds = len.min(samples.len()) as f64 / 8000.0;
    let out = dir.join("reference.wav");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-ss"])
        .arg(format!("{begin:.2}"))
        .args(["-t"])
        .arg(format!("{seconds:.2}"))
        .arg("-i")
        .arg(src)
        .args(["-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"])
        .arg(&out);
    attach_output(&mut cmd);
    let status = cmd.status().context("cutting the reference")?;
    if !status.success() {
        bail!("ffmpeg failed while cutting the reference");
    }
    if total < 10.0 {
        bail!("the source is {total:.1} s long, too short to use as a reference");
    }
    Ok(out)
}

/// Copies the reference to the GPU VM and returns the path the audio server
/// should read. With an empty `SSH_HOST` the local file is used as is.
pub fn push_reference(cfg: &crate::config::Config, local: &Path, slug: &str) -> Result<String> {
    if cfg.ssh_host.trim().is_empty() {
        let absolute = std::fs::canonicalize(local).unwrap_or_else(|_| local.to_path_buf());
        return Ok(absolute.to_string_lossy().into_owned());
    }
    let remote = format!("{}/{}.wav", cfg.remote_ref_dir.trim_end_matches('/'), slug);
    let mut mkdir = Command::new("ssh");
    mkdir
        .arg(&cfg.ssh_host)
        .arg(format!("mkdir -p {}", cfg.remote_ref_dir));
    attach_output(&mut mkdir);
    let status = mkdir.status().context("creating the remote directory")?;
    if !status.success() {
        bail!(
            "could not create {} on {}",
            cfg.remote_ref_dir,
            cfg.ssh_host
        );
    }
    let mut copy = Command::new("scp");
    copy.arg("-q")
        .arg(local)
        .arg(format!("{}:{remote}", cfg.ssh_host));
    attach_output(&mut copy);
    let status = copy.status().context("copying the reference")?;
    if !status.success() {
        bail!("scp failed while copying the reference");
    }
    Ok(remote)
}

/// Keeps the take as a standalone song: loudness normalization and configurable
/// edge fades. It never crossfades the outro into the intro, so the natural
/// ending and the opening build stay in place.
pub fn master_take(cfg: &crate::config::Config, src: &Path, dir: &Path) -> Result<PathBuf> {
    let fade_in = cfg.fade_in_sec.max(0.0);
    let fade_out = cfg.fade_out_sec.max(0.0);
    let total = duration_secs(src)?;
    if total <= fade_in + fade_out + 5.0 {
        bail!("the take is {total:.1} s long, too short to master");
    }
    let mut chain = format!("loudnorm=I={}:TP=-1.5", cfg.loudness_lufs);
    if fade_in > 0.0 {
        chain.push_str(&format!(",afade=t=in:st=0:d={fade_in}"));
    }
    if fade_out > 0.0 {
        chain.push_str(&format!(
            ",afade=t=out:st={:.3}:d={fade_out}",
            total - fade_out
        ));
    }
    let graph = format!("[0:a]{chain}[out]");
    let out = dir.join("master.wav");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args(["-filter_complex", &graph, "-map", "[out]"])
        .args(["-c:a", "pcm_s16le"])
        .arg(&out);
    attach_output(&mut cmd);
    let status = cmd.status().context("mastering the take")?;
    if !status.success() {
        bail!("ffmpeg failed while mastering the take");
    }
    Ok(out)
}

struct Particle {
    x: f64,
    y: f64,
    sway: f64,
    bob: f64,
    speed: f64,
    phase: f64,
    size: f64,
    color: &'static str,
    sparkle: bool,
}

const PARTICLES: [Particle; 10] = [
    Particle {
        x: 0.10,
        y: 0.18,
        sway: 40.0,
        bob: 25.0,
        speed: 0.50,
        phase: 0.0,
        size: 18.0,
        color: "0x56d4dd",
        sparkle: false,
    },
    Particle {
        x: 0.24,
        y: 0.35,
        sway: 55.0,
        bob: 30.0,
        speed: 0.70,
        phase: 1.7,
        size: 14.0,
        color: "0xe3b341",
        sparkle: false,
    },
    Particle {
        x: 0.42,
        y: 0.12,
        sway: 70.0,
        bob: 20.0,
        speed: 0.40,
        phase: 3.1,
        size: 22.0,
        color: "0xf0f6fc",
        sparkle: false,
    },
    Particle {
        x: 0.62,
        y: 0.22,
        sway: 45.0,
        bob: 35.0,
        speed: 0.60,
        phase: 4.4,
        size: 14.0,
        color: "0x56d4dd",
        sparkle: false,
    },
    Particle {
        x: 0.78,
        y: 0.10,
        sway: 60.0,
        bob: 25.0,
        speed: 0.80,
        phase: 2.2,
        size: 18.0,
        color: "0xe3b341",
        sparkle: false,
    },
    Particle {
        x: 0.88,
        y: 0.40,
        sway: 35.0,
        bob: 30.0,
        speed: 0.50,
        phase: 5.0,
        size: 12.0,
        color: "0xf0f6fc",
        sparkle: false,
    },
    Particle {
        x: 0.53,
        y: 0.55,
        sway: 80.0,
        bob: 25.0,
        speed: 0.30,
        phase: 0.9,
        size: 12.0,
        color: "0x56d4dd",
        sparkle: false,
    },
    Particle {
        x: 0.33,
        y: 0.70,
        sway: 40.0,
        bob: 20.0,
        speed: 0.90,
        phase: 3.8,
        size: 10.0,
        color: "0xe3b341",
        sparkle: false,
    },
    Particle {
        x: 0.70,
        y: 0.32,
        sway: 30.0,
        bob: 20.0,
        speed: 0.60,
        phase: 1.2,
        size: 32.0,
        color: "0xf0f6fc",
        sparkle: true,
    },
    Particle {
        x: 0.18,
        y: 0.52,
        sway: 50.0,
        bob: 25.0,
        speed: 0.50,
        phase: 2.7,
        size: 28.0,
        color: "0xe3b341",
        sparkle: true,
    },
];

/// Builds the radial sprite used by the particle overlays.
fn particle_sprite(dir: &Path) -> Result<PathBuf> {
    let path = dir.join("particle.png");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=128x128:d=1")
        .args([
            "-vf",
            "format=rgba,geq=r=255:g=255:b=255:a='255*exp(-((X-64)^2+(Y-64)^2)/(2*22^2))'",
        ])
        .args(["-frames:v", "1"])
        .arg(&path);
    attach_output(&mut cmd);
    let status = cmd.status().context("building the particle sprite")?;
    if !status.success() {
        bail!("ffmpeg failed while building the particle sprite");
    }
    Ok(path)
}

/// One overlay per particle. `overlay` with `eval=frame` is what makes them
/// move; the drawbox expressions are evaluated only once.
fn particle_overlay() -> (String, String) {
    let count = PARTICLES.len();
    let mut filters = Vec::new();
    let labels: Vec<String> = (0..count).map(|index| format!("sp{index}")).collect();
    let outputs: String = labels.iter().map(|label| format!("[{label}]")).collect();
    filters.push(format!("[2:v]split={count}{outputs}"));
    let mut base = "bg".to_string();
    for (index, particle) in PARTICLES.iter().enumerate() {
        let size = (particle.size * 4.0).max(40.0);
        let half = size / 2.0;
        let (red, green, blue) = match particle.color {
            "0x56d4dd" => (0.34, 0.83, 0.87),
            "0xe3b341" => (0.89, 0.70, 0.25),
            _ => (0.94, 0.96, 0.99),
        };
        let alpha = if particle.sparkle { 0.75 } else { 0.55 };
        filters.push(format!(
            "[sp{index}]scale={size:.0}:{size:.0},format=rgba,\
             colorchannelmixer=rr={red}:gg={green}:bb={blue}:aa={alpha}[o{index}]"
        ));
        let cx = format!(
            "W*{:.3}+{:.1}*sin(t*{:.2}+{:.2})",
            particle.x, particle.sway, particle.speed, particle.phase
        );
        let cy = format!(
            "H*{:.3}+{:.1}*cos(t*{:.2}+{:.2})",
            particle.y,
            particle.bob,
            particle.speed * 0.8,
            particle.phase
        );
        filters.push(format!(
            "[{base}][o{index}]overlay=x='{cx}-{half:.1}':y='{cy}-{half:.1}':eval=frame[b{index}]"
        ));
        base = format!("b{index}");
    }
    (filters.join(";"), base)
}

/// Builds the final video from the chosen still and the mastered track: a slow
/// zoom, glowing particles, an audio waveform and edge fades.
pub fn assemble_video(
    cfg: &crate::config::Config,
    image: &Path,
    audio: &Path,
    dir: &Path,
) -> Result<PathBuf> {
    let total = duration_secs(audio)?;
    if total < 5.0 {
        bail!("the track is {total:.1} s long, too short to assemble a video");
    }
    let width = cfg.video_width.max(320) & !1;
    let height = cfg.video_height.max(240) & !1;
    let band = (height / 5).max(120);
    let wave_height = (height / 6).max(90);
    let fade_out_start = (total - 3.0).max(0.0);
    std::fs::create_dir_all(dir.join("work"))?;
    let sprite = particle_sprite(&dir.join("work"))?;
    let (particles, base) = particle_overlay();
    let graph = format!(
        "[0:v]scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height},\
         zoompan=z='min(zoom+0.00015,1.08)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s={width}x{height}:fps=30,\
         drawbox=y=ih-{band}:w=iw:h={band}:color=black@0.22:t=fill[bg];\
         {particles};\
         [1:a]asplit=2[aout][awave];\
         [awave]showwaves=s={width}x{wave_height}:mode=cline:rate=30:colors=0x56d4dd,format=rgba,colorchannelmixer=aa=0.55[wave];\
         [{base}][wave]overlay=0:H-h-30:format=auto,\
         fade=t=in:st=0:d=1.5,fade=t=out:st={fade_out_start:.3}:d=3,format=yuv420p[v]"
    );
    let out = dir.join("video.mp4");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-loop", "1", "-framerate", "30", "-i"])
        .arg(image)
        .arg("-i")
        .arg(audio)
        .args(["-loop", "1", "-framerate", "30", "-i"])
        .arg(&sprite)
        .args(["-filter_complex", &graph, "-map", "[v]", "-map", "[aout]"])
        .args(["-t"])
        .arg(format!("{total:.3}"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "21",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            "-shortest",
        ])
        .arg(&out);
    attach_output(&mut cmd);
    let status = cmd.status().context("assembling the video")?;
    if !status.success() {
        bail!("ffmpeg failed while assembling the video");
    }
    Ok(out)
}

/// Runs an optional shell command on the GPU host. Empty commands are skipped.
pub fn ssh_run(cfg: &crate::config::Config, command: &str) -> Result<()> {
    if command.trim().is_empty() {
        return Ok(());
    }
    let mut cmd = Command::new("ssh");
    cmd.arg(&cfg.ssh_host).arg(command);
    attach_output(&mut cmd);
    let status = cmd.status().with_context(|| format!("ssh {command}"))?;
    if !status.success() {
        bail!("the remote command failed ({status}): {command}");
    }
    Ok(())
}

pub fn encode_mp3(wav: &Path, dir: &Path, kbps: u32) -> Result<PathBuf> {
    let out = dir.join("audio.mp3");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"])
        .arg(wav)
        .args(["-codec:a", "libmp3lame", "-b:a", &format!("{kbps}k")])
        .arg(&out);
    attach_output(&mut cmd);
    let status = cmd.status().context("encoding the mp3")?;
    if !status.success() {
        bail!("ffmpeg failed while encoding the mp3");
    }
    Ok(out)
}

pub fn duration_secs(path: &Path) -> Result<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .context("running ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe failed on {}", path.display());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim()
        .parse::<f64>()
        .with_context(|| format!("invalid duration: {text:?}"))
}

/// Renders the audio waveform used by the Remotion composition. It is the same
/// ffmpeg `showwaves` line as the fast renderer; the black background
/// disappears with `mix-blend-mode: screen` in the composition.
pub fn waveform_overlay(audio: &Path, output: &Path, width: u32, height: u32) -> Result<()> {
    let total = duration_secs(audio)?;
    let width = width.max(320) & !1;
    let height = height.max(240) & !1;
    let wave = (height / 6).max(90);
    let top = height - wave - 30;
    let graph = format!(
        "showwaves=s={width}x{wave}:mode=cline:rate=30:colors=0x56d4dd,\
         pad={width}:{height}:0:{top}:color=black,format=yuv420p"
    );
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"])
        .arg(audio)
        .args([
            "-filter_complex",
            &graph,
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "21",
            "-t",
        ])
        .arg(format!("{total:.3}"))
        .arg(output);
    attach_output(&mut cmd);
    let status = cmd.status().context("rendering the waveform overlay")?;
    if !status.success() {
        bail!("ffmpeg failed while rendering the waveform overlay");
    }
    Ok(())
}
