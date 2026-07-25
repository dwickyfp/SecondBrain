use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};
use thiserror::Error;

const INITIAL_MIGRATION: &str = include_str!("migrations/0001_initial.sql");

#[derive(Debug, Error)]
pub enum Error {
    #[error("SQLite index operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An owned connection to the derived workspace index.
pub struct IndexDatabase {
    connection: Connection,
}

impl IndexDatabase {
    /// Opens or creates an index and configures this connection for integrity and concurrency.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        Ok(Self { connection })
    }

    /// Applies all unapplied schema migrations atomically.
    pub fn migrate(&mut self) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        let applied = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 1)",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !applied {
            transaction.execute_batch(INITIAL_MIGRATION)?;
            transaction.execute("INSERT INTO schema_migrations (version) VALUES (1)", [])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Exposes the configured connection for index queries and updates.
    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }
}
