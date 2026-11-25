use super::app::{App, Prompt, Tone, View, display_path};
use crate::{providers::clip, transcript::ago, workspace::Scope};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
};
use std::time::SystemTime;

const TEAL: Color = Color::Rgb(78, 184, 170);
const FOOTER: &str = "↑↓ Move · Enter Preview · c Copy · e Export · ? Help";
pub(super) const SPLIT_WIDTH: u16 = 80;

fn provider_color(provider: &str) -> Color {
    match provider {
        "claude" => Color::Rgb(198, 151, 127),
        "codex" => Color::Rgb(140, 172, 152),
        "gemini" => Color::Rgb(145, 166, 202),
        "copilot" => Color::Rgb(172, 152, 191),
        "cline" => Color::Rgb(183, 166, 132),
        _ => Color::Reset,
    }
}

fn tone(tone: Tone) -> Style {
    match tone {
        Tone::Info => Style::new(),
        Tone::Ok => Style::new().fg(Color::Green),
        Tone::Warn => Style::new().fg(Color::Yellow),
        Tone::Error => Style::new().fg(Color::Red),
        Tone::Heading => Style::new().add_modifier(Modifier::BOLD),
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    if frame.area().width < 40 || frame.area().height < 10 {
        frame.render_widget(
            Paragraph::new("Resize to at least 40 × 10.\nq / Ctrl+C quit")
                .wrap(Wrap { trim: false }),
            frame.area(),
        );
        return;
    }
    let keys = match (&app.prompt, app.view) {
        (Some(Prompt::Import { .. }), _) => "Enter Preview · Esc Cancel",
        (Some(Prompt::Export { .. }), _) => "Enter Save · Esc Cancel · Ctrl+U Clear",
        (Some(Prompt::Overwrite { .. }), _) => "y Overwrite · n Edit path",
        (_, View::Help) => "↑↓ Scroll · Esc Back",
        (_, View::Diagnostics) => "↑↓ Scroll · Esc Back · ? Help",
        _ => FOOTER,
    };
    let footer_height = Paragraph::new(keys)
        .wrap(Wrap { trim: true })
        .line_count(frame.area().width) as u16;
    let [header, body, status, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(footer_height),
    ])
    .areas(frame.area());
    draw_header(frame, header, app);
    match app.view {
        View::Help => draw_help(frame, body, app),
        View::Sessions | View::Preview
            if body.width >= SPLIT_WIDTH && app.pending_import.is_none() =>
        {
            let [sessions, preview] =
                Layout::horizontal([Constraint::Percentage(39), Constraint::Fill(1)]).areas(body);
            draw_sessions(frame, sessions, app);
            draw_preview(frame, preview, app);
        }
        View::Sessions => draw_sessions(frame, body, app),
        View::Preview => draw_preview(frame, body, app),
        View::Diagnostics => draw_diagnostics(frame, body, app),
    }
    draw_status(frame, status, app);
    frame.render_widget(
        Paragraph::new(keys)
            .wrap(Wrap { trim: true })
            .style(Style::new().add_modifier(Modifier::DIM)),
        footer,
    );
}

pub fn draw_loading(frame: &mut Frame) {
    frame.render_widget(Paragraph::new(" Loading sessions…"), frame.area());
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let home = app.home();
    let scope = match &app.listing.scope {
        Scope::Checkout(root) => display_path(root, home),
        Scope::Global => "all workspaces".into(),
    };
    let count = app.listing.sessions.len();
    frame.render_widget(
        Line::from(vec![
            Span::styled(" bridgy ", Style::new().reversed().bold()),
            Span::raw(format!("  {scope}  ")),
            Span::styled(
                format!("{count} session{}", if count == 1 { "" } else { "s" }),
                Style::new().add_modifier(Modifier::DIM),
            ),
        ]),
        area,
    );
}

