//! `bridgy` with no arguments: pick a session from this checkout, preview it, copy or export it.

pub mod app;
mod ui;

use crate::transcript::{self, Transcript};
use crate::{clipboard, config::Config, workspace::Scope};
use anyhow::Result;
use app::{App, View};
use ratatui::crossterm::event::{self, Event};
use std::{
    path::PathBuf,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Restores raw mode and the main screen on every exit path, including `?` errors.
/// `ratatui::try_init` also installs a panic hook that restores before printing the panic.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

const PREVIEW_DELAY: Duration = Duration::from_millis(150);

#[derive(Default)]
struct PreviewLoader {
    pending: Option<(u64, Instant)>,
    worker: Option<(u64, JoinHandle<Result<Transcript>>)>,
}

impl PreviewLoader {
    /// Only one disk read runs at once. Superseded reads may finish, but cannot publish.
    fn tick(&mut self, app: &mut App, wide: bool, now: Instant) -> Option<Duration> {
        let wants_preview = wide
            && app.view == View::Sessions
            && app.prompt.is_none()
            && app.loaded.is_none()
            && app.preview_error.is_none()
            && app.selected_session().is_some();
        if self
            .worker
            .as_ref()
            .is_some_and(|(_, job)| job.is_finished())
        {
            let (revision, job) = self.worker.take().unwrap();
            let result = job
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("preview worker stopped")));
            if wants_preview && revision == app.revision {
                app.accept_preview(result);
                self.pending = None;
                return None;
            }
        }
        if !wants_preview {
            self.pending = None;
            return None;
        }
        if self
            .pending
            .is_none_or(|(revision, _)| revision != app.revision)
        {
            self.pending = Some((app.revision, now + PREVIEW_DELAY));
        }
        let (_, deadline) = self.pending.unwrap();
        if deadline > now {
            return Some(deadline - now);
        }
        if self.worker.is_none() {
            let session = app.selected_session().unwrap().clone();
            let limits = app.limits;
            let config = Config {
                home: app.home().to_owned(),
            };
            self.worker = Some((
                app.revision,
                thread::spawn(move || transcript::load(&config, &session, limits)),
            ));
        }
        Some(Duration::from_millis(25))
    }
}

pub fn run(config: &Config, scope: Scope, cwd: PathBuf) -> Result<()> {
    let _restore = Restore;
    let mut terminal = ratatui::try_init()?;
    terminal.draw(ui::draw_loading)?;
    let mut app = App::new(config, scope, cwd, clipboard::copy);
    let mut previews = PreviewLoader::default();
    let mut redraw = true;
    while !app.quit {
        let size = terminal.size()?;
        let wide = size.width >= ui::SPLIT_WIDTH && size.height >= 10;
        let waiting = previews.tick(&mut app, wide, Instant::now());
        if redraw || waiting.is_none() {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
        }
        redraw = false;
        if let Some(timeout) = waiting
            && !event::poll(timeout)?
        {
            continue;
        }
        redraw = true;
        if let Event::Key(key) = event::read()? {
            app.handle(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyCode;
    use std::fs;

    #[test]
    fn preview_waits_for_selection_and_discards_superseded_reads() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::resolve(Some(temp.path().to_owned())).unwrap();
        let store = temp.path().join(".claude/projects/test");
        fs::create_dir_all(&store).unwrap();
        for name in ["first", "second"] {
            fs::write(
                store.join(format!("{name}.jsonl")),
                serde_json::json!({"type":"user", "message":{"content":name}}).to_string(),
            )
            .unwrap();
        }
        let mut app = App::new(&config, Scope::Global, temp.path().to_owned(), |_| {
            Ok("test")
        });
        let mut loader = PreviewLoader::default();
        let now = Instant::now();
        assert_eq!(loader.tick(&mut app, false, now), None);
        assert_eq!(loader.tick(&mut app, true, now), Some(PREVIEW_DELAY));
        app.handle(KeyCode::Down.into());
        let moved = now + PREVIEW_DELAY / 2;
        assert_eq!(loader.tick(&mut app, true, moved), Some(PREVIEW_DELAY));
        loader.tick(&mut app, true, now + PREVIEW_DELAY);
        assert!(loader.worker.is_none());

        let due = moved + PREVIEW_DELAY;
        loader.tick(&mut app, true, due);
        app.handle(KeyCode::Up.into());
        // Wait for disk I/O while retaining the completed result in the scheduler.
        let finish = || Instant::now() + Duration::from_secs(5);
        let timeout = finish();
        while !loader.worker.as_ref().unwrap().1.is_finished() {
            assert!(Instant::now() < timeout);
            thread::yield_now();
        }
        assert_eq!(loader.tick(&mut app, true, due), Some(PREVIEW_DELAY));
        assert!(app.loaded.is_none());
        let timeout = finish();
        while loader.tick(&mut app, true, due + PREVIEW_DELAY).is_some() {
            assert!(Instant::now() < timeout);
            thread::yield_now();
        }
        let selected = app.selected_session().unwrap();
        let loaded = app.loaded.as_ref().unwrap();
        assert_eq!(loaded.reference, selected.reference);
        assert_eq!(loaded.turns[0].text, selected.title.as_deref().unwrap());
    }
}
