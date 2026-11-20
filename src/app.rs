use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Padding, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::config::Config;
use crate::pipeline::{self, Msg, STAGES, State};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Url,
    Vibe,
}

struct App {
    cfg: Config,
    url: String,
    vibe: String,
    field: Field,
    states: Vec<State>,
    logs: Vec<String>,
    rx: Option<Receiver<Msg>>,
    running: bool,
    summary: Option<String>,
    tick: u64,
    log: Option<File>,
    splash_until: Instant,
}

impl App {
    fn new(cfg: Config) -> Self {
        Self {
            cfg,
            url: String::new(),
            vibe: String::new(),
            field: Field::Url,
            states: vec![State::Pending; STAGES.len()],
            logs: Vec::new(),
            rx: None,
            running: false,
            summary: None,
            tick: 0,
            log: None,
            splash_until: Instant::now() + Duration::from_millis(1500),
        }
    }

    fn start(&mut self) {
        if self.running || self.url.trim().is_empty() || self.vibe.trim().is_empty() {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.states = vec![State::Pending; STAGES.len()];
        self.logs.clear();
        self.summary = None;
        self.running = true;
        self.open_log();
        pipeline::spawn(
            self.cfg.clone(),
            self.url.trim().to_string(),
            self.vibe.trim().to_string(),
            tx,
        );
    }

    fn splash_active(&self) -> bool {
        Instant::now() < self.splash_until
    }

    fn open_log(&mut self) {
        let path = self.cfg.out_dir.join(format!("run-{}.log", unix_seconds()));
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            self.log = None;
            return;
        }
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => {
                crate::media::set_child_log(path.clone());
                self.log = Some(file);
                let label = format!("log: {}", path.display());
                write_line(
                    &mut self.log,
                    &format!("♪ melodias-studio · {}", self.cfg.ssh_host),
                );
                write_line(&mut self.log, &label);
                self.logs.push(label);
            }
            Err(_) => self.log = None,
        }
    }

    fn drain(&mut self) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(Msg::Stage(idx, state)) => {
                    if let Some(label) = pipeline::stage_label(idx, state) {
                        write_line(&mut self.log, &label);
                    }
                    if let Some(slot) = self.states.get_mut(idx) {
                        *slot = state;
                    }
                }
                Ok(Msg::Log(text)) => {
                    write_line(&mut self.log, &format!("   · {text}"));
                    self.logs.push(text);
                }
                Ok(Msg::Done(result)) => {
                    self.running = false;
                    match result {
                        Ok(outcome) => {
                            write_line(&mut self.log, "");
                            write_line(
                                &mut self.log,
                                &format!(
                                    "✔ done · seed {} · {}",
                                    outcome.seed,
                                    outcome.audio.display()
                                ),
                            );
                            write_line(
                                &mut self.log,
                                &format!("✔ thumbnail · {}", outcome.image.display()),
                            );
                            self.summary = Some(format!(
                                "done · seed {} · {}",
                                outcome.seed,
                                outcome.audio.display()
                            ));
                            self.logs
                                .push(format!("thumbnail: {}", outcome.image.display()));
                            self.logs.push(format!("prompt: {}", outcome.image_prompt));
                        }
                        Err(err) => {
                            write_line(&mut self.log, &format!("✖ failed: {err}"));
                            self.summary = Some(format!("failed: {err}"));
                        }
                    }
                    self.log = None;
                    self.rx = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.running = false;
                    self.rx = None;
                    break;
                }
            }
        }
    }
}

pub fn run(terminal: &mut DefaultTerminal, cfg: Config) -> std::io::Result<()> {
    let mut app = App::new(cfg);
    loop {
        terminal.draw(|frame| draw(frame, &app))?;
        app.drain();
        app.tick = app.tick.wrapping_add(1);
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            app.splash_until = Instant::now();
            match key.code {
                KeyCode::Char('q') if !app.running => return Ok(()),
                KeyCode::Esc if !app.running => return Ok(()),
                KeyCode::Tab => {
                    app.field = match app.field {
                        Field::Url => Field::Vibe,
                        Field::Vibe => Field::Url,
                    }
                }
                KeyCode::Enter => {
                    if app.field == Field::Url {
                        app.field = Field::Vibe;
                    } else {
                        app.start();
                    }
                }
                KeyCode::Backspace => match app.field {
                    Field::Url => {
                        app.url.pop();
                    }
                    Field::Vibe => {
                        app.vibe.pop();
                    }
                },
                KeyCode::Char(c) if !app.running => match app.field {
                    Field::Url => app.url.push(c),
                    Field::Vibe => app.vibe.push(c),
                },
                _ => {}
            }
        }
    }
}

