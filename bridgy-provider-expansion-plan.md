# Bridgy provider expansion plan

## Goal

Let developers recover a coding conversation from as many agents as practical, even after the source agent has closed. Keep the existing one-prompt handoff for verified receivers and the TUI picker for selecting a specific conversation. Bridgy remains one Rust binary that reads history on demand.

## Phase 1 · Refactor the provider registry

Reuse the current Claude Code, Codex, and Gemini integrations as regression coverage before adding new providers.

- Give each provider a stable ID, CLI aliases, and one or more session sources. A source discovers sessions and reads recent messages from a specific session. Keep instruction-target resolution independent.
- Track support per client surface and platform. Record three independent capabilities: automatic local discovery, transcript import, and one-prompt handoff. A provider can ship as a readable source before its receiver setup is ready.
- Normalize sessions to a stable provider/source/session reference plus workspace, last activity, title, and client surface when known. Deduplicate only when metadata establishes that two discoveries represent the same session. Filter to the current checkout before sorting. Keep separate Git worktrees separate, and require an explicit global lookup for sessions whose workspace is unknown.
- Keep the TUI and CLI on the same discovery, selection, and rendering code. Load transcript text on selection, preserve the selected session for copy/export, and retain the existing default of 40 user/assistant messages and 48 KiB of message text. Omit tool output, images, and hidden reasoning.
- Resolve only known or explicitly configured history roots for each adapter. Read local databases in read-only mode and leave agent stores untouched. Report missing history, unreadable stores, and unsupported formats separately in `bridgy doctor`. A failing source should leave the other sources usable.

## Phase 2 · Add the first native sources

| Order | Provider | First implementation target |
|---|---|---|
| 1 | GitHub Copilot CLI | Read the documented per-session `events.jsonl` store. Check the workspace metadata in real fixtures and verify personal instructions separately. Treat VS Code Copilot Chat as its own compatibility target. [Configuration directory](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference) |
| 2 | OpenCode | Use documented session list/export to produce test fixtures and validate the normalized transcript. Prefer a verified read-only source for automatic discovery. If direct storage support is pending, ship JSON import first. [OpenCode CLI](https://opencode.ai/docs/cli/commands/) |
| 3 | Cline | Test the current CLI and native VS Code extension. Support older extension history only where fixtures establish the format and workspace mapping. [Cline history locations](https://groundedby.com/agents/cline) |

Accept each adapter independently. Start with the surfaces and platforms actually tested, then expand the matrix as more are verified.

## Phase 3 · Add transcript import

Add `bridgy import <path>` and expose the same action in the TUI. Start with plain-text Markdown and versioned readers for known JSON exports. Plain Markdown gets a bounded text preview. Preserve speaker roles when the format provides them.

- Preview the imported file before transfer. Let the user associate it with the current checkout. Without workspace metadata or an explicit association, treat it as unscoped.
- Keep the original file unchanged. Store only what Bridgy needs to reopen a confirmed import, under its own data directory. Reuse the existing copy and Markdown export actions.
- Make Cursor Markdown and Kilo JSON the first app-specific import examples. Verify Cursor editor and CLI history separately before enabling automatic discovery. [Cursor transcript export](https://forum.cursor.com/t/using-chathistory-on-ssh-sessions/149364) · [Kilo CLI exports](https://kilo.ai/docs/code-with-ai/platforms/cli-reference)

Any agent that can accept pasted text can receive an exported handoff. Managed receiver setup remains opt-in through `bridgy install` and updates only Bridgy's instruction block.

## Phase 4 · Expand in batches

| Batch | Candidates | First thing to verify |
|---|---|---|
| More terminal sources | Pi, Qwen Code, Goose, Aider | Pi branch selection, Qwen Code and Goose export formats, and whether Aider history supports reliable session boundaries. |
| More editor surfaces | Cursor, Kilo Code, JetBrains Junie, Zed | Which histories are local, how to match them to a checkout, and whether a native or existing adapter owns the transcript. |
| Additional local sources | Kiro CLI, Crush, Amp, Antigravity CLI | Supported export or readable local format, host boundaries, and reliable workspace metadata. |
| Legacy import | Roo Code and older Cline/Kilo formats | Recover readable conversations from sanitized fixtures before listing compatibility. |

## Acceptance criteria

- Existing Claude Code, Codex, and Gemini behavior passes regression checks. The TUI and `bridgy tail` still work as before.
- Each new native adapter has a sanitized real transcript fixture with the client version and surface recorded. Test normal messages, malformed or partial data, workspace selection, and exact selection of an older session.
- A handoff from each new source reaches an existing receiver through the TUI or CLI. Verify a one-prompt receiver claim in a fresh session with Bridgy on the receiver's `PATH` and the source app closed.
- A Markdown export from an unsupported agent can be imported, previewed, and carried into another agent. Import and preview do not edit the source.
- The README lists verified surfaces and platforms under **Auto-discover**, **Import**, and **One-prompt handoff**. Update it with each adapter and document how contributors can add a sanitized fixture.

**First release cut:** the shared registry, Copilot CLI, OpenCode, and Cline coverage to the extent verified, plus Markdown import. Ship verified adapters in smaller releases if their platform checks finish at different times.
