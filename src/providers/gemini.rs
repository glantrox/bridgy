use super::{Header, Provider, SessionSource, Turn, turn};
use crate::{config::Config, tail::TailResult};
use anyhow::{Context, Result, ensure};
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    cell::OnceCell,
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const SNIFF_BYTES: u64 = 64 * 1024;

/// Recognize metadata without replaying messages. Return as soon as a known field is
/// found, so a legacy JSON document may extend beyond the bounded prefix.
struct Sniff<'a>(&'a mut bool);

impl<'de> Visitor<'de> for Sniff<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Gemini session metadata")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<(), M::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "sessionId" | "messages" | "$patch" | "$set" | "$rewindTo" => {
                    *self.0 = true;
                    return Ok(());
                }
                "type" => {
                    let kind = map.next_value::<String>()?;
                    if matches!(kind.as_str(), "user" | "gemini") {
                        *self.0 = true;
                        return Ok(());
                    }
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

pub struct Gemini;
impl Provider for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["gemini-cli"]
    }
    fn directory(&self) -> &'static str {
        ".gemini"
    }
    fn instructions_name(&self) -> Option<&'static str> {
        Some("GEMINI.md")
    }
    fn sources(&self) -> Vec<Box<dyn SessionSource>> {
        vec![Box::new(Chats::default())]
    }
}

#[derive(Default)]
struct Chats {
    /// Project directory name (a short name or the SHA-256 of the root) → project root.
    projects: OnceCell<HashMap<String, PathBuf>>,
}

fn normalize(record: &Value) -> Option<Turn> {
    let role = match record.get("type")?.as_str()? {
        "user" => "user",
        "gemini" => "assistant",
        _ => return None,
    };
    turn(
        role,
        record.get("content")?,
        record.get("timestamp"),
        record.get("id"),
    )
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Chats {
    fn projects(&self, config: &Config) -> &HashMap<String, PathBuf> {
        self.projects.get_or_init(|| {
            let mut map = HashMap::new();
            let index = config.home.join(".gemini/projects.json");
            if let Ok(text) = fs::read_to_string(index)
                && let Ok(value) = serde_json::from_str::<Value>(&text)
                && let Some(projects) = value.get("projects").and_then(Value::as_object)
            {
                for (root, name) in projects {
                    map.insert(sha256_hex(root), PathBuf::from(root));
                    if let Some(name) = name.as_str() {
                        map.insert(name.to_owned(), PathBuf::from(root));
                    }
                }
            }
            map
        })
    }

    /// Newer releases write `.project_root` beside `chats/`; older ones only hash the root.
    fn workspace(&self, config: &Config, project_dir: &Path) -> Option<PathBuf> {
        if let Ok(root) = fs::read_to_string(project_dir.join(".project_root")) {
            let root = root.trim();
            if !root.is_empty() {
                return Some(PathBuf::from(root));
            }
        }
        let name = project_dir.file_name()?.to_str()?;
        self.projects(config).get(name).cloned()
    }

    fn load(&self, path: &Path, max_bytes: u64) -> Result<(Replay, TailResult)> {
        // JSON documents and append-only patches cannot safely be treated as independent tail lines.
        let file = File::open(path).with_context(|| format!("open session {}", path.display()))?;
        ensure!(
            file.metadata()?.len() <= max_bytes,
            "Gemini replay exceeds --max-bytes; increase the limit or choose a smaller --file"
        );
        let mut bytes = Vec::new();
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= max_bytes,
            "Gemini session grew beyond --max-bytes; retry with a larger limit"
        );
        let mut result = TailResult {
            bytes_read: bytes.len() as u64,
            ..Default::default()
        };
        let mut replay = Replay::default();
        if path.extension().is_some_and(|e| e == "json") {
            let document: Value =
                serde_json::from_slice(&bytes).context("parse Gemini JSON session")?;
            let messages = document
                .get("messages")
                .and_then(Value::as_array)
                .context("Gemini JSON session has no messages array")?;
            for message in messages {
                replay.upsert(message.clone());
            }
        } else {
            let mut invalid = 0;
            for line in bytes.split(|byte| *byte == b'\n') {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                match serde_json::from_slice::<Value>(line) {
                    Ok(record) => replay.apply(record),
                    Err(_) => invalid += 1,
                }
            }
            if invalid > 0 {
                result.warnings.push(format!(
                    "skipped {invalid} malformed Gemini record(s); replay may be incomplete"
                ));
            }
        }
        Ok((replay, result))
    }
}

