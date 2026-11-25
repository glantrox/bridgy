use bridgy::{
    config::Config,
    imports, providers,
    sessions::{self, ListOptions},
    transcript::Limits,
    tui::app::{App, Prompt, View},
    workspace::Scope,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

const COPILOT: &str = include_str!("fixtures/expansion/events.jsonl");
const CLINE_META: &str = include_str!("fixtures/expansion/example.json");
const CLINE_MSG: &str = include_str!("fixtures/expansion/example.messages.json");
const OPENCODE: &str = include_str!("fixtures/expansion/opencode-export.json");

fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn key(app: &mut App, code: KeyCode) {
    app.handle(KeyEvent::new(code, KeyModifiers::NONE));
}
fn config(tmp: &TempDir) -> Config {
    Config::resolve(Some(tmp.path().into())).unwrap()
}

#[test]
fn exact_reference_scopes_and_reads_fixture_sources_independently() {
    let tmp = TempDir::new().unwrap();
    let c = config(&tmp);
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let workspace = serde_json::to_string(&repo).unwrap();
    for name in ["older", "other"] {
        let cwd = if name == "older" {
            workspace.clone()
        } else {
            serde_json::to_string(tmp.path()).unwrap()
        };
        let events = COPILOT
            .replace("\"/example/project\"", &cwd)
            .replace("example", name);
        write(
            &tmp.path()
                .join(format!(".copilot/session-state/{name}/events.jsonl")),
            &format!("{events}{{partial"),
        );
        let root = tmp.path().join(format!(".cline/data/sessions/{name}"));
        write(
            &root.join(format!("{name}.json")),
            &CLINE_META
                .replace("\"/example/project\"", &cwd)
                .replace("\"example\"", &format!("\"{name}\"")),
        );
        write(
            &root.join(format!("{name}.messages.json")),
            &CLINE_MSG.replace("\"example\"", &format!("\"{name}\"")),
        );
    }
    let scope = Scope::resolve(&repo, None, false).unwrap();
    for id in ["copilot", "cline"] {
        let provider = providers::find(id).unwrap();
        let listing = sessions::list(
            &c,
            &scope,
            ListOptions {
                provider: Some(&*provider),
                titles: true,
            },
        );
        assert_eq!(listing.sessions.len(), 1);
        let selected = &listing.sessions[0];
        assert_eq!(selected.id, "older");
        if id == "copilot" {
            let newest = tmp
                .path()
                .join(".copilot/session-state/newest/events.jsonl");
            write(
                &newest,
                &COPILOT
                    .replace("\"/example/project\"", &workspace)
                    .replace("Check the tests", "different prompt"),
            );
        } else {
            let dir = tmp.path().join(".cline/data/sessions/newest");
            write(
                &dir.join("newest.json"),
                &CLINE_META
                    .replace("\"/example/project\"", &workspace)
                    .replace("\"example\"", "\"newest\""),
            );
            write(
                &dir.join("newest.messages.json"),
                &CLINE_MSG
                    .replace("\"example\"", "\"newest\"")
                    .replace("user fixture text", "different prompt"),
            );
        }
        let turns = sessions::read(&c, &selected.reference, 40, 1 << 20).unwrap();
        assert_eq!(turns.turns.len(), 2);
        assert_eq!(turns.turns[0].role, "user");
        assert_eq!(turns.turns[1].role, "assistant");
        assert!(!turns.turns[0].text.contains("different"));
        if id == "copilot" {
            assert!(!turns.warnings.is_empty());
        }
        let out = Command::new(env!("CARGO_BIN_EXE_bridgy"))
            .env("BRIDGY_HOME", tmp.path())
            .current_dir(&repo)
            .args(["tail", id, "--file"])
            .arg(&selected.reference.path)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains(&turns.turns[1].text));
    }
    let root = tmp.path().join(".cline/data/sessions/older");
    write(&root.join("older.messages.json"), "{");
    let provider = providers::find("cline").unwrap();
    let old = sessions::describe_file(&c, &*provider, &root.join("older.messages.json"));
    assert!(sessions::read(&c, &old.reference, 40, 1 << 20).is_err());
    let setup = Command::new(env!("CARGO_BIN_EXE_bridgy"))
        .env("BRIDGY_HOME", tmp.path())
        .args(["install", "--providers", "copilot"])
        .output()
        .unwrap();
    assert!(!setup.status.success());
    assert!(!tmp.path().join(".copilot/copilot-instructions.md").exists());
}

#[test]
fn confirmed_imports_reopen_without_copying_source_text() {
    let temp = TempDir::new().unwrap();
    let c = config(&temp);
    let path = temp.path().join("Cursor export.md");
    let text = "**User**\n\nQuestion\n\n**Cursor**\n\nAnswer";
    write(&path, text);
    let mut app = App::new(
        &c,
        Scope::resolve(temp.path(), None, false).unwrap(),
        temp.path().into(),
        |s| {
            assert!(s.ends_with("Answer"));
            Ok("test clipboard")
        },
    );
    key(&mut app, KeyCode::Char('i'));
    assert!(matches!(app.prompt, Some(Prompt::Import { .. })));
    for ch in "Cursor export.md".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.view, View::Preview);
    assert_eq!(app.loaded.as_ref().unwrap().text, text);
    assert!(!c.state_dir().exists());
    key(&mut app, KeyCode::Char('c'));
    key(&mut app, KeyCode::Char('e'));
    app.prompt = Some(Prompt::Export {
        input: "transferred.md".into(),
    });
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        fs::read_to_string(temp.path().join("transferred.md")).unwrap(),
        text
    );
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.selected_session().unwrap().reference.provider, "import");
    let record = &app.selected_session().unwrap().reference.path;
    assert!(!fs::read_to_string(record).unwrap().contains("Question"));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.loaded.unwrap().text, text);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn cli_import_scope_limits_and_json_role_filtering() {
    let tmp = TempDir::new().unwrap();
    let c = config(&tmp);
    let md = tmp.path().join("large.md");
    write(&md, &"é".repeat(80_000));
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_bridgy"))
            .current_dir(tmp.path())
            .env("BRIDGY_HOME", tmp.path())
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&["import", "large.md", "--save"]);
    assert!(output.status.success());
    assert!(output.stdout.len() <= 48 * 1024);
    assert!(!run(&["tail", "import"]).status.success());
    assert_eq!(run(&["tail", "import", "--global"]).stdout, output.stdout);
    let json = tmp.path().join("export.json");
    let mut v: serde_json::Value = serde_json::from_str(OPENCODE).unwrap();
    v["messages"][1]["parts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"type":"reasoning","text":"omitted"}));
    write(&json, &v.to_string());
    let (_, t) = imports::preview(&json, Limits::default()).unwrap();
    assert_eq!(t.turns.len(), 2);
    assert_eq!(t.turns[1].role, "assistant");
    assert!(!t.turns.iter().any(|t| t.text.contains("omitted")));
    write(&json, "{}");
    assert!(imports::preview(&json, Limits::default()).is_err());
    let src = providers::find("import").unwrap();
    assert_eq!(
        sessions::list(
            &c,
            &Scope::Global,
            ListOptions {
                provider: Some(&*src),
                titles: true
            }
        )
        .sessions
        .len(),
        1
    );
}
