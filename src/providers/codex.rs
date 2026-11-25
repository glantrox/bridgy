use super::{Header, Provider, SessionSource, Turn, prompt_title, turn};
use crate::{
    config::Config,
    tail::{TailResult, head_records, read_jsonl},
};
use anyhow::Result;
use serde_json::Value;
use std::{
    cell::OnceCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

const HEAD_BYTES: u64 = 1024 * 1024;
const INDEX_BYTES: u64 = 16 * 1024 * 1024;

pub struct Codex;
impl Provider for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["chatgpt", "openai"]
    }
    fn directory(&self) -> &'static str {
        ".codex"
    }
    fn instructions_name(&self) -> Option<&'static str> {
        Some("AGENTS.md")
    }
    fn sources(&self) -> Vec<Box<dyn SessionSource>> {
        // The CLI and the VS Code extension write the same rollout format; session_meta names the client.
        vec![Box::new(Rollouts::default())]
    }
}

#[derive(Default)]
struct Rollouts {
    titles: OnceCell<HashMap<String, String>>,
}

/// Context Codex injects as user-role messages; it is setup, not conversation.
fn injected(text: &str) -> bool {
    let text = text.trim_start();
    [
        "<environment_context",
        "<user_instructions",
        "<permissions instructions",
        "<INSTRUCTIONS>",
        "# AGENTS.md instructions",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn normalize(record: &Value) -> Option<Turn> {
    // event_msg mirrors response_item and must not duplicate conversation text.
    let message = match record.get("type")?.as_str()? {
        "response_item" => record.get("payload")?,
        "message" => record,
        _ => return None,
    };
    if message.get("type")?.as_str()? != "message" {
        return None;
    }
    if message.get("channel").and_then(Value::as_str) == Some("analysis") {
        return None;
    }
    let turn = turn(
        message.get("role")?.as_str()?,
        message.get("content")?,
        record.get("timestamp"),
        message.get("id"),
    )?;
    (!injected(&turn.text)).then_some(turn)
}

impl Rollouts {
    /// Thread names live in an append-only index; the last entry for an id wins.
    fn titles(&self, config: &Config) -> &HashMap<String, String> {
        self.titles.get_or_init(|| {
            let mut titles = HashMap::new();
            let _ = head_records(
                &config.home.join(".codex/session_index.jsonl"),
                INDEX_BYTES,
                |record| {
                    if let (Some(id), Some(name)) = (
                        record.get("id").and_then(Value::as_str),
                        record.get("thread_name").and_then(Value::as_str),
                    ) && !name.trim().is_empty()
                    {
                        titles.insert(id.to_owned(), name.trim().to_owned());
                    }
                    true
                },
            );
            titles
        })
    }
}

impl SessionSource for Rollouts {
    fn id(&self) -> &'static str {
        "rollouts"
    }
    fn root(&self, config: &Config) -> PathBuf {
        config.home.join(".codex/sessions")
    }
    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "jsonl")
    }
    fn describe(&self, _config: &Config, path: &Path) -> Result<Option<Header>> {
        let mut header = None;
        head_records(path, HEAD_BYTES, |record| {
            let meta = match record.get("type").and_then(Value::as_str) {
                Some("session_meta") => record.get("payload").unwrap_or(&Value::Null),
                // Early rollouts began with a bare {id, timestamp} line and had no cwd.
                None if record.get("id").is_some() && record.get("timestamp").is_some() => record,
                // Legacy message-only rollouts remain readable globally even without cwd.
                Some("message" | "response_item") if normalize(record).is_some() => {
                    header.get_or_insert_with(Header::default);
                    return true;
                }
                _ => return true,
            };
            let text = |key| meta.get(key).and_then(Value::as_str).map(str::to_owned);
            header = Some(Header {
                id: text("id"),
                workspace: text("cwd").map(PathBuf::from),
                title: None,
                surface: text("source")
                    .or_else(|| text("originator"))
                    .map(|s| s.trim_start_matches("codex_").to_owned()),
            });
            false
        })?;
        Ok(header)
    }
    fn title(&self, config: &Config, path: &Path, header: &Header) -> Option<String> {
        if let Some(title) = header
            .id
            .as_ref()
            .and_then(|id| self.titles(config).get(id))
        {
            return Some(title.clone());
        }
        let mut title = None;
        head_records(path, HEAD_BYTES, |record| {
            if let Some(turn) = normalize(record)
                && turn.role == "user"
            {
                title = prompt_title(&turn.text);
            }
            title.is_none()
        })
        .ok()?;
        title
    }
    fn read(&self, path: &Path, count: usize, max_bytes: u64) -> Result<TailResult> {
        read_jsonl(path, count, max_bytes, normalize)
    }
}
