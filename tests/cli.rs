use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bridgy"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env_remove("BRIDGY_HOME")
        .output()
        .unwrap()
}
fn ok(home: &Path, args: &[&str]) -> Output {
    let result = run(home, args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}
fn write(path: &Path, content: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
fn lines(values: &[Value]) -> String {
    values.iter().map(|v| format!("{v}\n")).collect()
}
fn tail(home: &Path, provider: &str) -> Value {
    // Fixtures without workspace metadata are only visible across all workspaces.
    serde_json::from_slice(&ok(home, &["tail", provider, "--json", "-n", "2", "--global"]).stdout)
        .unwrap()
}

#[test]
fn help_version_usage_and_home_precedence() {
    let temp = TempDir::new().unwrap();
    assert!(String::from_utf8_lossy(&ok(temp.path(), &["--help"]).stdout).contains("uninstall"));
    assert!(String::from_utf8_lossy(&ok(temp.path(), &["--version"]).stdout).contains("0.1.0"));
    for args in [
        vec!["tail"],
        vec!["--bad"],
        vec!["tail", "claude", "-n", "0"],
    ] {
        let result = run(temp.path(), &args);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.is_empty());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_bridgy"))
        .env("BRIDGY_HOME", "/does/not/exist")
        .arg("--home")
        .arg(temp.path())
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let result = run(temp.path(), &["tail", "unknown"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
}

#[test]
fn lifecycle_preserves_rules_custom_protocol_backups_and_logs() {
    let temp = tempfile::Builder::new()
        .prefix("bridgy home ")
        .tempdir()
        .unwrap();
    let claude = temp.path().join(".claude/CLAUDE.md");
    let codex = temp.path().join(".codex/AGENTS.md");
    write(&claude, "# Keep my rules\n");
    write(&codex, "codex-specific rules");
    let log = temp.path().join(".claude/projects/p/session.jsonl");
    write(
        &log,
        lines(&[json!({"type":"user", "message":{"content":"hello"}})]),
    );
    ok(temp.path(), &["install"]);
    let first = fs::read(&claude).unwrap();
    let backups = || {
        fs::read_dir(claude.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("bridgy-backup-"))
            .count()
    };
    assert_eq!(backups(), 1);
    ok(temp.path(), &["install"]);
    ok(temp.path(), &["sync"]);
    assert_eq!(first, fs::read(&claude).unwrap());
    assert_eq!(backups(), 1);
    let central = temp.path().join(".bridgy/PROTOCOL.md");
    write(&central, "# My custom passive protocol\n");
    ok(temp.path(), &["sync"]);
    assert!(
        fs::read_to_string(&codex)
            .unwrap()
            .contains("My custom passive protocol")
    );
    ok(temp.path(), &["doctor"]);
    ok(temp.path(), &["uninstall"]);
    assert_eq!(fs::read_to_string(&claude).unwrap(), "# Keep my rules\n");
    assert_eq!(fs::read_to_string(&codex).unwrap(), "codex-specific rules");
    assert!(log.exists());
    assert!(backups() >= 2);
    assert!(!central.exists());
    ok(temp.path(), &["uninstall"]);
}

#[test]
fn preflight_rejects_invalid_markers_before_writing_any_provider() {
    let temp = TempDir::new().unwrap();
    let claude = temp.path().join(".claude/CLAUDE.md");
    let codex = temp.path().join(".codex/AGENTS.md");
    write(&claude, "keep");
    write(&codex, "<!-- BRIDGY:START -->\nbroken");
    assert!(!run(temp.path(), &["install"]).status.success());
    assert_eq!(fs::read_to_string(&claude).unwrap(), "keep");
    assert!(!temp.path().join(".bridgy/PROTOCOL.md").exists());
    assert!(!run(temp.path(), &["uninstall"]).status.success());
}

#[test]
fn claude_filters_tools_and_corruption_and_reads_chronologically() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join(".claude/projects/project/session.jsonl");
    let mut text = lines(&[
        json!({"type":"user","uuid":"1","message":{"content":"first"}}),
        json!({"type":"assistant","uuid":"2","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"second"},{"type":"tool_use","name":"shell"}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","content":"do not include"}]}}),
        json!({"type":"assistant","uuid":"3","message":{"content":"こんにちは"}}),
        json!({"type":"assistant","uuid":"3","message":{"content":"こんにちは"}}),
    ]);
    text.push_str("{partial");
    write(&path, text);
    let value = tail(temp.path(), "claude");
    assert_eq!(value["turns"].as_array().unwrap().len(), 2);
    assert_eq!(value["turns"][0]["text"], "second");
    assert_eq!(value["turns"][1]["text"], "こんにちは");
    assert_eq!(value["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn codex_uses_response_items_without_event_duplicates_or_analysis() {
    let temp = TempDir::new().unwrap();
    write(
        &temp.path().join(".codex/sessions/2026/11/rollout.jsonl"),
        lines(&[
            json!({"type":"session_meta","payload":{"cwd":"/project"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"question"}]}}),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"question"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"type":"output_text","text":"internal"}]}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"tool"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"final","content":[{"type":"output_text","text":"answer"}]}}),
        ]),
    );
    let value = tail(temp.path(), "chatgpt");
    assert_eq!(value["provider"], "codex");
    assert_eq!(value["turns"][0]["text"], "question");
    assert_eq!(value["turns"][1]["text"], "answer");
}

