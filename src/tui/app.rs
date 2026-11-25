//! TUI state and key handling, free of terminal I/O so tests can drive it directly.

use crate::{
    commands::{doctor, status},
    config::{self, Config},
    sessions::{self, ListOptions, Listing, SessionMeta, SessionRef, StoreState},
    transcript::{self, Limits},
    workspace::{self, Scope},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Sessions,
    Preview,
    Diagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    Import {
        input: String,
    },
    /// Editing the export path.
    Export {
        input: String,
    },
    /// The path exists; waiting for y/n.
    Overwrite {
        input: String,
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Ok,
    Warn,
    Error,
    Heading,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub tone: Tone,
    pub text: String,
}

/// The rendered transcript of one exact session; preview, copy, and export all use `text`.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub reference: SessionRef,
    pub title: String,
    pub text: String,
    pub messages: usize,
}

pub type Copier = fn(&str) -> anyhow::Result<&'static str>;

pub struct App<'a> {
    config: &'a Config,
    /// The checkout Bridgy was opened in; `g` toggles between it and all workspaces.
    pub home_scope: Scope,
    pub listing: Listing,
    pub selected: usize,
    pub view: View,
    pub loaded: Option<Loaded>,
    pub scroll: u16,
    /// Largest useful scroll offset for the current view, set while drawing.
    pub max_scroll: u16,
    pub page: u16,
    pub status: Option<Line>,
    pub prompt: Option<Prompt>,
    pub diagnostics: Vec<Line>,
    pub limits: Limits,
    pub cwd: PathBuf,
    pub quit: bool,
    copier: Copier,
    pub pending_import: Option<crate::imports::Bookmark>,
}

impl<'a> App<'a> {
    pub fn new(config: &'a Config, scope: Scope, cwd: PathBuf, copier: Copier) -> Self {
        let mut app = Self {
            config,
            listing: list(config, &scope),
            home_scope: if scope == Scope::Global {
                Scope::Checkout(workspace::checkout_root(&cwd))
            } else {
                scope
            },
            selected: 0,
            view: View::Sessions,
            loaded: None,
            scroll: 0,
            max_scroll: u16::MAX,
            page: 10,
            status: None,
            prompt: None,
            diagnostics: Vec::new(),
            limits: Limits::default(),
            cwd,
            quit: false,
            copier,
            pending_import: None,
        };
        app.flag_broken_stores();
        app
    }

    pub fn home(&self) -> &Path {
        &self.config.home
    }

    pub fn selected_session(&self) -> Option<&SessionMeta> {
        self.listing.sessions.get(self.selected)
    }

    fn flag_broken_stores(&mut self) {
        let broken: Vec<_> = self
            .listing
            .reports
            .iter()
            .filter(|r| matches!(r.state(), StoreState::Unreadable | StoreState::Unsupported))
            .map(|r| r.provider)
            .collect();
        if !broken.is_empty() {
            self.set(
                Tone::Warn,
                format!(
                    "Could not read {} history; press d for diagnostics",
                    broken.join(", ")
                ),
            );
        }
    }

    fn set(&mut self, tone: Tone, text: impl Into<String>) {
        self.status = Some(Line {
            tone,
            text: text.into(),
        });
    }

    fn reload(&mut self, scope: Scope) {
        let keep = self.selected_session().map(|s| s.reference.clone());
        self.listing = list(self.config, &scope);
        self.selected = keep
            .and_then(|r| self.listing.sessions.iter().position(|s| s.reference == r))
            .unwrap_or(0);
        self.loaded = None;
        self.status = None;
        self.flag_broken_stores();
    }

