use crossterm::style::Stylize;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const ART: [&str; 5] = [
    "#   # ##### #      ###  ####  #####  ###   ####",
    "## ## #     #     #   # #   #   #   #   # #    ",
    "# # # ####  #     #   # #   #   #   #####  ### ",
    "#   # #     #     #   # #   #   #   #   #     #",
    "#   # ##### #####  ###  ####  ##### #   # #### ",
];

const TAGLINE: &str = "Original 8-bit content, made locally";

/// Prints the banner on the headless start, where there is no alternate screen.
pub fn print(pretty: bool, gpu: &str) {
    println!();
    for line in ART {
        if pretty {
            println!("  {}", line.cyan().bold());
        } else {
            println!("  {line}");
        }
    }
    let sub = "S T U D I O";
    let meta = format!("v{VERSION} · {gpu}");
    println!();
    if pretty {
        println!("        {}", sub.dark_grey().bold());
        println!("  {}", TAGLINE.white());
        println!("  {}", meta.dark_grey());
    } else {
        println!("        {sub}");
        println!("  {TAGLINE}");
        println!("  {meta}");
    }
    println!();
}

pub fn draw(frame: &mut Frame, area: Rect, gpu: &str) {
    let mut lines: Vec<Line> = Vec::new();
    if area.width >= 70 && area.height >= 16 {
        lines.push(Line::from(""));
        for line in ART {
            lines.push(Line::from(Span::styled(
                line,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "S T U D I O",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "♪ MELODIAS STUDIO",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(TAGLINE));
    lines.push(Line::from(Span::styled(
        format!("v{VERSION} · {gpu}"),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "press any key",
        Style::default().fg(Color::DarkGray),
    )));

    let height = lines.len() as u16;
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        centered(area, height),
    );
}

fn centered(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: area.width,
        height,
    }
}
