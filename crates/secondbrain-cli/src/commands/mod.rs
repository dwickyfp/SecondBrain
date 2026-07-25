//! One module per command, each owning that command's report and nothing else.
//!
//! No module here contains domain logic. Every one of them composes calls into
//! the library crates, because the whole point of this binary is to exercise
//! the same library paths the desktop app, the MCP server, and the local API
//! will exercise. A second write path or a reimplemented diff living here would
//! be a path nothing else uses, and therefore a path nothing else tests.

pub mod diff;
pub mod doctor;
pub mod index;
pub mod init;
pub mod note;
pub mod reconcile;
pub mod recovery;
pub mod search;
pub mod transaction;
pub mod validate;

/// The actor a change this binary makes is attributed to.
///
/// Phase 0 has no identity management, so the CLI names itself rather than
/// claiming to be a person it cannot authenticate. Attribution becomes real in
/// the phase that introduces actor identity and scopes.
pub const CLI_ACTOR: &str = "cli";

/// The device a change this binary makes is attributed to.
///
/// Named once for both the commands that use it, because two commands
/// attributing the same machine differently would put two names for one device
/// into the durable journal.
pub const CLI_DEVICE: &str = "local";
