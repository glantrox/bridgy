//! One bounded transcript and one Markdown rendering, shared by `tail`, preview, copy, and export.

use crate::{
    config::Config,
    providers::Turn,
    sessions::{self, SessionMeta},
};
use anyhow::Result;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_MESSAGES: usize = 40;
pub const DEFAULT_MAX_CHARS: usize = 48 * 1024;
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub messages: usize,
    /// Cap on rendered message text; the oldest messages are dropped first.
    pub max_chars: usize,
    /// Cap on log bytes scanned.
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            messages: DEFAULT_MESSAGES,
            max_chars: DEFAULT_MAX_CHARS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub session: SessionMeta,
    pub turns: Vec<Turn>,
    /// Messages dropped to fit `max_chars`.
    pub omitted: usize,
    /// The newest message alone exceeded `max_chars` and was cut.
    pub truncated: bool,
    pub bytes_read: u64,
    pub warnings: Vec<String>,
}

const CUT: &str = "\n\n[… message truncated by Bridgy]";

pub fn load(config: &Config, session: &SessionMeta, limits: Limits) -> Result<Transcript> {
    let result = sessions::read(
        config,
        &session.reference,
        limits.messages,
        limits.max_bytes,
    )?;
    Ok(fit(
        session.clone(),
        result.turns,
        limits.max_chars,
        result.bytes_read,
        result.warnings,
    ))
}

pub(crate) fn fit(
    session: SessionMeta,
    mut turns: Vec<Turn>,
    max_chars: usize,
    bytes_read: u64,
    warnings: Vec<String>,
) -> Transcript {
    let mut used = 0;
    let mut keep = 0;
    for turn in turns.iter().rev() {
        let size = turn.text.len();
        if keep > 0 && used + size > max_chars {
            break;
        }
        used += size;
        keep += 1;
    }
    let omitted = turns.len() - keep;
    turns.drain(..omitted);
    let mut truncated = false;
    if let Some(last) = turns.last_mut()
        && last.text.len() > max_chars
    {
        let mut at = max_chars.saturating_sub(CUT.len());
        while !last.text.is_char_boundary(at) {
            at -= 1;
        }
        last.text.truncate(at);
        last.text.push_str(CUT);
        truncated = true;
    }
    Transcript {
        session,
        turns,
        omitted,
        truncated,
        bytes_read,
        warnings,
    }
}

impl Transcript {
    /// The Markdown used by copy, export, and `tail`, built from the same turns as the preview.
    pub fn markdown(&self) -> String {
        if let [turn] = self.turns.as_slice()
            && turn.role == "document"
        {
            return turn.text.clone();
        }
        let s = &self.session;
        let mut out = format!("# {} session", display_name(s.reference.provider));
        if let Some(title) = &s.title {
            out.push_str(&format!(": {title}"));
        }
        out.push_str("\n\n");
        let provider = match &s.surface {
            Some(surface) => format!("{} ({surface})", s.reference.provider),
            None => s.reference.provider.to_owned(),
        };
        out.push_str(&format!("- Provider: {provider}\n"));
        out.push_str(&format!(
            "- Workspace: {}\n",
            s.workspace
                .as_ref()
                .map_or("unknown".into(), |w| w.display().to_string())
        ));
        out.push_str(&format!("- Session: {}\n", s.reference.path.display()));
        out.push_str(&format!("- Last activity: {}\n", utc(s.last_activity)));
        let mut shown = format!("- Messages: last {}", self.turns.len());
        if self.omitted > 0 {
            shown.push_str(&format!(
                " ({} earlier omitted to fit the size limit)",
                self.omitted
            ));
        }
        out.push_str(&shown);
        out.push_str(
            "\n\n> Saved user/assistant text only. Tool output and hidden reasoning are omitted.\n",
        );
        for turn in &self.turns {
            out.push_str(&format!(
                "\n## {}{}\n\n{}\n",
                if turn.role == "user" {
                    "User"
                } else {
                    "Assistant"
                },
                turn.timestamp
                    .as_deref()
                    .map(|t| format!(" · {t}"))
                    .unwrap_or_default(),
                turn.text.trim_end()
            ));
        }
        if self.turns.is_empty() {
            out.push_str("\n_No conversational text found in this session._\n");
        }
        out
    }
}

pub fn display_name(provider: &str) -> &str {
    match provider {
        "claude" => "Claude",
        "codex" => "Codex",
        "gemini" => "Gemini",
        other => other,
    }
}

/// `2026-11-25 06:14 UTC`, without a date-time dependency.
pub fn utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub fn ago(time: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(time).unwrap_or(Duration::ZERO).as_secs();
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        86_400..2_592_000 => format!("{}d ago", secs / 86_400),
        _ => utc(time)[..10].to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionRef;

    fn meta() -> SessionMeta {
        SessionMeta {
            reference: SessionRef {
                source: "projects",
                provider: "claude",
                path: "/s.jsonl".into(),
            },
            id: "s".into(),
            workspace: None,
            last_activity: UNIX_EPOCH,
            title: None,
            surface: None,
        }
    }
    fn turn(text: &str) -> Turn {
        Turn {
            role: "user".into(),
            text: text.into(),
            timestamp: None,
            id: None,
        }
    }

    #[test]
    fn drops_oldest_first_and_truncates_only_an_oversized_newest_message() {
        let t = fit(
            meta(),
            vec![turn("aaaa"), turn("bbbb"), turn("cccc")],
            9,
            0,
            vec![],
        );
        assert_eq!(t.omitted, 1);
        assert_eq!(t.turns[0].text, "bbbb");
        let t = fit(
            meta(),
            vec![turn("old"), turn(&"é".repeat(200))],
            100,
            0,
            vec![],
        );
        assert_eq!(t.omitted, 1);
        assert!(t.truncated);
        assert!(t.turns[0].text.len() <= 100);
        assert!(t.markdown().contains("truncated by Bridgy"));
    }

    #[test]
    fn formats_utc_dates() {
        assert_eq!(utc(UNIX_EPOCH), "1970-01-01 00:00 UTC");
        assert_eq!(
            utc(UNIX_EPOCH + Duration::from_secs(1_795_669_440)),
            "2026-11-26 05:04 UTC"
        );
    }
}
