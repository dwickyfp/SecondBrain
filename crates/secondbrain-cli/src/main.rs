#![forbid(unsafe_code)]

//! The `secondbrain` command-line surface over the Phase 0 libraries.
//!
//! This binary holds no domain logic. It parses arguments, calls into
//! `secondbrain-vault`, `secondbrain-markdown`, `secondbrain-index`, and
//! `secondbrain-transaction`, and renders what came back. That constraint is
//! the point rather than a style preference: the desktop app, the MCP server,
//! and the local API will later call the same library APIs, so anything this
//! binary decided for itself would be a behaviour only the CLI has.
//!
//! Two contracts are stable and tested: the exit codes in [`exit`], and the
//! `--json` output shapes. Neither form of output ever contains ANSI.

mod commands;
mod exit;
mod output;
mod plan;
mod workspace;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::exit::CliError;
use crate::output::Format;

/// Local-first Markdown knowledge workspace.
#[derive(Parser)]
#[command(
    name = "secondbrain",
    version,
    about = "Inspect and operate on a SecondBrain workspace",
    long_about = None,
)]
struct Cli {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the internal state of a workspace, leaving Markdown untouched.
    Init {
        /// The workspace directory.
        workspace: PathBuf,
    },
    /// Safely adopt an existing Obsidian-compatible vault in place.
    Import {
        #[command(subcommand)]
        command: ImportCommand,
    },
    /// Check that every note parses, round-trips, and claims a unique identity.
    Validate {
        /// The workspace directory.
        workspace: PathBuf,
    },
    /// Work with the derived search index.
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Search the derived index.
    Search {
        /// The workspace directory.
        workspace: PathBuf,
        /// The text to search for.
        query: String,
    },
    /// Export the versioned graph derived from the workspace index.
    Graph {
        /// The workspace directory.
        workspace: PathBuf,
    },
    /// Inspect notes.
    Note {
        #[command(subcommand)]
        command: NoteCommand,
    },
    /// Read, preview, and apply typed note properties.
    Property {
        #[command(subcommand)]
        command: PropertyCommand,
    },
    /// Preview the transaction an incoming file implies, without writing.
    Diff {
        /// The workspace directory.
        workspace: PathBuf,
        /// The workspace-relative path of the note to change.
        path: String,
        /// A file holding the note's intended new content.
        incoming: PathBuf,
        /// Write the plan here instead of printing it.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Work with transactions.
    Transaction {
        #[command(subcommand)]
        command: TransactionCommand,
    },
    /// Work with crash recovery.
    Recovery {
        #[command(subcommand)]
        command: RecoveryCommand,
    },
    /// Journal the edits made to tracked notes outside the workspace.
    Reconcile {
        /// The workspace directory.
        workspace: PathBuf,
    },
    /// Report on the health of a workspace.
    Doctor {
        /// The workspace directory.
        workspace: PathBuf,
    },
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Rebuild the derived index from the Markdown on disk.
    Rebuild {
        /// The workspace directory.
        workspace: PathBuf,
    },
}

#[derive(Subcommand)]
enum ImportCommand {
    /// Inventory and validate a vault without writing anything.
    Preview {
        workspace: PathBuf,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Apply a reviewed preview after revalidating the whole vault.
    Apply {
        workspace: PathBuf,
        preview: PathBuf,
    },
}

#[derive(Subcommand)]
enum NoteCommand {
    /// Report a note's identity, convergence, and links in both directions.
    Inspect {
        /// The workspace directory.
        workspace: PathBuf,
        /// The workspace-relative path of the note.
        path: String,
    },
    /// Preview creation of a new note from a Markdown source file.
    Create {
        workspace: PathBuf,
        path: String,
        source: PathBuf,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Apply a reviewed note-creation preview.
    ApplyCreate {
        workspace: PathBuf,
        preview: PathBuf,
    },
    /// Open an existing daily note or preview creation for an explicit date.
    Daily {
        workspace: PathBuf,
        date: String,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TransactionCommand {
    /// Validate a plan's preconditions and apply it.
    Apply {
        /// The workspace directory.
        workspace: PathBuf,
        /// A plan file produced by `secondbrain diff`.
        plan: PathBuf,
    },
}

#[derive(Subcommand)]
enum PropertyCommand {
    /// Read editable properties as typed JSON values.
    Read { workspace: PathBuf, path: String },
    /// Preview setting a property from a JSON value.
    Set {
        workspace: PathBuf,
        path: String,
        key: String,
        value: String,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Preview removing a property.
    Remove {
        workspace: PathBuf,
        path: String,
        key: String,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Apply a property preview after reviewing it.
    Apply {
        workspace: PathBuf,
        preview: PathBuf,
    },
}

#[derive(Subcommand)]
enum RecoveryCommand {
    /// Finish interrupted transactions and report every action taken.
    Check {
        /// The workspace directory.
        workspace: PathBuf,
    },
}

fn main() -> ExitCode {
    // Argument errors are turned into an exit code here rather than left to
    // `clap::Parser::parse`, which exits the process itself. The usage code is
    // part of this binary's contract, so this binary decides it.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let usage_error = error.use_stderr();
            let _ = error.print();
            return ExitCode::from(if usage_error { exit::USAGE } else { exit::OK });
        }
    };
    let format = Format::from_flag(cli.json);
    match dispatch(format, cli.command) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            output::emit_error(format, &error);
            ExitCode::from(error.exit_code())
        }
    }
}

fn dispatch(format: Format, command: Command) -> Result<u8, CliError> {
    match command {
        Command::Init { workspace } => commands::init::run(format, &workspace),
        Command::Import { command } => match command {
            ImportCommand::Preview { workspace, out } => {
                commands::import::preview(format, &workspace, out.as_deref())
            }
            ImportCommand::Apply { workspace, preview } => {
                commands::import::apply(format, &workspace, &preview)
            }
        },
        Command::Validate { workspace } => commands::validate::run(format, &workspace),
        Command::Index {
            command: IndexCommand::Rebuild { workspace },
        } => commands::index::rebuild(format, &workspace),
        Command::Search { workspace, query } => commands::search::run(format, &workspace, &query),
        Command::Graph { workspace } => commands::graph::run(format, &workspace),
        Command::Note {
            command: NoteCommand::Inspect { workspace, path },
        } => commands::note::inspect(format, &workspace, &path),
        Command::Note {
            command:
                NoteCommand::Create {
                    workspace,
                    path,
                    source,
                    out,
                },
        } => commands::create::preview(format, &workspace, &path, &source, out.as_deref()),
        Command::Note {
            command: NoteCommand::ApplyCreate { workspace, preview },
        } => commands::create::apply(format, &workspace, &preview),
        Command::Note {
            command:
                NoteCommand::Daily {
                    workspace,
                    date,
                    out,
                },
        } => commands::create::daily(format, &workspace, &date, out.as_deref()),
        Command::Property { command } => match command {
            PropertyCommand::Read { workspace, path } => {
                commands::property::read(format, &workspace, &path)
            }
            PropertyCommand::Set {
                workspace,
                path,
                key,
                value,
                out,
            } => {
                let value = serde_json::from_str(&value)
                    .map_err(|source| CliError::PlanUnreadable { source })?;
                commands::property::preview(
                    format,
                    &workspace,
                    &path,
                    secondbrain_markdown::PropertyEdit::Set { key, value },
                    out.as_deref(),
                )
            }
            PropertyCommand::Remove {
                workspace,
                path,
                key,
                out,
            } => commands::property::preview(
                format,
                &workspace,
                &path,
                secondbrain_markdown::PropertyEdit::Remove { key },
                out.as_deref(),
            ),
            PropertyCommand::Apply { workspace, preview } => {
                commands::property::apply(format, &workspace, &preview)
            }
        },
        Command::Diff {
            workspace,
            path,
            incoming,
            out,
        } => commands::diff::run(format, &workspace, &path, &incoming, out.as_deref()),
        Command::Transaction {
            command: TransactionCommand::Apply { workspace, plan },
        } => commands::transaction::apply(format, &workspace, &plan),
        Command::Recovery {
            command: RecoveryCommand::Check { workspace },
        } => commands::recovery::check(format, &workspace),
        Command::Reconcile { workspace } => commands::reconcile::run(format, &workspace),
        Command::Doctor { workspace } => commands::doctor::run(format, &workspace),
    }
}
