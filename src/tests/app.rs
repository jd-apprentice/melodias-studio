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
            "working in output/epic-8-bit-emotional-artcore-orchestra".to_string(),
            "audio seed 198759983 · image seed 424242 · 3 candidates".to_string(),
            "image prompt: anime girl with soft green eyes, 8-bit forest, warm sunset".to_string(),
            "image candidate 1/3: output/epic-8-bit-emotional-artcore-orchestra/candidates/image-01.png (seed 424242, 2.2 MB)".to_string(),
            "image pool: 3 candidates in 14.8 min".to_string(),
            "source: output/epic-8-bit-emotional-artcore-orchestra/work/source.wav".to_string(),
            "reference: output/epic-8-bit-emotional-artcore-orchestra/work/reference.wav (60 s)".to_string(),
            "prompt audio: epic 8-bit emotional artcore & orchestra, 8-bit chiptune, instrumental, soft intro that builds gradually".to_string(),
            "audio draft 1/3: output/epic-8-bit-emotional-artcore-orchestra/candidates/audio-01.wav (seed 198759983, 60 s)".to_string(),
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

        app.review = Some(Review {
            images: vec![
                pipeline::Candidate {
                    file: PathBuf::from("candidates/image-01.png"),
                    seed: 424242,
                },
                pipeline::Candidate {
                    file: PathBuf::from("candidates/image-02.png"),
                    seed: 424243,
                },
                pipeline::Candidate {
                    file: PathBuf::from("candidates/image-03.png"),
                    seed: 424244,
                },
            ],
            audios: vec![
                pipeline::Candidate {
                    file: PathBuf::from("candidates/audio-01.wav"),
                    seed: 198759983,
                },
                pipeline::Candidate {
                    file: PathBuf::from("candidates/audio-02.wav"),
                    seed: 198759984,
                },
                pipeline::Candidate {
                    file: PathBuf::from("candidates/audio-03.wav"),
                    seed: 198759985,
                },
            ],
            image: 1,
            audio: 0,
            field: ReviewField::Images,
            note: Some("image 2 · audio 1 · score 88 · warm light and coherent mood".to_string()),
        });
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        dump(&terminal, width, height, "review");
        if let Some(dir) = std::env::var_os("DUMP_TUI_HTML_DIR") {
            let dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            dump_html(&terminal, width, height, &dir.join("tui-review.html"));
        }
        app.review = None;
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