fn draw(frame: &mut Frame, app: &App) {
    if app.splash_active() {
        crate::banner::draw(frame, frame.area(), &app.cfg.gpu_label);
        return;
    }
    let [header_area, inputs_area, gauge_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .spacing(1)
    .areas(frame.area());

    let [url_area, vibe_area] =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
            .spacing(1)
            .areas(inputs_area);

    draw_header(frame, app, header_area);
    draw_input(frame, app, url_area, Field::Url, "Reference URL");
    draw_input(frame, app, vibe_area, Field::Vibe, "Vibe or concept");
    draw_gauge(frame, app, gauge_area);
    draw_body(frame, app, body_area);
    draw_status(frame, app, status_area);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " ♪ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "melodias-studio",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {} · {}", app.cfg.ssh_host, app.cfg.gpu_label),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect, field: Field, title: &str) {
    let active = app.field == field && !app.running;
    let color = if active { Color::Cyan } else { Color::DarkGray };
    let marker = if active { "▶ " } else { "  " };
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(Style::default().fg(color))
        .title(Span::styled(
            format!("{marker}{title}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    let text = match field {
        Field::Url => app.url.as_str(),
        Field::Vibe => app.vibe.as_str(),
    };
    let mut spans = vec![Span::raw(text)];
    if active {
        let cursor = if app.tick % 4 < 2 { "▏" } else { " " };
        spans.push(Span::styled(cursor, Style::default().fg(Color::Cyan)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_gauge(frame: &mut Frame, app: &App, area: Rect) {
    let total = STAGES.len();
    let ok = app
        .states
        .iter()
        .filter(|state| **state == State::Ok)
        .count();
    let failed = app.states.contains(&State::Failed);
    let current = app
        .states
        .iter()
        .position(|state| *state == State::Running)
        .and_then(|idx| STAGES.get(idx).copied());
    let color = if failed {
        Color::Red
    } else if ok == total {
        Color::Green
    } else if current.is_some() {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let percent = ok * 100 / total;
    let label = match current {
        Some(name) => format!("{ok}/{total} · {percent}% · {name}"),
        None if ok == total => format!("{ok}/{total} · completo"),
        None => format!("{ok}/{total} · {percent}%"),
    };
    frame.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Progress ")
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .gauge_style(Style::default().fg(color))
            .ratio(ok as f64 / total as f64)
            .label(label),
        area,
    );
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    let [stages_area, log_area] = Layout::horizontal([Constraint::Length(28), Constraint::Min(20)])
        .spacing(1)
        .areas(area);
    draw_stages(frame, app, stages_area);
    draw_log(frame, app, log_area);
}

fn draw_stages(frame: &mut Frame, app: &App, area: Rect) {
    let spinner = ["◐", "◓", "◑", "◒"][(app.tick / 2 % 4) as usize];
    let ok = app
        .states
        .iter()
        .filter(|state| **state == State::Ok)
        .count();
    let items: Vec<ListItem> = STAGES
        .iter()
        .zip(&app.states)
        .enumerate()
        .map(|(idx, (name, state))| {
            let (mark, color, bold) = match state {
                State::Pending => ("○", Color::DarkGray, false),
                State::Running => (spinner, Color::Yellow, true),
                State::Ok => ("✔", Color::Green, false),
                State::Failed => ("✖", Color::Red, true),
            };
            let mut style = Style::default().fg(color);
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), style),
                Span::styled(
                    format!("{:02} ", idx + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(*name, style),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Stages · {ok}/{} ", STAGES.len()))
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

fn draw_log(frame: &mut Frame, app: &App, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(height);
    let lines: Vec<Line> = if app.logs.is_empty() {
        vec![Line::from(Span::styled(
            "No activity yet. Enter starts the run.",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.logs[start..]
            .iter()
            .map(|text| Line::from(Span::styled(text.as_str(), log_style(text))))
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Log ")
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn log_style(text: &str) -> Style {
    let lower = text.to_lowercase();
    if lower.contains("failed") {
        Style::default().fg(Color::Red)
    } else if lower.starts_with("warning") {
        Style::default().fg(Color::Yellow)
    } else if lower.starts_with("thumbnail") || lower.starts_with("master") {
        Style::default().fg(Color::Green)
    } else {
        Style::default()
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let line = if let Some(summary) = &app.summary {
        let color = if summary.starts_with("failed") {
            Color::Red
        } else {
            Color::Green
        };
        Line::from(vec![
            Span::styled(" ● ", Style::default().fg(color)),
            Span::styled(
                summary.as_str(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])
    } else if app.running {
        Line::from(vec![
            Span::styled(" ◐ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "running",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " · the current stage is highlighted above",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Tab", key_style()),
            Span::styled(" field  ", hint_style()),
            Span::styled(" Enter", key_style()),
            Span::styled(" next / run  ", hint_style()),
            Span::styled(" q", key_style()),
            Span::styled(" quit", hint_style()),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn key_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn hint_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn write_line(log: &mut Option<File>, line: &str) {
    if let Some(file) = log.as_mut() {
        let _ = writeln!(file, "{line}");
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use std::path::{Path, PathBuf};

    fn sample_app() -> App {
        let mut app = App::new(Config::from_env());
        app.cfg.ssh_host = "studio-gpu".to_string();
        app.cfg.gpu_label = "local models".to_string();
        app.url = "https://www.youtube.com/watch?v=sFcFugiM2g".to_string();
        app.vibe = "epic 8-bit emotional artcore & orchestra".to_string();
        for state in &mut app.states[..6] {
            *state = State::Ok;
        }
        app.states[6] = State::Running;
        app.running = true;
        app.logs = vec![
            "working in output/epic-8-bit-emotional-artcore-orchestra/work".to_string(),
            "audio seed 198759983 · image seed 424242".to_string(),
            "image prompt: anime girl with glowing pixel eyes, 8-bit forest, warm sunset".to_string(),
            "thumbnail: output/epic-8-bit-emotional-artcore-orchestra/thumbnail.png (1280x720, 1.5 MB, 5.9 min)".to_string(),
            "source: output/epic-8-bit-emotional-artcore-orchestra/work/source.wav".to_string(),
            "reference: output/epic-8-bit-emotional-artcore-orchestra/work/reference.wav (60 s)".to_string(),
            "prompt audio: epic 8-bit emotional artcore & orchestra, 8-bit chiptune, instrumental, soft intro that builds gradually".to_string(),
            "take: output/epic-8-bit-emotional-artcore-orchestra/work/take.wav (40.3 MB)".to_string(),
        ];
        app
    }

    #[test]
    fn draws_without_leaving_the_terminal() {
        let mut app = sample_app();

        for (width, height) in [(80, 24), (120, 36), (100, 30)] {
            app.splash_until = Instant::now() + Duration::from_secs(60);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            dump(&terminal, width, height, "splash");
            if let Some(dir) = std::env::var_os("DUMP_TUI_HTML_DIR") {
                let dir = PathBuf::from(dir);
                let _ = std::fs::create_dir_all(&dir);
                dump_html(&terminal, width, height, &dir.join("tui-splash.html"));
            }

            app.splash_until = Instant::now();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            dump(&terminal, width, height, "ui");
            if let Some(dir) = std::env::var_os("DUMP_TUI_HTML_DIR") {
                let dir = PathBuf::from(dir);
                let _ = std::fs::create_dir_all(&dir);
                dump_html(&terminal, width, height, &dir.join("tui.html"));
            }
        }
    }

    fn dump(terminal: &Terminal<TestBackend>, width: u16, height: u16, label: &str) {
        if std::env::var_os("PRINT_TUI").is_none() {
            return;
        }
        let buffer = terminal.backend().buffer();
        println!("--- {label} {width}x{height} ---");
        for y in 0..height {
            let mut line = String::new();
            for x in 0..width {
                line.push_str(buffer[(x, y)].symbol());
            }
            println!("{line}");
        }
    }

    fn dump_html(terminal: &Terminal<TestBackend>, width: u16, height: u16, path: &Path) {
        let buffer = terminal.backend().buffer();
        let mut rows = String::new();
        for y in 0..height {
            let mut spans: Vec<(String, String, String, bool, bool)> = Vec::new();
            for x in 0..width {
                let cell = &buffer[(x, y)];
                let style = cell.style();
                let fg = fg_hex(style.fg.unwrap_or(Color::Reset));
                let bg = bg_hex(style.bg.unwrap_or(Color::Reset));
                let bold = style.add_modifier.contains(Modifier::BOLD);
                let dim = style.add_modifier.contains(Modifier::DIM);
                let symbol = escape(cell.symbol());
                if let Some(last) = spans.last_mut()
                    && last.1 == fg
                    && last.2 == bg
                    && last.3 == bold
                    && last.4 == dim
                {
                    last.0.push_str(&symbol);
                    continue;
                }
                spans.push((symbol, fg, bg, bold, dim));
            }
            rows.push_str("<div>");
            for (text, fg, bg, bold, dim) in spans {
                let mut css = format!("color:{fg};");
                if bg != "#0b0e14" {
                    css.push_str(&format!("background:{bg};"));
                }
                if bold {
                    css.push_str("font-weight:600;");
                }
                if dim {
                    css.push_str("opacity:.6;");
                }
                rows.push_str(&format!("<span style=\"{css}\">{text}</span>"));
            }
            rows.push_str("</div>");
        }
        let html = format!(
            r#"<!doctype html>
<html><head><meta charset="utf-8"><style>
html {{ margin:0; }} body {{ margin:0; padding:24px; background:#0d1117; }}
#app {{ display:inline-block; }}
.bar {{ display:flex; align-items:center; gap:8px; padding:9px 14px;
        background:#161b22; border-bottom:1px solid #21262d;
        font:13px 'DejaVu Sans', sans-serif; color:#8b949e; }}
.dot {{ width:10px; height:10px; border-radius:50%; display:inline-block; }}
.r {{ background:#ff5f56; }} .y {{ background:#ffbd2e; }} .g {{ background:#27c93f; }}
.term {{ background:#0b0e14; padding:20px 24px; color:#c9d1d9;
         font:15px/1.35 'DejaVu Sans Mono', monospace; white-space:pre; }}
</style></head><body><div id="app">
<div class="bar"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span><span>melodias · TUI</span></div>
<div class="term">{rows}</div>
</div></body></html>
"#
        );
        let _ = std::fs::write(path, html);
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn fg_hex(color: Color) -> String {
        match color {
            Color::Reset => "#c9d1d9".to_string(),
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            other => palette(other),
        }
    }

    fn bg_hex(color: Color) -> String {
        match color {
            Color::Reset | Color::Black => "#0b0e14".to_string(),
            other => palette(other),
        }
    }

    fn palette(color: Color) -> String {
        let hex = match color {
            Color::Black => "#1f2328",
            Color::Red => "#f85149",
            Color::Green => "#3fb950",
            Color::Yellow => "#d29922",
            Color::Blue => "#58a6ff",
            Color::Magenta => "#bc8cff",
            Color::Cyan => "#39c5cf",
            Color::Gray => "#8b949e",
            Color::DarkGray => "#6e7681",
            Color::LightRed => "#ff7b72",
            Color::LightGreen => "#56d364",
            Color::LightYellow => "#e3b341",
            Color::LightBlue => "#79c0ff",
            Color::LightMagenta => "#d2a8ff",
            Color::LightCyan => "#56d4dd",
            Color::White => "#f0f6fc",
            _ => "#c9d1d9",
        };
        hex.to_string()
    }
}
