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
pub mod recovery;
pub mod search;
pub mod transaction;
pub mod validate;