#[test]
fn gemini_replays_patches_rewinds_and_legacy_json() {
    let temp = TempDir::new().unwrap();
    let path = temp
        .path()
        .join(".gemini/tmp/project/chats/session-current.jsonl");
    write(
        &path,
        lines(&[
            json!({"sessionId":"s","projectHash":"p"}),
            json!({"id":"u","type":"user","content":"question"}),
            json!({"id":"a","type":"gemini","content":"draft"}),
            json!({"$patch":{"id":"a","content":[{"text":"answer"},{"thought":true,"text":"internal"}]}}),
            json!({"id":"b","type":"user","content":"rewound"}),
            json!({"$rewindTo":"b"}),
        ]),
    );
    let value = tail(temp.path(), "gemini");
    assert_eq!(value["turns"][0]["text"], "question");
    assert_eq!(value["turns"][1]["text"], "answer");
    let legacy = temp.path().join("legacy.json");
    write(
        &legacy,
        json!({"messages":[{"id":"1","type":"user","content":"legacy"}]}).to_string(),
    );
    let result = ok(
        temp.path(),
        &[
            "tail",
            "gemini",
            "--file",
            legacy.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["turns"][0]["text"],
        "legacy"
    );
    assert!(
        !run(
            temp.path(),
            &["tail", "gemini", "--max-bytes", "10", "--global"]
        )
        .status
        .success()
    );
}

#[test]
fn newest_selection_uses_mtime_and_status_reports_session_size() {
    use std::time::{Duration, SystemTime};
    let temp = TempDir::new().unwrap();
    let old = temp.path().join(".claude/projects/z/old.jsonl");
    let new = temp.path().join(".claude/projects/a/new.jsonl");
    for path in [&old, &new] {
        write(
            path,
            lines(&[json!({"type":"user","message":{"content":"hello"}})]),
        );
    }
    let old_time = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(old_time)
        .unwrap();
    let value = tail(temp.path(), "claude");
    assert_eq!(Path::new(value["session"].as_str().unwrap()), new.as_path());
    let status: Value =
        serde_json::from_slice(&ok(temp.path(), &["status", "--json"]).stdout).unwrap();
    assert_eq!(status[0]["session_files"], 2);
    assert_eq!(
        status[0]["session_bytes"].as_u64().unwrap(),
        fs::metadata(old).unwrap().len() + fs::metadata(new).unwrap().len()
    );
}

#[cfg(unix)]
#[test]
fn existing_shared_instruction_symlink_is_preserved() {
    let temp = TempDir::new().unwrap();
    let claude = temp.path().join(".claude/CLAUDE.md");
    let codex = temp.path().join(".codex/AGENTS.md");
    write(&claude, "original\n");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&claude, &codex).unwrap();
    ok(temp.path(), &["install"]);
    assert!(codex.is_symlink());
    assert_eq!(
        fs::read_to_string(&claude)
            .unwrap()
            .matches("<!-- BRIDGY:START -->")
            .count(),
        1
    );
    ok(temp.path(), &["uninstall"]);
    assert!(codex.is_symlink());
    assert_eq!(fs::read_to_string(claude).unwrap(), "original\n");
}

