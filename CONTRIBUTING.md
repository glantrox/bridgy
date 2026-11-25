# Contributing to Bridgy

Bug reports, fixes, documentation improvements, and examples of session formats all help. Everyone participating in the project is expected to follow the [code of conduct](CODE_OF_CONDUCT.md).

## Report a bug or suggest a feature

For a bug, include your operating system, `bridgy --version`, the command or keys you used, and what you expected to happen. If a conversation is missing, include the agent version and whether you used its CLI or editor extension. For a feature request, describe the task you wanted to finish. For a new handoff, name the agent you started in and the one you wanted to continue in. [Open an issue](https://github.com/glantrox/bridgy/issues)

Use invented messages and paths in public examples. Report suspected vulnerabilities privately through the channel in the [security policy](SECURITY.md).

## Run Bridgy from source

Use stable Rust with Cargo. Fork the repository, clone your fork, and run these commands from the repository root:

```sh
cargo run --locked -- --version
cargo run --locked
```

The second command opens the browser in an interactive terminal. To browse sessions in another checkout, run `cargo run --locked -- --workspace "/path/to/project"`. When testing commands that edit agent instructions, pass `--home` with the absolute path to an existing temporary directory.

## Add support for a session format

Start with a small, sanitized fixture. Record the agent and version that produced it, the operating system, whether it came from a CLI or editor, and how you obtained it. Preserve the fields that distinguish conversations and identify their checkout. [Provider fixture guide](tests/fixtures/expansion/README.md)

Exercise the handoff a user will try. Create two conversations in one checkout and a third in another. Select the older conversation and verify that preview, copy, and export all use that session. Cover tool-output filtering and partial or malformed records. When documenting support, say whether Bridgy discovers sessions automatically or reads an explicitly imported file.

## Open a pull request

Create a branch for your change and open a pull request against `main`. Describe the user-visible change and how you verified it. For a TUI change, include the terminal size and keys needed to try it.

Run the checks relevant to your change. CI runs the Rust suite on Linux, macOS, and Windows. [CI workflow](.github/workflows/ci.yml)

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

For packaging changes, run the following with Python 3.12. For shell installer changes, also run `sh -n install.sh` on macOS or Linux.

```sh
python -m unittest discover -s scripts -p "test_*.py"
```
