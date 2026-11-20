use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct Config {
    /// audio.cpp server (ACE-Step) on the GPU VM.
    pub audio_url: String,
    pub audio_model: String,
    /// stable-diffusion.cpp server on the GPU VM.
    pub sd_url: String,
    /// llama-server on the GPU VM, used to write the image prompt.
    pub llm_url: String,
    pub llm_model: String,
    /// SSH target used to stage the reference audio on the VM.
    pub ssh_host: String,
    pub remote_ref_dir: String,
    pub out_dir: PathBuf,
    /// Optional lifecycle commands run over SSH between GPU stages.
    pub llm_start_cmd: String,
    pub llm_stop_cmd: String,
    pub sd_start_cmd: String,
    pub sd_stop_cmd: String,
    pub audio_start_cmd: String,
    pub audio_stop_cmd: String,
    /// Fixed medium for the music prompt, independent of the vibe.
    pub audio_style: String,
    /// Fixed style for the thumbnail prompt, pinned in the system message.
    pub image_style: String,
    /// Negative prompt for sd-server, sent through sd_cpp_extra_args.
    pub image_negative_prompt: String,
    /// Steers ACE-Step away from voices; the channel only publishes instrumentals.
    pub audio_negative_prompt: String,
    /// Label shown in the header and the splash, for example "Tesla P40".
    pub gpu_label: String,
    pub duration_sec: u32,
    pub steps: u32,
    pub guidance: f32,
    pub sampler: String,
    pub width: u32,
    pub height: u32,
    pub video_width: u32,
    pub video_height: u32,
    pub image_steps: u32,
    /// Optional fixed seed to reproduce an image candidate.
    pub image_seed: Option<u32>,
    /// Number of candidates per modality.
    pub candidates: u32,
    pub audio_draft_sec: u32,
    pub audio_draft_steps: u32,
    pub audio_draft_sampler: String,
    pub reference_sec: u32,
    pub fade_in_sec: f64,
    pub fade_out_sec: f64,
    pub loudness_lufs: f64,
}

impl Config {
    pub fn from_env() -> Self {
        let file = FileConfig::load();
        Self {
            audio_url: pick(
                env("AUDIO_URL"),
                file.audio_url,
                "http://192.168.88.38:8090",
            ),
            audio_model: pick(env("AUDIO_MODEL"), file.audio_model, "ace-step"),
            sd_url: pick(env("SD_URL"), file.sd_url, "http://192.168.88.38:1234"),
            llm_url: pick(env("LLM_URL"), file.llm_url, "http://192.168.88.38:8080"),
            llm_model: pick(env("LLM_MODEL"), file.llm_model, "local"),
            ssh_host: pick(env("SSH_HOST"), file.ssh_host, "root@192.168.88.38"),
            remote_ref_dir: pick(env("REMOTE_REF_DIR"), file.remote_ref_dir, "/models/ref"),
            out_dir: env("OUT_DIR")
                .map(PathBuf::from)
                .or(file.out_dir)
                .unwrap_or_else(|| PathBuf::from("output")),
            llm_start_cmd: pick(
                env("LLM_START_CMD"),
                file.llm_start_cmd,
                "/home/ubuntu/bin/serve-llm.sh start",
            ),
            llm_stop_cmd: pick(
                env("LLM_STOP_CMD"),
                file.llm_stop_cmd,
                "/home/ubuntu/bin/serve-llm.sh stop",
            ),
            sd_start_cmd: pick(
                env("SD_START_CMD"),
                file.sd_start_cmd,
                "/home/ubuntu/bin/serve-sd.sh start",
            ),
            sd_stop_cmd: pick(
                env("SD_STOP_CMD"),
                file.sd_stop_cmd,
                "/home/ubuntu/bin/serve-sd.sh stop",
            ),
            audio_start_cmd: pick(
                env("AUDIO_START_CMD"),
                file.audio_start_cmd,
                "/home/ubuntu/bin/serve-audio.sh start",
            ),
            audio_stop_cmd: pick(
                env("AUDIO_STOP_CMD"),
                file.audio_stop_cmd,
                "/home/ubuntu/bin/serve-audio.sh stop",
            ),
            audio_style: pick(env("AUDIO_STYLE"), file.audio_style, "8-bit chiptune"),
            image_style: pick(env("IMAGE_STYLE"), file.image_style, "8-bit pixel art"),
            image_negative_prompt: pick(
                env("IMAGE_NEGATIVE_PROMPT"),
                file.image_negative_prompt,
                "glowing eyes, neon eyes, empty eyes, oversaturated eyes, text, watermark, signature, extra fingers, deformed hands",
            ),
            audio_negative_prompt: pick(
                env("AUDIO_NEGATIVE_PROMPT"),
                file.audio_negative_prompt,
                "vocals, singing, voice, choir, lyrics, humming, vocal chops, spoken word",
            ),
            gpu_label: pick(env("GPU_LABEL"), file.gpu_label, "local models"),
            duration_sec: pick_parsed(env("DURATION_SEC"), file.duration_sec, 210),
            steps: pick_parsed(env("STEPS"), file.steps, 32),
            guidance: pick_parsed(env("GUIDANCE"), file.guidance, 5.0),
            sampler: pick(env("SAMPLER"), file.sampler, "heun"),
            width: pick_parsed(env("WIDTH"), file.width, 1280),
            height: pick_parsed(env("HEIGHT"), file.height, 720),
            video_width: pick_parsed(env("VIDEO_WIDTH"), file.video_width, 1920),
            video_height: pick_parsed(env("VIDEO_HEIGHT"), file.video_height, 1080),
            image_steps: pick_parsed(env("IMAGE_STEPS"), file.image_steps, 20),
            image_seed: env("IMAGE_SEED")
                .and_then(|value| value.parse().ok())
                .or(file.image_seed),
            candidates: pick_parsed(env("CANDIDATES"), file.candidates, 3),
            audio_draft_sec: pick_parsed(env("AUDIO_DRAFT_SEC"), file.audio_draft_sec, 60),
            audio_draft_steps: pick_parsed(env("AUDIO_DRAFT_STEPS"), file.audio_draft_steps, 16),
            audio_draft_sampler: pick(
                env("AUDIO_DRAFT_SAMPLER"),
                file.audio_draft_sampler,
                "euler",
            ),
            reference_sec: pick_parsed(env("REFERENCE_SEC"), file.reference_sec, 60),
            fade_in_sec: pick_parsed(env("FADE_IN_SEC"), file.fade_in_sec, 1.5),
            fade_out_sec: pick_parsed(env("FADE_OUT_SEC"), file.fade_out_sec, 3.0),
            loudness_lufs: pick_parsed(env("LOUDNESS_LUFS"), file.loudness_lufs, -15.0),
        }
    }

