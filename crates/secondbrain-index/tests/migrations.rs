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
        .query_row("SELECT max(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 3);
}

#[test]
fn query_indexes_exist_and_are_selected_by_sqlite() {
    let temp = tempdir().unwrap();
    let mut database = IndexDatabase::open(temp.path().join("index.sqlite3")).unwrap();
    database.migrate().unwrap();
    let connection = database.connection();
    for (sql, index) in [
        ("SELECT * FROM links WHERE label='x'", "links_label_idx"),
        ("SELECT * FROM links WHERE note_id='x'", "links_note_id_idx"),
        (
            "SELECT 1 FROM tags WHERE note_id='x' AND tag='rust'",
            "sqlite_autoindex_tags_1",
        ),
    ] {
        let detail: String = connection
            .query_row(&format!("EXPLAIN QUERY PLAN {sql}"), [], |row| row.get(3))
            .unwrap();
        assert!(detail.contains(index), "expected {index} in plan: {detail}");
    }
    let orphan_plan = connection.prepare("EXPLAIN QUERY PLAN SELECT n.note_id FROM notes n WHERE NOT EXISTS (SELECT 1 FROM links l WHERE l.note_id=n.note_id AND l.label IS NOT NULL) AND NOT EXISTS (SELECT 1 FROM links l WHERE l.label=n.note_id)").unwrap().query_map([], |row| row.get::<_, String>(3)).unwrap().collect::<Result<Vec<_>, _>>().unwrap().join("\n");
    assert!(orphan_plan.contains("links_note_id_idx"), "{orphan_plan}");
    assert!(orphan_plan.contains("links_label_idx"), "{orphan_plan}");
}

#[test]
fn forward_migration_upgrades_an_existing_version_one_database() {
    let temp = tempdir().unwrap();
    let mut database = IndexDatabase::open(temp.path().join("index.sqlite3")).unwrap();
    database
        .connection()
        .execute_batch(include_str!("../src/migrations/0001_initial.sql"))
        .unwrap();
    database.connection().execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); INSERT INTO schema_migrations(version) VALUES(1);").unwrap();
    database.migrate().unwrap();
    database.migrate().unwrap();
    let version: i64 = database
        .connection()
        .query_row("SELECT max(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 3);
}

#[test]
fn graph_migration_upgrades_version_two_without_changing_existing_links() {
    let temp = tempdir().unwrap();
    let mut database = IndexDatabase::open(temp.path().join("index.sqlite3")).unwrap();
    database
        .connection()
        .execute_batch(include_str!("../src/migrations/0001_initial.sql"))
        .unwrap();
    database
        .connection()
        .execute_batch(include_str!("../src/migrations/0002_query_indexes.sql"))
        .unwrap();
    database.connection().execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); INSERT INTO schema_migrations(version) VALUES(1),(2);").unwrap();
    let id = NoteId::new().to_string();
    database
        .connection()
        .execute("INSERT INTO notes(note_id) VALUES(?1)", [&id])
        .unwrap();
    database
        .connection()
        .execute(
            "INSERT INTO links(note_id,target,label) VALUES(?1,'missing',NULL)",
            [&id],
        )
        .unwrap();

    database.migrate().unwrap();

    assert_eq!(
        database
            .connection()
            .query_row("SELECT max(version) FROM schema_migrations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        3
    );
    assert_eq!(
        database
            .connection()
            .query_row("SELECT count(*) FROM links", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        database
            .connection()
            .query_row("SELECT count(*) FROM link_candidates", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
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
    assert!(database.connection().query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='link_candidates')", [], |row| row.get::<_, bool>(0)).unwrap());
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
