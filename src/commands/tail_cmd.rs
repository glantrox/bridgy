use crate::{
    config::Config,
    providers::{self, Turn},
    sessions,
    transcript::{self, Limits},
    workspace::Scope,
};
use anyhow::Result;
use serde::Serialize;
use std::{io::Write, path::Path};

#[derive(Serialize)]
struct Output<'a> {
    provider: &'a str,
    session: &'a Path,
    workspace: Option<&'a Path>,
    title: Option<&'a str>,
    surface: Option<&'a str>,
    turns: &'a [Turn],
    omitted: usize,
    bytes_read: u64,
    warnings: &'a [String],
}

pub struct Options<'a> {
    pub provider: &'a str,
    pub limits: Limits,
    pub json: bool,
    pub file: Option<&'a Path>,
    pub scope: Scope,
}

pub fn run(config: &Config, options: Options, out: &mut dyn Write) -> Result<()> {
    let provider = providers::find(options.provider)?;
    let session = match options.file {
        Some(file) => sessions::describe_file(config, provider.as_ref(), file),
        None => sessions::newest(config, &options.scope, provider.as_ref())?,
    };
    let transcript = transcript::load(config, &session, options.limits)?;
    for warning in &transcript.warnings {
        eprintln!("bridgy: warning: {warning}");
    }
    if transcript.turns.is_empty() {
        eprintln!(
            "bridgy: no conversational text found in {}",
            session.reference.path.display()
        );
    }
    if options.json {
        serde_json::to_writer_pretty(
            &mut *out,
            &Output {
                provider: provider.id(),
                session: &session.reference.path,
                workspace: session.workspace.as_deref(),
                title: session.title.as_deref(),
                surface: session.surface.as_deref(),
                turns: &transcript.turns,
                omitted: transcript.omitted,
                bytes_read: transcript.bytes_read,
                warnings: &transcript.warnings,
            },
        )?;
        writeln!(out)?;
    } else {
        out.write_all(transcript.markdown().as_bytes())?;
    }
    Ok(())
}
