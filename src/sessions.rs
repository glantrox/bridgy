//! Session discovery and reading shared by the CLI and the TUI.
//!
//! `list` returns metadata only. `read` takes the exact session a caller chose, so a preview,
//! copy, or export of an older session never drifts to a newer one.

use crate::{
    config::Config,
    providers::{self, Header, Provider},
    tail::TailResult,
    workspace::{Matcher, Scope},
};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// A stable reference: the provider plus the session file it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub source: &'static str,
    pub provider: &'static str,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub reference: SessionRef,
    /// The provider's native session ID (file stem when the format has none).
    pub id: String,
    pub workspace: Option<PathBuf>,
    pub last_activity: SystemTime,
    pub title: Option<String>,
    /// Client that wrote the session, such as `terminal` or `vscode`, when recorded.
    pub surface: Option<String>,
}

/// What a provider's stores held, so callers can tell empty, unreadable, and unsupported apart.
#[derive(Debug, Clone)]
pub struct StoreReport {
    pub provider: &'static str,
    pub roots: Vec<PathBuf>,
    pub missing: bool,
    /// The store itself could not be read; the provider contributes no sessions.
    pub error: Option<String>,
    /// Recognized sessions across every workspace.
    pub total: usize,
    pub matched: usize,
    pub without_workspace: usize,
    pub unsupported: usize,
    pub unreadable: Vec<String>,
}

pub enum StoreState {
    Ok,
    Missing,
    Unreadable,
    Unsupported,
    Empty,
    NoneHere,
}

impl StoreReport {
    fn new(provider: &'static str) -> Self {
        Self {
            provider,
            roots: Vec::new(),
            missing: true,
            error: None,
            total: 0,
            matched: 0,
            without_workspace: 0,
            unsupported: 0,
            unreadable: Vec::new(),
        }
    }

    pub fn state(&self) -> StoreState {
        if self.matched > 0 {
            StoreState::Ok
        } else if self.error.is_some() || (self.total == 0 && !self.unreadable.is_empty()) {
            StoreState::Unreadable
        } else if self.missing {
            StoreState::Missing
        } else if self.total == 0 && self.unsupported > 0 {
            StoreState::Unsupported
        } else if self.total == 0 {
            StoreState::Empty
        } else {
            StoreState::NoneHere
        }
    }

    fn roots_label(&self) -> String {
        self.roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// One line, phrased for both a terminal and an agent reading `tail` errors.
    pub fn describe(&self, scope: &Scope) -> String {
        let p = self.provider;
        let mut line = match self.state() {
            StoreState::Ok => format!("{p}: {} session(s) in {scope}", self.matched),
            StoreState::Missing => format!(
                "{p}: no session store at {} (not installed here, or never used)",
                self.roots_label()
            ),
            StoreState::Unreadable => format!(
                "{p}: session store unreadable: {}",
                self.error
                    .clone()
                    .or_else(|| self.unreadable.first().cloned())
                    .unwrap_or_default()
            ),
            StoreState::Unsupported => format!(
                "{p}: {} session file(s) in {} use an unsupported format",
                self.unsupported,
                self.roots_label()
            ),
            StoreState::Empty => format!("{p}: session store {} is empty", self.roots_label()),
            StoreState::NoneHere => {
                let mut text = format!(
                    "{p}: no history for this checkout ({scope}); {} session(s) belong to other workspaces",
                    self.total
                );
                if self.without_workspace > 0 {
                    text.push_str(&format!(
                        ", {} have no workspace metadata",
                        self.without_workspace
                    ));
                }
                text.push_str("; use --global or --workspace <path> to look elsewhere");
                text
            }
        };
        let skipped = self.unsupported * usize::from(self.total > 0);
        if skipped > 0 {
            line.push_str(&format!(" ({skipped} unsupported file(s) skipped)"));
        }
        if !self.unreadable.is_empty() && self.total > 0 {
            line.push_str(&format!(
                " ({} unreadable file(s) skipped)",
                self.unreadable.len()
            ));
        }
        line
    }
}

pub struct Listing {
    pub scope: Scope,
    /// Newest first; ties break by path so selection is repeatable.
    pub sessions: Vec<SessionMeta>,
    pub reports: Vec<StoreReport>,
}

pub struct ListOptions<'a> {
    pub provider: Option<&'a dyn Provider>,
    /// Resolve titles for matched sessions (the TUI wants them; `tail` does not).
    pub titles: bool,
}

