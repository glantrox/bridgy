use super::sync;
use crate::{
    config::{Config, write_atomic},
    providers::{self, Provider},
};
use anyhow::{Context, Result, ensure};
use std::{collections::HashSet, fs, io::Write, path::Path};

pub fn run(
    config: &Config,
    requested: &[String],
    bin_dir: Option<&Path>,
    link_codex: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let _lock = config.lock()?;
    config.check_state_dir()?;
    let mut selected: Vec<Box<dyn Provider>> = providers::registry()
        .into_iter()
        .filter(|p| p.detected(config))
        .collect();
    let mut ids: HashSet<_> = selected.iter().map(|p| p.id()).collect();
    let requested = requested.iter().map(String::as_str).chain(if link_codex {
        vec!["claude", "codex"]
    } else {
        Vec::new()
    });
    for name in requested {
        let provider = providers::find(name)?;
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
        #[cfg(not(unix))]
        anyhow::bail!("--link-codex is supported only on Unix");
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
    if let Some(dir) = bin_dir {
        let current = std::env::current_exe().context("locate running executable")?;
        let target = dir.join("bridgy");
        if fs::canonicalize(&target).ok().as_ref() != Some(&current) {
            write_atomic(&target, &fs::read(&current)?, true)?;
            fs::set_permissions(&target, fs::metadata(&current)?.permissions())?;
        }
        writeln!(
            out,
            "Executable: {} (add this directory to PATH)",
            target.display()
        )?;
    }
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
