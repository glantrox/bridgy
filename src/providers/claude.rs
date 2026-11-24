use super::{Header, Provider, SessionSource, Turn, prompt_title, turn};
use crate::{
    config::Config,
    tail::{TailResult, head_records, read_jsonl, tail_records},
};
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

const HEAD_BYTES: u64 = 512 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;

pub struct Claude;
impl Provider for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["claude-code"]
    }
    fn directory(&self) -> &'static str {
        ".claude"
    }
    fn instructions_name(&self) -> &'static str {
        "CLAUDE.md"
    }
    fn sources(&self) -> Vec<Box<dyn SessionSource>> {
        // The CLI, VS Code, and desktop clients share one store; `entrypoint` names the client.
        vec![Box::new(Projects)]
    }
}

struct Projects;

fn normalize(record: &Value) -> Option<Turn> {
    if record.get("isMeta") == Some(&Value::Bool(true)) {
        return None;
    }
    let role = record.get("type")?.as_str()?;
    let message = record.get("message")?;
    let turn = turn(
        role,
        message.get("content")?,
        record.get("timestamp"),
        record.get("uuid").or_else(|| message.get("id")),
    )?;
    // Output of local slash commands is command output, not conversation.
    (!turn.text.starts_with("<local-command-")).then_some(turn)
}

fn surface(entrypoint: &str) -> String {
    match entrypoint {
        "cli" => "terminal",
        "claude-vscode" => "vscode",
        "claude-desktop" => "desktop",
        other if other.starts_with("sdk") => "sdk",
        other => other,
    }
    .to_owned()
}

/// A user-set title beats the generated one; both are re-emitted throughout a session.
#[derive(Default)]
struct Titles {
    custom: Option<String>,
    generated: Option<String>,
}
impl Titles {
    fn observe(&mut self, record: &Value) {
        let text = |key| record.get(key).and_then(Value::as_str).map(str::to_owned);
        match record.get("type").and_then(Value::as_str) {
            Some("custom-title") if self.custom.is_none() => self.custom = text("customTitle"),
            Some("ai-title") if self.generated.is_none() => self.generated = text("aiTitle"),
            Some("summary") if self.generated.is_none() => self.generated = text("summary"),
            _ => {}
        }
    }
    fn best(self) -> Option<String> {
        self.custom.or(self.generated)
    }
}

#[derive(Default)]
struct Scan {
    header: Header,
    recognized: bool,
    titles: Titles,
}
impl Scan {
    /// Returns false once the workspace and a title are known.
    fn visit(&mut self, record: &Value) -> bool {
        let header = &mut self.header;
        if matches!(
            record.get("type").and_then(Value::as_str),
            Some(
                "user"
                    | "assistant"
                    | "summary"
                    | "custom-title"
                    | "ai-title"
                    | "file-history-snapshot"
                    | "progress"
                    | "system"
            )
        ) {
            self.recognized = true;
        }
        self.titles.observe(record);
        if header.workspace.is_none()
            && let Some(cwd) = record.get("cwd").and_then(Value::as_str)
        {
            header.workspace = Some(PathBuf::from(cwd));
            header.surface = record
                .get("entrypoint")
                .and_then(Value::as_str)
                .map(surface);
        }
        if header.title.is_none()
            && record.get("type").and_then(Value::as_str) == Some("user")
            && let Some(turn) = normalize(record)
            && !["<command-", "<system-reminder", "<ide_"]
                .iter()
                .any(|p| turn.text.trim_start().starts_with(p))
        {
            header.title = prompt_title(&turn.text);
        }
        header.workspace.is_none() || header.title.is_none()
    }
}

impl SessionSource for Projects {
    fn root(&self, config: &Config) -> PathBuf {
        config.home.join(".claude/projects")
    }
    fn accepts(&self, path: &Path) -> bool {
        // Subagent transcripts are nested under their parent session and are not sessions.
        path.extension().is_some_and(|e| e == "jsonl")
            && !path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "subagents")
    }
    fn describe(&self, _config: &Config, path: &Path) -> Result<Option<Header>> {
        let mut scan = Scan {
            header: Header {
                id: path.file_stem().map(|s| s.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        head_records(path, HEAD_BYTES, |r| scan.visit(r))?;
        if scan.header.workspace.is_none() {
            // A head that is padding or a huge record still leaves recent records readable.
            tail_records(path, TAIL_BYTES, |r| scan.visit(r))?;
        }
        if !scan.recognized {
            return Ok(None);
        }
        let mut header = scan.header;
        if let Some(title) = scan.titles.best() {
            header.title = Some(title);
        }
        Ok(Some(header))
    }
    fn title(&self, _config: &Config, path: &Path, _header: &Header) -> Option<String> {
        let mut titles = Titles::default();
        tail_records(path, TAIL_BYTES, |record| {
            titles.observe(record);
            titles.custom.is_none()
        })
        .ok()?;
        titles.best()
    }
    fn read(&self, path: &Path, count: usize, max_bytes: u64) -> Result<TailResult> {
        read_jsonl(path, count, max_bytes, normalize)
    }
}