/// One provider's failure is recorded in its report and never hides other providers.
pub fn list(config: &Config, scope: &Scope, options: ListOptions) -> Listing {
    let registry;
    let providers: Vec<&dyn Provider> = match options.provider {
        Some(provider) => vec![provider],
        None => {
            registry = providers::registry();
            registry.iter().map(AsRef::as_ref).collect()
        }
    };
    let mut matcher = Matcher::new(scope);
    let mut sessions = Vec::new();
    let mut reports = Vec::new();
    for provider in providers {
        let mut report = StoreReport::new(provider.id());
        for source in provider.sources() {
            let root = source.root(config);
            report.roots.push(root.clone());
            let walk = match providers::walk(&root, |p| source.accepts(p)) {
                Ok(walk) => walk,
                Err(e) => {
                    report.missing = false;
                    report.error = Some(format!("{e:#}"));
                    continue;
                }
            };
            report.missing &= walk.missing;
            report.unreadable.extend(walk.warnings);
            for file in walk.files {
                let header: Header = match source.describe(config, &file.path) {
                    Ok(Some(header)) => header,
                    Ok(None) => {
                        report.unsupported += 1;
                        continue;
                    }
                    Err(e) => {
                        report.unreadable.push(format!("{e:#}"));
                        continue;
                    }
                };
                report.total += 1;
                if header.workspace.is_none() {
                    report.without_workspace += 1;
                }
                if !matcher.matches(header.workspace.as_deref()) {
                    continue;
                }
                report.matched += 1;
                let title = if options.titles {
                    source
                        .title(config, &file.path, &header)
                        .or(header.title.clone())
                } else {
                    header.title.clone()
                };
                sessions.push(SessionMeta {
                    id: header.id.clone().unwrap_or_else(|| {
                        file.path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    }),
                    reference: SessionRef {
                        source: source.id(),
                        provider: provider.id(),
                        path: file.path,
                    },
                    workspace: header.workspace,
                    last_activity: file.modified,
                    title,
                    surface: header.surface,
                });
            }
        }
        reports.push(report);
    }
    sessions.sort_by(|a, b| {
        (b.last_activity, &b.reference.path).cmp(&(a.last_activity, &a.reference.path))
    });
    Listing {
        scope: scope.clone(),
        sessions,
        reports,
    }
}

/// `tail`'s selection: the newest session for one provider in the scope, or why there is none.
pub fn newest(config: &Config, scope: &Scope, provider: &dyn Provider) -> Result<SessionMeta> {
    let listing = list(
        config,
        scope,
        ListOptions {
            provider: Some(provider),
            titles: false,
        },
    );
    match listing.sessions.into_iter().next() {
        Some(mut session) => {
            let path = &session.reference.path;
            let header = Header {
                id: Some(session.id.clone()),
                workspace: session.workspace.clone(),
                title: session.title.clone(),
                surface: session.surface.clone(),
            };
            if let Some(title) = provider
                .source_for(config, path)
                .title(config, path, &header)
            {
                session.title = Some(title);
            }
            Ok(session)
        }
        None => bail!(
            "{}",
            listing
                .reports
                .first()
                .map(|r| r.describe(scope))
                .unwrap_or_default()
        ),
    }
}

/// Metadata for an explicit `--file`, which may live outside the provider's store.
pub fn describe_file(config: &Config, provider: &dyn Provider, path: &Path) -> SessionMeta {
    let source = provider.source_for(config, path);
    let header = source
        .describe(config, path)
        .ok()
        .flatten()
        .unwrap_or_default();
    SessionMeta {
        reference: SessionRef {
            source: source.id(),
            provider: provider.id(),
            path: path.to_owned(),
        },
        id: header.id.clone().unwrap_or_default(),
        title: source.title(config, path, &header).or(header.title),
        workspace: header.workspace,
        last_activity: std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH),
        surface: header.surface,
    }
}

/// Read the exact referenced session; never re-resolves "newest".
pub fn read(
    _config: &Config,
    session: &SessionRef,
    count: usize,
    max_bytes: u64,
) -> Result<TailResult> {
    let provider = providers::find(session.provider)?;
    provider
        .sources()
        .into_iter()
        .find(|src| src.id() == session.source)
        .ok_or_else(|| anyhow::anyhow!("unknown source {}", session.source))?
        .read(&session.path, count, max_bytes)
}