#[cfg(unix)]
#[test]
fn installer_handles_spaces_and_leaves_configs_untouched_on_invalid_input() {
    let temp = tempfile::Builder::new()
        .prefix("installer home ")
        .tempdir()
        .unwrap();
    let installed = Command::new("sh")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
        .args(["--binary", env!("CARGO_BIN_EXE_bridgy")])
        .env("BRIDGY_HOME", temp.path())
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(temp.path().join(".local/bin/bridgy").exists());
    assert!(!temp.path().join(".claude").exists());
    assert!(!temp.path().join(".codex").exists());
    assert!(!temp.path().join(".bridgy").exists());
    // Handoff setup remains a separate, explicit action.
    ok(temp.path(), &["install", "--providers", "claude,codex"]);
    ok(temp.path(), &["doctor"]);
    let invalid = Command::new("sh")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
        .arg("--binary")
        .env("BRIDGY_HOME", temp.path())
        .output()
        .unwrap();
    assert!(!invalid.status.success());
}

#[cfg(unix)]
#[test]
fn installer_path_commands_work_in_fresh_shells_outside_the_repo() {
    for shell in ["/bin/bash", "/bin/zsh"] {
        if !Path::new(shell).exists() {
            continue;
        }
        let temp = tempfile::Builder::new()
            .prefix("installer '$dollar `literal` ")
            .tempdir()
            .unwrap();
        let home = temp.path();
        let destination = home.join("custom bin");
        let environment = |cmd: &mut Command| {
            cmd.env("HOME", home)
                .env("BRIDGY_HOME", home)
                .env("SHELL", shell)
                .env_remove("ZDOTDIR")
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        };
        let mut installer = Command::new("sh");
        environment(&mut installer);
        let output = installer
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .args(["--binary", env!("CARGO_BIN_EXE_bridgy"), "--bin-dir"])
            .arg(&destination)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("not on PATH"));
        // Execute exactly the persisted setup commands the installer prints.
        for setup in stdout.lines().filter(|line| line.starts_with("  printf ")) {
            let mut command = Command::new("sh");
            environment(&mut command);
            assert!(command.args(["-c", setup]).status().unwrap().success());
        }
        for mode in ["-ic", "-lic"] {
            let mut command = Command::new(shell);
            environment(&mut command);
            let version = command
                .current_dir(home)
                .args([mode, "bridgy --version"])
                .output()
                .unwrap();
            assert!(
                version.status.success(),
                "{}",
                String::from_utf8_lossy(&version.stderr)
            );
            assert!(String::from_utf8_lossy(&version.stdout).contains("bridgy 0.1.0"));
        }
        assert!(!home.join(".bridgy").exists());
        assert!(!home.join(".claude").exists());
        let path = format!("{}:/usr/bin:/bin", destination.display());
        let again = installer.env("PATH", path).output().unwrap();
        assert!(again.status.success());
        let stdout = String::from_utf8(again.stdout).unwrap();
        assert!(stdout.contains("already on PATH"));
        assert!(!stdout.contains("export PATH="));
    }
}

#[cfg(windows)]
#[test]
fn windows_manual_install_copies_command_without_enabling_handoffs() {
    let temp = TempDir::new().unwrap();
    let destination = temp.path().join("custom bin");
    let output = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.ps1"))
        .args(["-Binary", env!("CARGO_BIN_EXE_bridgy"), "-BinDir"])
        .arg(&destination)
        .env("BRIDGY_HOME", temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("SetEnvironmentVariable"));
    let version = Command::new(destination.join("bridgy.exe"))
        .arg("--version")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(!temp.path().join(".bridgy").exists());
}

