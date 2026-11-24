//! One checkout resolver shared by `bridgy tail` and the TUI.
//!
//! A checkout is the nearest ancestor holding `.git`: a directory for a main checkout, a file for
//! a linked worktree or submodule. Linked worktrees therefore stay separate from the main
//! checkout. Outside Git the directory itself is the scope.

use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    fmt, fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Checkout(PathBuf),
    Global,
}

impl Scope {
    /// `cwd` is the default; `--workspace` replaces it; `--global` disables filtering.
    pub fn resolve(cwd: &Path, workspace: Option<&Path>, global: bool) -> Result<Self> {
        if global {
            return Ok(Self::Global);
        }
        let base = match workspace {
            Some(path) => {
                let path = cwd.join(path);
                ensure!(
                    path.is_dir(),
                    "workspace is not a directory: {}",
                    path.display()
                );
                path
            }
            None => cwd.to_owned(),
        };
        Ok(Self::Checkout(checkout_root(&base)))
    }

    pub fn current(workspace: Option<&Path>, global: bool) -> Result<Self> {
        Self::resolve(&std::env::current_dir()?, workspace, global)
    }

    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::Checkout(root) => Some(root),
            Self::Global => None,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Checkout(root) => write!(f, "{}", root.display()),
            Self::Global => f.write_str("all workspaces"),
        }
    }
}

/// Session working directories may be relative to symlinks or deleted since; resolve the
/// longest existing prefix and keep the remainder lexically.
pub fn normalize(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    let mut existing = clean.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(existing) {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return clean,
        }
    }
}

pub fn checkout_root(path: &Path) -> PathBuf {
    let path = normalize(path);
    path.ancestors()
        .find(|dir| fs::symlink_metadata(dir.join(".git")).is_ok())
        .map(Path::to_path_buf)
        .unwrap_or(path)
}

pub fn is_git_checkout(root: &Path) -> bool {
    fs::symlink_metadata(root.join(".git")).is_ok()
}

/// Many sessions share a working directory, so checkout lookups are cached per listing.
pub struct Matcher<'a> {
    scope: &'a Scope,
    cache: HashMap<PathBuf, bool>,
}

impl<'a> Matcher<'a> {
    pub fn new(scope: &'a Scope) -> Self {
        Self {
            scope,
            cache: HashMap::new(),
        }
    }

    /// Sessions without workspace metadata never match a checkout: no silent global fallback.
    pub fn matches(&mut self, workspace: Option<&Path>) -> bool {
        let Scope::Checkout(root) = self.scope else {
            return true;
        };
        let Some(workspace) = workspace else {
            return false;
        };
        *self
            .cache
            .entry(workspace.to_owned())
            .or_insert_with(|| checkout_root(workspace) == *root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subdirectories_resolve_to_checkout_and_worktrees_stay_separate() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let worktree = repo.join(".claude/worktrees/feature");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("src/deep")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            "gitdir: ../../../.git/worktrees/feature",
        )
        .unwrap();
        let repo = fs::canonicalize(repo).unwrap();
        let scope = Scope::resolve(&repo.join("src/deep"), None, false).unwrap();
        assert_eq!(scope, Scope::Checkout(repo.clone()));
        let mut matcher = Matcher::new(&scope);
        assert!(matcher.matches(Some(&repo.join("src"))));
        assert!(matcher.matches(Some(&repo.join("src/deleted/later"))));
        assert!(!matcher.matches(Some(&repo.join(".claude/worktrees/feature"))));
        assert!(!matcher.matches(Some(temp.path())));
        assert!(!matcher.matches(None));
        assert!(Matcher::new(&Scope::Global).matches(None));
    }

    #[test]
    fn outside_git_the_directory_is_the_scope() {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(dir.join("child")).unwrap();
        let scope = Scope::resolve(&dir, None, false).unwrap();
        let mut matcher = Matcher::new(&scope);
        assert!(matcher.matches(Some(&dir)));
        assert!(!matcher.matches(Some(&dir.join("child"))));
        assert!(Scope::resolve(&dir, Some(Path::new("missing")), false).is_err());
    }
}
