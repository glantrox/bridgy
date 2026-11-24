use crate::{
    config::{self, Config},
    protocol,
    providers::{self, Provider},
};
use anyhow::{Context, Result, ensure};
use std::{collections::HashSet, io::Write, path::PathBuf};

pub struct Plan {
    protocol: String,
    pub(crate) edits: Vec<(PathBuf, String)>,
}

pub fn plan(config: &Config, selected: &[Box<dyn Provider>], regenerate: bool) -> Result<Plan> {
    let existing = config::read_optional(&config.protocol())?;
    let protocol = if regenerate {
        protocol::generate(selected)
    } else {
        existing.unwrap_or_else(|| protocol::generate(selected))
    };
    let block = config::render_block(&protocol)?;
    let mut edits = Vec::new();
    let mut seen = HashSet::new();
    for provider in selected {
        let path = provider.instructions(config);
        let target = config::target_path(&path)?;
        if !seen.insert(target.clone()) {
            continue;
        }
        let original = config::read_optional(&path)?.unwrap_or_default();
        let updated = config::inject(&original, &block)
            .with_context(|| format!("invalid markers in {}", path.display()))?;
        edits.push((path, updated));
    }
    Ok(Plan { protocol, edits })
}

pub fn apply(config: &Config, plan: Plan, out: &mut dyn Write) -> Result<()> {
    config::write_atomic(&config.protocol(), plan.protocol.as_bytes(), true)?;
    for (path, text) in plan.edits {
        let changed = config::write_atomic(&path, text.as_bytes(), true)?;
        writeln!(
            out,
            "{}: {}",
            if changed { "Updated" } else { "Unchanged" },
            path.display()
        )?;
    }
    writeln!(out, "Protocol: {}", config.protocol().display())?;
    Ok(())
}

pub fn run(config: &Config, regenerate: bool, out: &mut dyn Write) -> Result<()> {
    let _lock = config.lock()?;
    config.check_state_dir()?;
    ensure!(
        config.protocol().try_exists()?,
        "Bridgy is not installed; run bridgy install first"
    );
    let selected = providers::registry()
        .into_iter()
        .filter(|p| p.detected(config))
        .collect::<Vec<_>>();
    ensure!(!selected.is_empty(), "no providers detected");
    apply(config, plan(config, &selected, regenerate)?, out)
}