    /// Load the selected session once; later copy/export reuse the exact previewed text.
    fn ensure_loaded(&mut self) -> bool {
        if self.pending_import.is_some() && self.view == View::Preview {
            return true;
        }
        let Some(session) = self.selected_session().cloned() else {
            self.set(Tone::Warn, "No session selected");
            return false;
        };
        if self
            .loaded
            .as_ref()
            .is_some_and(|l| l.reference == session.reference)
        {
            return true;
        }
        match transcript::load(self.config, &session, self.limits) {
            Ok(t) => {
                for warning in &t.warnings {
                    self.set(Tone::Warn, warning.clone());
                }
                self.loaded = Some(Loaded {
                    title: session.title.clone().unwrap_or_else(|| session.id.clone()),
                    reference: session.reference,
                    messages: t.turns.len(),
                    text: t.markdown(),
                });
                true
            }
            Err(e) => {
                self.set(Tone::Error, format!("Could not read session: {e:#}"));
                false
            }
        }
    }

    pub fn handle(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.prompt.is_some() {
            self.handle_prompt(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => match self.view {
                View::Sessions => self.quit = true,
                _ => self.back(),
            },
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageUp => self.step(-i32::from(self.page)),
            KeyCode::PageDown | KeyCode::Char(' ') => self.step(i32::from(self.page)),
            KeyCode::Home if self.view != View::Sessions => self.scroll = 0,
            KeyCode::End if self.view != View::Sessions => self.scroll = self.max_scroll,
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.listing.sessions.len().saturating_sub(1),
            KeyCode::Enter if self.view == View::Sessions => self.open_preview(),
            KeyCode::Char('i') if self.view == View::Sessions => {
                self.prompt = Some(Prompt::Import {
                    input: String::new(),
                })
            }
            KeyCode::Char(k @ ('s' | 'a')) if self.pending_import.is_some() => {
                let scope = self.home_scope.root();
                let result = self
                    .pending_import
                    .as_ref()
                    .unwrap()
                    .save(self.config, if k == 'a' { scope } else { None });
                match result {
                    Ok(()) => {
                        self.back();
                        self.reload(self.listing.scope.clone());
                        self.set(Tone::Ok, "Import saved");
                    }
                    Err(e) => self.set(Tone::Error, format!("{e:#}")),
                }
            }
            KeyCode::Char('c') if self.view != View::Diagnostics => self.copy(),
            KeyCode::Char('e') if self.view != View::Diagnostics => self.start_export(),
            KeyCode::Char('d')
                if self.pending_import.is_none() && self.view != View::Diagnostics =>
            {
                self.open_diagnostics()
            }
            KeyCode::Char('g') if self.view == View::Sessions => self.toggle_scope(),
            KeyCode::Char('r') if self.view == View::Sessions => {
                self.reload(self.listing.scope.clone())
            }
            _ => {}
        }
    }

    fn back(&mut self) {
        if self.pending_import.take().is_some() {
            self.loaded = None;
        }
        self.view = View::Sessions;
        self.scroll = 0;
    }

    fn step(&mut self, delta: i32) {
        if self.view == View::Sessions {
            let last = self.listing.sessions.len().saturating_sub(1) as i64;
            self.selected = (self.selected as i64 + i64::from(delta)).clamp(0, last) as usize;
        } else {
            let next =
                (i64::from(self.scroll) + i64::from(delta)).clamp(0, i64::from(self.max_scroll));
            self.scroll = next as u16;
        }
    }

    fn toggle_scope(&mut self) {
        let next = match self.listing.scope {
            Scope::Global => self.home_scope.clone(),
            Scope::Checkout(_) => Scope::Global,
        };
        self.reload(next);
    }

    pub fn open_preview(&mut self) {
        if self.ensure_loaded() {
            self.view = View::Preview;
            self.scroll = 0;
            self.max_scroll = u16::MAX;
        }
    }

    pub fn copy(&mut self) {
        if !self.ensure_loaded() {
            return;
        }
        let loaded = self.loaded.as_ref().expect("loaded");
        let (messages, bytes) = (loaded.messages, loaded.text.len());
        match (self.copier)(&loaded.text) {
            Ok(tool) => self.set(
                Tone::Ok,
                format!(
                    "Copied {messages} message(s), {} to the clipboard ({tool})",
                    size(bytes)
                ),
            ),
            Err(e) => self.set(
                Tone::Error,
                format!("{e:#}. Press e to export Markdown instead"),
            ),
        }
    }

    pub fn start_export(&mut self) {
        if !self.ensure_loaded() {
            return;
        }
        let loaded = self.loaded.as_ref().expect("loaded");
        let id: String = Path::new(&loaded.reference.path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .rev()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let name = format!("bridgy-{}-{id}.md", loaded.reference.provider);
        self.prompt = Some(Prompt::Export {
            input: display_path(&self.cwd.join(name), &self.config.home),
        });
    }

    fn handle_prompt(&mut self, key: KeyEvent) {
        match self.prompt.take().expect("prompt") {
            Prompt::Import { mut input } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let path = expand(input.trim(), &self.cwd, &self.config.home);
                    match crate::imports::preview(&path, self.limits) {
                        Ok((b, t)) => {
                            self.loaded = Some(Loaded {
                                reference: t.session.reference.clone(),
                                title: t.session.title.clone().unwrap_or_default(),
                                text: t.markdown(),
                                messages: t.turns.len(),
                            });
                            self.pending_import = Some(b);
                            self.view = View::Preview;
                            self.scroll = 0;
                            self.status = None;
                        }
                        Err(e) => self.set(Tone::Error, format!("Import: {e:#}")),
                    }
                }
                code => {
                    match code {
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => input.push(c),
                        _ => {}
                    }
                    self.prompt = Some(Prompt::Import { input });
                }
            },
            Prompt::Export { mut input } => match key.code {
                KeyCode::Esc => self.set(Tone::Info, "Export cancelled"),
                KeyCode::Enter => self.submit_export(input),
                KeyCode::Backspace => {
                    input.pop();
                    self.prompt = Some(Prompt::Export { input });
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.prompt = Some(Prompt::Export {
                        input: String::new(),
                    })
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    self.prompt = Some(Prompt::Export { input });
                }
                _ => self.prompt = Some(Prompt::Export { input }),
            },
            Prompt::Overwrite { input, path } => match key.code {
                KeyCode::Char('y' | 'Y') => self.write_export(&path, true),
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.prompt = Some(Prompt::Export { input })
                }
                _ => self.prompt = Some(Prompt::Overwrite { input, path }),
            },
        }
    }

    fn submit_export(&mut self, input: String) {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.set(Tone::Error, "Export path is empty");
            self.prompt = Some(Prompt::Export { input });
            return;
        }
        let path = expand(trimmed, &self.cwd, &self.config.home);
        let dest = path.canonicalize().ok();
        if let Some(loaded) = &self.loaded
            && let Some(dest) = dest.as_ref()
        {
            let origin = &loaded.reference.path;
            if origin.canonicalize().as_ref().ok() == Some(dest)
                || (loaded.reference.source == "bookmark-v1"
                    && crate::imports::original(origin).as_ref() == Some(dest))
            {
                self.set(
                    Tone::Error,
                    "Choose an export path different from the source",
                );
                return;
            }
        }
        if path.is_dir() {
            self.set(Tone::Error, format!("{} is a directory", path.display()));
            self.prompt = Some(Prompt::Export { input });
        } else if path.symlink_metadata().is_ok() {
            self.prompt = Some(Prompt::Overwrite { input, path });
        } else {
            self.write_export(&path, false);
        }
    }

    fn write_export(&mut self, path: &Path, overwrite: bool) {
        let text = self.loaded.as_ref().expect("loaded").text.clone();
        let result = if overwrite {
            config::write_atomic(path, text.as_bytes(), false).map(|_| ())
        } else {
            // create_new: never replace a file that appeared after the existence check.
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .and_then(|mut f| f.write_all(text.as_bytes()))
                .map_err(|e| match e.kind() {
                    ErrorKind::AlreadyExists => anyhow::anyhow!("file appeared meanwhile; retry"),
                    _ => e.into(),
                })
        };
        match result {
            Ok(()) => self.set(Tone::Ok, format!("Saved Markdown to {}", path.display())),
            Err(e) => self.set(
                Tone::Error,
                format!("Could not write {}: {e:#}", path.display()),
            ),
        }
    }

    pub fn open_diagnostics(&mut self) {
        self.diagnostics = diagnostics(self.config, &self.listing);
        self.view = View::Diagnostics;
        self.scroll = 0;
        self.max_scroll = u16::MAX;
    }
}