#[test]
fn refuses_concurrent_config_writes_and_reports_missing_sessions() {
    let temp = TempDir::new().unwrap();
    let config = bridgy::config::Config::resolve(Some(temp.path().to_owned())).unwrap();
    let _lock = config.lock().unwrap();
    let output = run(temp.path(), &["install", "--providers", "claude"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("another Bridgy"));
    let output = run(temp.path(), &["tail", "claude"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("claude: no session store at"));
}

#[cfg(unix)]
#[test]
fn requested_codex_link_is_idempotent_and_never_replaces_rules() {
    let temp = TempDir::new().unwrap();
    ok(temp.path(), &["install", "--link-codex"]);
    let codex = temp.path().join(".codex/AGENTS.md");
    assert!(codex.is_symlink());
    ok(temp.path(), &["install", "--link-codex"]);
    ok(temp.path(), &["doctor"]);
    ok(temp.path(), &["uninstall"]);
    assert!(codex.is_symlink());
    let other = TempDir::new().unwrap();
    let existing = other.path().join(".codex/AGENTS.md");
    write(&existing, "keep these rules");
    assert!(
        !run(other.path(), &["install", "--link-codex"])
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(existing).unwrap(), "keep these rules");
    assert!(!other.path().join(".claude/CLAUDE.md").exists());
}

#[test]
fn sparse_large_session_reads_only_the_final_chunk() {
    use std::io::{Seek, SeekFrom, Write};
    let temp = TempDir::new().unwrap();
    let path = temp.path().join(".claude/projects/large/session.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = fs::File::create(&path).unwrap();
    file.set_len(256 * 1024 * 1024).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    writeln!(
        file,
        "\n{}",
        json!({"type":"user","message":{"content":"recent message"}})
    )
    .unwrap();
    let output = ok(
        temp.path(),
        &["tail", "claude", "--json", "-n", "1", "--global"],
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["turns"][0]["text"], "recent message");
    assert!(result["bytes_read"].as_u64().unwrap() <= 8192);
    assert_eq!(result["warnings"], json!([]));
}

#[test]
fn gemini_checkpoints_updates_removals_and_ordering() {
    let temp = TempDir::new().unwrap();
    write(
        &temp.path().join(".gemini/tmp/p/chats/session-replay.jsonl"),
        lines(&[
            json!({"id":"old","type":"user","content":"obsolete"}),
            json!({"$set":{"messages":[
                {"id":"a","type":"user","content":"first"},
                {"id":"b","type":"gemini","content":"draft"},
                {"id":"c","type":"user","content":"removed"}
            ]}}),
            json!({"$patch":{"updates":[{"id":"b","content":{"text":"updated"}}],"removeIds":["c"],"orderIds":["b","a"]}}),
        ]),
    );
    let result = tail(temp.path(), "gemini");
    assert_eq!(result["turns"][0]["text"], "updated");
    assert_eq!(result["turns"][1]["text"], "first");
}

#[cfg(unix)]
#[test]
fn discovery_skips_symlink_loops_and_config_rejects_dangling_links() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join(".claude/projects");
    write(
        &root.join("p/session.jsonl"),
        lines(&[json!({"type":"user","message":{"content":"hello"}})]),
    );
    std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
    assert_eq!(tail(temp.path(), "claude")["turns"][0]["text"], "hello");
    let instruction = temp.path().join(".claude/CLAUDE.md");
    std::os::unix::fs::symlink(temp.path().join("nonexistent"), &instruction).unwrap();
    assert!(!run(temp.path(), &["install"]).status.success());
    assert!(instruction.is_symlink());
    assert!(!temp.path().join("nonexistent").exists());
}

#[cfg(unix)]
#[test]
fn release_installer_verifies_download_before_running_it() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let fake_bin = temp.path().join("tools");
    let curl = fake_bin.join("curl");
    write(
        &curl,
        r#"#!/bin/sh
set -eu
for arg in "$@"; do
    case "$arg" in https://*) url=$arg ;; esac
done
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then output=$2; break; fi
    shift
done
case "$url" in
    *.sha256)
        if [ "${MOCK_CORRUPT:-}" = 1 ]; then
            printf '%064d  binary\n' 0 > "$output"
        elif command -v sha256sum >/dev/null 2>&1; then
            sha256sum "$MOCK_BINARY" > "$output"
        else
            shasum -a 256 "$MOCK_BINARY" > "$output"
        fi ;;
    *) cp "$MOCK_BINARY" "$output" ;;
esac
"#,
    );
    fs::set_permissions(curl, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(fake_bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for corrupt in [false, true] {
        let home = temp.path().join(if corrupt {
            "bad download"
        } else {
            "good download"
        });
        fs::create_dir(&home).unwrap();
        let output = Command::new("sh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .env("PATH", &path)
            .env("BRIDGY_HOME", &home)
            .env("BRIDGY_REPO", "test/bridgy")
            .env("MOCK_BINARY", env!("CARGO_BIN_EXE_bridgy"))
            .env("MOCK_CORRUPT", if corrupt { "1" } else { "0" })
            .output()
            .unwrap();
        if corrupt {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
            assert!(!home.join(".claude").exists());
            assert!(!home.join(".local/bin/bridgy").exists());
        } else {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(home.join(".local/bin/bridgy").exists());
            assert!(!home.join(".claude").exists());
            assert!(!home.join(".bridgy").exists());
        }
    }
}
