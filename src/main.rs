mod app;
mod clients;
mod config;
mod media;
mod pipeline;

use std::io;
use std::sync::mpsc::channel;

use pipeline::Msg;

const USAGE: &str = "uso: melodias-studio                 (TUI)\n     melodias-studio <url> <vibe>     (sin TUI)";

fn main() -> io::Result<()> {
    let cfg = config::Config::from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "-h" || arg == "--help") {
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

/// Runs the same pipeline the TUI drives, printing each stage and log line. It
/// exists so a long run can happen in a terminal without a full screen UI.
fn run_headless(cfg: config::Config, args: &[String]) -> io::Result<()> {
    let args: Vec<&String> = args.iter().filter(|arg| *arg != "--headless").collect();
    let [url, vibe] = args.as_slice() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };

    let (tx, rx) = channel();
    pipeline::spawn(
        cfg,
        url.trim().to_string(),
        vibe.trim().to_string(),
        tx,
    );

    for msg in rx {
        match msg {
            Msg::Stage(idx, state) => println!("[{}] {state:?}", pipeline::STAGES[idx]),
            Msg::Log(text) => println!("    {text}"),
            Msg::Done(Ok(outcome)) => {
                println!(
                    "listo · seed {} · {}\nminiatura · {}",
                    outcome.seed,
                    outcome.audio.display(),
                    outcome.image.display()
                );
                return Ok(());
            }
            Msg::Done(Err(err)) => {
                eprintln!("falló: {err}");
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
