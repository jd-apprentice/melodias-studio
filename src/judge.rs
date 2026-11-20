use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};

use crate::config::Config;
use crate::media;
use crate::pipeline::Candidate;

const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

const SYSTEM: &str = "You are the reviewer for an 8-bit YouTube channel. The channel publishes \
original instrumental chiptune and pixel-art thumbnails. Judge only inside that brief. Return \
JSON only, with no prose and no code fences.";

pub struct Verdict {
    pub image: usize,
    pub audio: usize,
    pub score: u8,
    pub reason: String,
}

impl Verdict {
    pub fn summary(&self) -> Value {
        json!({
            "image": self.image + 1,
            "audio": self.audio + 1,
            "score": self.score,
            "reason": self.reason,
        })
    }
}

pub fn enabled(cfg: &Config) -> bool {
    cfg.judge_enabled && !cfg.openrouter_api_key.trim().is_empty()
}

/// Sends compressed evidence to the perception model, then asks the text judge
/// for the final pair. The caller decides what to do when this fails.
pub fn decide(
    cfg: &Config,
    vibe: &str,
    images: &[Candidate],
    audios: &[Candidate],
    work: &Path,
) -> Result<Verdict> {
    let dir = work.join("judge");
    std::fs::create_dir_all(&dir).context("creating the judge directory")?;

    let mut content = vec![text(format!(
        "Brief: {vibe}\n\n\
         The channel publishes original 8-bit instrumental music and pixel-art thumbnails. \
         The candidates must be instrumental, 8-bit, 16:9, with no text or watermark. \
         You will receive {} images and {} audio clips, labeled in order. \
         Score every image for adherence to the brief and channel style: composition, palette, \
         readability at thumbnail size, natural eyes, no text. \
         Score every audio clip for instrumental quality, energy, intro and outro, no vocals. \
         Score the mood coherence of every image/audio pair. \
         Return exactly this JSON shape:\n\
         {{\"images\":[{{\"index\":1,\"score\":0,\"reason\":\"\"}}],\
         \"audios\":[{{\"index\":1,\"score\":0,\"reason\":\"\"}}],\
         \"pairs\":[{{\"image\":1,\"audio\":1,\"coherence\":0,\"reason\":\"\"}}],\
         \"best\":{{\"image\":1,\"audio\":1,\"score\":0}},\"reason\":\"\"}}",
        images.len(),
        audios.len()
    ))];

    for (index, candidate) in images.iter().enumerate() {
        content.push(text(format!("Image {}", index + 1)));
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": image_data_url(cfg, candidate, &dir, index)? }
        }));
    }
    for (index, candidate) in audios.iter().enumerate() {
        content.push(text(format!("Audio {}", index + 1)));
        content.push(json!({
            "type": "input_audio",
            "input_audio": {
                "data": audio_base64(cfg, candidate, &dir, index)?,
                "format": "mp3"
            }
        }));
    }

    let omni = chat(cfg, &cfg.judge_omni_model, content)?;
    let judge_prompt = format!(
        "Brief: {vibe}\n\n\
         A perception model saw the images and heard the clips. Its report:\n{omni}\n\n\
         You are the final judge. Choose the image and the audio that make the best pair for the \
         channel, balancing quality and mood coherence. Return exactly this JSON:\n\
         {{\"verdict\":\"approve\",\"image\":1,\"audio\":1,\"score\":0,\
         \"must_fix\":[],\"nice_to_have\":[],\"reason\":\"\"}}"
    );
    let judge = match chat(cfg, &cfg.judge_reasoner_model, vec![text(judge_prompt)]) {
        Ok(value) => value,
        Err(_) => Value::Null,
    };

    let image = pick_index(&judge["image"], images.len())
        .or_else(|| pick_index(&omni["best"]["image"], images.len()))
        .unwrap_or(0);
    let audio = pick_index(&judge["audio"], audios.len())
        .or_else(|| pick_index(&omni["best"]["audio"], audios.len()))
        .unwrap_or(0);
    let score = judge["score"]
        .as_u64()
        .or_else(|| omni["best"]["score"].as_u64())
        .unwrap_or(0)
        .min(100) as u8;
    let reason = judge["reason"]
        .as_str()
        .or_else(|| omni["reason"].as_str())
        .unwrap_or("no reason")
        .to_string();

    std::fs::write(
        dir.join("verdict.json"),
        serde_json::to_vec_pretty(&json!({
            "omni": omni,
            "judge": judge,
            "selection": { "image": image + 1, "audio": audio + 1 },
        }))?,
    )?;

    Ok(Verdict {
        image,
        audio,
        score,
        reason,
    })
}

