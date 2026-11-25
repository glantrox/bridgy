# IMPLEMENTATION — Bridgy

## Project and boundaries

Bridgy is a local Rust CLI for on-demand cross-agent conversation context. Agents own session persistence. Bridgy never asks a model to save memory during normal work and never copies logs into a second store. The sole shared configuration source is `~/.bridgy/PROTOCOL.md`.

The package is a single binary and library in a one-member Cargo workspace. Split crates only when independently reusable components justify that boundary. Cargo dependencies are build-time inputs; the binary has no external application runtime. Installation scripts only distribute/invoke the Rust executable.

## Command contract

| Command | Inputs | Effects / output |
|---|---|---|
| no subcommand | optional `--workspace`, `--global` | Browse, preview, copy, export; CLI help if stdin or stdout is not a terminal |
| `install` | detected directories; optional `--providers`, `--link-codex` | Generate protocol if absent, preflight all markers, back up and update instructions; optional new Codex symlink |
| `sync` | canonical protocol; optional `--regenerate` | Reapply existing protocol or explicitly rebuild it from detected providers |
| `tail PROVIDER` | `-n/--turns` (1–10,000, default 40), `--workspace`, `--global`, `--file`, `--json`, `--max-chars`, `--max-bytes` | Read newest session in scope or explicit file; chronological text messages |
| `status` | optional `--json` | Detection, marker/protocol equality, newest session, session bytes/count, traversal warnings |
| `doctor` | current installation | Check sync, malformed markers, readability and permission mode bits; nonzero if problems found |
| `uninstall` | installed files | Strip managed spans; remove canonical protocol; remove empty state directory |

All commands resolve home via `--home` > `BRIDGY_HOME` > `HOME`. Home must be an existing absolute directory. Logs and global instructions use standard provider paths under it. Aliases are case-insensitive. The picker and `tail` resolve the nearest ancestor containing `.git`, including worktree/submodule `.git` files; outside Git they use the current directory. Session working directories use the same resolver. Unknown workspaces never match a checkout. `--workspace` changes the base directory, `--global` disables filtering, and `--file` selects an exact session independently of workspace. These overrides are mutually exclusive.

Normal data is stdout; errors and tail warnings are stderr. JSON modes never mix diagnostics into their stdout. Exit 0 means success, 1 runtime failure, and 2 argument error. Broken stdout pipes exit successfully. `tail` can succeed with an empty message list and a stderr diagnostic if no supported text is present.

## Files and mutation

`config.rs` handles UTF-8 instruction reads (bounded to 4 MiB), exact marker spans, private random backup files, staged writes, and atomic replacement. Each file preserves content outside its marker span and retains existing permission bits. New files and backups use tempfile's private permissions. Instruction symlink targets are resolved and shared targets are deduplicated. A dangling link is never silently overwritten.

`~/.bridgy.lock` uses an OS file lock for install/sync/uninstall. The empty lock file persists after uninstall to avoid changing the lock inode while another process may be waiting. A symlink at the lock or state directory is rejected. This is cooperative concurrency protection, not a defense against a hostile process with the same filesystem privileges.

Installation preflights every marker before writes. Backups precede atomic replacement of each changed existing file. An unchanged file creates no backup. Multi-file changes are deliberately not described as transactional: failure can leave a partially synced installation, and rerunning sync repairs it after the underlying error is fixed.

The default keeps global instruction files independent. `--link-codex` explicitly creates a missing Codex instruction file as a relative symlink to Claude's. Existing distinct files or links to another target are rejected; shared links are idempotent. Uninstall preserves links, all user content, logs, binary, and backups; it does not recursively remove unknown files from the state directory.

`sync` preserves edits to the central protocol. `sync --regenerate` replaces it with a generated template, with a backup. This explicit choice prevents a newly detected provider from silently destroying local protocol customizations.

## Provider API

Each product implements `Provider` with its ID, aliases, directory, instruction filename, and one or more `SessionSource` adapters. A source supplies its root, candidate predicate, bounded metadata description, optional title lookup, and exact-file transcript reader. Compatible terminal/editor clients share a source. Adding a provider requires one module and one registry entry; CLI dispatch and protocol generation remain generic.

Discovery walks directory entries iteratively and skips nested symlinks. The `status` inventory inspects filesystem metadata; session listing additionally reads bounded headers and optional titles. Candidate files are compared by modification time, then full path to break ties. Claude's nested subagent logs are excluded. Store reports distinguish missing, empty, unsupported, unreadable, and no sessions in this checkout; a failed provider does not hide another provider's sessions.

