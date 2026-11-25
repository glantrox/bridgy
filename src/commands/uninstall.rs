use crate::{
    config::{self, Config},
    providers,
};
use anyhow::{Context, Result};
use std::{collections::HashSet, fs, io::Write};

pub fn run(config: &Config, out: &mut dyn Write) -> Result<()> {
    let _lock = config.lock()?;
    config.check_state_dir()?;
    let mut edits = Vec::new();
    let mut seen = HashSet::new();
    for provider in providers::registry() {
        let Some(path) = provider.instructions(config) else {
            continue;
        };
        if !seen.insert(config::target_path(&path)?) {
            continue;
        }
        if let Some(text) = config::read_optional(&path)? {
            let stripped = config::strip(&text)
                .with_context(|| format!("invalid markers in {}", path.display()))?;
            if stripped != text {
                edits.push((path, stripped));
            }
        }
    }
    for (path, text) in edits {
        config::write_atomic(&path, text.as_bytes(), true)?;
        writeln!(out, "Removed managed block: {}", path.display())?;
    }
    match fs::remove_file(config.protocol()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("remove central protocol"),
    }
    let state = config.state_dir();
    if state.try_exists()? {
        if fs::read_dir(&state)?.next().is_none() {
            fs::remove_dir(&state)?;
        } else {
            writeln!(
                out,
                "Retained {}: contains backups or other files",
                state.display()
            )?;
        }
    }
    writeln!(
        out,
        "Uninstalled protocol. Session logs, backup files, executable, and ~/.bridgy.lock retained."
    )?;
    Ok(())
}
