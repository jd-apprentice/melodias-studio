mod app;
mod clients;
mod config;
mod media;
mod pipeline;

use std::io;

fn main() -> io::Result<()> {
    let cfg = config::Config::from_env();
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, cfg);
    ratatui::restore();
    result
}
