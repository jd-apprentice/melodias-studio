use std::path::PathBuf;

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
    pub llm_stop_cmd: String,
    pub sd_start_cmd: String,
    pub sd_stop_cmd: String,
    pub audio_start_cmd: String,
    pub duration_sec: u32,
    pub steps: u32,
    pub guidance: f32,
    pub sampler: String,
    pub width: u32,
    pub height: u32,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            audio_url: env_or("AUDIO_URL", "http://192.168.88.38:8090"),
            audio_model: env_or("AUDIO_MODEL", "ace-step"),
            sd_url: env_or("SD_URL", "http://192.168.88.38:1234"),
            llm_url: env_or("LLM_URL", "http://192.168.88.38:8080"),
            llm_model: env_or("LLM_MODEL", "local"),
            ssh_host: env_or("SSH_HOST", "root@192.168.88.38"),
            remote_ref_dir: env_or("REMOTE_REF_DIR", "/models/ref"),
            out_dir: PathBuf::from(env_or("OUT_DIR", "output")),
            llm_stop_cmd: env_or("LLM_STOP_CMD", ""),
            sd_start_cmd: env_or("SD_START_CMD", ""),
            sd_stop_cmd: env_or("SD_STOP_CMD", ""),
            audio_start_cmd: env_or("AUDIO_START_CMD", ""),
            duration_sec: env_or("DURATION_SEC", "210").parse().unwrap_or(210),
            steps: env_or("STEPS", "32").parse().unwrap_or(32),
            guidance: env_or("GUIDANCE", "5").parse().unwrap_or(5.0),
            sampler: env_or("SAMPLER", "heun"),
            width: env_or("WIDTH", "1280").parse().unwrap_or(1280),
            height: env_or("HEIGHT", "720").parse().unwrap_or(720),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
