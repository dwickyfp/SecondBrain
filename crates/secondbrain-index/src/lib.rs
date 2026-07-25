#![forbid(unsafe_code)]

//! Rebuildable, derived SQLite index for a SecondBrain workspace.

mod database;
mod indexer;

pub use database::{Error, IndexDatabase, Result};
pub use indexer::{
    DumpLink, DumpNote, IndexConfig, IndexError, IndexReport, LogicalDump, logical_dump, rebuild,
};
