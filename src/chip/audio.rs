//! Small DSP helpers for the chip cover: decode, onset flux, tempo, drums and
//! section energy. Everything is computed locally with rustfft; ffmpeg only
//! decodes to mono f32.

use std::f32::consts::PI;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

const FRAME: usize = 1024;
const HOP: usize = 512;

pub fn decode_mono(path: &Path, rate: u32) -> Result<Vec<f32>> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-ac", "1", "-ar", &rate.to_string(), "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("decoding {}", path.display()))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .context("ffmpeg stdout")?
        .read_to_end(&mut bytes)
        .context("reading decoded audio")?;
    let status = child.wait().context("waiting for ffmpeg")?;
    if !status.success() {
        bail!("ffmpeg could not decode {}", path.display());
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

fn flux(samples: &[f32]) -> Vec<f64> {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FRAME);
    let mut buffer = vec![Complex32::new(0.0, 0.0); FRAME];
    let mut previous = vec![0.0; FRAME / 2];
    let mut envelope = Vec::new();
    let mut start = 0;
    while start + FRAME <= samples.len() {
        for (index, slot) in buffer.iter_mut().enumerate() {
            let window = 0.5 - 0.5 * (2.0 * PI * index as f32 / (FRAME - 1) as f32).cos();
            *slot = Complex32::new(samples[start + index] * window, 0.0);
        }
        fft.process(&mut buffer);
        let mut sum = 0.0;
        for bin in 1..FRAME / 2 {
            let magnitude = buffer[bin].norm() as f64;
            let difference = magnitude - previous[bin];
            if difference > 0.0 {
                sum += difference;
            }
            previous[bin] = magnitude;
        }
        envelope.push(sum);
        start += HOP;
    }
    envelope
}

/// Autocorrelation of the onset envelope, folded into 70..180 BPM.
pub fn tempo(samples: &[f32], rate: u32) -> f64 {
    let envelope = flux(samples);
    if envelope.len() < 8 {
        return 120.0;
    }
    let hop_seconds = HOP as f64 / rate as f64;
    let minimum = (0.3 / hop_seconds).round() as usize;
    let maximum = ((1.0 / hop_seconds).round() as usize).min(envelope.len() - 1);
    if minimum >= maximum {
        return 120.0;
    }
    let mean = envelope.iter().sum::<f64>() / envelope.len() as f64;
    let centered: Vec<f64> = envelope.iter().map(|value| value - mean).collect();
    let mut best_lag = minimum;
    let mut best = f64::MIN;
    for lag in minimum..=maximum {
        let correlation: f64 = centered
            .iter()
            .take(centered.len() - lag)
            .zip(centered.iter().skip(lag))
            .map(|(a, b)| a * b)
            .sum();
        if correlation > best {
            best = correlation;
            best_lag = lag;
        }
    }
    let mut bpm = 60.0 / (best_lag as f64 * hop_seconds);
    while bpm < 70.0 {
        bpm *= 2.0;
    }
    while bpm > 180.0 {
        bpm /= 2.0;
    }
    if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    }
}

pub fn section_energies(
    samples: &[f32],
    rate: u32,
    sections: usize,
    seconds_per_section: f64,
) -> Vec<f64> {
    (0..sections)
        .map(|index| {
            let start = (index as f64 * seconds_per_section * rate as f64) as usize;
            let end = (((index + 1) as f64 * seconds_per_section * rate as f64) as usize)
                .min(samples.len());
            if end <= start {
                return 0.0;
            }
            let slice = &samples[start..end];
            (slice
                .iter()
                .map(|value| (*value as f64).powi(2))
                .sum::<f64>()
                / slice.len() as f64)
                .sqrt()
        })
        .collect()
}

/// Kick (36), snare (38) and hat (42) from the drums stem, by band energy of
/// each onset window.
pub fn drums(samples: &[f32], rate: u32) -> Vec<(f64, u8)> {
    let envelope = flux(samples);
    if envelope.len() < 4 {
        return Vec::new();
    }
    let mean = envelope.iter().sum::<f64>() / envelope.len() as f64;
    let variance = envelope
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / envelope.len() as f64;
    let threshold = mean + variance.sqrt();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FRAME);
    let mut hits = Vec::new();
    let mut last = -1.0;
    for index in 1..envelope.len().saturating_sub(1) {
        if envelope[index] < threshold
            || envelope[index] < envelope[index - 1]
            || envelope[index] <= envelope[index + 1]
        {
            continue;
        }
        let time = index as f64 * HOP as f64 / rate as f64;
        if time - last < 0.06 {
            continue;
        }
        last = time;
        let start = (time * rate as f64) as usize;
        let end = (start + (0.06 * rate as f64) as usize).min(samples.len());
        if end <= start {
            continue;
        }
        hits.push((time, classify(&samples[start..end], rate, fft.as_ref())));
    }
    hits
}

fn classify(window: &[f32], rate: u32, fft: &dyn Fft<f32>) -> u8 {
    let mut buffer = vec![Complex32::new(0.0, 0.0); FRAME];
    for (index, slot) in buffer.iter_mut().enumerate() {
        if index < window.len() {
            let window_value = 0.5 - 0.5 * (2.0 * PI * index as f32 / (FRAME - 1) as f32).cos();
            *slot = Complex32::new(window[index] * window_value, 0.0);
        }
    }
    fft.process(&mut buffer);
    let mut low = 0.0;
    let mut mid = 0.0;
    let mut high = 0.0;
    for (bin, value) in buffer.iter().enumerate().take(FRAME / 2).skip(1) {
        let frequency = bin as f64 * rate as f64 / FRAME as f64;
        let energy = value.norm_sqr() as f64;
        if frequency < 120.0 {
            low += energy;
        } else if frequency < 1500.0 {
            mid += energy;
        } else if frequency >= 2500.0 {
            high += energy;
        }
    }
    if low > mid * 1.4 {
        36
    } else if high > mid * 0.25 {
        42
    } else {
        38
    }
}

#[cfg(test)]
#[path = "../tests/chip_audio.rs"]
mod tests;
