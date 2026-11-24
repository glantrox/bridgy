use crate::{
    config::{self, Config},
    providers,
};
use anyhow::Result;
use serde::Serialize;
use std::{io::Write, path::PathBuf};

#[derive(Serialize)]
pub struct Status {
    pub provider: String,
    pub detected: bool,
    pub instructions: PathBuf,
    pub sync: String,
    pub newest_session: Option<PathBuf>,
    pub session_files: u64,
    pub session_bytes: u64,
    pub warnings: Vec<String>,
}

pub fn collect(config: &Config) -> Result<Vec<Status>> {
    config.check_state_dir()?;
    let protocol = config::read_optional(&config.protocol())?;
    let expected = protocol.as_deref().map(config::render_block).transpose()?;
    let mut statuses = Vec::new();
    for provider in providers::registry() {
        let mut warnings = Vec::new();
        let instructions = provider.instructions(config);
        let sync = match config::read_optional(&instructions) {
            Ok(Some(text)) => match config::block_span(&text) {
                Ok(Some(span)) => {
                    if expected.as_deref() == Some(&text[span]) {
                        "synced"
                    } else {
                        "out-of-sync"
                    }
                }
                Ok(None) => "unmanaged",
                Err(e) => {
                    warnings.push(e.to_string());
                    "invalid-markers"
                }
            },
            Ok(None) => "missing",
            Err(e) => {
                warnings.push(format!("{e:#}"));
                "unreadable"
            }
        }
        .to_owned();
        let inventory = match providers::inventory(provider.as_ref(), config) {
            Ok(inventory) => inventory,
            Err(e) => {
                warnings.push(format!("{e:#}"));
                Default::default()
            }
        };
        warnings.extend(inventory.warnings);
        statuses.push(Status {
            provider: provider.id().into(),
            detected: provider.detected(config),
            instructions,
            sync,
            newest_session: inventory.newest,
            session_files: inventory.files,
            session_bytes: inventory.bytes,
            warnings,
        });
    }
    Ok(statuses)
}

pub fn run(config: &Config, json: bool, out: &mut dyn Write) -> Result<()> {
    let statuses = collect(config)?;
    if json {
        serde_json::to_writer_pretty(&mut *out, &statuses)?;
        writeln!(out)?;
    } else {
        for status in statuses {
            writeln!(
                out,
                "{}: {} | {} | {} sessions, {} bytes",
                status.provider,
                if status.detected {
                    "detected"
                } else {
                    "not detected"
                },
                status.sync,
                status.session_files,
                status.session_bytes
            )?;
            if let Some(path) = status.newest_session {
                writeln!(out, "  Newest: {}", path.display())?;
            }
            for warning in status.warnings {
                writeln!(out, "  Warning: {warning}")?;
            }
        }
    }
    Ok(())
}
