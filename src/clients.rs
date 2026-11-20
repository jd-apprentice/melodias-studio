use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::json;

use crate::config::Config;

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()
        .context("creando cliente HTTP")
}

pub fn check_audio(cfg: &Config) -> Result<()> {
    let url = format!("{}/health", cfg.audio_url.trim_end_matches('/'));
    let body = client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("audio.cpp no responde en {url}"))?
        .text()?;
    if !body.contains("\"ok\"") {
        bail!("audio.cpp respondió algo inesperado: {body}");
    }
    Ok(())
}

pub fn check_llm(cfg: &Config) -> Result<()> {
    let url = format!("{}/health", cfg.llm_url.trim_end_matches('/'));
    client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("llama-server no responde en {url}"))?;
    Ok(())
}

pub fn check_sd(cfg: &Config) -> Result<()> {
    let url = format!("{}/v1/models", cfg.sd_url.trim_end_matches('/'));
    client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("sd-server no responde en {url}"))?;
    Ok(())
}

pub fn ask_image_prompt(cfg: &Config, vibe: &str) -> Result<String> {
    let url = format!("{}/v1/chat/completions", cfg.llm_url.trim_end_matches('/'));
    let system = "You write prompts for a text-to-image model. Reply with a single \
line in English, no quotes, no explanation. Always describe a 16:9 YouTube thumbnail \
in 8-bit pixel art style with an anime character, detailed pixel background, \
nostalgic mood, no text and no watermark.";
    let user = format!(
        "Thumbnail concept, keep the same scene but adapt the palette and props to it: {vibe}"
    );
    let body = json!({
        "model": cfg.llm_model,
        "temperature": 0.7,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    });
    let resp: serde_json::Value = client()?
        .post(&url)
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("llama-server falló en {url}"))?
        .json()?;
    let text = resp["choices"][0]["message"]["content"]
        .as_str()
        .context("respuesta del LLM sin content")?
        .trim()
        .trim_matches('"')
        .to_string();
    if text.is_empty() {
        bail!("el LLM devolvió un prompt vacío");
    }
    Ok(text)
}

pub fn unload_audio(cfg: &Config) -> Result<()> {
    let url = format!(
        "{}/v1/tasks/unload_all_models",
        cfg.audio_url.trim_end_matches('/')
    );
    client()?
        .post(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("no pude liberar VRAM del audio en {url}"))?;
    Ok(())
}

pub fn generate_audio(cfg: &Config, prompt: &str, reference: &str, seed: u32) -> Result<Vec<u8>> {
    let url = format!("{}/v1/tasks/run", cfg.audio_url.trim_end_matches('/'));
    let body = json!({
        "model": cfg.audio_model,
        "request": {
            "route": "text2music",
            "text": prompt,
            "duration_sec": cfg.duration_sec,
            "seed": seed,
            "reference_audio": reference,
            "num_inference_steps": cfg.steps,
            "guidance_scale": cfg.guidance,
            "sampler_mode": cfg.sampler
        }
    });
    let resp: serde_json::Value = client()?
        .post(&url)
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("audio.cpp falló en {url}"))?
        .json()?;
    let b64 = resp["audio"]
        .as_str()
        .context("respuesta sin campo audio")?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("audio base64 inválido")
}

pub fn generate_image(cfg: &Config, prompt: &str) -> Result<Vec<u8>> {
    let url = format!("{}/v1/images/generations", cfg.sd_url.trim_end_matches('/'));
    let body = json!({
        "prompt": prompt,
        "n": 1,
        "size": format!("{}x{}", cfg.width, cfg.height),
        "output_format": "png"
    });
    let resp: serde_json::Value = client()?
        .post(&url)
        .json(&body)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("sd-server falló en {url}"))?
        .json()?;
    let b64 = resp["data"][0]["b64_json"]
        .as_str()
        .context("respuesta sin data[0].b64_json")?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("imagen base64 inválida")
}
