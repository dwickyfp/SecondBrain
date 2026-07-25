use std::fs;

use secondbrain_core::id::NoteId;
use secondbrain_index::IndexDatabase;
use tempfile::tempdir;

const TABLES: [&str; 8] = [
    "notes",
    "paths",
    "properties",
    "links",
    "tags",
    "headings",
    "tasks",
    "index_state",
];

#[test]
fn open_enables_wal_and_foreign_keys_on_every_connection() {
    let temp = tempdir().unwrap();
    let database = IndexDatabase::open(temp.path().join("index.sqlite3")).unwrap();
    let journal_mode: String = database
        .connection()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    let foreign_keys: i64 = database
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "wal");
    assert_eq!(foreign_keys, 1);
}

#[test]
fn migration_is_idempotent_and_records_schema_version() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    let mut database = IndexDatabase::open(&path).unwrap();
    database.migrate().unwrap();
    database.migrate().unwrap();
    drop(database);
    let database = IndexDatabase::open(path).unwrap();
    let version: i64 = database
        .connection()
        .query_row("SELECT version FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 1);
}

#[test]
fn migration_creates_derived_schema_with_fts5_and_constraints() {
    let temp = tempdir().unwrap();
    let mut database = IndexDatabase::open(temp.path().join("index.sqlite3")).unwrap();
    database.migrate().unwrap();
    for table in TABLES {
        let exists: bool = database
            .connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "missing table {table}");
    }
    let fts_sql: String = database
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'notes_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(fts_sql.contains("VIRTUAL TABLE") && fts_sql.contains("fts5"));
    assert!(fts_sql.contains("note_id UNINDEXED"));

    let note_id = NoteId::new().to_string();
    database
        .connection()
        .execute("INSERT INTO notes (note_id) VALUES (?1)", [&note_id])
        .unwrap();
    database
        .connection()
        .execute(
            "INSERT INTO paths (note_id, path, is_current) VALUES (?1, 'a.md', 1)",
            [&note_id],
        )
        .unwrap();
    assert!(
        database
            .connection()
            .execute(
                "INSERT INTO paths (note_id, path, is_current) VALUES (?1, 'b.md', 1)",
                [&note_id]
            )
            .is_err()
    );
    database
        .connection()
        .execute(
            "INSERT INTO tags (note_id, tag) VALUES (?1, 'rust')",
            [&note_id],
        )
        .unwrap();
    database
        .connection()
        .execute("DELETE FROM notes WHERE note_id = ?1", [&note_id])
        .unwrap();
    let children: i64 = database
        .connection()
        .query_row("SELECT count(*) FROM tags", [], |row| row.get(0))
        .unwrap();
    assert_eq!(children, 0);
}

#[test]
fn deleting_index_database_never_touches_workspace_markdown() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let note = workspace.join("note.md");
    fs::write(&note, "# Durable content\n").unwrap();
    let index = temp.path().join("derived.sqlite3");
    let mut database = IndexDatabase::open(&index).unwrap();
    database.migrate().unwrap();
    drop(database);
    fs::remove_file(index).unwrap();
    assert_eq!(fs::read_to_string(note).unwrap(), "# Durable content\n");
}
