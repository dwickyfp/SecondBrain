#![forbid(unsafe_code)]

//! Rebuildable, derived SQLite index for a SecondBrain workspace.

mod database;
mod indexer;
mod query;

pub use database::{Error, IndexDatabase, QueryValidationError, Result};
pub use indexer::{
    DumpLink, DumpNote, IndexConfig, IndexError, IndexReport, LogicalDump, logical_dump, rebuild,
};
pub use query::{BrokenLink, LinkHit, NoteSummary, SearchHit, SearchQuery};
