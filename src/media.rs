use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

pub fn preflight() -> Result<()> {
    for bin in ["yt-dlp", "ffmpeg", "ffprobe", "scp"] {
        if which(bin).is_none() {
            bail!("{bin} no está en PATH");
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
        .with_context(|| format!("abriendo {}", path.display()))?
        .read_exact(&mut header)
        .with_context(|| format!("leyendo la cabecera de {}", path.display()))?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        bail!("{} no es un PNG válido", path.display());
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    Ok((width, height))
}

pub fn download_audio(url: &str, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let template = dir.join("source.%(ext)s");
    let status = Command::new("yt-dlp")
        .args(["--no-playlist", "-x", "--audio-format", "wav"])
        .args(["--postprocessor-args", "-ar 48000 -ac 2"])
        .arg("-o")
        .arg(&template)
        .arg(url)
        .status()
        .context("ejecutando yt-dlp")?;
    if !status.success() {
        bail!("yt-dlp falló con {status}");
    }
    let wav = dir.join("source.wav");
    if !wav.is_file() {
        bail!("yt-dlp no dejó {}", wav.display());
    }
    Ok(wav)
}

/// Picks the 60 s window with the highest average energy, which tends to skip
/// intros and breakdowns, and writes it as a 48 kHz stereo reference WAV.
pub fn pick_reference(src: &Path, dir: &Path) -> Result<PathBuf> {
    let total = duration_secs(src)?;
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(src)
        .args(["-f", "f32le", "-ac", "1", "-ar", "8000", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .context("decodificando para medir energía")?;

    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .context("stdout de ffmpeg")?
        .read_to_end(&mut raw)?;
    let _ = child.wait();

    let samples: Vec<f32> = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let win = 5 * 8000usize;
    let len = 60 * 8000usize;

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
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-ss"])
        .arg(format!("{begin:.2}"))
        .args(["-t"])
        .arg(format!("{seconds:.2}"))
        .arg("-i")
        .arg(src)
        .args(["-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"])
        .arg(&out)
        .status()
        .context("cortando referencia")?;
    if !status.success() {
        bail!("ffmpeg falló cortando la referencia");
    }
    if total < 10.0 {
        bail!("la fuente dura {total:.1} s, muy corta para usarla como referencia");
    }
    Ok(out)
}

/// Copies the reference to the GPU VM and returns its remote path.
pub fn push_reference(cfg: &crate::config::Config, local: &Path, slug: &str) -> Result<String> {
    let remote = format!("{}/{}.wav", cfg.remote_ref_dir.trim_end_matches('/'), slug);
    let mkdir = Command::new("ssh")
        .arg(&cfg.ssh_host)
        .arg(format!("mkdir -p {}", cfg.remote_ref_dir))
        .status()
        .context("creando el directorio remoto")?;
    if !mkdir.success() {
        bail!("no pude crear {} en {}", cfg.remote_ref_dir, cfg.ssh_host);
    }
    let status = Command::new("scp")
        .arg("-q")
        .arg(local)
        .arg(format!("{}:{remote}", cfg.ssh_host))
        .status()
        .context("copiando la referencia")?;
    if !status.success() {
        bail!("scp falló copiando la referencia");
    }
    Ok(remote)
}

/// Fades the last 3 s into the first 3 s and normalizes to -15 LUFS.
pub fn make_loop(src: &Path, dir: &Path) -> Result<PathBuf> {
    let total = duration_secs(src)?;
    let xf = 3.0f64;
    if total <= xf * 2.0 + 5.0 {
        bail!("la toma dura {total:.1} s, muy corta para armar el loop");
    }
    let cut = total - xf;
    let graph = format!(
        "[0:a]atrim=0:{xf},asetpts=PTS-STARTPTS[head];\
         [0:a]atrim={xf}:{cut},asetpts=PTS-STARTPTS[core];\
         [0:a]atrim={cut}:{total},asetpts=PTS-STARTPTS[tail];\
         [tail][head]acrossfade=d={xf}:c1=tri:c2=tri[seam];\
         [core][seam]concat=n=2:v=0:a=1,loudnorm=I=-15:TP=-1.5[out]"
    );
    let out = dir.join("loop.wav");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args(["-filter_complex", &graph, "-map", "[out]"])
        .args(["-c:a", "pcm_s16le"])
        .arg(&out)
        .status()
        .context("armando el loop")?;
    if !status.success() {
        bail!("ffmpeg falló armando el loop");
    }
    Ok(out)
}

/// Runs an optional shell command on the GPU host. Empty commands are skipped.
pub fn ssh_run(cfg: &crate::config::Config, command: &str) -> Result<()> {
    if command.trim().is_empty() {
        return Ok(());
    }
    let status = Command::new("ssh")
        .arg(&cfg.ssh_host)
        .arg(command)
        .status()
        .with_context(|| format!("ssh {command}"))?;
    if !status.success() {
        bail!("el comando remoto falló ({status}): {command}");
    }
    Ok(())
}

pub fn encode_mp3(wav: &Path, dir: &Path, kbps: u32) -> Result<PathBuf> {
    let out = dir.join("audio.mp3");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(wav)
        .args(["-codec:a", "libmp3lame", "-b:a", &format!("{kbps}k")])
        .arg(&out)
        .status()
        .context("codificando mp3")?;
    if !status.success() {
        bail!("ffmpeg falló codificando el mp3");
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
        .context("ejecutando ffprobe")?;
    if !out.status.success() {
        bail!("ffprobe falló con {}", path.display());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim()
        .parse::<f64>()
        .with_context(|| format!("duración inválida: {text:?}"))
}