`sessions::list` returns metadata including a stable provider/path reference. `sessions::read` reads that exact reference. `tail` chooses the newest matching reference, while the picker keeps the selected one even when a newer file appears. `transcript.rs` enforces message-text bounds and renders the same Markdown for CLI, preview, clipboard, and export. The TUI caches the preview so later writes to the selected log cannot change the text being transferred.

## Terminal browser

`tui/app.rs` owns selection, preview, scroll, diagnostics, and export prompts. A clipboard function is injected for deterministic transfer tests; real clipboard access uses a platform helper and reports failure inline with a Markdown-export fallback. Export accepts a user-edited path, confirms replacement, and uses exclusive creation for new files. Confirmed overwrites use the existing atomic writer. Diagnostics share `doctor::checks` with the CLI.

`tui/mod.rs` owns terminal initialization and its RAII restoration guard. Ratatui also restores the terminal before reporting panics. Only interactive stdin and stdout enter raw mode and the alternate screen; ordinary subcommands keep normal stdout/stderr. `tui/ui.rs` renders the views and clamps scrolling after resize. Metadata scans and bounded reads are synchronous; large stores can delay input while loading.

## Reverse reader

`tail.rs` snapshots EOF with `SeekFrom::End(0)`. It reads 8 KiB blocks backward, assembles bytes up to each newline, reverses a completed line, and parses JSON. Accumulating bytes in reverse avoids repeatedly copying a growing prefix for long records. UTF-8 sequences split between blocks remain intact because decoding occurs only after line assembly.

The reader stops after N normalized messages, not N JSONL lines. It handles optional final newlines, blank lines, CRLF, malformed records, UTF-8, and large records across chunk boundaries. It scans at most `--max-bytes` (64 MiB by default), drops records above 16 MiB with a warning, and never parses an incomplete prefix produced by hitting the byte limit. Message IDs suppress older duplicate records within the scanned region. Output is reversed once to restore chronology. A test asserts bytes read on a sparse, large file to verify it does not scan from byte zero.

Claude normalization extracts `message.content` from top-level `user`/`assistant` records. Codex normalization extracts `response_item` payloads of type `message` and legacy direct messages; mirrored `event_msg` records are ignored. Content extraction admits plain text and `text`/`input_text`/`output_text` parts, excluding tools, images, thoughts, and Codex analysis-channel messages. There is no promise to replay every provider's conversation branching semantics; these readers present the most recent saved textual records.

## Gemini exception

Gemini discovery only accepts `session-*.json`/`session-*.jsonl` directly inside `chats` directories, avoiding unrelated temporary JSON files. Legacy JSON uses a `messages` array. Current JSONL can mutate earlier messages through duplicate IDs, `$patch` updates/removals/orderings, `$set.messages` checkpoints, and `$rewindTo` records. Tail-only decoding would return stale or revoked text.

The Gemini adapter therefore reads and replays a bounded complete file before selecting N recent messages. Its memory and I/O are proportional to the bounded session size; it has no reverse-tail performance claim. It rejects a file above the byte limit and checks the size again after reading to catch growth. Malformed records are skipped with an explicit incomplete-replay warning. Unknown extensions can require future parser updates.

Schema reference: [Gemini recording types](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/chatRecordingTypes.ts). Replay semantics reference: [Gemini recording service](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/chatRecordingService.ts).

## Distribution and verification

`install.sh` supports local binaries, explicit source builds, or checksummed GitHub release downloads selected by OS/architecture. The actual repository is provided via `BRIDGY_REPO`; no nonexistent canonical release URL is embedded. Downloads are HTTPS-only, verified before execution, and removed on exit. The shell installer stages only the executable into a user-writable bin directory and prints exact PATH setup commands when needed. The PowerShell installer offers the same separation on Windows. Neither installer touches provider instructions. Only the Rust `bridgy install` command enables handoffs.

CI runs format, check, process/unit tests, Clippy, and release builds on macOS/Linux/Windows, with shell syntax checks on Unix. Release automation targets Apple Intel/ARM, Linux Intel/ARM musl, and Windows x64. It generates Homebrew and WinGet definitions from real artifact hashes; see packaging/README.md. Publishing and cross-target verification require the hosted workflows to run; no release is published by creating these files locally.

Tests use temporary homes and synthetic fixtures. They exercise provider parsing, corruption, newest-file selection, byte bounds, JSON/stdout/exit contracts, idempotence, backups, exact user-rule preservation, custom protocol sync, symlink safety, concurrent-writer rejection, and installation. No test reads real user histories or changes real global instructions.
