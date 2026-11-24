use super::status;
use crate::{
    config::{self, Config},
    providers,
    sessions::{self, SessionRef},
};
use anyhow::{Result, ensure};
use std::{fs, io::Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub level: Level,
    /// The full line `bridgy doctor` prints.
    pub line: String,
}

/// The checks behind `bridgy doctor`, also shown in the TUI's diagnostics view.
pub fn checks(config: &Config) -> Result<Vec<Check>> {
    let mut checks = Vec::new();
    let mut push = |level, line: String| checks.push(Check { level, line });
    if config::read_optional(&config.protocol())?.is_none() {
        push(
            Level::Fail,
            "FAIL: central protocol missing; run bridgy install".into(),
        );
    }
    for entry in status::collect(config)? {
        if !entry.detected {
            push(
                Level::Skip,
                format!("SKIP {}: not detected", entry.provider),
            );
            continue;
        }
        if entry.sync != "synced" {
            push(
                Level::Fail,
                format!(
                    "FAIL {}: instructions {}; run install/sync or repair markers",
                    entry.provider, entry.sync
                ),
            );
        } else {
            push(Level::Ok, format!("OK {}: protocol synced", entry.provider));
        }
        for warning in entry.warnings {
            push(Level::Fail, format!("FAIL {}: {warning}", entry.provider));
        }
        let target = config::target_path(&entry.instructions)?;
        for path in [Some(target.as_path()), target.parent()]
            .into_iter()
            .flatten()
        {
            if let Ok(metadata) = fs::metadata(path)
                && metadata.permissions().readonly()
            {
                push(
                    Level::Fail,
                    format!("FAIL: {} has read-only permissions", path.display()),
                );
            }
        }
        if let Some(path) = entry.newest_session {
            let provider = providers::find(&entry.provider)?;
            let session = SessionRef {
                provider: provider.id(),
                path,
            };
            match sessions::read(config, &session, 1, 64 * 1024 * 1024) {
                Ok(result) => {
                    push(
                        Level::Ok,
                        format!(
                            "OK {}: latest session readable ({} text message)",
                            entry.provider,
                            result.turns.len()
                        ),
                    );
                    for warning in result.warnings {
                        push(Level::Warn, format!("WARN {}: {warning}", entry.provider));
                    }
                }
                Err(e) => push(Level::Fail, format!("FAIL {}: {e:#}", entry.provider)),
            }
        } else {
            push(
                Level::Warn,
                format!("WARN {}: no session logs yet", entry.provider),
            );
        }
    }
    Ok(checks)
}

pub fn run(config: &Config, out: &mut dyn Write) -> Result<()> {
    let checks = checks(config)?;
    for check in &checks {
        writeln!(out, "{}", check.line)?;
    }
    let failures = checks.iter().filter(|c| c.level == Level::Fail).count();
    ensure!(failures == 0, "doctor found {failures} problem(s)");
    Ok(())
}
