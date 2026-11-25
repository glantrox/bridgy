use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

pub const START: &str = "<!-- BRIDGY:START -->";
pub const END: &str = "<!-- BRIDGY:END -->";
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

pub struct Config {
    pub home: PathBuf,
}

impl Config {
    pub fn resolve(home: Option<PathBuf>) -> Result<Self> {
        let home = home
            .or_else(|| {
                if cfg!(windows) {
                    std::env::var_os("USERPROFILE").map(PathBuf::from)
                } else {
                    None
                }
            })
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .context("HOME is unset; pass --home")?;
        ensure!(
            home.is_absolute(),
            "home must be an absolute path: {}",
            home.display()
        );
        ensure!(
            home.is_dir(),
            "home directory does not exist: {}",
            home.display()
        );
        Ok(Self { home })
    }

    pub fn state_dir(&self) -> PathBuf {
        self.home.join(".bridgy")
    }
    pub fn protocol(&self) -> PathBuf {
        self.state_dir().join("PROTOCOL.md")
    }

    pub fn lock(&self) -> Result<File> {
        let path = self.home.join(".bridgy.lock");
        ensure!(
            !path.is_symlink(),
            "refusing symlink lock: {}",
            path.display()
        );
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        file.try_lock()
            .context("another Bridgy command is updating this home; retry after it finishes")?;
        Ok(file)
    }

    pub fn check_state_dir(&self) -> Result<()> {
        let path = self.state_dir();
        ensure!(
            !path.is_symlink(),
            "Bridgy state directory must not be a symlink: {}",
            path.display()
        );
        if path.exists() {
            ensure!(path.is_dir(), "{} is not a directory", path.display());
        }
        Ok(())
    }
}

pub fn read_optional(path: &Path) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !path.is_symlink() => {
            return Ok(None);
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    ensure!(
        file.metadata()?.is_file(),
        "not a regular file: {}",
        path.display()
    );
    let mut content = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut content)
        .with_context(|| format!("read UTF-8 instructions {}", path.display()))?;
    ensure!(
        content.len() as u64 <= MAX_CONFIG_BYTES,
        "instruction file exceeds 4 MiB: {}",
        path.display()
    );
    Ok(Some(content))
}

/// Span includes only the managed bytes; all user bytes outside it survive unchanged.
pub fn block_span(text: &str) -> Result<Option<std::ops::Range<usize>>> {
    let starts: Vec<_> = text.match_indices(START).map(|(i, _)| i).collect();
    let ends: Vec<_> = text.match_indices(END).map(|(i, _)| i).collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([start], [end]) if start < end => Ok(Some(*start..*end + END.len())),
        _ => bail!("malformed or duplicate BRIDGY markers; repair them before retrying"),
    }
}

pub fn render_block(protocol: &str) -> Result<String> {
    ensure!(
        !protocol.contains(START) && !protocol.contains(END),
        "central protocol must not contain BRIDGY markers"
    );
    ensure!(
        !protocol.trim().is_empty(),
        "central protocol must not be empty"
    );
    Ok(format!("{START}\n{}\n{END}", protocol.trim_end()))
}

pub fn inject(text: &str, block: &str) -> Result<String> {
    if let Some(span) = block_span(text)? {
        Ok(format!(
            "{}{}{}",
            &text[..span.start],
            block,
            &text[span.end..]
        ))
    } else {
        // No unmanaged separator: uninstall restores even a missing final newline exactly.
        Ok(format!("{text}{block}"))
    }
}

pub fn strip(text: &str) -> Result<String> {
    match block_span(text)? {
        Some(span) => Ok(format!("{}{}", &text[..span.start], &text[span.end..])),
        None => Ok(text.to_owned()),
    }
}

pub fn target_path(path: &Path) -> Result<PathBuf> {
    if path.is_symlink() {
        fs::canonicalize(path)
            .with_context(|| format!("resolve instruction symlink {}", path.display()))
    } else {
        Ok(path.to_owned())
    }
}

/// Backups live beside the original and survive uninstall. Atomic rename never replaces a symlink.
pub fn write_atomic(path: &Path, bytes: &[u8], backup: bool) -> Result<bool> {
    let target = target_path(path)?;
    let old = match fs::read(&target) {
        Ok(old) => Some(old),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("read {}", target.display())),
    };
    if old.as_deref() == Some(bytes) {
        return Ok(false);
    }
    let parent = target.parent().context("target has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    if backup && let Some(old) = &old {
        let name = target
            .file_name()
            .context("target has no filename")?
            .to_string_lossy();
        let mut copy = tempfile::Builder::new()
            .prefix(&format!("{name}.bridgy-backup-"))
            .tempfile_in(parent)?;
        copy.write_all(old)?;
        copy.as_file().sync_all()?;
        copy.keep().context("retain backup")?;
    }
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    if old.is_some() {
        staged
            .as_file()
            .set_permissions(fs::metadata(&target)?.permissions())?;
    }
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged
        .persist(&target)
        .with_context(|| format!("replace {}", target.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_user_bytes_and_is_idempotent() {
        for original in ["", "rules without newline", "# Rules\n\n", "日本語\r\n"] {
            let block = render_block("protocol").unwrap();
            let installed = inject(original, &block).unwrap();
            assert_eq!(inject(&installed, &block).unwrap(), installed);
            assert_eq!(strip(&installed).unwrap(), original);
        }
        let text = format!("before\n{START}\nold\n{END}\nafter\n");
        assert_eq!(strip(&text).unwrap(), "before\n\nafter\n");
    }
    #[test]
    fn rejects_broken_markers() {
        for text in [
            START.to_owned(),
            END.to_owned(),
            format!("{END}{START}"),
            format!("{START}{START}{END}"),
        ] {
            assert!(block_span(&text).is_err());
        }
    }
}
