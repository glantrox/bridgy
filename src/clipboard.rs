//! Clipboard through the platform's own tool. A helper process keeps X11 selections alive after
//! Bridgy exits, and a missing display (SSH, containers) fails cleanly instead of hanging.

use anyhow::{Context, Result, bail};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn candidates() -> Vec<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        return vec![("pbcopy", &[])];
    }
    if cfg!(windows) {
        return vec![("clip", &[])];
    }
    let mut tools = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        tools.push(("wl-copy", &[][..]));
    }
    if std::env::var_os("DISPLAY").is_some() {
        tools.push(("xclip", &["-selection", "clipboard", "-in"][..]));
        tools.push(("xsel", &["--clipboard", "--input"][..]));
    }
    tools
}

/// Returns the tool used. Fails when no clipboard is reachable from this session.
pub fn copy(text: &str) -> Result<&'static str> {
    let tools = candidates();
    if tools.is_empty() {
        bail!("no clipboard display in this session (common over SSH or in containers)");
    }
    let mut errors = Vec::new();
    for (tool, args) in tools {
        let mut child = match Command::new(tool)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                errors.push(format!("{tool}: {e}"));
                continue;
            }
        };
        let written = child
            .stdin
            .take()
            .context("clipboard stdin")?
            .write_all(text.as_bytes());
        let status = child.wait()?;
        match (written, status.success()) {
            (Ok(()), true) => return Ok(tool),
            (Err(e), _) => errors.push(format!("{tool}: {e}")),
            (_, false) => errors.push(format!("{tool} exited with {status}")),
        }
    }
    bail!("clipboard unavailable ({})", errors.join("; "))
}
