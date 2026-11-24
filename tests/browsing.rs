use bridgy::{
    config::Config,
    sessions::{self, ListOptions, StoreState},
    transcript::{self, Limits},
    tui::app::{App, Prompt, Tone, View},
    workspace::Scope,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, UNIX_EPOCH},
};
use tempfile::TempDir;

fn write(path: &Path, text: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn records(path: &Path, values: &[Value]) {
    write(
        path,
        values.iter().map(|v| format!("{v}\n")).collect::<String>(),
    );
}

fn claude(home: &Path, id: &str, workspace: &Path, modified: u64) -> PathBuf {
    let path = home.join(format!(".claude/projects/project/{id}.jsonl"));
    records(
        &path,
        &[
            json!({"type":"user", "cwd":workspace, "entrypoint":"cli", "message":{"content":format!("question {id}")}}),
            json!({"type":"assistant", "message":{"content":format!("answer {id}")}}),
            json!({"type":"custom-title", "customTitle":format!("Title {id}")}),
        ],
    );
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(modified))
        .unwrap();
    path
}

struct Fixture {
    _temp: TempDir,
    config: Config,
    repo: PathBuf,
    worktree: PathBuf,
    other: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        fs::create_dir(&home).unwrap();
        let config = Config::resolve(Some(home)).unwrap();
        let repo = temp.path().join("repo");
        let worktree = repo.join(".claude/worktrees/feature");
        let other = temp.path().join("other");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("src/deep")).unwrap();
        fs::create_dir_all(other.join(".git")).unwrap();
        write(
            &worktree.join(".git"),
            "gitdir: ../../../.git/worktrees/feature",
        );
        Self {
            _temp: temp,
            config,
            repo,
            worktree,
            other,
        }
    }

    fn scope(&self) -> Scope {
        Scope::resolve(&self.repo.join("src/deep"), None, false).unwrap()
    }

    fn app(&self) -> App<'_> {
        App::new(&self.config, self.scope(), self.repo.clone(), capture)
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bridgy"))
            .arg("--home")
            .arg(&self.config.home)
            .args(args)
            .env_remove("BRIDGY_HOME")
            .current_dir(self.repo.join("src/deep"))
            .output()
            .unwrap()
    }
}

fn key(app: &mut App, code: KeyCode) {
    app.handle(KeyEvent::new(code, KeyModifiers::NONE));
}

thread_local! {
    static COPIED: RefCell<String> = const { RefCell::new(String::new()) };
}

fn capture(text: &str) -> anyhow::Result<&'static str> {
    COPIED.with(|copied| *copied.borrow_mut() = text.to_owned());
    Ok("test clipboard")
}

fn no_clipboard(_: &str) -> anyhow::Result<&'static str> {
    anyhow::bail!("clipboard unavailable")
}

#[test]
fn picker_and_tail_share_checkout_filtering_and_explicit_overrides() {
    let f = Fixture::new();
    let own = claude(&f.config.home, "own", &f.repo.join("src"), 100);
    let other = claude(&f.config.home, "other", &f.other, 200);
    let worktree = claude(&f.config.home, "worktree", &f.worktree, 300);
    claude(&f.config.home, "parent/subagents/child", &f.repo, 400);
    let app = f.app();
    assert_eq!(app.listing.sessions.len(), 1);
    assert_eq!(app.listing.sessions[0].reference.path, own);
    assert_eq!(app.listing.sessions[0].title.as_deref(), Some("Title own"));
    for (args, expected) in [
        (vec!["tail", "claude", "--json"], &own),
        (vec!["tail", "claude", "--json", "--global"], &worktree),
        (vec!["--global", "tail", "claude", "--json"], &worktree),
        (
            vec![
                "tail",
                "claude",
                "--json",
                "--workspace",
                f.other.to_str().unwrap(),
            ],
            &other,
        ),
        (
            vec![
                "--workspace",
                f.worktree.to_str().unwrap(),
                "tail",
                "claude",
                "--json",
            ],
            &worktree,
        ),
        (
            vec!["tail", "claude", "--json", "--file", own.to_str().unwrap()],
            &own,
        ),
    ] {
        let result = f.cli(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["session"], expected.to_str().unwrap());
    }
    for args in [
        vec![
            "--global",
            "tail",
            "claude",
            "--file",
            own.to_str().unwrap(),
        ],
        vec![
            "--workspace",
            f.repo.to_str().unwrap(),
            "tail",
            "claude",
            "--global",
        ],
    ] {
        assert_eq!(f.cli(&args).status.code(), Some(2));
    }
}

