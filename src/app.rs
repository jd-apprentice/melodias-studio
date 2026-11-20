use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::config::Config;
use crate::pipeline::{self, Msg, STAGES, State};

#[derive(PartialEq, Eq)]
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
        pipeline::spawn(
            self.cfg.clone(),
            self.url.trim().to_string(),
            self.vibe.trim().to_string(),
            tx,
        );
    }

    fn drain(&mut self) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(Msg::Stage(idx, state)) => {
                    if let Some(slot) = self.states.get_mut(idx) {
                        *slot = state;
                    }
                }
                Ok(Msg::Log(text)) => self.logs.push(text),
                Ok(Msg::Done(result)) => {
                    self.running = false;
                    match result {
                        Ok(outcome) => {
                            self.summary = Some(format!(
                                "listo · seed {} · {}",
                                outcome.seed,
                                outcome.audio.display()
                            ));
                            self.logs
                                .push(format!("miniatura: {}", outcome.image.display()));
                            self.logs.push(format!("prompt: {}", outcome.image_prompt));
                        }
                        Err(err) => self.summary = Some(format!("falló: {err}")),
                    }
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
    let [
        url_area,
        vibe_area,
        stages_area,
        gauge_area,
        log_area,
        status_area,
    ] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(9),
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let active = |field: &Field| {
        if &app.field == field && !app.running {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };

    frame.render_widget(
        Paragraph::new(app.url.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" YouTube URL ")
                    .border_style(active(&Field::Url)),
            )
            .wrap(Wrap { trim: true }),
        url_area,
    );

    frame.render_widget(
        Paragraph::new(app.vibe.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Vibe / prompt del tema ")
                    .border_style(active(&Field::Vibe)),
            )
            .wrap(Wrap { trim: true }),
        vibe_area,
    );

    let items: Vec<ListItem> = STAGES
        .iter()
        .zip(&app.states)
        .map(|(name, state)| {
            let (mark, style) = match state {
                State::Pending => ("  ", Style::default().fg(Color::DarkGray)),
                State::Running => ("» ", Style::default().fg(Color::Yellow)),
                State::Ok => ("✓ ", Style::default().fg(Color::Green)),
                State::Failed => ("✗ ", Style::default().fg(Color::Red)),
            };
            ListItem::new(Line::from(format!("{mark}{name}"))).style(style)
        })
        .collect();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" Etapas ")),
        stages_area,
    );

    let ok = app
        .states
        .iter()
        .filter(|state| **state == State::Ok)
        .count();
    let total = STAGES.len();
    let running = app
        .states
        .iter()
        .position(|state| *state == State::Running)
        .and_then(|idx| STAGES.get(idx));
    let label = match running {
        Some(name) => format!("{ok}/{total} · {name}"),
        None if ok == total => format!("{ok}/{total} · completo"),
        None => format!("{ok}/{total}"),
    };
    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" Progreso "))
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ok as f64 / total as f64)
            .label(label),
        gauge_area,
    );

    let height = log_area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(height);
    let text: Vec<Line> = app.logs[start..]
        .iter()
        .map(|line| Line::from(line.as_str()))
        .collect();
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Log "))
            .wrap(Wrap { trim: false }),
        log_area,
    );

    let status = if let Some(summary) = &app.summary {
        Line::from(summary.as_str()).style(Style::default().fg(Color::Green))
    } else if app.running {
        Line::from("generando…  q para salir al terminar").style(Style::default().fg(Color::Yellow))
    } else {
        Line::from("Tab cambia de campo · Enter avanza/genera · q sale")
            .style(Style::default().add_modifier(Modifier::DIM))
    };
    frame.render_widget(Paragraph::new(status), status_area);
}