fn draw_sessions(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::bordered().title(" Sessions ");
    if app.listing.sessions.is_empty() {
        let mut lines = vec![
            Line::from(format!("No sessions for {}.", app.listing.scope)).bold(),
            Line::default(),
        ];
        for report in &app.listing.reports {
            lines.push(Line::from(report.describe(&app.listing.scope)));
        }
        lines.push(Line::default());
        lines.push(
            Line::from("Press g to browse all workspaces, or d for diagnostics.")
                .add_modifier(Modifier::DIM),
        );
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let visible = usize::from(inner.height / 2).max(1);
    if app.view == View::Sessions {
        app.page = visible as u16;
    }
    app.list_offset = app
        .list_offset
        .clamp((app.selected + 1).saturating_sub(visible), app.selected)
        .min(app.listing.sessions.len().saturating_sub(visible));
    let now = SystemTime::now();
    let mut lines = Vec::with_capacity(2 * visible);
    for (idx, session) in app
        .listing
        .sessions
        .iter()
        .enumerate()
        .skip(app.list_offset)
        .take(visible)
    {
        let selected = idx == app.selected;
        let title = session.title.as_deref().unwrap_or(&session.id);
        lines.push(Line::from(vec![
            Span::styled(if selected { "▎ " } else { "  " }, Style::new().fg(TEAL)),
            Span::styled(
                clip(title, inner.width.saturating_sub(3) as usize),
                if selected {
                    Style::new().bold()
                } else {
                    Style::new()
                },
            ),
        ]));
        let provider = session.reference.provider;
        let mut details = vec![
            Span::raw("  "),
            Span::styled(provider, Style::new().fg(provider_color(provider))),
        ];
        if let Some(client) = &session.surface {
            details.push(Span::raw(format!(" · {client}")).dim());
        }
        details.push(Span::raw(format!(" · {}", ago(session.last_activity, now))).dim());
        if app.listing.scope == Scope::Global
            && let Some(ws) = &session.workspace
        {
            details.push(Span::raw(format!(" · {}", display_path(ws, app.home()))).dim());
        }
        lines.push(Line::from(details));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_preview(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::bordered().title(" Preview ");
    let Some(loaded) = &app.loaded else {
        let hint = app
            .preview_error
            .as_deref()
            .unwrap_or(if app.selected_session().is_some() {
                "Loading conversation…"
            } else {
                "No conversation selected"
            });
        frame.render_widget(
            Paragraph::new(hint)
                .dim()
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    };
    let block = block.title_bottom(if app.view == View::Preview {
        " Esc Back "
    } else {
        ""
    });
    let mut lines = vec![
        Line::from(clip(&loaded.title, area.width.saturating_sub(3) as usize)).bold(),
        Line::default(),
    ];
    for turn in &loaded.turns {
        let role = match turn.role.as_str() {
            "user" => "You",
            "assistant" => "Assistant",
            role => role,
        };
        if role != "document" {
            let color = if role == "You" {
                TEAL
            } else {
                provider_color(loaded.reference.provider)
            };
            let mut label = vec![Span::styled(role, Style::new().fg(color).bold())];
            if let Some(time) = &turn.timestamp {
                label.push(Span::raw(format!("  {time}")).dim());
            }
            lines.push(Line::from(label));
        }
        lines.extend(turn.text.lines().map(Line::from));
        lines.push(Line::default());
    }
    if loaded.turns.is_empty() {
        lines.push(Line::from("No conversational text found.").dim());
    }
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let offset = if app.view == View::Preview {
        let inner = block.inner(area);
        app.max_scroll = paragraph
            .line_count(inner.width)
            .saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16;
        app.page = inner.height.saturating_sub(1).max(1);
        app.scroll = app.scroll.min(app.max_scroll);
        app.scroll
    } else {
        0
    };
    frame.render_widget(paragraph.block(block).scroll((offset, 0)), area);
}

fn draw_help(frame: &mut Frame, area: Rect, app: &mut App) {
    let text = Text::from(vec![
        Line::from("Navigation").bold(),
        Line::from("↑ ↓ / k j     Move or scroll"),
        Line::from("PgUp / PgDn   Move one page"),
        Line::from("Space         Page down"),
        Line::from("Home / End    First or last"),
        Line::from("Enter         Focus the selected preview"),
        Line::from("Esc           Go back, or quit from the list"),
        Line::from("?             Open or close Help"),
        Line::from("q / Ctrl+C    Quit"),
        Line::default(),
        Line::from("Sessions").bold(),
        Line::from("c / e         Copy / export Markdown"),
        Line::from("g             Toggle checkout / all workspaces"),
        Line::from("r             Refresh the session list"),
        Line::from("d             Open diagnostics"),
        Line::from("i             Import a conversation file"),
        Line::default(),
        Line::from("Import and export").bold(),
        Line::from("s / a         Remember import / attach to this checkout"),
        Line::from("Enter / Esc   Submit / cancel path"),
        Line::from("Ctrl+U        Clear export path"),
        Line::from("y / n         Overwrite / edit export path"),
    ]);
    scrolling(frame, area, app, Block::bordered().title(" Help "), text);
}

fn draw_diagnostics(frame: &mut Frame, area: Rect, app: &mut App) {
    let text: Text = app
        .diagnostics
        .iter()
        .map(|l| Line::styled(l.text.clone(), tone(l.tone)))
        .collect();
    scrolling(
        frame,
        area,
        app,
        Block::bordered().title(" Diagnostics "),
        text,
    );
}

fn scrolling(frame: &mut Frame, area: Rect, app: &mut App, block: Block, text: Text) {
    let inner = block.inner(area);
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let total = paragraph.line_count(inner.width);
    app.max_scroll = total
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16;
    app.page = inner.height.saturating_sub(1).max(1);
    app.scroll = app.scroll.min(app.max_scroll);
    frame.render_widget(paragraph.block(block).scroll((app.scroll, 0)), area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let line = match &app.prompt {
        Some(Prompt::Import { input }) => Line::from(format!(" Import path: {input}█")),
        Some(Prompt::Export { input }) => Line::from(vec![
            Span::styled(" Export to: ", Style::new().bold()),
            Span::raw(input.clone()),
            Span::styled("█", Style::new().add_modifier(Modifier::SLOW_BLINK)),
        ]),
        Some(Prompt::Overwrite { path, .. }) => Line::from(vec![Span::styled(
            format!(" {} exists. Overwrite? (y/n)", path.display()),
            Style::new().fg(Color::Yellow).bold(),
        )]),
        None => match &app.status {
            Some(status) => Line::styled(format!(" {}", status.text), tone(status.tone)),
            None => Line::default(),
        },
    };
    frame.render_widget(line, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyCode};
    use std::fs;

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn renders_empty_preview_diagnostics_and_narrow_terminal() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = Config::resolve(Some(temp.path().to_owned())).unwrap();
        let mut app = App::new(&config, Scope::Global, temp.path().to_owned(), |_| {
            Ok("test")
        });
        assert!(render(&mut app, 80, 24).contains("No sessions"));
        let store = temp.path().join(".claude/projects/p");
        fs::create_dir_all(&store).unwrap();
        fs::write(
            store.join("s.jsonl"),
            [
                serde_json::json!({"type":"user", "message":{"content":"First question"}}).to_string(),
                serde_json::json!({"type":"assistant", "message":{"content":"A long conversation line. ".repeat(200)}}).to_string(),
            ].join("\n"),
        )
        .unwrap();
        let mut app = App::new(&config, Scope::Global, temp.path().to_owned(), |_| {
            Ok("test")
        });
        let picker = render(&mut app, 80, 24);
        assert!(picker.contains("claude"));
        assert!(picker.contains(FOOTER));
        assert!(picker.contains("┌ Preview "));
        assert!(picker.find("▎ First question").unwrap() < picker.find("claude").unwrap());
        assert!(!render(&mut app, 79, 24).contains("┌ Preview "));
        app.handle(KeyCode::Enter.into());
        let preview = render(&mut app, 79, 24);
        assert!(!preview.contains(" Sessions "));
        assert!(preview.contains("You"));
        assert!(preview.contains("Assistant"));
        assert!(render(&mut app, 80, 24).contains(" Sessions "));
        app.handle(KeyCode::Down.into());
        app.handle(KeyCode::Char('?').into());
        assert!(render(&mut app, 80, 32).contains("Toggle checkout"));
        app.handle(KeyCode::Esc.into());
        assert_eq!(app.view, View::Preview);
        render(&mut app, 80, 24);
        assert_eq!(app.scroll, 1);
        assert!(app.max_scroll > 0);
        app.scroll = u16::MAX;
        render(&mut app, 40, 10);
        assert_eq!(app.scroll, app.max_scroll);
        app.handle(KeyCode::Esc.into());
        let picker = render(&mut app, 79, 24);
        assert!(picker.contains(" Sessions "));
        assert!(!picker.contains("┌ Preview "));
        app.open_diagnostics();
        assert!(render(&mut app, 80, 40).contains("Instruction targets"));
        assert!(render(&mut app, 20, 5).contains("Resize"));
    }
}
