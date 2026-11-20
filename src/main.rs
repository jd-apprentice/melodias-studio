mod app;
mod banner;
mod clients;
mod config;
mod media;
mod pipeline;

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::channel;

use crossterm::style::Stylize;

use pipeline::{Msg, State};

const USAGE: &str =
    "usage: melodias                 (TUI)\n       melodias <url> <vibe>     (headless)";

fn main() -> io::Result<()> {
    let cfg = config::Config::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "-h" || arg == "--help")
    {
        println!("{USAGE}");
        return Ok(());
    }
    if !args.is_empty() {
        return run_headless(cfg, &args);
    }

    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, cfg);
    ratatui::restore();
    result
}

/// Runs the same pipeline the TUI drives, with a compact terminal view. Stages
/// are printed as groups with icons and color, and the plain text goes to the
/// file named by MELODIAS_LOG when it is set.
fn run_headless(cfg: config::Config, args: &[String]) -> io::Result<()> {
    let args: Vec<&String> = args.iter().filter(|arg| *arg != "--headless").collect();
    let [url, vibe] = args.as_slice() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };

    let host = cfg.ssh_host.clone();
    let gpu = cfg.gpu_label.clone();
    let log_path = std::env::var_os("MELODIAS_LOG").map(PathBuf::from);
    if let Some(path) = &log_path {
        println!("log: {}", path.display());
    }
    let mut out = Reporter::new(log_path)?;

    banner::print(out.pretty, &gpu);

    let title = format!("♪ melodias-studio · {host}");
    let colored = out.pretty.then(|| title.as_str().cyan().bold().to_string());
    out.line(&title, colored);
    let separator = "────────────────────────────────────────";
    let colored = out.pretty.then(|| separator.dark_grey().to_string());
    out.line(separator, colored);

    let (tx, rx) = channel();
    pipeline::spawn(cfg, url.trim().to_string(), vibe.trim().to_string(), tx);

    for msg in rx {
        match msg {
            Msg::Stage(idx, state) => out.stage(idx, state),
            Msg::Log(text) => out.log_line(&text),
            Msg::Done(Ok(outcome)) => {
                out.line("", None);
                let line = format!(
                    "✔ done · seed {} · {}",
                    outcome.seed,
                    outcome.audio.display()
                );
                let colored = out.pretty.then(|| line.as_str().green().bold().to_string());
                out.line(&line, colored);
                let image = format!("✔ thumbnail · {}", outcome.image.display());
                let colored = out.pretty.then(|| image.as_str().green().to_string());
                out.line(&image, colored);
                return Ok(());
            }
            Msg::Done(Err(err)) => {
                out.error(&format!("✖ failed: {err}"));
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

struct Reporter {
    log: Option<File>,
    pretty: bool,
}

impl Reporter {
    fn new(path: Option<PathBuf>) -> io::Result<Self> {
        let log = match path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                media::set_child_log(path.clone());
                Some(OpenOptions::new().create(true).append(true).open(path)?)
            }
            None => None,
        };
        Ok(Self {
            log,
            pretty: io::stdout().is_terminal(),
        })
    }

    fn line(&mut self, plain: &str, colored: Option<String>) {
        if let Some(file) = self.log.as_mut() {
            let _ = writeln!(file, "{plain}");
        }
        match colored {
            Some(text) => println!("{text}"),
            None => println!("{plain}"),
        }
    }

    fn error(&mut self, plain: &str) {
        if let Some(file) = self.log.as_mut() {
            let _ = writeln!(file, "{plain}");
        }
        if self.pretty {
            eprintln!("{}", plain.red().bold());
        } else {
            eprintln!("{plain}");
        }
    }

    fn stage(&mut self, idx: usize, state: State) {
        let Some(plain) = pipeline::stage_label(idx, state) else {
            return;
        };
        if state == State::Running {
            self.line("", None);
        }
        let colored = match state {
            State::Running => self
                .pretty
                .then(|| plain.as_str().yellow().bold().to_string()),
            State::Ok => self.pretty.then(|| plain.as_str().green().to_string()),
            State::Failed => self.pretty.then(|| plain.as_str().red().bold().to_string()),
            State::Pending => None,
        };
        self.line(&plain, colored);
    }

    fn log_line(&mut self, text: &str) {
        let plain = format!("   · {text}");
        let colored = self.pretty.then(|| plain.as_str().dark_grey().to_string());
        self.line(&plain, colored);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_clean_log() {
        let path = std::env::temp_dir().join(format!("melodias-studio-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut out = Reporter::new(Some(path.clone())).unwrap();
        out.stage(0, State::Running);
        out.stage(0, State::Ok);
        out.log_line("image prompt: pixel art castle");

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("▶ 1/9 Image prompt"));
        assert!(text.contains("✔ 1/9 Image prompt"));
        assert!(text.contains("   · image prompt: pixel art castle"));
        let _ = std::fs::remove_file(path);
    }
}
