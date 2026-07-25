use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::id::{IdParseError, NoteId, NoteVersion};
use crate::path::WorkspacePathError;

/// A source error that can cross thread boundaries.
pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

/// The stable, shared error taxonomy for SecondBrain core operations.
///
/// Callers should branch on [`Error::code`] rather than matching human-facing
/// display text. Fields named `summary` must contain a sanitized explanation,
/// never source file contents, SQL, signatures, keys, or arbitrary payloads.
pub enum Error {
    /// A typed domain identifier could not be parsed.
    InvalidId {
        /// The focused identifier parse error.
        source: IdParseError,
    },
    /// A workspace-relative path failed lexical validation.
    InvalidWorkspacePath {
        /// The focused workspace path validation error.
        source: WorkspacePathError,
    },
    /// A resolved path would escape the workspace root.
    WorkspaceEscape {
        /// The rejected path.
        path: PathBuf,
    },
    /// A Markdown document is structurally invalid.
    InvalidMarkdown {
        /// The affected workspace path.
        path: PathBuf,
        /// A sanitized, bounded explanation supplied by the caller.
        summary: String,
    },
    /// A document uses an unsupported text encoding.
    UnsupportedEncoding {
        /// The affected workspace path.
        path: PathBuf,
        /// The detected encoding label, without document contents.
        encoding: String,
    },
    /// More than one workspace path declares the same note ID.
    DuplicateNoteId {
        /// The duplicated note ID.
        note_id: NoteId,
        /// A path containing the duplicate declaration.
        path: PathBuf,
    },
    /// A note's file no longer holds the content the workspace converged on.
    ///
    /// An editor outside the workspace saved over the note and nothing
    /// journaled that edit, so the note's recorded history no longer describes
    /// the bytes on disk. Every operation that derives a change from the
    /// converged base — planning, adopting, replaying — is wrong until the edit
    /// is reconciled, which is why this is a condition of its own rather than a
    /// stale precondition: no version was superseded, the workspace simply
    /// never saw what happened.
    NoteDiverged {
        /// The affected workspace path.
        path: PathBuf,
        /// The version of the base the file no longer holds.
        version: NoteVersion,
    },
    /// A note is absent from the derived index.
    ///
    /// Distinct from the index itself being missing or unreadable: the index
    /// answered, and the answer was that it does not know this note. The index
    /// is derived state, so this is repaired by rebuilding it and never by
    /// changing the Markdown.
    NoteNotIndexed {
        /// The workspace path that was asked about.
        path: PathBuf,
    },
    /// A resource changed after a caller observed it.
    StalePrecondition {
        /// A privacy-safe resource identifier.
        resource: String,
        /// The expected version or state identifier.
        expected: String,
        /// The actual version or state identifier.
        actual: String,
    },
    /// An operation is invalid for the transaction's current state.
    TransactionState {
        /// A privacy-safe transaction identifier.
        transaction: String,
        /// The current transaction state.
        state: String,
    },
    /// A stored record failed integrity or decoding validation.
    CorruptRecord {
        /// A privacy-safe record identifier.
        record: String,
        /// A sanitized, bounded explanation supplied by the caller.
        summary: String,
    },
    /// A cryptographic signature failed validation.
    SignatureInvalid {
        /// A privacy-safe signer identifier, never key or signature bytes.
        signer: String,
    },
    /// An operating-system I/O operation failed.
    Io {
        /// A short static operation label that contains no payload data.
        operation: &'static str,
        /// The original I/O error, retained for explicit source inspection.
        source: io::Error,
    },
    /// A SQLite operation failed.
    ///
    /// The boxed source keeps the core taxonomy independent of `rusqlite`.
    Sqlite {
        /// A short static operation label that contains no SQL.
        operation: &'static str,
        /// The original storage error, retained for explicit source inspection.
        source: BoxError,
    },
}

impl Error {
    /// Returns the durable diagnostic code for this error category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidId { .. } => "SB-ID-INVALID",
            Self::InvalidWorkspacePath { .. } => "SB-PATH-INVALID",
            Self::WorkspaceEscape { .. } => "SB-PATH-ESCAPE",
            Self::InvalidMarkdown { .. } => "SB-MD-INVALID",
            Self::UnsupportedEncoding { .. } => "SB-MD-ENCODING",
            Self::DuplicateNoteId { .. } => "SB-NOTE-DUPLICATE-ID",
            Self::NoteDiverged { .. } => "SB-NOTE-DIVERGED",
            Self::NoteNotIndexed { .. } => "SB-NOTE-NOT-INDEXED",
            Self::StalePrecondition { .. } => "SB-TXN-STALE-PRECONDITION",
            Self::TransactionState { .. } => "SB-TXN-STATE",
            Self::CorruptRecord { .. } => "SB-STORE-CORRUPT",
            Self::SignatureInvalid { .. } => "SB-SEC-SIGNATURE-INVALID",
            Self::Io { .. } => "SB-IO",
            Self::Sqlite { .. } => "SB-STORE-SQLITE",
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("code", &self.code())
            .field("message", &self.to_string())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { source } => write!(formatter, "invalid identifier: {source}"),
            Self::InvalidWorkspacePath { source } => {
                write!(formatter, "invalid workspace path: {source}")
            }
            Self::WorkspaceEscape { path } => {
                write!(formatter, "path escapes workspace: {}", path.display())
            }
            Self::InvalidMarkdown { path, summary } => {
                write!(
                    formatter,
                    "invalid Markdown at {}: {summary}",
                    path.display()
                )
            }
            Self::UnsupportedEncoding { path, encoding } => write!(
                formatter,
                "unsupported encoding {encoding} at {}",
                path.display()
            ),
            Self::DuplicateNoteId { note_id, path } => write!(
                formatter,
                "duplicate note ID {note_id} at {}",
                path.display()
            ),
            Self::NoteDiverged { path, version } => write!(
                formatter,
                "{} on disk is not the content the workspace last converged on (base version \
                 {}); that edit was made outside the workspace and has not been journaled, so \
                 it must be reconciled before anything derives a change from that base",
                path.display(),
                version.get()
            ),
            Self::NoteNotIndexed { path } => write!(
                formatter,
                "note {} is not in the derived index; rebuilding the index restores it",
                path.display()
            ),
            Self::StalePrecondition {
                resource,
                expected,
                actual,
            } => write!(
                formatter,
                "stale precondition for {resource}: expected {expected}, actual {actual}"
            ),
            Self::TransactionState { transaction, state } => write!(
                formatter,
                "transaction {transaction} cannot perform this operation in state {state}"
            ),
            Self::CorruptRecord { record, summary } => {
                write!(formatter, "corrupt record {record}: {summary}")
            }
            Self::SignatureInvalid { signer } => {
                write!(formatter, "invalid signature for signer {signer}")
            }
            Self::Io { operation, .. } => {
                write!(formatter, "I/O operation failed: {operation}")
            }
            Self::Sqlite { operation, .. } => {
                write!(formatter, "SQLite operation failed: {operation}")
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::InvalidId { source } => Some(source),
            Self::InvalidWorkspacePath { source } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<IdParseError> for Error {
    fn from(source: IdParseError) -> Self {
        Self::InvalidId { source }
    }
}

impl From<WorkspacePathError> for Error {
    fn from(source: WorkspacePathError) -> Self {
        Self::InvalidWorkspacePath { source }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::Io {
            operation: "I/O",
            source,
        }
    }
}

/// A result using the shared SecondBrain core error taxonomy.
pub type Result<T> = std::result::Result<T, Error>;
