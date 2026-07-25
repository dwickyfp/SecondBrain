use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::PathBuf;

use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_core::{Error, Result};

#[derive(Debug)]
struct FakeSqliteError(&'static str);

impl fmt::Display for FakeSqliteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl StdError for FakeSqliteError {}

fn invalid_id_source() -> secondbrain_core::id::IdParseError {
    "not-a-ulid"
        .parse::<NoteId>()
        .expect_err("fixture must be invalid")
}

fn invalid_path_source() -> secondbrain_core::path::WorkspacePathError {
    WorkspacePath::new("../outside.md").expect_err("fixture must be invalid")
}

fn sample_errors() -> Vec<(Error, &'static str)> {
    let note_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse::<NoteId>()
        .expect("fixture must be a canonical note ID");

    vec![
        (
            Error::InvalidId {
                source: invalid_id_source(),
            },
            "SB-ID-INVALID",
        ),
        (
            Error::InvalidWorkspacePath {
                source: invalid_path_source(),
            },
            "SB-PATH-INVALID",
        ),
        (
            Error::WorkspaceEscape {
                path: PathBuf::from("../outside.md"),
            },
            "SB-PATH-ESCAPE",
        ),
        (
            Error::InvalidMarkdown {
                path: PathBuf::from("notes/broken.md"),
                summary: "unterminated front matter".into(),
            },
            "SB-MD-INVALID",
        ),
        (
            Error::UnsupportedEncoding {
                path: PathBuf::from("notes/legacy.md"),
                encoding: "UTF-16LE".into(),
            },
            "SB-MD-ENCODING",
        ),
        (
            Error::DuplicateNoteId {
                note_id,
                path: PathBuf::from("notes/duplicate.md"),
            },
            "SB-NOTE-DUPLICATE-ID",
        ),
        (
            Error::NoteDiverged {
                path: PathBuf::from("notes/meeting.md"),
                version: secondbrain_core::id::NoteVersion::new(4),
            },
            "SB-NOTE-DIVERGED",
        ),
        (
            Error::NoteNotIndexed {
                path: PathBuf::from("notes/meeting.md"),
            },
            "SB-NOTE-NOT-INDEXED",
        ),
        (
            Error::StalePrecondition {
                resource: "note:01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                expected: "4".into(),
                actual: "5".into(),
            },
            "SB-TXN-STALE-PRECONDITION",
        ),
        (
            Error::TransactionState {
                transaction: "txn:01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                state: "committed".into(),
            },
            "SB-TXN-STATE",
        ),
        (
            Error::CorruptRecord {
                record: "note:01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                summary: "content hash mismatch".into(),
            },
            "SB-STORE-CORRUPT",
        ),
        (
            Error::SignatureInvalid {
                signer: "device:work-laptop".into(),
            },
            "SB-SEC-SIGNATURE-INVALID",
        ),
        (
            Error::Io {
                operation: "read note",
                source: io::Error::new(io::ErrorKind::PermissionDenied, "private file contents"),
            },
            "SB-IO",
        ),
        (
            Error::Sqlite {
                operation: "load note",
                source: Box::new(FakeSqliteError(
                    "SELECT secret_key FROM credentials WHERE owner = 'alice'",
                )),
            },
            "SB-STORE-SQLITE",
        ),
    ]
}

#[test]
fn every_error_variant_has_its_exact_stable_code() {
    for (error, expected) in sample_errors() {
        assert_eq!(error.code(), expected, "variant: {error:?}");
    }
}

#[test]
fn result_alias_uses_the_public_error_type() {
    fn fail() -> Result<()> {
        Err(invalid_id_source().into())
    }

    assert_eq!(fail().expect_err("fixture fails").code(), "SB-ID-INVALID");
}

#[test]
fn focused_parse_errors_convert_with_code_and_source_chaining() {
    let id_error = Error::from(invalid_id_source());
    assert_eq!(id_error.code(), "SB-ID-INVALID");
    assert!(id_error.to_string().contains("invalid ULID"));
    assert!(
        id_error
            .source()
            .is_some_and(|source| source.is::<secondbrain_core::id::IdParseError>())
    );

    let path_error = Error::from(invalid_path_source());
    assert_eq!(path_error.code(), "SB-PATH-INVALID");
    assert!(
        path_error
            .to_string()
            .contains("cannot contain a parent component")
    );
    assert!(
        path_error
            .source()
            .is_some_and(|source| { source.is::<secondbrain_core::path::WorkspacePathError>() })
    );
}

#[test]
fn io_and_sqlite_preserve_sources_without_exposing_source_messages() {
    let io_error = Error::Io {
        operation: "read note",
        source: io::Error::new(io::ErrorKind::InvalidData, "private note body"),
    };
    assert!(
        io_error
            .source()
            .is_some_and(|source| source.is::<io::Error>())
    );
    assert_eq!(io_error.to_string(), "I/O operation failed: read note");
    assert!(!io_error.to_string().contains("private note body"));

    let sqlite_error = Error::Sqlite {
        operation: "load note",
        source: Box::new(FakeSqliteError(
            "SELECT secret_key FROM credentials WHERE owner = 'alice'",
        )),
    };
    assert!(
        sqlite_error
            .source()
            .is_some_and(|source| source.is::<FakeSqliteError>())
    );
    assert_eq!(
        sqlite_error.to_string(),
        "SQLite operation failed: load note"
    );
    assert!(!sqlite_error.to_string().contains("SELECT"));
    assert!(!sqlite_error.to_string().contains("secret_key"));
}

#[test]
fn displays_are_concise_and_use_only_structured_diagnostic_fields() {
    for (error, _) in sample_errors() {
        let display = error.to_string();
        assert!(!display.is_empty(), "variant: {error:?}");
        assert!(!display.contains("private file contents"));
        assert!(!display.contains("SELECT secret_key"));
        assert!(!display.contains("signature bytes"));
    }
}

#[test]
fn the_conditions_every_surface_can_meet_are_named_once_here() {
    // A note whose file no longer holds its converged base, and a note absent
    // from the derived index, are facts about the domain rather than about any
    // one surface. Each surface that meets them — the CLI today, the MCP server
    // and the local API later — must report the same code, which is only
    // possible if the taxonomy defines it. Left to the surfaces, the second one
    // to meet the condition mints a second code for it.
    let diverged = Error::NoteDiverged {
        path: PathBuf::from("notes/meeting.md"),
        version: secondbrain_core::id::NoteVersion::new(4),
    };
    let not_indexed = Error::NoteNotIndexed {
        path: PathBuf::from("notes/meeting.md"),
    };

    assert_eq!(diverged.code(), "SB-NOTE-DIVERGED");
    assert_eq!(not_indexed.code(), "SB-NOTE-NOT-INDEXED");
    assert_ne!(
        not_indexed.code(),
        Error::Sqlite {
            operation: "open index",
            source: Box::new(FakeSqliteError("no such table")),
        }
        .code(),
        "a note missing from an index that exists is not the index failing"
    );
    assert!(
        diverged.to_string().contains("notes/meeting.md") && diverged.to_string().contains('4'),
        "the message must say which note and which base version: {diverged}"
    );
}