#[test]
fn empty_checkout_never_falls_back_to_another_project() {
    let f = Fixture::new();
    claude(&f.config.home, "other", &f.other, 100);
    records(
        &f.config.home.join(".claude/projects/p/unknown.jsonl"),
        &[json!({"type":"user", "message":{"content":"no workspace"}})],
    );
    let mut app = f.app();
    assert!(app.listing.sessions.is_empty());
    assert!(matches!(
        app.listing.reports[0].state(),
        StoreState::NoneHere
    ));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.view, View::Sessions);
    let result = f.cli(&["tail", "claude"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no history for this checkout"));
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.listing.sessions.len(), 2);
    key(&mut app, KeyCode::Char('g'));
    assert!(app.listing.sessions.is_empty());
}

#[test]
fn global_start_can_return_to_current_checkout() {
    let f = Fixture::new();
    claude(&f.config.home, "own", &f.repo, 100);
    claude(&f.config.home, "other", &f.other, 200);
    let mut app = App::new(&f.config, Scope::Global, f.repo.join("src"), capture);
    assert_eq!(app.listing.sessions.len(), 2);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.listing.scope, f.scope());
    assert_eq!(app.listing.sessions.len(), 1);
}

#[test]
fn selected_older_session_stays_exact_for_preview_copy_and_export() {
    let f = Fixture::new();
    let old = claude(&f.config.home, "old", &f.repo, 100);
    claude(&f.config.home, "new", &f.repo, 200);
    let mut app = f.app();
    key(&mut app, KeyCode::Down);
    let selected = app.selected_session().unwrap().clone();
    // A new session after listing must not redirect the selected reference.
    claude(&f.config.home, "newest", &f.repo, 300);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.view, View::Preview);
    let preview = app.loaded.as_ref().unwrap().text.clone();
    assert_eq!(app.loaded.as_ref().unwrap().reference.path, old);
    assert!(preview.contains("answer old"));
    assert!(!preview.contains("answer new"));
    assert_eq!(
        preview,
        transcript::load(&f.config, &selected, Limits::default())
            .unwrap()
            .markdown()
    );
    let cli = f.cli(&["tail", "claude", "--file", old.to_str().unwrap()]);
    assert!(cli.status.success());
    assert_eq!(preview.as_bytes(), cli.stdout);
    // Even a rewrite of the selected file must not change the transfer after preview.
    write(&old, "changed after preview");
    key(&mut app, KeyCode::Char('c'));
    COPIED.with(|copied| assert_eq!(*copied.borrow(), preview));
    key(&mut app, KeyCode::Char('e'));
    assert!(matches!(app.prompt, Some(Prompt::Export { .. })));
    app.prompt = Some(Prompt::Export {
        input: "chosen transcript.md".into(),
    });
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        fs::read_to_string(f.repo.join("chosen transcript.md")).unwrap(),
        preview
    );
    assert!(app.status.as_ref().unwrap().text.contains("Saved Markdown"));
}

#[test]
fn export_requires_overwrite_confirmation_and_survives_clipboard_failure() {
    let f = Fixture::new();
    claude(&f.config.home, "own", &f.repo, 100);
    let mut app = App::new(&f.config, f.scope(), f.repo.clone(), no_clipboard);
    key(&mut app, KeyCode::Enter);
    let preview = app.loaded.as_ref().unwrap().text.clone();
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(app.status.as_ref().unwrap().tone, Tone::Error);
    assert!(
        app.status
            .as_ref()
            .unwrap()
            .text
            .contains("export Markdown")
    );
    let path = f.repo.join("existing.md");
    write(&path, "keep this");
    app.prompt = Some(Prompt::Export {
        input: "existing.md".into(),
    });
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.prompt, Some(Prompt::Overwrite { .. })));
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep this");
    key(&mut app, KeyCode::Char('n'));
    assert!(matches!(app.prompt, Some(Prompt::Export { .. })));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&path).unwrap(), preview);
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Esc);
    assert!(app.prompt.is_none());
    key(&mut app, KeyCode::Char('d'));
    assert_eq!(app.view, View::Diagnostics);
    assert!(
        app.diagnostics
            .iter()
            .any(|l| l.text.contains("Instruction targets"))
    );
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn empty_unsupported_and_broken_stores_are_distinct_and_do_not_hide_other_providers() {
    let f = Fixture::new();
    claude(&f.config.home, "own", &f.repo, 100);
    let root = f.config.home.join(".codex/sessions");
    fs::create_dir_all(&root).unwrap();
    let check = |expected: fn(StoreState) -> bool| {
        let mut app = f.app();
        assert_eq!(app.listing.sessions.len(), 1);
        assert!(expected(app.listing.reports[1].state()));
        key(&mut app, KeyCode::Enter);
        assert!(app.loaded.as_ref().unwrap().text.contains("answer own"));
        key(&mut app, KeyCode::Char('d'));
        assert!(app.diagnostics.iter().any(|l| l.text.contains("log root:")));
    };
    check(|s| matches!(s, StoreState::Empty));
    write(&root.join("unknown.jsonl"), "{\"future_schema\":true}\n");
    check(|s| matches!(s, StoreState::Unsupported));
    fs::remove_file(root.join("unknown.jsonl")).unwrap();
    fs::remove_dir(&root).unwrap();
    write(&root, "not a directory");
    check(|s| matches!(s, StoreState::Unreadable));
}

