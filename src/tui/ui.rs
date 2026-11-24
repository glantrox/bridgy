use super::app::{App, Prompt, Tone, View, display_path};
use crate::{
    providers::clip,
    transcript::{ago, display_name},
    workspace::Scope,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
};
use std::time::SystemTime;

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
    let [header, body, status, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_header(frame, header, app);
    match app.view {
        View::Sessions => draw_sessions(frame, body, app),
        View::Preview => draw_preview(frame, body, app),
        View::Diagnostics => draw_diagnostics(frame, body, app),
    }
    draw_status(frame, status, app);
    frame.render_widget(
        Paragraph::new(keys(app)).style(Style::new().add_modifier(Modifier::DIM)),
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
    let now = SystemTime::now();
    let global = app.listing.scope == Scope::Global;
    let home = app.home().to_owned();
    let inner = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .listing
        .sessions
        .iter()
        .map(|s| {
            let prefix = format!(
                "{:<7} {:<8} {:>9}  ",
                s.reference.provider,
                s.surface.as_deref().unwrap_or(""),
                ago(s.last_activity, now)
            );
            let mut title = s.title.clone().unwrap_or_else(|| s.id.clone());
            if global && let Some(ws) = &s.workspace {
                title = format!("{title}  — {}", display_path(ws, &home));
            }
            let room = inner.saturating_sub(prefix.chars().count()).max(8);
            ListItem::new(Line::from(vec![
                Span::styled(prefix, Style::new().add_modifier(Modifier::DIM)),
                Span::raw(clip(&title, room)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().reversed())
        .highlight_symbol("› ");
    let mut state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(loaded) = &app.loaded else {
        return;
    };
    let session = app.selected_session();
    let mut title = format!(" {} ", display_name(loaded.reference.provider));
    if let Some(surface) = session.and_then(|s| s.surface.as_deref()) {
        title.push_str(&format!("· {surface} "));
    }
    title.push_str(&format!("· {} ", clip(&loaded.title, 60)));
    let text: Text = loaded
        .text
        .lines()
        .map(|line| {
            if line.starts_with("## User") {
                Line::from(line.to_owned()).fg(Color::Cyan).bold()
            } else if line.starts_with("## Assistant") {
                Line::from(line.to_owned()).fg(Color::Magenta).bold()
            } else if line.starts_with("# ") {
                Line::from(line.to_owned()).bold()
            } else if line.starts_with("> ") || line.starts_with("- ") && line.contains(": ") {
                Line::from(line.to_owned()).add_modifier(Modifier::DIM)
            } else {
                Line::from(line.to_owned())
            }
        })
        .collect();
    scrolling(frame, area, app, Block::bordered().title(title), text);
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

fn keys(app: &App) -> String {
    match (&app.prompt, app.view) {
        (Some(Prompt::Export { .. }), _) => " Enter save · Esc cancel · Ctrl+U clear".into(),
        (Some(Prompt::Overwrite { .. }), _) => " y overwrite · n choose another path".into(),
        (None, View::Sessions) => format!(
            " ↑↓ move · Enter preview · c copy · e export · d diagnostics · g {} · r refresh · q quit",
            if app.listing.scope == Scope::Global {
                "this checkout"
            } else {
                "all workspaces"
            }
        ),
        (None, View::Preview) => {
            " ↑↓ scroll · PgUp/PgDn page · c copy · e export · d diagnostics · Esc back · q quit"
                .into()
        }
        (None, View::Diagnostics) => " ↑↓ scroll · PgUp/PgDn page · Esc back · q quit".into(),
    }
}
