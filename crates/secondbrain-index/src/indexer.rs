use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::extract::{PropertyValue, extract};
use secondbrain_markdown::{SourceDocument, parse_metadata};
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
    #[error("duplicate note ID {note_id} in {first} and {second}")]
    DuplicateId {
        note_id: NoteId,
        first: String,
        second: String,
    },
    #[error("identity resolution failed for {path}: {message}")]
    Identity { path: String, message: String },
    #[error("SQLite index operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

struct Note {
    path: WorkspacePath,
    source: String,
    document: SourceDocument,
    id: Option<NoteId>,
}

pub fn rebuild(root: impl AsRef<Path>, config: &IndexConfig) -> Result<IndexReport, IndexError> {
    let root = root.as_ref();
    let mut skipped = 0;
    let mut files = Vec::new();
    scan(root, root, config, &mut files, &mut skipped)?;
    files.sort();

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
            id: metadata.id,
        });
    }

    establish_ids(root, &mut notes)?;
    let internal = root.join(".secondbrain");
    fs::create_dir_all(&internal).map_err(|source| IndexError::Io {
        path: internal.clone(),
        source,
    })?;
    let active = internal.join("index.sqlite");
    let temporary = internal.join("index.sqlite.rebuild");
    remove_sqlite_files(&temporary)?;

    let mut database = crate::IndexDatabase::open(&temporary).map_err(database_error)?;
    database.migrate().map_err(database_error)?;
    let mut report = populate(database.connection(), &notes)?;
    report.skipped = skipped;
    check_database(database.connection())?;
    database
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(database);
    remove_sidecars(&active)?;
    fs::rename(&temporary, &active).map_err(|source| IndexError::Io {
        path: active,
        source,
    })?;
    Ok(report)
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
        let relative = path
            .strip_prefix(root)
            .expect("scanned path is below root")
            .to_string_lossy()
            .replace('\\', "/");
        let internal = relative == ".git" || relative == ".secondbrain";
        let excluded = config
            .exclusions
            .iter()
            .any(|pattern| relative == *pattern || relative.starts_with(&format!("{pattern}/")));
        if internal || excluded {
            *skipped += count_markdown(&path)?;
        } else if path.is_dir() {
            scan(root, &path, config, files, skipped)?;
        } else if is_markdown(&path) {
            files.push((relative, path));
        }
    }
    Ok(())
}

fn count_markdown(path: &Path) -> Result<usize, IndexError> {
    if path.is_file() {
        return Ok(usize::from(is_markdown(path)));
    }
    let mut count = 0;
    for entry in fs::read_dir(path).map_err(|source| IndexError::Io {
        path: path.to_path_buf(),
        source,
    })? {
        let child = entry
            .map_err(|source| IndexError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .path();
        count += count_markdown(&child)?;
    }
    Ok(count)
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("md" | "markdown")
    )
}

fn establish_ids(root: &Path, notes: &mut [Note]) -> Result<(), IndexError> {
    let workspace = WorkspaceRoot::open(root).map_err(|error| IndexError::Identity {
        path: root.display().to_string(),
        message: error.to_string(),
    })?;
    let mut map = IdentityMap::open(&workspace).map_err(|error| IndexError::Identity {
        path: root.display().to_string(),
        message: error.to_string(),
    })?;
    for note in notes {
        if note.id.is_some() {
            continue;
        }
        let hash = ContentHash::digest(note.source.as_bytes());
        let fingerprint = note.document.semantic_fingerprint();
        let outcome = map
            .resolve_identity(&note.path, hash, fingerprint)
            .map_err(|error| IndexError::Identity {
                path: note.path.to_string(),
                message: error.to_string(),
            })?;
        note.id = Some(match outcome {
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
        });
    }
    Ok(())
}

fn populate(connection: &Connection, notes: &[Note]) -> Result<IndexReport, IndexError> {
    connection.execute_batch("BEGIN IMMEDIATE;")?;
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

fn populate_inner(connection: &Connection, notes: &[Note]) -> Result<IndexReport, IndexError> {
    let mut lookup = BTreeMap::new();
    for note in notes {
        let metadata = parse_metadata(&note.source).expect("metadata validated before population");
        let id = note.id.expect("identity established before population");
        lookup.insert(note.path.as_str().to_lowercase(), id);
        lookup.insert(
            note.path
                .as_str()
                .trim_end_matches(".md")
                .trim_end_matches(".markdown")
                .to_lowercase(),
            id,
        );
        if let Some(name) = note
            .path
            .as_path()
            .file_stem()
            .and_then(|value| value.to_str())
        {
            lookup.insert(name.to_lowercase(), id);
        }
        if let Some(title) = &metadata.title {
            lookup.insert(title.to_lowercase(), id);
        }
        for alias in aliases(&metadata.properties) {
            lookup.insert(alias.to_lowercase(), id);
        }
    }
    let mut linked = BTreeSet::new();
    let mut broken = 0;
    for note in notes {
        let id = note.id.expect("identity established");
        let metadata = parse_metadata(&note.source).expect("validated");
        let extracted = extract(&note.document);
        let title = metadata.title.or_else(|| {
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
            let resolved = lookup.get(&link.target.to_lowercase()).copied();
            if let Some(target) = resolved {
                linked.insert(id);
                linked.insert(target);
            } else {
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
