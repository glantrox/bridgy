use super::sync;
use crate::{
    config::Config,
    providers::{self, Provider},
};
use anyhow::{Result, ensure};
use std::{collections::HashSet, fs, io::Write};

pub fn run(
    config: &Config,
    requested: &[String],
    link_codex: bool,
    out: &mut dyn Write,
) -> Result<()> {
    #[cfg(not(unix))]
    ensure!(!link_codex, "--link-codex is supported only on Unix");
    let _lock = config.lock()?;
    config.check_state_dir()?;
    let mut selected: Vec<Box<dyn Provider>> = providers::registry()
        .into_iter()
        .filter(|p| p.detected(config) && p.instructions_name().is_some())
        .collect();
    let mut ids: HashSet<_> = selected.iter().map(|p| p.id()).collect();
    let requested = requested.iter().map(String::as_str).chain(if link_codex {
        vec!["claude", "codex"]
    } else {
        Vec::new()
    });
    for name in requested {
        let provider = providers::find(name)?;
        ensure!(
            provider.instructions_name().is_some(),
            "{name} has no verified receiver setup"
        );
        if ids.insert(provider.id()) {
            selected.push(provider);
        }
    }
    ensure!(
        !selected.is_empty(),
        "no providers detected; use --providers claude,codex to create their instruction files"
    );
    // Validate all instruction files and the central protocol before changing anything.
    let mut plan = sync::plan(config, &selected, false)?;
    let codex_path = config.home.join(".codex/AGENTS.md");
    let create_link = if link_codex {
        if codex_path.is_symlink() {
            ensure!(
                fs::canonicalize(&codex_path)?
                    == fs::canonicalize(config.home.join(".claude/CLAUDE.md"))?,
                "Codex instructions already link elsewhere; leaving them unchanged"
            );
            false
        } else {
            ensure!(
                !codex_path.try_exists()?,
                "--link-codex requires an absent Codex instruction file; existing rules will not be replaced"
            );
            plan.edits.retain(|(path, _)| path != &codex_path);
            true
        }
    } else {
        false
    };
    sync::apply(config, plan, out)?;
    if create_link {
        fs::create_dir_all(config.home.join(".codex"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink("../.claude/CLAUDE.md", &codex_path)?;
        writeln!(
            out,
            "Linked: {} -> ../.claude/CLAUDE.md",
            codex_path.display()
        )?;
    }
    Ok(())
}
