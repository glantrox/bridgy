use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    process::ExitCode,
};

use bridgy::{
    commands::{self, tail_cmd},
    config::Config,
    transcript::{DEFAULT_MAX_BYTES, DEFAULT_MAX_CHARS, Limits},
    tui,
    workspace::Scope,
};
use clap::{CommandFactory, Parser, Subcommand};

/// Run without a command in a terminal to browse this checkout's sessions.
#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "Run `bridgy` with no command in an interactive terminal to pick, preview, copy, or export a session from the current checkout."
)]
struct Cli {
    /// Override the home used for all provider and Bridgy paths
    #[arg(long, global = true, env = "BRIDGY_HOME")]
    home: Option<PathBuf>,
    /// Browse sessions from this checkout instead of the current one
    #[arg(long, global = true, conflicts_with = "global")]
    workspace: Option<PathBuf>,
    /// Browse sessions from every workspace
    #[arg(long, global = true)]
    global: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Preview a Markdown or supported JSON export. Add --save to remember its path.
    Import {
        path: PathBuf,
        #[arg(long)]
        save: bool,
        /// Associate the saved reference with the selected checkout
        #[arg(long, requires = "save", conflicts_with = "global")]
        here: bool,
    },
    /// Detect providers, back up instructions, and install the shared protocol
    Install {
        /// Set up these providers even if their directories do not exist
        #[arg(long, value_delimiter = ',')]
        providers: Vec<String>,
        /// Link a NEW Codex instruction file to Claude's; never replace an existing file
        #[arg(long)]
        link_codex: bool,
    },
    /// Copy the central protocol into detected providers' instruction files
    Sync {
        /// Rebuild the central protocol for currently detected providers
        #[arg(long)]
        regenerate: bool,
    },
    /// Print recent user/assistant text from the newest session in the current checkout
    Tail {
        /// Provider ID, e.g. claude, codex, gemini, copilot, cline, or import
        provider: String,
        /// Number of individual messages, in chronological order
        #[arg(short = 'n', long, default_value_t = 40, value_parser = clap::value_parser!(u32).range(1..=10000))]
        turns: u32,
        /// Emit a structured JSON object
        #[arg(long)]
        json: bool,
        /// Read a specific session instead of discovering the newest one
        #[arg(long, conflicts_with_all = ["workspace", "global"])]
        file: Option<PathBuf>,
        /// Maximum characters of message text; the oldest messages are dropped first
        #[arg(long, default_value_t = DEFAULT_MAX_CHARS, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(256..))]
        max_chars: usize,
        /// Maximum log bytes to scan; Gemini replay must fit within this limit
        #[arg(long, default_value_t = DEFAULT_MAX_BYTES, value_parser = clap::value_parser!(u64).range(1..))]
        max_bytes: u64,
    },
    /// Show detection, sync state, newest sessions, and session bytes on disk
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Check instruction markers, permissions, and log readability
    Doctor,
    /// Remove managed blocks and Bridgy state; retain backups and the executable
    Uninstall,
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let config = Config::resolve(cli.home)?;
    let Some(command) = cli.command else {
        // Only a real terminal gets the TUI; pipes and agents get help, never escape codes.
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            let cwd = std::env::current_dir()?;
            let scope = Scope::resolve(&cwd, cli.workspace.as_deref(), cli.global)?;
            return tui::run(&config, scope, cwd);
        }
        Cli::command().print_help()?;
        return Ok(());
    };
    let mut out = io::stdout().lock();
    match command {
        Command::Import { path, save, here } => {
            use std::io::Write;
            let (bookmark, preview) = bridgy::imports::preview(&path, Limits::default())?;
            out.write_all(preview.markdown().as_bytes())?;
            out.flush()?;
            if save {
                let scope = Scope::current(cli.workspace.as_deref(), false)?;
                bookmark.save(
                    &config,
                    if here || cli.workspace.is_some() {
                        scope.root()
                    } else {
                        None
                    },
                )?;
                eprintln!("Remembered {}", bookmark.path.display());
            }
            Ok(())
        }
        Command::Install {
            providers,
            link_codex,
        } => commands::install::run(&config, &providers, link_codex, &mut out),
        Command::Sync { regenerate } => commands::sync::run(&config, regenerate, &mut out),
        Command::Tail {
            provider,
            turns,
            json,
            file,
            max_chars,
            max_bytes,
        } => tail_cmd::run(
            &config,
            tail_cmd::Options {
                provider: &provider,
                limits: Limits {
                    messages: turns as usize,
                    max_chars,
                    max_bytes,
                },
                json,
                file: file.as_deref(),
                scope: Scope::current(cli.workspace.as_deref(), cli.global)?,
            },
            &mut out,
        ),
        Command::Status { json } => commands::status::run(&config, json, &mut out),
        Command::Doctor => commands::doctor::run(&config, &mut out),
        Command::Uninstall => commands::uninstall::run(&config, &mut out),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Global options supplied before a subcommand can bypass clap's local conflict
    // checks. Validate the fully propagated values before any I/O.
    let scoped_file = matches!(&cli.command, Some(Command::Tail { file: Some(_), .. }))
        && (cli.global || cli.workspace.is_some());
    if scoped_file || (cli.global && cli.workspace.is_some()) {
        Cli::command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                "--file, --workspace, and --global are mutually exclusive",
            )
            .exit();
    }
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error)
            if error.chain().any(|e| {
                e.downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
            }) =>
        {
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bridgy: {error:#}");
            ExitCode::FAILURE
        }
    }
}