fn text(value: impl Into<String>) -> Value {
    json!({ "type": "text", "text": value.into() })
}

fn pick_index(value: &Value, count: usize) -> Option<usize> {
    let index = value.as_u64()? as usize;
    if (1..=count).contains(&index) {
        Some(index - 1)
    } else {
        None
    }
}

fn image_data_url(cfg: &Config, candidate: &Candidate, dir: &Path, index: usize) -> Result<String> {
    let path = dir.join(format!("image-{:02}.jpg", index + 1));
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"])
        .arg(&candidate.file)
        .args([
            "-vf",
            &format!("scale={}:-2", cfg.judge_image_px.max(256)),
            "-q:v",
            "4",
        ])
        .arg(&path);
    let output = cmd.output().context("preparing the judge image")?;
    if !output.status.success() {
        bail!(
            "ffmpeg failed preparing the judge image: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = std::fs::read(&path)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn audio_base64(cfg: &Config, candidate: &Candidate, dir: &Path, index: usize) -> Result<String> {
    let duration = media::duration_secs(&candidate.file).unwrap_or(60.0);
    let start = (duration / 3.0).max(0.0);
    let seconds = cfg.judge_audio_sec.clamp(5, 30);
    let path = dir.join(format!("audio-{:02}.mp3", index + 1));
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-ss"])
        .arg(format!("{start:.2}"))
        .args(["-t", &seconds.to_string(), "-i"])
        .arg(&candidate.file)
        .args(["-ac", "1", "-ar", "22050", "-b:a", "64k", "-f", "mp3"])
        .arg(&path);
    let output = cmd.output().context("preparing the judge clip")?;
    if !output.status.success() {
        bail!(
            "ffmpeg failed preparing the judge clip: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = std::fs::read(&path)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn chat(cfg: &Config, model: &str, content: Vec<Value>) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(cfg.judge_timeout_sec.max(30)))
        .build()
        .context("creating the judge client")?;
    let body = json!({
        "model": model,
        "temperature": 0.1,
        "max_tokens": 2000,
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": content }
        ]
    });
    let response = client
        .post(ENDPOINT)
        .header(
            "Authorization",
            format!("Bearer {}", cfg.openrouter_api_key),
        )
        .header("X-Title", "melodias-studio")
        .json(&body)
        .send()
        .context("calling OpenRouter")?;
    let status = response.status();
    let body = response.text().context("reading the OpenRouter response")?;
    if !status.is_success() {
        let hint = if status.as_u16() == 402 && body.contains("audio") {
            " (audio input requires at least $0.50 in OpenRouter balance)"
        } else {
            ""
        };
        bail!("OpenRouter {status}{hint}: {}", truncate(&body, 500));
    }
    let parsed: Value = serde_json::from_str(&body).context("OpenRouter returned invalid JSON")?;
    let content = &parsed["choices"][0]["message"]["content"];
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => bail!("OpenRouter response has no content"),
    };
    extract_json(&text)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        format!("{}...", &text[..limit])
    }
}

fn extract_json(text: &str) -> Result<Value> {
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    let start = text.find('{').context("no JSON object in the response")?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (offset, character) in text[start..].char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if character == '\\' {
                escape = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let slice = &text[start..=start + offset];
                    return serde_json::from_str(slice)
                        .context("invalid JSON object in the response");
                }
            }
            _ => {}
        }
    }
    bail!("unterminated JSON object in the response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fences_and_prose() {
        let text = "Here is the verdict:\n```json\n{\"image\":2,\"audio\":1,\"score\":88}\n```";
        let value = extract_json(text).unwrap();
        assert_eq!(value["image"], 2);
        assert_eq!(value["score"], 88);
    }

    #[test]
    fn ignores_braces_inside_strings() {
        let text = "{\"reason\":\"a } inside\",\"image\":1}";
        let value = extract_json(text).unwrap();
        assert_eq!(value["image"], 1);
    }

    #[test]
    fn accepts_only_valid_indices() {
        assert_eq!(pick_index(&json!(2), 3), Some(1));
        assert_eq!(pick_index(&json!(0), 3), None);
        assert_eq!(pick_index(&json!(4), 3), None);
    }
}