    pub fn config_path() -> PathBuf {
        if let Some(path) = env("MELODIAS_CONFIG") {
            return PathBuf::from(path);
        }
        let base = env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("melodias-studio").join("config.toml")
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn pick(env_value: Option<String>, file_value: Option<String>, default: &str) -> String {
    env_value
        .or(file_value)
        .unwrap_or_else(|| default.to_string())
}

fn pick_parsed<T>(env_value: Option<String>, file_value: Option<T>, default: T) -> T
where
    T: std::str::FromStr,
{
    env_value
        .and_then(|value| value.parse().ok())
        .or(file_value)
        .unwrap_or(default)
}

/// Everything in `~/.config/melodias-studio/config.toml` is optional. Environment
/// variables win, then the file, then the defaults above.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    audio_url: Option<String>,
    audio_model: Option<String>,
    sd_url: Option<String>,
    llm_url: Option<String>,
    llm_model: Option<String>,
    ssh_host: Option<String>,
    remote_ref_dir: Option<String>,
    out_dir: Option<PathBuf>,
    llm_start_cmd: Option<String>,
    llm_stop_cmd: Option<String>,
    sd_start_cmd: Option<String>,
    sd_stop_cmd: Option<String>,
    audio_start_cmd: Option<String>,
    audio_stop_cmd: Option<String>,
    audio_style: Option<String>,
    image_style: Option<String>,
    image_negative_prompt: Option<String>,
    audio_negative_prompt: Option<String>,
    gpu_label: Option<String>,
    duration_sec: Option<u32>,
    steps: Option<u32>,
    guidance: Option<f32>,
    sampler: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    video_width: Option<u32>,
    video_height: Option<u32>,
    image_steps: Option<u32>,
    image_seed: Option<u32>,
    candidates: Option<u32>,
    audio_draft_sec: Option<u32>,
    audio_draft_steps: Option<u32>,
    audio_draft_sampler: Option<String>,
    reference_sec: Option<u32>,
    fade_in_sec: Option<f64>,
    fade_out_sec: Option<f64>,
    loudness_lufs: Option<f64>,
}

impl FileConfig {
    fn load() -> Self {
        let path = Config::config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                eprintln!("warning: could not read {}: {err}", path.display());
                return Self::default();
            }
        };
        match toml::from_str(&text) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("warning: invalid config at {}: {err}", path.display());
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_optional_fields() {
        let text = r#"
            ssh_host = "user@10.0.0.5"
            duration_sec = 240
            fade_in_sec = 2.0
            audio_negative_prompt = "vocals"
            audio_style = "lofi"
            image_style = "watercolor"
            image_negative_prompt = "glowing eyes"
            image_steps = 12
            image_seed = 42
            candidates = 3
            audio_draft_sec = 60
        "#;
        let file: FileConfig = toml::from_str(text).unwrap();
        assert_eq!(file.ssh_host.as_deref(), Some("user@10.0.0.5"));
        assert_eq!(file.duration_sec, Some(240));
        assert_eq!(file.fade_in_sec, Some(2.0));
        assert_eq!(file.audio_negative_prompt.as_deref(), Some("vocals"));
        assert_eq!(file.audio_style.as_deref(), Some("lofi"));
        assert_eq!(file.image_style.as_deref(), Some("watercolor"));
        assert_eq!(file.image_negative_prompt.as_deref(), Some("glowing eyes"));
        assert_eq!(file.image_steps, Some(12));
        assert_eq!(file.image_seed, Some(42));
        assert_eq!(file.candidates, Some(3));
        assert_eq!(file.audio_draft_sec, Some(60));
        assert_eq!(file.steps, None);
    }
}
