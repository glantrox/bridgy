//! Import previews are transient. A confirmed import stores a versioned pointer.
use crate::{
    config::{Config, write_atomic},
    providers::{Header, Provider, SessionSource, Turn},
    sessions::{SessionMeta, SessionRef},
    tail::TailResult,
    transcript::{self, Limits, Transcript},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

pub(crate) fn read_file(path: &Path, max: u64) -> Result<Vec<u8>> {
    ensure!(
        fs::metadata(path)?.is_file(),
        "expected a regular file: {}",
        path.display()
    );
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= max,
        "{} exceeds the {} byte limit",
        path.display(),
        max
    );
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "file grew beyond size limit");
    Ok(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    version: u8,
    pub path: PathBuf,
    workspace: Option<PathBuf>,
    title: String,
    format: String,
}
impl Bookmark {
    pub fn save(&self, config: &Config, workspace: Option<&Path>) -> Result<()> {
        let mut record = self.clone();
        if let Some(ws) = workspace {
            record.workspace = Some(ws.to_owned());
        }
        let _lock = config.lock()?;
        config.check_state_dir()?;
        let root = Imports.root(config);
        ensure!(
            !root.is_symlink(),
            "imports directory must not be a symlink"
        );
        let key = serde_json::to_vec(&(&record.path, &record.workspace))?;
        let id: String = Sha256::digest(key)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let dest = root.join(format!("{id}.json"));
        ensure!(!dest.is_symlink(), "import record must not be a symlink");
        write_atomic(&dest, &serde_json::to_vec(&record)?, false)?;
        Ok(())
    }
    fn open(p: &Path) -> Result<Self> {
        let b: Self = serde_json::from_slice(&read_file(p, 64 * 1024)?)?;
        ensure!(
            b.version == 1 && b.path.is_absolute(),
            "unsupported import reference"
        );
        Ok(b)
    }
}
pub struct Imports;
impl Provider for Imports {
    fn id(&self) -> &'static str {
        "import"
    }
    fn directory(&self) -> &'static str {
        ".bridgy/imports"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["imported"]
    }
    fn sources(&self) -> Vec<Box<dyn SessionSource>> {
        vec![Box::new(Imports)]
    }
}
impl SessionSource for Imports {
    fn id(&self) -> &'static str {
        "bookmark-v1"
    }
    fn root(&self, c: &Config) -> PathBuf {
        c.state_dir().join("imports")
    }
    fn accepts(&self, p: &Path) -> bool {
        p.extension().is_some_and(|s| s == "json")
    }
    fn describe(&self, _: &Config, p: &Path) -> Result<Option<Header>> {
        let b = Bookmark::open(p)?;
        Ok(Some(Header {
            workspace: b.workspace,
            title: Some(b.title),
            surface: Some(b.format),
            ..Default::default()
        }))
    }
    fn read(&self, p: &Path, count: usize, max: u64) -> Result<TailResult> {
        let b = Bookmark::open(p)?;
        let (_, result) = decode(&b.path, count, max)?;
        Ok(result)
    }
}
fn decode(path: &Path, count: usize, max: u64) -> Result<(Bookmark, TailResult)> {
    let data = read_file(path, max)?;
    let mut b = Bookmark {
        version: 1,
        path: path.into(),
        workspace: None,
        title: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        format: "Markdown".into(),
    };
    let turns = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "md" | "markdown" | "txt" => vec![Turn {
            role: "document".into(),
            text: std::str::from_utf8(&data)?.to_owned(),
            timestamp: None,
            id: None,
        }],
        "json" => {
            let v: Value = serde_json::from_slice(&data).context("invalid JSON export")?;
            let info = &v["info"];
            ensure!(
                info["id"].as_str().is_some_and(|s| s.starts_with("ses_"))
                    && info["version"].as_str().is_some_and(|s| s
                        .split('.')
                        .next()
                        .is_some_and(|n| n.parse::<u32>().is_ok())),
                "unsupported JSON export (expected an OpenCode/Kilo session export with info and messages)"
            );
            let messages = v["messages"]
                .as_array()
                .context("export missing messages")?;
            b.workspace = info["directory"]
                .as_str()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute());
            b.format = "session JSON v1".into();
            if let Some(title) = info["title"].as_str() {
                b.title = title.into();
            }
            let mut turns = Vec::new();
            for m in messages.iter().rev() {
                let Some(role @ ("user" | "assistant")) = m["info"]["role"].as_str() else {
                    continue;
                };
                let parts = m["parts"]
                    .as_array()
                    .context("export message missing parts")?;
                let text = parts
                    .iter()
                    .filter(|p| {
                        p["type"] == "text" && p["synthetic"] != true && p["ignored"] != true
                    })
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    turns.push(Turn {
                        role: role.into(),
                        text,
                        timestamp: None,
                        id: m["info"]["id"].as_str().map(String::from),
                    });
                }
                if turns.len() >= count {
                    break;
                }
            }
            turns.reverse();
            turns
        }
        _ => {
            bail!("unsupported import extension; choose Markdown, text, or a supported JSON export")
        }
    };
    Ok((
        b,
        TailResult {
            turns,
            bytes_read: data.len() as u64,
            ..Default::default()
        },
    ))
}
pub fn preview(path: &Path, limits: Limits) -> Result<(Bookmark, Transcript)> {
    let path = path.canonicalize().context("resolve import path")?;
    let (b, result) = decode(&path, limits.messages, limits.max_bytes)?;
    let session = SessionMeta {
        reference: SessionRef {
            provider: "import",
            source: "file",
            path: path.clone(),
        },
        id: b.title.clone(),
        workspace: b.workspace.clone(),
        last_activity: fs::metadata(&path)?.modified()?,
        title: Some(b.title.clone()),
        surface: Some(b.format.clone()),
    };
    Ok((
        b,
        transcript::fit(
            session,
            result.turns,
            limits.max_chars,
            result.bytes_read,
            result.warnings,
        ),
    ))
}

pub(crate) fn original(record: &Path) -> Option<PathBuf> {
    Bookmark::open(record).ok()?.path.canonicalize().ok()
}
