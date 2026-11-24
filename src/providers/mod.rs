mod claude;
mod codex;
mod gemini;

use crate::{config::Config, tail::TailResult};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    pub role: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// One agent product: its names and the instruction file Bridgy manages.
pub trait Provider {
    fn id(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str];
    fn directory(&self) -> &'static str;
    fn instructions_name(&self) -> &'static str;
    /// History stores. Terminal and editor clients share a source when their logs are compatible.
    fn sources(&self) -> Vec<Box<dyn SessionSource>>;
    fn instructions(&self, config: &Config) -> PathBuf {
        config
            .home
            .join(self.directory())
            .join(self.instructions_name())
    }
    fn detected(&self, config: &Config) -> bool {
        config.home.join(self.directory()).is_dir()
    }
    /// The source owning `path`, or the primary source for an explicit `--file` elsewhere.
    fn source_for(&self, config: &Config, path: &Path) -> Box<dyn SessionSource> {
        let mut sources = self.sources();
        let index = sources
            .iter()
            .position(|s| path.starts_with(s.root(config)) && s.accepts(path))
            .unwrap_or(0);
        sources.swap_remove(index)
    }
}

/// Cheap metadata from a session file, read without loading the transcript.
#[derive(Debug, Default, Clone)]
pub struct Header {
    pub id: Option<String>,
    pub workspace: Option<PathBuf>,
    pub title: Option<String>,
    pub surface: Option<String>,
}

/// One on-disk history store and its format-specific parsing.
pub trait SessionSource {
    fn root(&self, config: &Config) -> PathBuf;
    fn accepts(&self, path: &Path) -> bool;
    /// `Ok(None)` means the file is not in a format this source understands.
    fn describe(&self, config: &Config, path: &Path) -> Result<Option<Header>>;
    /// A slower title lookup, run only for sessions that survive workspace filtering.
    fn title(&self, _config: &Config, _path: &Path, _header: &Header) -> Option<String> {
        None
    }
    fn read(&self, path: &Path, count: usize, max_bytes: u64) -> Result<TailResult>;
}

pub fn registry() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude::Claude),
        Box::new(codex::Codex),
        Box::new(gemini::Gemini),
    ]
}

pub fn find(name: &str) -> Result<Box<dyn Provider>> {
    registry()
        .into_iter()
        .find(|p| {
            p.id().eq_ignore_ascii_case(name)
                || p.aliases().iter().any(|a| a.eq_ignore_ascii_case(name))
        })
        .with_context(|| format!("unknown provider '{name}'; choose claude, codex, or gemini"))
}

/// Only textual content parts are surfaced; tool results, thinking, images and metadata are excluded.
pub fn text_content(content: &Value) -> String {
    fn part_text(part: &Value) -> Option<&str> {
        if let Some(text) = part.as_str() {
            return Some(text);
        }
        let kind = part.get("type").and_then(Value::as_str);
        if matches!(kind, None | Some("text" | "input_text" | "output_text"))
            && part.get("thought") != Some(&Value::Bool(true))
        {
            part.get("text").and_then(Value::as_str)
        } else {
            None
        }
    }
    match content {
        Value::Array(parts) => parts
            .iter()
            .filter_map(part_text)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => part_text(content).unwrap_or_default().to_owned(),
    }
}

pub fn turn(
    role: &str,
    content: &Value,
    timestamp: Option<&Value>,
    id: Option<&Value>,
) -> Option<Turn> {
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    let text = text_content(content);
    if text.trim().is_empty() {
        return None;
    }
    Some(Turn {
        role: role.into(),
        text,
        timestamp: timestamp.and_then(Value::as_str).map(str::to_owned),
        id: id.and_then(Value::as_str).map(str::to_owned),
    })
}

/// A one-line title from the first user prompt, for stores without saved titles.
/// Tag-only lines such as `<pasted_content …>` are wrappers, not the prompt.
pub fn prompt_title(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !(l.starts_with('<') && l.ends_with('>')))?;
    let line = line.trim_start_matches(['#', '*', '>', '-', ' ']);
    if line.is_empty() {
        return None;
    }
    Some(clip(line, 80))
}

pub fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_owned(),
    }
}

pub struct FileEntry {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}

#[derive(Default)]
pub struct Walk {
    pub files: Vec<FileEntry>,
    /// The root itself was missing; distinct from an empty store.
    pub missing: bool,
    pub warnings: Vec<String>,
}

/// Walk metadata only and skip symlinks, so loops and links out of the store are ignored.
pub fn walk(root: &Path, accepts: impl Fn(&Path) -> bool) -> Result<Walk> {
    let mut result = Walk::default();
    if !root.try_exists()? {
        result.missing = true;
        return Ok(result);
    }
    if !root.is_dir() {
        bail!("session root is not a directory: {}", root.display());
    }
    // An unreadable root is a store-level failure, not an empty store.
    fs::read_dir(root).with_context(|| format!("read session root {}", root.display()))?;
    let mut dirs = vec![root.to_owned()];
    while let Some(dir) = dirs.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                result.warnings.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    result.warnings.push(e.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(e) => {
                    result.warnings.push(format!("{}: {e}", path.display()));
                    continue;
                }
            };
            if kind.is_dir() {
                dirs.push(path);
            } else if kind.is_file() && accepts(&path) {
                match entry.metadata().and_then(|m| Ok((m.len(), m.modified()?))) {
                    Ok((bytes, modified)) => result.files.push(FileEntry {
                        path,
                        bytes,
                        modified,
                    }),
                    Err(e) => result.warnings.push(format!("{}: {e}", path.display())),
                }
            }
        }
    }
    Ok(result)
}

#[derive(Default)]
pub struct Inventory {
    pub newest: Option<PathBuf>,
    pub files: u64,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// Store-wide size and newest file for `status`; breaks mtime ties by path for repeatability.
pub fn inventory(provider: &dyn Provider, config: &Config) -> Result<Inventory> {
    let mut result = Inventory::default();
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for source in provider.sources() {
        let walk = walk(&source.root(config), |p| source.accepts(p))?;
        result.warnings.extend(walk.warnings);
        for file in walk.files {
            result.files += 1;
            result.bytes = result.bytes.saturating_add(file.bytes);
            let candidate = (file.modified, file.path);
            if newest.as_ref().is_none_or(|current| candidate > *current) {
                newest = Some(candidate);
            }
        }
    }
    result.newest = newest.map(|(_, path)| path);
    Ok(result)
}
