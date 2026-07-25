#![forbid(unsafe_code)]

//! Rebuildable, derived SQLite index for a SecondBrain workspace.

mod database;

pub use database::{Error, IndexDatabase, Result};
