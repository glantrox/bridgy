use crate::providers::Turn;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const CHUNK: usize = 8192;
const MAX_LINE: usize = 16 * 1024 * 1024;

#[derive(Debug, Default, Serialize)]
pub struct TailResult {
    pub turns: Vec<Turn>,
    pub bytes_read: u64,
    pub warnings: Vec<String>,
}

/// One bounded buffer, linear work even for long records crossing many chunk boundaries.
pub fn reverse_lines<R: Read + Seek>(
    reader: &mut R,
    max_bytes: u64,
    mut visit: impl FnMut(&[u8]) -> bool,
) -> Result<(u64, usize, bool)> {
    let mut position = reader.seek(SeekFrom::End(0))?;
    let mut bytes_read = 0;
    let mut line = Vec::new();
    let mut oversized = 0;
    let mut dropping = false;
    let mut buffer = [0u8; CHUNK];
    while position > 0 && bytes_read < max_bytes {
        let size = position.min(CHUNK as u64).min(max_bytes - bytes_read) as usize;
        position -= size as u64;
        reader.seek(SeekFrom::Start(position))?;
        reader.read_exact(&mut buffer[..size])?;
        bytes_read += size as u64;
        for byte in buffer[..size].iter().rev() {
            if *byte == b'\n' {
                if !dropping && !line.is_empty() {
                    line.reverse();
                    if !visit(&line) {
                        return Ok((bytes_read, oversized, false));
                    }
                }
                line.clear();
                dropping = false;
            } else if !dropping {
                if line.len() == MAX_LINE {
                    line.clear();
                    dropping = true;
                    oversized += 1;
                } else {
                    line.push(*byte);
                }
            }
        }
    }
    if position == 0 && !dropping && !line.is_empty() {
        line.reverse();
        visit(&line);
    }
    Ok((bytes_read, oversized, position > 0))
}

/// Visit JSON records from the start of a file, within a byte budget, until `visit` returns false.
pub fn head_records(
    path: &Path,
    max_bytes: u64,
    mut visit: impl FnMut(&Value) -> bool,
) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open session {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file.take(max_bytes));
    let mut line = Vec::new();
    loop {
        line.clear();
        if std::io::BufRead::read_until(&mut reader, b'\n', &mut line)
            .with_context(|| format!("read session {}", path.display()))?
            == 0
        {
            return Ok(());
        }
        if let Ok(record) = serde_json::from_slice::<Value>(&line)
            && !visit(&record)
        {
            return Ok(());
        }
    }
}

/// Visit JSON records from the end of a file, newest first, within a byte budget.
pub fn tail_records(
    path: &Path,
    max_bytes: u64,
    mut visit: impl FnMut(&Value) -> bool,
) -> Result<()> {
    let mut file = File::open(path).with_context(|| format!("open session {}", path.display()))?;
    reverse_lines(&mut file, max_bytes, |line| {
        serde_json::from_slice::<Value>(line).map_or(true, |record| visit(&record))
    })
    .with_context(|| format!("read session {}", path.display()))?;
    Ok(())
}

pub fn read_jsonl(
    path: &Path,
    count: usize,
    max_bytes: u64,
    normalize: impl Fn(&Value) -> Option<Turn>,
) -> Result<TailResult> {
    let mut file = File::open(path).with_context(|| format!("open session {}", path.display()))?;
    let mut result = TailResult::default();
    if count == 0 {
        return Ok(result);
    }
    let mut invalid = 0;
    let mut ids = HashSet::new();
    let (bytes_read, oversized, limited) = reverse_lines(&mut file, max_bytes, |line| {
        if line.iter().all(u8::is_ascii_whitespace) {
            return true;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(record) => {
                if let Some(turn) = normalize(&record)
                    && turn.id.as_ref().is_none_or(|id| ids.insert(id.clone()))
                {
                    result.turns.push(turn);
                }
            }
            Err(_) => invalid += 1,
        }
        result.turns.len() < count
    })
    .with_context(|| format!("read session {}", path.display()))?;
    result.bytes_read = bytes_read;
    result.turns.reverse();
    if invalid > 0 {
        result.warnings.push(format!(
            "skipped {invalid} malformed JSON record(s), possibly an in-progress write"
        ));
    }
    if oversized > 0 {
        result
            .warnings
            .push(format!("skipped {oversized} record(s) larger than 16 MiB"));
    }
    if limited {
        result
            .warnings
            .push("scan byte limit reached; increase --max-bytes to look further back".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn reads_across_chunks_and_handles_final_newline() {
        let long = "🦀".repeat(CHUNK);
        for suffix in ["", "\n"] {
            let input = format!("first\n{long}\nlast{suffix}");
            let mut lines = Vec::new();
            reverse_lines(&mut Cursor::new(input), u64::MAX, |line| {
                lines.push(String::from_utf8(line.to_vec()).unwrap());
                true
            })
            .unwrap();
            assert_eq!(lines, vec!["last", &long, "first"]);
        }
    }
    #[test]
    fn reads_only_the_tail_and_discards_partial_prefix() {
        let mut input = vec![b'x'; 1_000_000];
        input.extend_from_slice(b"\nlast\n");
        let mut lines = Vec::new();
        let (bytes, _, limited) = reverse_lines(&mut Cursor::new(input.clone()), 100, |line| {
            lines.push(line.to_vec());
            false
        })
        .unwrap();
        assert_eq!(lines, vec![b"last".to_vec()]);
        assert_eq!(bytes, 100);
        assert!(!limited);
        let (_, _, limited) = reverse_lines(&mut Cursor::new(input), 100, |_| true).unwrap();
        assert!(limited);
    }
}