impl SessionSource for Chats {
    fn id(&self) -> &'static str {
        "chats"
    }
    fn root(&self, config: &Config) -> PathBuf {
        config.home.join(".gemini/tmp")
    }
    fn accepts(&self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|e| e == "json" || e == "jsonl")
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("session-"))
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "chats")
    }
    fn describe(&self, config: &Config, path: &Path) -> Result<Option<Header>> {
        let mut head = Vec::new();
        File::open(path)
            .with_context(|| format!("open session {}", path.display()))?
            .take(SNIFF_BYTES)
            .read_to_end(&mut head)
            .with_context(|| format!("read session {}", path.display()))?;
        let mut recognized = false;
        // Stopping after a key leaves an unfinished map, which serde reports as an
        // error. Recognition is kept separately; full validation belongs to read().
        let _ = serde_json::Deserializer::from_slice(&head).deserialize_map(Sniff(&mut recognized));
        if !recognized {
            return Ok(None);
        }
        let project_dir = path.parent().and_then(Path::parent);
        Ok(Some(Header {
            id: path.file_stem().map(|s| s.to_string_lossy().into_owned()),
            workspace: project_dir.and_then(|dir| self.workspace(config, dir)),
            title: None,
            surface: None,
        }))
    }
    fn read(&self, path: &Path, count: usize, max_bytes: u64) -> Result<TailResult> {
        let (replay, mut result) = self.load(path, max_bytes)?;
        result.turns = replay
            .order
            .iter()
            .rev()
            .filter_map(|id| replay.messages.get(id).and_then(normalize))
            .take(count)
            .collect();
        result.turns.reverse();
        Ok(result)
    }
}

#[derive(Default)]
struct Replay {
    order: Vec<String>,
    messages: HashMap<String, Value>,
}
impl Replay {
    fn upsert(&mut self, value: Value) {
        if let Some(id) = value.get("id").and_then(Value::as_str).map(str::to_owned) {
            if !self.messages.contains_key(&id) {
                self.order.push(id.clone());
            }
            self.messages.insert(id, value);
        }
    }
    fn patch(&mut self, value: &Value) {
        if let Some(id) = value.get("id").and_then(Value::as_str)
            && let Some(message) = self.messages.get_mut(id)
            && let Some(content) = value.get("content")
        {
            message["content"] = content.clone();
        }
    }
    fn apply(&mut self, value: Value) {
        if let Some(rewind) = value.get("$rewindTo").and_then(Value::as_str) {
            let at = self.order.iter().position(|id| id == rewind).unwrap_or(0);
            for id in self.order.drain(at..) {
                self.messages.remove(&id);
            }
        } else if let Some(patch) = value.get("$patch") {
            self.patch(patch);
            if let Some(updates) = patch.get("updates").and_then(Value::as_array) {
                for update in updates {
                    self.patch(update);
                }
            }
            if let Some(ids) = patch.get("removeIds").and_then(Value::as_array) {
                for id in ids.iter().filter_map(Value::as_str) {
                    self.messages.remove(id);
                }
                self.order.retain(|id| self.messages.contains_key(id));
            }
            if let Some(ids) = patch.get("orderIds").and_then(Value::as_array) {
                let mut suffix = Vec::new();
                for id in ids.iter().filter_map(Value::as_str) {
                    if self.messages.contains_key(id) && !suffix.iter().any(|s| s == id) {
                        suffix.push(id.to_owned());
                    }
                }
                self.order.retain(|id| !suffix.contains(id));
                self.order.extend(suffix);
            }
        } else if value.get("$set").is_some() {
            if let Some(messages) = value["$set"].get("messages").and_then(Value::as_array) {
                self.messages.clear();
                self.order.clear();
                for message in messages {
                    self.upsert(message.clone());
                }
            }
        } else if let Some(messages) = value.get("messages").and_then(Value::as_array) {
            for message in messages {
                self.upsert(message.clone());
            }
        } else {
            self.upsert(value);
        }
    }
}
