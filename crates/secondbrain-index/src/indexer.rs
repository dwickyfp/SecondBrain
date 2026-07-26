use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rusqlite::{Connection, params};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::extract::{PropertyValue, extract};
use secondbrain_markdown::{NoteMetadata, SourceDocument, parse_metadata};
use secondbrain_vault::base_snapshot::BaseSnapshotStore;
use secondbrain_vault::{IdentityMap, RecoveryOutcome, WorkspaceRoot};
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct IndexConfig {
    pub exclusions: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexReport {
    pub indexed: usize,
    pub skipped: usize,
    pub warnings: usize,
    pub errors: usize,
    pub orphans: usize,
    pub broken_links: usize,
}

pub const INDEX_SCHEMA_VERSION: i64 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexHealth {
    Missing,
    Valid,
    Stale,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexOpenReport {
    pub health: IndexHealth,
    pub rebuilt: bool,
    pub report: IndexReport,
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("index I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("malformed note {path}: {message}")]
    MalformedNote { path: String, message: String },
    #[error("workspace path is not portable UTF-8: {path}")]
    NonUtf8Path { path: PathBuf },
    #[error("portable path collision between {first} and {second}")]
    PathCollision { first: String, second: String },
    #[error("symbolic link is not supported during workspace traversal: {path}")]
    Symlink { path: String },
    #[error("duplicate note ID {note_id} in {first} and {second}")]
    DuplicateId {
        note_id: NoteId,
        first: String,
        second: String,
    },
    #[error("identity resolution failed for {path}: {message}")]
    Identity { path: String, message: String },
    #[error("converged base could not be recorded for {path}: {message}")]
    ConvergedBase { path: String, message: String },
    #[error("SQLite index operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

struct Note {
    path: WorkspacePath,
    source: String,
    document: SourceDocument,
    metadata: NoteMetadata,
    id: Option<NoteId>,
}

/// The internal directory name that owns the derived index.
const INTERNAL_DIR: &str = ".secondbrain";

/// The derived SQLite index file name inside the internal directory.
const INDEX_FILE: &str = "index.sqlite";

/// The SQLite file the derived index of the workspace at `root` lives in.
///
/// Stated here because this crate owns the file, so that callers that must
/// open, dump, or check for it — the CLI among them — ask rather than restate
/// a path that only this crate gets to choose.
#[must_use]
pub fn index_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(INTERNAL_DIR).join(INDEX_FILE)
}

/// Every Markdown note in the workspace at `root`, in deterministic order.
///
/// This applies exactly the rules [`rebuild`] applies — the same internal and
/// excluded directories, the same file extensions — because it runs the same
/// scan. A caller that wants to inspect notes without building an index, such
/// as workspace validation, asks here rather than walking the tree itself and
/// drifting from what the index considers a note.
pub fn note_paths(
    root: impl AsRef<Path>,
    config: &IndexConfig,
) -> Result<Vec<WorkspacePath>, IndexError> {
    let root = root.as_ref();
    let mut skipped = 0;
    let mut files = Vec::new();
    scan(root, root, config, &mut files, &mut skipped)?;
    files.sort();
    reject_path_collisions(&files)?;
    files
        .into_iter()
        .map(|(relative, _)| {
            WorkspacePath::new(&relative).map_err(|error| IndexError::MalformedNote {
                path: relative,
                message: error.to_string(),
            })
        })
        .collect()
}

pub fn rebuild(root: impl AsRef<Path>, config: &IndexConfig) -> Result<IndexReport, IndexError> {
    let root = root.as_ref();
    let started = Instant::now();
    let mut skipped = 0;
    let mut files = Vec::new();
    scan(root, root, config, &mut files, &mut skipped)?;
    files.sort();
    reject_path_collisions(&files)?;
    profile_stage("scan", started.elapsed());

    let parsing_started = Instant::now();
    let mut notes = Vec::new();
    let mut ids = BTreeMap::new();
    for (relative, absolute) in files {
        let source = fs::read_to_string(&absolute).map_err(|source| IndexError::Io {
            path: absolute,
            source,
        })?;
        let metadata = parse_metadata(&source).map_err(|error| IndexError::MalformedNote {
            path: relative.clone(),
            message: error.to_string(),
        })?;
        let document =
            SourceDocument::parse(&source).map_err(|error| IndexError::MalformedNote {
                path: relative.clone(),
                message: error.to_string(),
            })?;
        if let Some(id) = metadata.id
            && let Some(first) = ids.insert(id, relative.clone())
        {
            return Err(IndexError::DuplicateId {
                note_id: id,
                first,
                second: relative,
            });
        }
        notes.push(Note {
            path: WorkspacePath::new(&relative).map_err(|error| IndexError::MalformedNote {
                path: relative,
                message: error.to_string(),
            })?,
            source,
            document,
            metadata: metadata.clone(),
            id: metadata.id,
        });
    }
    profile_stage("parse", parsing_started.elapsed());

    let identity_started = Instant::now();
    establish_ids(root, &mut notes)?;
    profile_stage("identity_and_genesis", identity_started.elapsed());
    let internal = root.join(INTERNAL_DIR);
    fs::create_dir_all(&internal).map_err(|source| IndexError::Io {
        path: internal.clone(),
        source,
    })?;
    let active = index_path(root);
    let temporary = internal.join(format!("{INDEX_FILE}.rebuild"));
    remove_sqlite_files(&temporary)?;

    let mut database = crate::IndexDatabase::open(&temporary).map_err(database_error)?;
    database.migrate().map_err(database_error)?;
    let population_started = Instant::now();
    let mut report = populate(database.connection(), &notes)?;
    profile_stage("sqlite_population", population_started.elapsed());
    report.skipped = skipped;
    check_database(database.connection())?;
    database
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(database);
    remove_sidecars(&active)?;
    remove_one(&active)?;
    fs::rename(&temporary, &active).map_err(|source| IndexError::Io {
        path: active,
        source,
    })?;
    Ok(report)
}

fn profile_stage(stage: &str, elapsed: std::time::Duration) {
    if std::env::var_os("SECONDBRAIN_PROFILE_INDEX").is_some() {
        eprintln!(
            "secondbrain_index stage={stage} elapsed_us={}",
            elapsed.as_micros()
        );
    }
}

/// Reuses a complete current index, rebuilding only when it is missing, stale,
/// corrupt, or uses a different schema. This is the shared workspace-open
/// health contract; callers should not infer health from file existence alone.
pub fn ensure_index(
    root: impl AsRef<Path>,
    config: &IndexConfig,
) -> Result<IndexOpenReport, IndexError> {
    let root = root.as_ref();
    let health = index_health(root, config)?;
    if health == IndexHealth::Valid {
        let database = crate::IndexDatabase::open(index_path(root)).map_err(database_error)?;
        return Ok(IndexOpenReport {
            health,
            rebuilt: false,
            report: report_from_database(&database)?,
        });
    }
    Ok(IndexOpenReport {
        health,
        rebuilt: true,
        report: rebuild(root, config)?,
    })
}

pub fn index_health(
    root: impl AsRef<Path>,
    config: &IndexConfig,
) -> Result<IndexHealth, IndexError> {
    let root = root.as_ref();
    let path = index_path(root);
    if !path.is_file() {
        return Ok(IndexHealth::Missing);
    }
    let database = match crate::IndexDatabase::open(&path) {
        Ok(database) => database,
        Err(_) => return Ok(IndexHealth::Invalid),
    };
    let paths = note_paths(root, config)?;
    let connection = database.connection();
    let valid = (|| -> Result<bool, rusqlite::Error> {
        let schema: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if schema != INDEX_SCHEMA_VERSION {
            return Ok(false);
        }
        check_database(connection).map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut statement = connection.prepare(
            "SELECT p.path,n.content_hash FROM notes n JOIN paths p ON p.note_id=n.note_id AND p.is_current=1 ORDER BY p.path",
        )?;
        let indexed = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if indexed.len() != paths.len() {
            return Ok(false);
        }
        for ((indexed_path, indexed_hash), workspace_path) in indexed.iter().zip(paths) {
            if indexed_path != workspace_path.as_str() {
                return Ok(false);
            }
            let bytes = fs::read(root.join(workspace_path.as_path()))
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            if *indexed_hash != ContentHash::digest(&bytes).to_string() {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    match valid {
        Ok(true) => Ok(IndexHealth::Valid),
        Ok(false) => Ok(IndexHealth::Stale),
        Err(_) => Ok(IndexHealth::Invalid),
    }
}

fn report_from_database(database: &crate::IndexDatabase) -> Result<IndexReport, IndexError> {
    let indexed = database
        .connection()
        .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get::<_, i64>(0))?
        as usize;
    Ok(IndexReport {
        indexed,
        broken_links: database.broken_links().map_err(database_error)?.len(),
        orphans: database.orphans().map_err(database_error)?.len(),
        ..IndexReport::default()
    })
}

fn scan(
    root: &Path,
    directory: &Path,
    config: &IndexConfig,
    files: &mut Vec<(String, PathBuf)>,
    skipped: &mut usize,
) -> Result<(), IndexError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| IndexError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| IndexError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative_path = path.strip_prefix(root).expect("scanned path is below root");
        let relative = relative_path
            .to_str()
            .ok_or_else(|| IndexError::NonUtf8Path { path: path.clone() })?
            .replace('\\', "/");
        let internal = relative == ".git" || relative == ".secondbrain";
        let excluded = config
            .exclusions
            .iter()
            .any(|pattern| relative == *pattern || relative.starts_with(&format!("{pattern}/")));
        let file_type = entry.file_type().map_err(|source| IndexError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_symlink() {
            return Err(IndexError::Symlink { path: relative });
        } else if internal || excluded {
            *skipped += count_markdown(&path)?;
        } else if file_type.is_dir() {
            scan(root, &path, config, files, skipped)?;
        } else if file_type.is_file() && is_markdown(&path) {
            files.push((relative, path));
        }
    }
    Ok(())
}

fn reject_path_collisions(files: &[(String, PathBuf)]) -> Result<(), IndexError> {
    let mut portable = BTreeMap::new();
    for (relative, _) in files {
        let key = relative.to_lowercase();
        if let Some(first) = portable.insert(key, relative.clone())
            && first != *relative
        {
            let (first, second) = if first < *relative {
                (first, relative.clone())
            } else {
                (relative.clone(), first)
            };
            return Err(IndexError::PathCollision { first, second });
        }
    }
    Ok(())
}

fn count_markdown(path: &Path) -> Result<usize, IndexError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| IndexError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(IndexError::Symlink {
            path: path.display().to_string(),
        });
    }
    if metadata.is_file() {
        return Ok(usize::from(is_markdown(path)));
    }
    let mut count = 0;
    for entry in fs::read_dir(path).map_err(|source| IndexError::Io {
        path: path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| IndexError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if entry
            .file_type()
            .map_err(|source| IndexError::Io {
                path: entry.path(),
                source,
            })?
            .is_symlink()
        {
            return Err(IndexError::Symlink {
                path: entry.path().display().to_string(),
            });
        }
        count += count_markdown(&entry.path())?;
    }
    Ok(count)
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("md" | "markdown")
    )
}

/// Establishes the identity of every note, and the base each one converged at.
///
/// These are one step because they answer one question: whether the workspace
/// has taken responsibility for a note. Identity is the moment it does, and a
/// note the workspace is responsible for has a converged base — the state an
/// external editor's next save is measured against. Recording the base anywhere
/// else would leave a window in which a note is tracked but undiffable, and
/// leaving it to a caller would make the rule the CLI's rather than the
/// library's, which the desktop app and the MCP server would each have to
/// reimplement.
///
/// A base that already exists is never rewritten. See
/// [`BaseSnapshotStore::ensure_genesis`]: a file that has moved past its base
/// holds an external edit, and reconciliation — not indexing — is what turns
/// that into attributed history.
fn establish_ids(root: &Path, notes: &mut [Note]) -> Result<(), IndexError> {
    let workspace = WorkspaceRoot::open(root).map_err(|error| IndexError::Identity {
        path: root.display().to_string(),
        message: error.to_string(),
    })?;
    let mut map = IdentityMap::open(&workspace).map_err(|error| IndexError::Identity {
        path: root.display().to_string(),
        message: error.to_string(),
    })?;
    let bases = BaseSnapshotStore::new(&workspace);
    let fresh_map = map.is_empty();
    // Every note this scan found, including the ones whose identity came from
    // their own frontmatter: what the identity map needs is which recorded
    // paths still hold a file, and a note declaring its own id occupies its
    // path exactly as much as one that does not.
    let present: BTreeSet<WorkspacePath> = notes.iter().map(|note| note.path.clone()).collect();
    let fresh = if fresh_map {
        let distinct = notes
            .iter()
            .filter(|note| note.id.is_none())
            .map(|note| {
                (
                    note.path.clone(),
                    ContentHash::digest(note.source.as_bytes()),
                    note.document.semantic_fingerprint(),
                )
            })
            .collect::<Vec<_>>();
        Some(
            map.register_distinct_batch(&distinct)
                .map_err(|error| IndexError::Identity {
                    path: root.display().to_string(),
                    message: error.to_string(),
                })?
                .into_iter(),
        )
    } else {
        None
    };
    let mut fresh = fresh;
    for note in &mut *notes {
        let hash = ContentHash::digest(note.source.as_bytes());
        let fingerprint = note.document.semantic_fingerprint();
        let note_id = if let Some(note_id) = note.id {
            map.register_known(note_id, &note.path, hash, fingerprint)
                .map_err(|error| IndexError::Identity {
                    path: note.path.to_string(),
                    message: error.to_string(),
                })?;
            map.update_path(&note_id, &note.path)
                .map_err(|error| IndexError::Identity {
                    path: note.path.to_string(),
                    message: error.to_string(),
                })?;
            note_id
        } else if fresh_map {
            fresh
                .as_mut()
                .and_then(Iterator::next)
                .expect("fresh identity registered for every note without an id")
        } else {
            // Resolved against the scan rather than the file alone: a rebuild
            // is the one caller that knows which recorded paths still hold a
            // file, and that is what tells a note that moved from a note that
            // was copied.
            let outcome = map
                .resolve_in_scan(&note.path, hash, fingerprint, &present)
                .map_err(|error| IndexError::Identity {
                    path: note.path.to_string(),
                    message: error.to_string(),
                })?;
            match outcome {
                RecoveryOutcome::Resolved(id) => id,
                RecoveryOutcome::New | RecoveryOutcome::Duplicate { .. } => map
                    .register(&note.path, hash, fingerprint)
                    .map_err(|error| IndexError::Identity {
                        path: note.path.to_string(),
                        message: error.to_string(),
                    })?,
                RecoveryOutcome::NeedsReview { .. } => {
                    return Err(IndexError::Identity {
                        path: note.path.to_string(),
                        message: "identity requires review".into(),
                    });
                }
            }
        };
        note.id = Some(note_id);
    }
    let genesis = notes
        .iter()
        .map(|note| {
            (
                note.id.expect("identity established"),
                note.path.clone(),
                note.source.clone(),
            )
        })
        .collect::<Vec<_>>();
    bases
        .ensure_genesis_batch(&genesis)
        .map_err(|error| IndexError::ConvergedBase {
            path: root.display().to_string(),
            message: error.to_string(),
        })?;
    for note in notes {
        let note_id = note.id.expect("identity established");
        // A full scan may have recovered a move. Keep the old converged bytes,
        // hash and version, but make their location follow the identity.
        bases
            .update_path(note_id, &note.path)
            .map_err(|error| IndexError::ConvergedBase {
                path: note.path.to_string(),
                message: error.to_string(),
            })?;
    }
    Ok(())
}

fn populate(connection: &Connection, notes: &[Note]) -> Result<IndexReport, IndexError> {
    begin_population(connection)?;
    let result = populate_inner(connection, notes);
    match result {
        Ok(report) => {
            connection.execute_batch("COMMIT;")?;
            Ok(report)
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

fn begin_population(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch("BEGIN IMMEDIATE;")
}

fn populate_inner(connection: &Connection, notes: &[Note]) -> Result<IndexReport, IndexError> {
    let mut lookup: BTreeMap<String, BTreeSet<NoteId>> = BTreeMap::new();
    for note in notes {
        let metadata = &note.metadata;
        let id = note.id.expect("identity established before population");
        lookup
            .entry(note.path.as_str().to_lowercase())
            .or_default()
            .insert(id);
        lookup
            .entry(
                note.path
                    .as_str()
                    .trim_end_matches(".md")
                    .trim_end_matches(".markdown")
                    .to_lowercase(),
            )
            .or_default()
            .insert(id);
        if let Some(name) = note
            .path
            .as_path()
            .file_stem()
            .and_then(|value| value.to_str())
        {
            lookup.entry(name.to_lowercase()).or_default().insert(id);
        }
        if let Some(title) = &metadata.title {
            lookup.entry(title.to_lowercase()).or_default().insert(id);
        }
        for alias in aliases(&metadata.properties) {
            lookup.entry(alias.to_lowercase()).or_default().insert(id);
        }
    }
    let mut linked = BTreeSet::new();
    let mut broken = 0;
    for note in notes {
        let id = note.id.expect("identity established");
        let metadata = &note.metadata;
        let extracted = extract(&note.document);
        let title = metadata.title.clone().or_else(|| {
            extracted
                .headings
                .first()
                .map(|heading| heading.text.clone())
        });
        connection.execute(
            "INSERT INTO notes(note_id,title,content_hash) VALUES (?1,?2,?3)",
            params![
                id.to_string(),
                title,
                ContentHash::digest(note.source.as_bytes()).to_string()
            ],
        )?;
        connection.execute(
            "INSERT INTO paths(note_id,path) VALUES (?1,?2)",
            params![id.to_string(), note.path.as_str()],
        )?;
        connection.execute(
            "INSERT INTO notes_fts(note_id,title,body) VALUES (?1,?2,?3)",
            params![id.to_string(), title, extracted.plain_text],
        )?;
        for (name, value) in extracted.properties {
            connection.execute(
                "INSERT INTO properties(note_id,name,value) VALUES (?1,?2,?3)",
                params![id.to_string(), name, property_text(&value)],
            )?;
        }
        for tag in extracted.tags {
            connection.execute(
                "INSERT INTO tags(note_id,tag) VALUES (?1,?2)",
                params![id.to_string(), tag.text],
            )?;
        }
        for heading in extracted.headings {
            connection.execute(
                "INSERT INTO headings(note_id,level,text,line) VALUES (?1,?2,?3,?4)",
                params![
                    id.to_string(),
                    heading.level,
                    heading.text,
                    line_at(&note.source, heading.span.start)
                ],
            )?;
        }
        for task in extracted.tasks {
            connection.execute(
                "INSERT INTO tasks(note_id,text,completed,line) VALUES (?1,?2,?3,?4)",
                params![
                    id.to_string(),
                    task.text,
                    task.checked,
                    line_at(&note.source, task.span.start)
                ],
            )?;
        }
        for link in extracted.links {
            let candidates = lookup.get(&link.target.to_lowercase());
            let resolved = candidates.and_then(|values| {
                (values.len() == 1).then(|| *values.first().expect("one candidate"))
            });
            if let Some(target) = resolved {
                linked.insert(id);
                linked.insert(target);
            } else if candidates.is_none() {
                broken += 1;
            }
            connection.execute(
                "INSERT INTO links(note_id,target,label) VALUES (?1,?2,?3)",
                params![
                    id.to_string(),
                    link.target,
                    resolved.map(|value| value.to_string())
                ],
            )?;
            let link_id = connection.last_insert_rowid();
            if let Some(candidates) = candidates.filter(|values| values.len() > 1) {
                for candidate in candidates {
                    connection.execute(
                        "INSERT INTO link_candidates(link_id,note_id) VALUES (?1,?2)",
                        params![link_id, candidate.to_string()],
                    )?;
                }
            }
        }
    }
    Ok(IndexReport {
        indexed: notes.len(),
        broken_links: broken,
        orphans: notes.len().saturating_sub(linked.len()),
        ..IndexReport::default()
    })
}

fn aliases(properties: &serde_yaml::Mapping) -> Vec<String> {
    properties
        .get("aliases")
        .map_or_else(Vec::new, |value| match value {
            serde_yaml::Value::Sequence(values) => values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect(),
            serde_yaml::Value::String(value) => vec![value.clone()],
            _ => Vec::new(),
        })
}
fn property_text(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".into(),
        PropertyValue::Bool(v) => v.to_string(),
        PropertyValue::Int(v) => v.to_string(),
        PropertyValue::Float(v) => v.to_string(),
        PropertyValue::Str(v) => v.clone(),
        PropertyValue::List(v) => v.join(","),
    }
}
fn line_at(source: &str, offset: usize) -> i64 {
    (source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1) as i64
}
fn database_error(error: crate::Error) -> IndexError {
    match error {
        crate::Error::Sqlite(error) => IndexError::Sqlite(error),
        crate::Error::InvalidQuery(_) | crate::Error::InvalidStoredNoteId { .. } => {
            IndexError::Sqlite(rusqlite::Error::InvalidQuery)
        }
    }
}
fn check_database(connection: &Connection) -> Result<(), IndexError> {
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(IndexError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    let foreign_keys: i64 =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_keys != 0 {
        return Err(IndexError::Sqlite(rusqlite::Error::InvalidQuery));
    }
    Ok(())
}
fn remove_sqlite_files(path: &Path) -> Result<(), IndexError> {
    remove_one(path)?;
    remove_sidecars(path)
}
fn remove_sidecars(path: &Path) -> Result<(), IndexError> {
    for suffix in ["-wal", "-shm"] {
        remove_one(Path::new(&format!("{}{suffix}", path.display())))?;
    }
    Ok(())
}
fn remove_one(path: &Path) -> Result<(), IndexError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(IndexError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalDump {
    pub notes: Vec<DumpNote>,
    pub links: Vec<DumpLink>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DumpNote {
    pub note_id: String,
    pub path: String,
    pub title: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DumpLink {
    pub source_note_id: String,
    pub target: String,
    pub resolved_note_id: Option<String>,
}
pub fn logical_dump(path: impl AsRef<Path>) -> Result<LogicalDump, IndexError> {
    let connection = Connection::open(path)?;
    let notes = connection.prepare("SELECT n.note_id,p.path,n.title FROM notes n JOIN paths p USING(note_id) ORDER BY p.path,n.note_id")?.query_map([], |row| Ok(DumpNote { note_id: row.get(0)?, path: row.get(1)?, title: row.get(2)? }))?.collect::<Result<Vec<_>, _>>()?;
    let links = connection
        .prepare("SELECT note_id,target,label FROM links ORDER BY note_id,target,label")?
        .query_map([], |row| {
            Ok(DumpLink {
                source_note_id: row.get(0)?,
                target: row.get(1)?,
                resolved_note_id: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LogicalDump { notes, links })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn population_uses_one_enclosing_transaction() {
        let connection = Connection::open_in_memory().unwrap();

        begin_population(&connection).unwrap();

        assert!(!connection.is_autocommit());
        assert!(begin_population(&connection).is_err());
        connection.execute_batch("ROLLBACK").unwrap();
        assert!(connection.is_autocommit());
    }
}
