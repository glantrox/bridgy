//! `bridgy` with no arguments: pick a session from this checkout, preview it, copy or export it.

pub mod app;
mod ui;

use crate::{clipboard, config::Config, workspace::Scope};
use anyhow::Result;
use app::App;
use ratatui::crossterm::event::{self, Event};
use std::path::PathBuf;

/// Restores raw mode and the main screen on every exit path, including `?` errors.
/// `ratatui::try_init` also installs a panic hook that restores before printing the panic.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub fn run(config: &Config, scope: Scope, cwd: PathBuf) -> Result<()> {
    let _restore = Restore;
    let mut terminal = ratatui::try_init()?;
    terminal.draw(ui::draw_loading)?;
    let mut app = App::new(config, scope, cwd, clipboard::copy);
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        if let Event::Key(key) = event::read()? {
            app.handle(key);
        }
    }
    Ok(())
}
