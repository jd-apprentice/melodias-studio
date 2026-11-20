use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    draw_input(frame, app, url_area, Field::Url, "YouTube URL");
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
            format!("   {} · one P40 for three services", app.cfg.ssh_host),
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
    } else if lower.starts_with("thumbnail") || lower.starts_with("loop") {
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

    #[test]
    fn draws_without_leaving_the_terminal() {
        let mut app = App::new(Config::from_env());
        app.url = "https://www.youtube.com/watch?v=AeA34N7d-rk&list=WL&index=1".to_string();
        app.vibe = "epic 8-bit orchestral finale".to_string();
        app.states[0] = State::Ok;
        app.states[1] = State::Ok;
        app.states[2] = State::Running;
        app.logs
            .push("image prompt: pixel art castle at sunset".to_string());
        app.logs
            .push("thumbnail: output/destiny/thumbnail.png".to_string());

        for (width, height) in [(80, 24), (120, 36)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            if std::env::var_os("PRINT_TUI").is_some() {
                let buffer = terminal.backend().buffer();
                println!("--- {width}x{height} ---");
                for y in 0..height {
                    let mut line = String::new();
                    for x in 0..width {
                        line.push_str(buffer[(x, y)].symbol());
                    }
                    println!("{line}");
                }
            }
        }
    }
}
