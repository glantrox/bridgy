# Expansion fixtures

The JSON specimens are reduced, sanitized extracts of real sessions on macOS. Identifiers, paths, prompts, answers, and timestamps were replaced. The files contain only fields needed to exercise the reader.

| Specimen | Producer and surface | Provenance |
|---|---|---|
| events.jsonl | Copilot agent 1.0.103-unstable.r40031404408.gba6c003, session-state events version 1 | session.start with context.cwd, user.message and assistant.message from the local events file |
| example.json and example.messages.json | Cline 4.1.26, origin.source = vscode, schema version 1 | Paired metadata and messages from the shared sessions store |
| opencode-export.json | OpenCode CLI 1.18.3 | Read-only SQLite backup of the local database into an isolated temporary home, followed by session list and export. Only role, text parts, and session metadata were retained. |

To contribute a native adapter, record the OS, client, surface, exact version, and how you obtained the specimen. Preserve field types and message structure. Replace private paths, IDs, text, repository names, tool arguments, and tool outputs before committing. Remove credentials and unrelated fields. Add tests in tests/expansion.rs for scoped discovery, selecting an older session, tool filtering, partial or invalid files, and transfer to a receiver. Mark each capability in the README only after its own verification.

Batch verification still needs specimens for Pi, Qwen Code, Goose, Aider session boundaries, Cursor native editor/CLI, Kilo native storage, Junie, Zed, Crush, Amp, Antigravity CLI, and legacy Roo/Cline/Kilo formats. On this machine Kiro's CLI directory contained version-v1 JSONL and matching session metadata, but the installed CLI link had no target to establish a producer version. Cursor's project history existed without reliable evidence identifying its originating surface. These native adapters remain pending. Generic Markdown import is available for exported text.
