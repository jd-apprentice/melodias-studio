use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::json;

use crate::config::Config;

fn short_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .context("creating the HTTP client")
}

/// One generation on the P40 can take half an hour, so generation requests get
/// their own budget instead of the short one used by the health probes.
fn long_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3600))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("creating the HTTP client")
}

pub fn check_audio(cfg: &Config) -> Result<()> {
    let url = format!("{}/health", cfg.audio_url.trim_end_matches('/'));
    let body = short_client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("audio.cpp does not answer at {url}"))?
        .text()?;
    if !body.contains("\"ok\"") {
        bail!("audio.cpp returned something unexpected: {body}");
    }
    Ok(())
}

pub fn check_llm(cfg: &Config) -> Result<()> {
    let url = format!("{}/health", cfg.llm_url.trim_end_matches('/'));
    short_client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("llama-server does not answer at {url}"))?;
    Ok(())
}

pub fn check_sd(cfg: &Config) -> Result<()> {
    let url = format!("{}/v1/models", cfg.sd_url.trim_end_matches('/'));
    short_client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("sd-server does not answer at {url}"))?;
    Ok(())
}

pub fn wait_audio(cfg: &Config, timeout: Duration) -> Result<()> {
    wait("audio.cpp", timeout, || check_audio(cfg))
}

pub fn wait_llm(cfg: &Config, timeout: Duration) -> Result<()> {
    wait("llama-server", timeout, || check_llm(cfg))
}

pub fn wait_sd(cfg: &Config, timeout: Duration) -> Result<()> {
    wait("sd-server", timeout, || check_sd(cfg))
}

fn wait(name: &str, timeout: Duration, probe: impl Fn() -> Result<()>) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        if probe().is_ok() {
            return Ok(());
        }
        if start.elapsed() > timeout {
            bail!("{name} did not answer within {} s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

pub fn ask_image_prompt(cfg: &Config, vibe: &str) -> Result<String> {
    let url = format!("{}/v1/chat/completions", cfg.llm_url.trim_end_matches('/'));
    let system = format!(
        "You write prompts for a text-to-image model. Reply with ONE line of at most 60 \
words, in English, no quotes, no explanation. Describe a 16:9 YouTube thumbnail in {} \
style: one anime character in a scene, detailed background, nostalgic cozy mood. The \
character has natural eyes with a visible iris and pupil; never glowing, neon or empty \
eyes. Never mention audio, BPM, tempo, music, songs, text or watermarks.",
        cfg.image_style
    );
    let user =
        format!("Thumbnail concept, keep the scene but adapt the palette and props to it: {vibe}");
    let body = json!({
        "model": cfg.llm_model,
        "temperature": 0.5,
        "max_tokens": 220,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    });
    let resp: serde_json::Value = long_client()?
        .post(&url)
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("llama-server failed at {url}"))?
        .json()?;
    let raw = resp["choices"][0]["message"]["content"]
        .as_str()
        .context("LLM response without content")?;
    let text = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('"')
        .to_string();
    if text.is_empty() {
        bail!("the LLM returned an empty prompt");
    }
    Ok(text)
}

/// One take: drafts and the final share the same request shape, only the
/// duration, steps and sampler change.
pub struct AudioRequest<'a> {
    pub prompt: &'a str,
    pub reference: &'a str,
    pub seed: u32,
    pub duration_sec: u32,
    pub steps: u32,
    pub sampler: &'a str,
}

pub fn generate_audio(cfg: &Config, take: AudioRequest<'_>) -> Result<Vec<u8>> {
    let url = format!("{}/v1/tasks/run", cfg.audio_url.trim_end_matches('/'));
    // The server only forwards the nested `options` object to the ACE-Step
    // parser. duration_sec, reference_audio and sampler_mode are ignored when
    // they sit at the top level of `request`.
    let body = json!({
        "model": cfg.audio_model,
        "request": {
            "route": "text2music",
            "text": take.prompt,
            "seed": take.seed,
            "num_inference_steps": take.steps,
            "guidance_scale": cfg.guidance,
            "options": {
                "reference_audio": take.reference,
                "duration_sec": take.duration_sec,
                "sampler_mode": take.sampler,
                "negative_prompt": cfg.audio_negative_prompt
            }
        }
    });
    let resp: serde_json::Value = long_client()?
        .post(&url)
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("audio.cpp failed at {url}"))?
        .json()?;
    let b64 = resp["audio"]
        .as_str()
        .context("response without an audio field")?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("invalid base64 audio")
}

/// sd-server strips this block before generation. It carries the native
/// stable-diffusion.cpp fields that the OpenAI endpoint does not expose.
fn with_extra_args(prompt: &str, seed: u32, steps: u32, negative_prompt: &str) -> String {
    let mut extra = json!({
        "seed": seed,
        "sample_params": { "sample_steps": steps }
    });
    if !negative_prompt.trim().is_empty() {
        extra["negative_prompt"] = json!(negative_prompt);
    }
    format!("{prompt} <sd_cpp_extra_args>{extra}</sd_cpp_extra_args>")
}

pub fn generate_image(cfg: &Config, prompt: &str, seed: u32) -> Result<Vec<u8>> {
    let url = format!("{}/v1/images/generations", cfg.sd_url.trim_end_matches('/'));
    let prompt = with_extra_args(prompt, seed, cfg.image_steps, &cfg.image_negative_prompt);
    let body = json!({
        "prompt": prompt,
        "n": 1,
        "size": format!("{}x{}", cfg.width, cfg.height),
        "output_format": "png"
    });
    let resp: serde_json::Value = long_client()?
        .post(&url)
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("sd-server failed at {url}"))?
        .json()?;
    let b64 = resp["data"][0]["b64_json"]
        .as_str()
        .context("response without data[0].b64_json")?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("invalid base64 image")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_extra_args_carry_seed_and_steps() {
        let prompt = with_extra_args("pixel art", 42, 12, "glowing eyes");
        assert!(prompt.starts_with("pixel art <sd_cpp_extra_args>"));
        assert!(prompt.contains("\"seed\":42"));
        assert!(prompt.contains("\"sample_steps\":12"));
        assert!(prompt.contains("\"negative_prompt\":\"glowing eyes\""));
        assert!(prompt.ends_with("</sd_cpp_extra_args>"));
    }
}
