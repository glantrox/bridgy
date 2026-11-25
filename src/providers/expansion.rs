use super::{Header, Provider, SessionSource, Turn, turn};
use crate::{
    config::Config,
    imports::read_file,
    tail::{TailResult, head_records, read_jsonl},
};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Copilot;
impl Provider for Copilot {
    fn id(&self) -> &'static str {
        "copilot"
    }
    fn directory(&self) -> &'static str {
        ".copilot"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["copilot-cli", "github-copilot"]
    }
    fn sources(&self) -> Vec<Box<dyn SessionSource>> {
        vec![Box::new(Events)]
    }
}
struct Events;
impl SessionSource for Events {
    fn native_identity(&self) -> bool {
        true
    }
    fn id(&self) -> &'static str {
        "events-v1"
    }
    fn root(&self, c: &Config) -> PathBuf {
        c.home.join(".copilot/session-state")
    }
    fn accepts(&self, p: &Path) -> bool {
        p.file_name().is_some_and(|x| x == "events.jsonl")
            && p.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .is_some_and(|n| n == "session-state")
    }
    fn describe(&self, _: &Config, p: &Path) -> Result<Option<Header>> {
        let mut header = None;
        head_records(p, 512 * 1024, |v| {
            let d = &v["data"];
            match v["type"].as_str() {
                Some("session.start") if d["version"] == 1 => {
                    header = Some(Header {
                        id: d["sessionId"].as_str().map(String::from),
                        workspace: d["context"]["cwd"].as_str().map(PathBuf::from),
                        surface: Some("cli".into()),
                        ..Header::default()
                    })
                }
                Some("user.message") if header.is_some() => {
                    header.as_mut().unwrap().title =
                        d["content"].as_str().and_then(super::prompt_title);
                    return false;
                }
                _ => {}
            }
            true
        })?;
        Ok(header)
    }
    fn read(&self, p: &Path, count: usize, max: u64) -> Result<TailResult> {
        read_jsonl(p, count, max, |v| {
            let role = match v["type"].as_str()? {
                "user.message" => "user",
                "assistant.message" => "assistant",
                _ => return None,
            };
            let d = &v["data"];
            if !d["parentAgentTaskId"].is_null() {
                return None;
            }
            turn(role, &d["content"], v.get("timestamp"), d.get("messageId"))
        })
    }
}
pub struct Cline;
impl Provider for Cline {
    fn id(&self) -> &'static str {
        "cline"
    }
    fn directory(&self) -> &'static str {
        ".cline"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
    fn sources(&self) -> Vec<Box<dyn SessionSource>> {
        vec![Box::new(ClineSessions)]
    }
}
struct ClineSessions;
impl SessionSource for ClineSessions {
    fn native_identity(&self) -> bool {
        true
    }
    fn id(&self) -> &'static str {
        "sessions-v1"
    }
    fn root(&self, c: &Config) -> PathBuf {
        c.home.join(".cline/data/sessions")
    }
    fn accepts(&self, p: &Path) -> bool {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".messages.json"))
    }
    fn describe(&self, _: &Config, p: &Path) -> Result<Option<Header>> {
        let Some(id) = p
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".messages.json"))
        else {
            return Ok(None);
        };
        let meta = p.with_file_name(format!("{id}.json"));
        if meta.is_symlink() {
            return Ok(None);
        }
        let bytes = read_file(&meta, 4 * 1024 * 1024)?;
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(None);
        };
        if v["version"] != 1 || v["session_id"] != id {
            return Ok(None);
        }
        Ok(Some(Header {
            id: Some(id.to_string()),
            workspace: v["cwd"]
                .as_str()
                .or(v["workspace_root"].as_str())
                .map(PathBuf::from),
            title: v["metadata"]["title"].as_str().map(String::from),
            surface: v["source"].as_str().map(String::from),
        }))
    }
    fn read(&self, p: &Path, count: usize, max: u64) -> Result<TailResult> {
        let bytes = read_file(p, max)?;
        let v: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            v["version"] == 1 && v["messages"].is_array(),
            "unsupported Cline transcript version or format"
        );
        let mut turns: Vec<Turn> = v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .filter_map(|m| turn(m["role"].as_str()?, &m["content"], None, m.get("id")))
            .take(count)
            .collect();
        turns.reverse();
        Ok(TailResult {
            turns,
            bytes_read: bytes.len() as u64,
            ..Default::default()
        })
    }
}
