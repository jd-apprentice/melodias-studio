use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::media;

pub const RUBRIC: &str = "score-v1";
pub const WEIGHT_TECHNICAL: f64 = 0.6;
pub const TARGET_TOTAL: f64 = 80.0;
pub const TARGET_TECHNICAL: f64 = 85.0;
pub const TARGET_EDITORIAL: f64 = 70.0;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub source: &'static str,
    pub severity: &'static str,
    pub metric: &'static str,
    pub value: f64,
    pub threshold: f64,
    pub delta: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideScore {
    pub score: f64,
    pub points: f64,
    pub evaluated: f64,
    pub findings: Vec<Finding>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateScore {
    pub index: usize,
    pub file: String,
    pub seed: Option<u32>,
    pub selected: bool,
    pub mode: &'static str,
    pub score: f64,
    pub points: f64,
    pub evaluated: f64,
    pub findings: Vec<Finding>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct RunScore {
    pub rubric: &'static str,
    pub dir: String,
    pub technical: f64,
    pub editorial: Option<f64>,
    pub total: f64,
    pub accepted: bool,
    pub gates: Vec<Finding>,
    pub images: Vec<CandidateScore>,
    pub audios: Vec<CandidateScore>,
    pub final_image: CandidateScore,
    pub final_audio: CandidateScore,
}

#[derive(Debug, Clone)]
pub struct ImageMetrics {
    pub width: u32,
    pub height: u32,
    pub aspect: f64,
    pub unique_colors: u32,
    pub luma_std: f64,
    pub saturation: f64,
    pub glow: f64,
    pub ocr_words: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct AudioMetrics {
    pub duration: f64,
    pub lufs: f64,
    pub peak: f64,
    pub centroid: f64,
    pub high_share: f64,
    pub build_db: f64,
    pub silence: f64,
}

/// One line of the rubric: the code, the measured value and the points it earned.
struct Metric {
    code: &'static str,
    metric: &'static str,
    points: f64,
    max: f64,
    value: f64,
    threshold: f64,
    message: String,
}

#[derive(Default)]
struct Builder {
    points: f64,
    evaluated: f64,
    findings: Vec<Finding>,
    skipped: Vec<&'static str>,
}

impl Builder {
    fn add(&mut self, metric: Metric) {
        self.points += metric.points;
        self.evaluated += metric.max;
        let delta = metric.points - metric.max;
        if delta < -1e-9 {
            self.findings.push(Finding {
                code: metric.code,
                source: "deterministic",
                severity: severity(delta),
                metric: metric.metric,
                value: round2(metric.value),
                threshold: round2(metric.threshold),
                delta: round2(delta),
                message: metric.message,
            });
        }
    }

    fn skip(&mut self, code: &'static str) {
        self.skipped.push(code);
    }

    fn finish(self) -> SideScore {
        let score = if self.evaluated > 0.0 {
            self.points / self.evaluated * 100.0
        } else {
            0.0
        };
        // Kept unrounded: the run total is built from these scores and only
        // rounded once, at the end.
        SideScore {
            score,
            points: self.points,
            evaluated: self.evaluated,
            findings: self.findings,
            skipped: self.skipped,
        }
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn severity(delta: f64) -> &'static str {
    let delta = delta.abs();
    if delta >= 10.0 {
        "high"
    } else if delta >= 4.0 {
        "medium"
    } else {
        "low"
    }
}

fn lerp(x: f64, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    if (x1 - x0).abs() < f64::EPSILON {
        return y1;
    }
    let value = y0 + (x - x0) * (y1 - y0) / (x1 - x0);
    value.clamp(y0.min(y1), y0.max(y1))
}

pub fn score_image(metrics: &ImageMetrics) -> SideScore {
    let mut builder = Builder::default();
    builder.add(image_palette(metrics.unique_colors));
    builder.add(image_contrast(metrics.luma_std));
    match metrics.ocr_words {
        Some(words) => builder.add(image_text(words)),
        None => builder.skip("IMG_TEXT"),
    }
    builder.add(image_glow(metrics.glow));
    builder.add(image_saturation(metrics.saturation));
    builder.finish()
}

fn image_palette(colors: u32) -> Metric {
    let value = colors as f64;
    let points = if value < 1.0 {
        0.0
    } else if value <= 256.0 {
        20.0
    } else if value <= 2048.0 {
        lerp(value.log2(), 8.0, 11.0, 20.0, 10.0)
    } else if value <= 16384.0 {
        lerp(value.log2(), 11.0, 14.0, 10.0, 0.0)
    } else {
        0.0
    };
    Metric {
        code: "IMG_PALETTE",
        metric: "unique_colors",
        points,
        max: 20.0,
        value,
        threshold: 16384.0,
        message: format!("{colors} colors: full-color render, not pixel art"),
    }
}

fn image_contrast(std: f64) -> Metric {
    let points = if std >= 0.18 {
        10.0
    } else if std >= 0.10 {
        10.0 * (std - 0.10) / 0.08
    } else {
        0.0
    };
    Metric {
        code: "IMG_CONTRAST",
        metric: "luma_std",
        points,
        max: 10.0,
        value: std,
        threshold: 0.18,
        message: format!("luma std {std:.3} at 168x94"),
    }
}

fn image_text(words: u32) -> Metric {
    let points = (10.0 - 10.0 * words as f64).max(0.0);
    Metric {
        code: "IMG_TEXT",
        metric: "ocr_words",
        points,
        max: 10.0,
        value: words as f64,
        threshold: 0.0,
        message: format!("{words} OCR words"),
    }
}

fn image_glow(share: f64) -> Metric {
    let points = if share <= 0.002 {
        5.0
    } else if share <= 0.02 {
        5.0 * (0.02 - share) / 0.018
    } else {
        0.0
    };
    Metric {
        code: "IMG_GLOW",
        metric: "glow_share",
        points,
        max: 5.0,
        value: share,
        threshold: 0.002,
        message: format!("{:.2}% of pixels glow", share * 100.0),
    }
}

fn image_saturation(saturation: f64) -> Metric {
    let points = if (0.30..=0.75).contains(&saturation) {
        5.0
    } else {
        let distance = if saturation < 0.30 {
            0.30 - saturation
        } else {
            saturation - 0.75
        };
        (5.0 * (1.0 - distance / 0.15)).max(0.0)
    };
    Metric {
        code: "IMG_SATURATION",
        metric: "saturation",
        points,
        max: 5.0,
        value: saturation,
        threshold: 0.30,
        message: format!("mean saturation {saturation:.2}"),
    }
}

pub fn score_audio(metrics: &AudioMetrics, target: f64, master: bool) -> SideScore {
    let mut builder = Builder::default();
    builder.add(audio_centroid(metrics.centroid));
    builder.add(audio_highs(metrics.high_share));
    builder.add(audio_build(metrics.build_db));
    builder.add(audio_silence(metrics.silence));
    if master {
        builder.add(audio_loudness(metrics.lufs));
        builder.add(audio_peak(metrics.peak));
        builder.add(audio_duration(metrics.duration, target));
    } else {
        builder.skip("AUD_LOUDNESS");
        builder.skip("AUD_PEAK");
        builder.skip("AUD_DURATION");
    }
    builder.finish()
}

fn audio_loudness(lufs: f64) -> Metric {
    let delta = (lufs + 15.0).abs();
    let points = if delta <= 1.0 {
        8.0
    } else if delta <= 2.0 {
        5.0
    } else {
        0.0
    };
    Metric {
        code: "AUD_LOUDNESS",
        metric: "lufs",
        points,
        max: 8.0,
        value: lufs,
        threshold: -15.0,
        message: format!("{lufs:.1} LUFS against -15"),
    }
}

fn audio_peak(peak: f64) -> Metric {
    let points = if peak <= -1.0 {
        8.0
    } else if peak <= -0.3 {
        4.0
    } else {
        0.0
    };
    Metric {
        code: "AUD_PEAK",
        metric: "true_peak",
        points,
        max: 8.0,
        value: peak,
        threshold: -1.0,
        message: format!("{peak:.1} dBFS true peak"),
    }
}

fn audio_centroid(centroid: f64) -> Metric {
    let points = if (2000.0..=4500.0).contains(&centroid) {
        8.0
    } else if (1200.0..2000.0).contains(&centroid) {
        lerp(centroid, 1200.0, 2000.0, 4.0, 8.0)
    } else if (4500.0..=6000.0).contains(&centroid) {
        lerp(centroid, 4500.0, 6000.0, 8.0, 4.0)
    } else {
        0.0
    };
    Metric {
        code: "AUD_CENTROID",
        metric: "centroid_hz",
        points,
        max: 8.0,
        value: centroid,
        threshold: 2000.0,
        message: format!("{centroid:.0} Hz centroid"),
    }
}

fn audio_highs(share: f64) -> Metric {
    let points = if share >= 0.08 {
        8.0
    } else if share >= 0.03 {
        lerp(share, 0.03, 0.08, 4.0, 8.0)
    } else {
        lerp(share, 0.0, 0.03, 0.0, 4.0)
    };
    Metric {
        code: "AUD_HIGHS",
        metric: "high_share",
        points,
        max: 8.0,
        value: share,
        threshold: 0.08,
        message: format!("{:.1}% of energy above 5 kHz", share * 100.0),
    }
}

fn audio_build(build: f64) -> Metric {
    let points = if build <= -2.0 {
        8.0
    } else if build <= 0.0 {
        lerp(build, -2.0, 0.0, 8.0, 4.0)
    } else {
        lerp(build, 0.0, 2.0, 4.0, 0.0)
    };
    Metric {
        code: "AUD_BUILD",
        metric: "build_db",
        points,
        max: 8.0,
        value: build,
        threshold: -2.0,
        message: format!("intro is {build:.1} dB against the middle"),
    }
}

fn audio_silence(seconds: f64) -> Metric {
    let points = if seconds <= 1.0 {
        5.0
    } else if seconds <= 2.0 {
        5.0 * (2.0 - seconds)
    } else {
        0.0
    };
    Metric {
        code: "AUD_SILENCE",
        metric: "silence_sec",
        points,
        max: 5.0,
        value: seconds,
        threshold: 1.0,
        message: format!("{seconds:.2} s silence in the body"),
    }
}

fn audio_duration(duration: f64, target: f64) -> Metric {
    let points = if (duration - target).abs() <= 2.0 {
        5.0
    } else {
        0.0
    };
    Metric {
        code: "AUD_DURATION",
        metric: "duration_sec",
        points,
        max: 5.0,
        value: duration,
        threshold: target,
        message: format!("{duration:.1} s against {target:.1}"),
    }
}

#[allow(clippy::chunks_exact_to_as_chunks)] // as_chunks needs a newer MSRV than 1.85
pub fn image_metrics(path: &Path) -> Result<ImageMetrics> {
    let (width, height) = media::png_size(path).context("reading the image size")?;
    let rgb = stdout_bytes(&[
        "-v".into(),
        "error".into(),
        "-i".into(),
        path.display().to_string(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-".into(),
    ])?;
    let expected = width as usize * height as usize * 3;
    if rgb.len() != expected {
        bail!(
            "ffmpeg returned {} bytes for {}x{}, expected {expected}",
            rgb.len(),
            width,
            height
        );
    }

    let mut seen = vec![0u64; (1 << 24) / 64];
    let mut unique = 0u32;
    let mut saturation_sum = 0.0f64;
    let mut glow_pixels = 0u64;
    for pixel in rgb.chunks_exact(3) {
        let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
        let key = (r as u32) << 16 | (g as u32) << 8 | b as u32;
        let slot = (key >> 6) as usize;
        let bit = 1u64 << (key & 63);
        if seen[slot] & bit == 0 {
            seen[slot] |= bit;
            unique += 1;
        }
        let max = r.max(g).max(b) as f64;
        let min = r.min(g).min(b) as f64;
        if max > 0.0 {
            let s = (max - min) / max;
            saturation_sum += s;
            if max >= 243.0 && s >= 0.85 {
                glow_pixels += 1;
            }
        }
    }
    let pixels = (width as usize * height as usize).max(1) as f64;

    let gray = stdout_bytes(&[
        "-v".into(),
        "error".into(),
        "-i".into(),
        path.display().to_string(),
        "-vf".into(),
        "scale=168:94:flags=area".into(),
        "-pix_fmt".into(),
        "gray".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ])?;
    if gray.len() != 168 * 94 {
        bail!("ffmpeg returned {} bytes for the thumbnail", gray.len());
    }
    let luma_std = stddev(&gray) / 255.0;

    Ok(ImageMetrics {
        width,
        height,
        aspect: width as f64 / height as f64,
        unique_colors: unique,
        luma_std,
        saturation: saturation_sum / pixels,
        glow: glow_pixels as f64 / pixels,
        ocr_words: ocr_words(path),
    })
}

fn stddev(bytes: &[u8]) -> f64 {
    let count = bytes.len() as f64;
    let mean = bytes.iter().map(|v| *v as f64).sum::<f64>() / count;
    let variance = bytes
        .iter()
        .map(|v| {
            let d = *v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / count;
    variance.sqrt()
}

fn ocr_words(path: &Path) -> Option<u32> {
    let output = Command::new("tesseract")
        .arg(path)
        .arg("-")
        .args(["--psm", "11", "tsv"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(count_ocr(&text))
}

fn count_ocr(text: &str) -> u32 {
    let mut words = 0;
    for line in text.lines().skip(1) {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 12 {
            continue;
        }
        let word = columns[11].trim();
        if word.chars().count() < 4 {
            continue;
        }
        let alnum = word.chars().filter(|c| c.is_alphanumeric()).count();
        if alnum < 2 {
            continue;
        }
        let confidence = columns[10].trim().parse::<f64>().unwrap_or(0.0);
        if confidence >= 75.0 {
            words += 1;
        }
    }
    words
}

pub fn audio_metrics(path: &Path) -> Result<AudioMetrics> {
    let duration = media::duration_secs(path)?;
    let ebur = stderr_of(&[
        "-hide_banner".into(),
        "-v".into(),
        "info".into(),
        "-i".into(),
        path.display().to_string(),
        "-af".into(),
        "ebur128=peak=true:framelog=quiet".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ])?;
    let (lufs, peak) = parse_ebur(&ebur);
    let lufs = lufs.context("ebur128 did not report integrated loudness")?;
    let peak = peak.context("ebur128 did not report a true peak")?;

    let centroid_text = stdout_of(&filter_args(
        path,
        "aspectralstats=measure=centroid,ametadata=mode=print:file=-",
    ))?;
    let centroid = parse_centroid(&centroid_text).context("no spectral centroid in the output")?;

    let full = overall_rms(path, "anull")?;
    let high = overall_rms(path, "highpass=f=5000")?;
    let high_share = 10f64.powf((high - full) / 10.0);

    let silence_text = stderr_of(&[
        "-hide_banner".into(),
        "-v".into(),
        "info".into(),
        "-i".into(),
        path.display().to_string(),
        "-af".into(),
        "silencedetect=noise=-50dB:d=0.5".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ])?;
    let silence = parse_silence(&silence_text, duration);

    let intro_end = 20.0f64.min(duration / 4.0);
    let mid_start = (duration / 2.0 - 10.0).max(0.0);
    let mid_len = 20.0f64.min(duration - mid_start);
    let intro = segment_rms(path, 0.0, intro_end)?;
    let mid = segment_rms(path, mid_start, mid_len)?;

    Ok(AudioMetrics {
        duration,
        lufs,
        peak,
        centroid,
        high_share,
        build_db: intro - mid,
        silence,
    })
}

fn filter_args(path: &Path, filter: &str) -> Vec<String> {
    vec![
        "-v".into(),
        "error".into(),
        "-i".into(),
        path.display().to_string(),
        "-af".into(),
        filter.into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]
}

fn overall_rms(path: &Path, filter: &str) -> Result<f64> {
    let chain = format!("{filter},astats=metadata=1,ametadata=print:file=-");
    let text = stdout_of(&filter_args(path, &chain))?;
    parse_rms(&text).with_context(|| format!("astats did not report RMS for {filter}"))
}

fn segment_rms(path: &Path, start: f64, length: f64) -> Result<f64> {
    let text = stderr_of(&[
        "-hide_banner".into(),
        "-v".into(),
        "info".into(),
        "-ss".into(),
        format!("{start:.2}"),
        "-t".into(),
        format!("{length:.2}"),
        "-i".into(),
        path.display().to_string(),
        "-af".into(),
        "astats".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ])?;
    parse_last_rms(&text).context("astats did not report RMS for the segment")
}

fn stdout_bytes(args: &[String]) -> Result<Vec<u8>> {
    let output = Command::new("ffmpeg")
        .args(args)
        .output()
        .context("running ffmpeg")?;
    if !output.status.success() {
        bail!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn stdout_of(args: &[String]) -> Result<String> {
    Ok(String::from_utf8_lossy(&stdout_bytes(args)?).into_owned())
}

fn stderr_of(args: &[String]) -> Result<String> {
    let output = Command::new("ffmpeg")
        .args(args)
        .output()
        .context("running ffmpeg")?;
    if !output.status.success() {
        bail!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stderr).into_owned())
}

fn parse_ebur(text: &str) -> (Option<f64>, Option<f64>) {
    (last_number(text, "I:"), last_number(text, "Peak:"))
}

fn last_number(text: &str, needle: &str) -> Option<f64> {
    text.lines()
        .filter(|line| line.contains(needle))
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|value| value.parse().ok())
        .next_back()
}

fn parse_centroid(text: &str) -> Option<f64> {
    let values: Vec<f64> = text
        .lines()
        .filter_map(|line| line.split("centroid=").nth(1))
        .filter_map(|value| value.trim().parse().ok())
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

fn parse_rms(text: &str) -> Option<f64> {
    text.lines()
        .filter(|line| line.contains("Overall.RMS_level="))
        .filter_map(|line| line.split("Overall.RMS_level=").nth(1))
        .filter_map(|value| value.trim().parse().ok())
        .next_back()
}

fn parse_last_rms(text: &str) -> Option<f64> {
    text.lines()
        .filter(|line| line.contains("RMS level dB:"))
        .filter_map(|line| line.split("RMS level dB:").nth(1))
        .filter_map(|value| value.split_whitespace().next()?.parse().ok())
        .next_back()
}

fn parse_silence(text: &str, duration: f64) -> f64 {
    let mut start = 0.0;
    let mut longest = 0.0f64;
    for line in text.lines() {
        if let Some(value) = value_after(line, "silence_start:") {
            start = value;
        }
        if let Some(value) = value_after(line, "silence_duration:") {
            // The tail fade is intentional, so only the body counts.
            if start < duration - 5.0 {
                longest = longest.max(value);
            }
        }
    }
    longest
}

fn value_after(line: &str, needle: &str) -> Option<f64> {
    line.split(needle)
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn candidate(
    index: usize,
    file: &Path,
    seed: Option<u32>,
    selected: bool,
    mode: &'static str,
    side: &SideScore,
) -> CandidateScore {
    CandidateScore {
        index,
        file: file.display().to_string(),
        seed,
        selected,
        mode,
        score: round2(side.score),
        points: round2(side.points),
        evaluated: round2(side.evaluated),
        findings: side.findings.clone(),
        skipped: side.skipped.clone(),
    }
}

fn gate(
    code: &'static str,
    metric: &'static str,
    value: f64,
    threshold: f64,
    message: String,
) -> Finding {
    Finding {
        code,
        source: "deterministic",
        severity: "high",
        metric,
        value: round2(value),
        threshold: round2(threshold),
        delta: 0.0,
        message,
    }
}

fn gates(image: &ImageMetrics, audio: &AudioMetrics, target: f64) -> Vec<Finding> {
    let mut findings = Vec::new();
    let expected = 16.0 / 9.0;
    if (image.aspect - expected).abs() > expected * 0.01 || image.width < 1280 || image.height < 720
    {
        findings.push(gate(
            "GATE_TYPE",
            "aspect",
            image.aspect,
            expected,
            format!(
                "{}x{} with aspect {:.3}: expected 16:9 at 1280x720 or more",
                image.width, image.height, image.aspect
            ),
        ));
    }
    if (audio.duration - target).abs() > 2.0 {
        findings.push(gate(
            "GATE_DURATION",
            "duration_sec",
            audio.duration,
            target,
            format!("{:.1} s against {target:.1}", audio.duration),
        ));
    }
    if audio.peak > 0.0 {
        findings.push(gate(
            "GATE_PEAK",
            "true_peak",
            audio.peak,
            0.0,
            format!("{:.1} dBFS clips", audio.peak),
        ));
    }
    findings
}

fn editorial(dir: &Path) -> Option<f64> {
    let raw = fs::read(dir.join("work/judge/verdict.json")).ok()?;
    let verdict: Value = serde_json::from_slice(&raw).ok()?;
    verdict
        .pointer("/omni/best/score")
        .and_then(Value::as_f64)
        .or_else(|| verdict.pointer("/judge/score").and_then(Value::as_f64))
}

/// Writes the full report to `work/score.json` and returns the path.
pub fn write_json(dir: &Path, run: &RunScore) -> Result<PathBuf> {
    let work = dir.join("work");
    fs::create_dir_all(&work)?;
    let path = work.join("score.json");
    fs::write(&path, serde_json::to_vec_pretty(run)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn pool_path(dir: &Path, file: &str) -> PathBuf {
    let name = Path::new(file)
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_default();
    dir.join("candidates").join(name)
}

/// Scores a finished run: every candidate, plus the final pair that the run
/// actually produced. Writes nothing; the caller owns the report.
pub fn score_run(dir: &Path) -> Result<RunScore> {
    let meta: Value =
        serde_json::from_slice(&fs::read(dir.join("meta.json")).context("reading meta.json")?)
            .context("meta.json is not valid JSON")?;

    let target = meta
        .pointer("/audio/final/duration_sec")
        .and_then(Value::as_f64)
        .or_else(|| meta.pointer("/audio/duration_sec").and_then(Value::as_f64))
        .unwrap_or(210.0);

    let selected_image = meta
        .pointer("/image/selected")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;

    let mut images = Vec::new();
    if let Some(list) = meta.pointer("/image/candidates").and_then(Value::as_array) {
        for (offset, entry) in list.iter().enumerate() {
            let Some(file) = entry["file"].as_str() else {
                continue;
            };
            let path = pool_path(dir, file);
            if !path.is_file() {
                continue;
            }
            let index = offset + 1;
            let metrics =
                image_metrics(&path).with_context(|| format!("scoring {}", path.display()))?;
            let side = score_image(&metrics);
            let seed = entry["seed"].as_u64().map(|seed| seed as u32);
            images.push(candidate(
                index,
                &path,
                seed,
                index == selected_image,
                "image",
                &side,
            ));
        }
    }

    let final_image_path = dir.join("thumbnail.png");
    let final_image_metrics = image_metrics(&final_image_path)
        .with_context(|| format!("scoring {}", final_image_path.display()))?;
    let final_image_side = score_image(&final_image_metrics);

    let mut audios = Vec::new();
    if let Some(list) = meta
        .pointer("/audio/draft/candidates")
        .and_then(Value::as_array)
    {
        for (offset, entry) in list.iter().enumerate() {
            let Some(file) = entry["file"].as_str() else {
                continue;
            };
            let path = pool_path(dir, file);
            if !path.is_file() {
                continue;
            }
            let metrics =
                audio_metrics(&path).with_context(|| format!("scoring {}", path.display()))?;
            let side = score_audio(&metrics, target, false);
            let seed = entry["seed"].as_u64().map(|seed| seed as u32);
            audios.push(candidate(offset + 1, &path, seed, false, "draft", &side));
        }
    }

    let final_audio_path = dir.join("audio.mp3");
    let final_audio_metrics = audio_metrics(&final_audio_path)
        .with_context(|| format!("scoring {}", final_audio_path.display()))?;
    let final_audio_side = score_audio(&final_audio_metrics, target, true);

    let technical = round2((final_image_side.score + final_audio_side.score) / 2.0);

    let final_image = candidate(
        selected_image,
        &final_image_path,
        None,
        true,
        "image",
        &final_image_side,
    );
    let final_audio = candidate(
        0,
        &final_audio_path,
        None,
        true,
        "master",
        &final_audio_side,
    );

    let editorial = editorial(dir);
    let total = match editorial {
        Some(editorial) => {
            round2(WEIGHT_TECHNICAL * technical + (1.0 - WEIGHT_TECHNICAL) * editorial)
        }
        None => technical,
    };
    let gate_findings = gates(&final_image_metrics, &final_audio_metrics, target);
    let accepted = gate_findings.is_empty()
        && technical >= TARGET_TECHNICAL
        && editorial.is_none_or(|value| value >= TARGET_EDITORIAL)
        && total >= TARGET_TOTAL;

    Ok(RunScore {
        rubric: RUBRIC,
        dir: dir.display().to_string(),
        technical,
        editorial: editorial.map(round2),
        total,
        accepted,
        gates: gate_findings,
        images,
        audios,
        final_image,
        final_audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.02,
            "expected {expected}, got {actual}"
        );
    }

    fn worked_image() -> ImageMetrics {
        ImageMetrics {
            width: 1280,
            height: 720,
            aspect: 16.0 / 9.0,
            unique_colors: 216_853,
            luma_std: 0.173,
            saturation: 0.378,
            glow: 0.0,
            ocr_words: Some(0),
        }
    }

    fn worked_audio() -> AudioMetrics {
        AudioMetrics {
            duration: 210.0,
            lufs: -14.6,
            peak: -1.4,
            centroid: 3258.0,
            high_share: 0.039_67,
            build_db: -0.232_3,
            silence: 0.0,
        }
    }

    #[test]
    fn palette_buckets() {
        close(image_palette(100).points, 20.0);
        close(image_palette(256).points, 20.0);
        close(image_palette(2048).points, 10.0);
        close(image_palette(16384).points, 0.0);
        close(image_palette(216_853).points, 0.0);
    }

    #[test]
    fn contrast_and_saturation_curves() {
        close(image_contrast(0.18).points, 10.0);
        close(image_contrast(0.10).points, 0.0);
        close(image_contrast(0.14).points, 5.0);
        close(image_contrast(0.05).points, 0.0);
        close(image_saturation(0.378).points, 5.0);
        close(image_saturation(0.80).points, 5.0 * (1.0 - 0.05 / 0.15));
        close(image_saturation(0.10).points, 0.0);
    }

    #[test]
    fn glow_and_text_penalties() {
        close(image_glow(0.0).points, 5.0);
        close(image_glow(0.02).points, 0.0);
        close(image_glow(0.001).points, 5.0);
        close(image_text(0).points, 10.0);
        close(image_text(1).points, 0.0);
    }

    #[test]
    fn audio_buckets() {
        close(audio_loudness(-14.6).points, 8.0);
        close(audio_loudness(-17.5).points, 0.0);
        close(audio_peak(-1.4).points, 8.0);
        close(audio_peak(-0.5).points, 4.0);
        close(audio_peak(0.5).points, 0.0);
        close(audio_centroid(3258.0).points, 8.0);
        close(audio_centroid(1000.0).points, 0.0);
        close(audio_highs(0.08).points, 8.0);
        close(audio_highs(0.02).points, 2.67);
        close(audio_build(-2.0).points, 8.0);
        close(audio_build(0.0).points, 4.0);
        close(audio_build(2.0).points, 0.0);
        close(audio_silence(0.0).points, 5.0);
        close(audio_silence(1.5).points, 2.5);
        close(audio_silence(2.5).points, 0.0);
        close(audio_duration(210.0, 210.0).points, 5.0);
        close(audio_duration(207.0, 210.0).points, 0.0);
    }

    #[test]
    fn reproduced_worked_example() {
        let image = score_image(&worked_image());
        close(image.points, 29.13);
        close(image.score, 58.25);

        let audio = score_audio(&worked_audio(), 210.0, true);
        close(audio.points, 43.24);
        close(audio.score, 86.48);

        let technical = (image.score + audio.score) / 2.0;
        close(technical, 72.35);
        close(
            WEIGHT_TECHNICAL * technical + (1.0 - WEIGHT_TECHNICAL) * 67.0,
            70.2,
        );
    }

    #[test]
    fn missing_ocr_renormalizes_the_image() {
        let mut metrics = worked_image();
        metrics.unique_colors = 128;
        metrics.luma_std = 0.20;
        metrics.saturation = 0.5;
        metrics.ocr_words = None;
        let score = score_image(&metrics);
        close(score.evaluated, 40.0);
        close(score.score, 100.0);
        assert_eq!(score.skipped, vec!["IMG_TEXT"]);
    }

    #[test]
    fn drafts_skip_the_master_metrics() {
        let score = score_audio(&worked_audio(), 60.0, false);
        close(score.evaluated, 29.0);
        assert!(score.skipped.contains(&"AUD_LOUDNESS"));
        assert!(score.skipped.contains(&"AUD_PEAK"));
        assert!(score.skipped.contains(&"AUD_DURATION"));
    }

    #[test]
    fn parses_ffmpeg_output() {
        let ebur = "  Integrated loudness:\n    I:         -14.6 LUFS\n\n  True peak:\n    Peak:       -1.4 dBFS\n";
        let (lufs, peak) = parse_ebur(ebur);
        close(lufs.unwrap(), -14.6);
        close(peak.unwrap(), -1.4);

        let centroid =
            "lavfi.aspectralstats.1.centroid=3000.0\nlavfi.aspectralstats.1.centroid=3516.0\n";
        close(parse_centroid(centroid).unwrap(), 3258.0);

        let rms = "lavfi.astats.Overall.RMS_level=-20.0\nlavfi.astats.Overall.RMS_level=-17.5\n";
        close(parse_rms(rms).unwrap(), -17.5);

        let segment = "[Parsed_astats_0 @ 0x1] Channel: 1\n[Parsed_astats_0 @ 0x1] RMS level dB: -20.0\n[Parsed_astats_0 @ 0x1] Overall\n[Parsed_astats_0 @ 0x1] RMS level dB: -17.4\n";
        close(parse_last_rms(segment).unwrap(), -17.4);

        let silence = "silence_start: 100.0\nsilence_end: 101.2 | silence_duration: 1.2\nsilence_start: 205.4\nsilence_end: 210 | silence_duration: 4.6\n";
        close(parse_silence(silence, 210.0), 1.2);
    }

    #[test]
    fn counts_only_credible_ocr_words() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t90.0\tEPIC\n\
5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t90.0\tab\n\
5\t1\t1\t1\t1\t3\t0\t0\t10\t10\t60.0\tSOFT\n\
5\t1\t1\t1\t1\t4\t0\t0\t10\t10\t90.0\t'-H‘\n";
        assert_eq!(count_ocr(tsv), 1);
    }
}