#[test]
fn metadata_lists_codex_and_gemini_sessions_in_the_same_checkout() {
    let f = Fixture::new();
    records(
        &f.config.home.join(".codex/sessions/a.jsonl"),
        &[
            json!({"type":"session_meta", "payload":{"id":"a", "cwd":f.repo.join("src"), "source":"vscode"}}),
            json!({"type":"response_item", "payload":{"type":"message", "role":"user", "content":"Codex question"}}),
        ],
    );
    records(
        &f.config.home.join(".codex/session_index.jsonl"),
        &[
            json!({"id":"a", "thread_name":"Earlier title"}),
            json!({"id":"a", "thread_name":"Saved Codex title"}),
        ],
    );
    let project = f.config.home.join(".gemini/tmp/project");
    write(&project.join(".project_root"), f.repo.to_str().unwrap());
    records(
        &project.join("chats/session-a.jsonl"),
        &[
            json!({"sessionId":"a", "projectHash":"project"}),
            json!({"id":"1", "type":"user", "content":"Gemini question"}),
        ],
    );
    let listing = sessions::list(
        &f.config,
        &f.scope(),
        ListOptions {
            provider: None,
            titles: true,
        },
    );
    assert_eq!(listing.sessions.len(), 2);
    let codex = listing
        .sessions
        .iter()
        .find(|s| s.reference.provider == "codex")
        .unwrap();
    assert_eq!(codex.title.as_deref(), Some("Saved Codex title"));
    assert_eq!(codex.surface.as_deref(), Some("vscode"));
    for session in &listing.sessions {
        let transcript = transcript::load(&f.config, session, Limits::default()).unwrap();
        assert_eq!(transcript.turns.len(), 1);
    }
}

#[test]
fn no_arguments_in_a_pipe_prints_help_without_terminal_control_codes() {
    let f = Fixture::new();
    let output = f.cli(&[]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Usage:"));
    assert!(text.contains("tail"));
}

#[test]
fn unfamiliar_json_is_reported_as_unsupported_by_every_provider() {
    let f = Fixture::new();
    for path in [
        ".claude/projects/p/future.jsonl",
        ".codex/sessions/future.jsonl",
        ".gemini/tmp/p/chats/session-future.jsonl",
        ".gemini/tmp/p/chats/session-future.json",
    ] {
        records(
            &f.config.home.join(path),
            &[json!({"type":"future-format", "payload":"unknown"})],
        );
    }
    let listing = sessions::list(
        &f.config,
        &Scope::Global,
        ListOptions {
            provider: None,
            titles: true,
        },
    );
    assert!(listing.sessions.is_empty());
    for report in listing.reports {
        assert!(
            matches!(report.state(), StoreState::Unsupported),
            "{}",
            report.provider
        );
    }
}

#[test]
fn legacy_codex_message_only_sessions_remain_readable_globally() {
    let f = Fixture::new();
    records(
        &f.config.home.join(".codex/sessions/legacy.jsonl"),
        &[
            json!({"type":"message", "role":"user", "content":"legacy question"}),
            json!({"type":"message", "role":"assistant", "content":"legacy answer"}),
        ],
    );
    let result = f.cli(&["tail", "codex", "--global", "--json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["turns"][1]["text"], "legacy answer");
    assert!(f.app().listing.sessions.is_empty());
}

#[test]
fn large_legacy_gemini_metadata_does_not_require_loading_the_transcript() {
    let f = Fixture::new();
    let project = f.config.home.join(".gemini/tmp/p");
    write(&project.join(".project_root"), f.repo.to_str().unwrap());
    write(
        &project.join("chats/session-large.json"),
        json!({
            "messages":[{"id":"u", "type":"user", "content":"x".repeat(100_000)}]
        })
        .to_string(),
    );
    let app = f.app();
    assert_eq!(app.listing.sessions.len(), 1);
    assert!(app.loaded.is_none());
    let transcript =
        transcript::load(&f.config, &app.listing.sessions[0], Limits::default()).unwrap();
    assert!(transcript.truncated);
}