fn list(config: &Config, scope: &Scope) -> Listing {
    sessions::list(
        config,
        scope,
        ListOptions {
            provider: None,
            titles: true,
        },
    )
}

pub fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn expand(input: &str, cwd: &Path, home: &Path) -> PathBuf {
    if input == "~" {
        home.to_owned()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else {
        cwd.join(input)
    }
}

/// Workspace, per-store results, instruction targets, and the `bridgy doctor` checks.
pub fn diagnostics(config: &Config, listing: &Listing) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut push = |tone, text: String| lines.push(Line { tone, text });
    let home = &config.home;
    push(Tone::Heading, "Workspace".into());
    match &listing.scope {
        Scope::Checkout(root) => push(
            Tone::Info,
            format!(
                "  {}  ({})",
                display_path(root, home),
                if workspace::is_git_checkout(root) {
                    "Git checkout"
                } else {
                    "directory; not inside a Git checkout"
                }
            ),
        ),
        Scope::Global => push(Tone::Info, "  All workspaces (press g to return)".into()),
    }
    push(Tone::Heading, String::new());
    push(Tone::Heading, "Session stores".into());
    for report in &listing.reports {
        let tone = match report.state() {
            StoreState::Ok => Tone::Ok,
            StoreState::Missing | StoreState::Empty | StoreState::NoneHere => Tone::Info,
            StoreState::Unsupported => Tone::Warn,
            StoreState::Unreadable => Tone::Error,
        };
        let roots: Vec<_> = report.roots.iter().map(|r| display_path(r, home)).collect();
        push(tone, format!("  {}", report.describe(&listing.scope)));
        push(Tone::Info, format!("      log root: {}", roots.join(", ")));
        for problem in report.unreadable.iter().take(3) {
            push(Tone::Warn, format!("      {problem}"));
        }
    }
    push(Tone::Heading, String::new());
    push(Tone::Heading, "Instruction targets".into());
    match status::collect(config) {
        Ok(entries) => {
            for entry in entries {
                let Some(instructions) = &entry.instructions else {
                    continue;
                };
                let tone = match entry.sync.as_str() {
                    "synced" => Tone::Ok,
                    _ if !entry.detected => Tone::Info,
                    _ => Tone::Warn,
                };
                push(
                    tone,
                    format!(
                        "  {:<7} {}  {}",
                        entry.provider,
                        display_path(instructions, home),
                        if entry.detected {
                            entry.sync
                        } else {
                            "not detected".into()
                        }
                    ),
                );
            }
        }
        Err(e) => push(Tone::Error, format!("  {e:#}")),
    }
    push(Tone::Heading, String::new());
    push(Tone::Heading, "Doctor checks".into());
    match doctor::checks(config) {
        Ok(checks) => {
            for check in checks {
                let tone = match check.level {
                    doctor::Level::Ok => Tone::Ok,
                    doctor::Level::Warn => Tone::Warn,
                    doctor::Level::Fail => Tone::Error,
                    doctor::Level::Skip => Tone::Info,
                };
                push(tone, format!("  {}", check.line));
            }
        }
        Err(e) => push(Tone::Error, format!("  {e:#}")),
    }
    lines
}
