#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_index::{
    IndexConfig, IndexDatabase, SearchQuery, index_path, logical_dump, rebuild,
};
use secondbrain_vault::{WorkspaceRoot, load_manifest};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    note_id: String,
    path: String,
    title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    note_id: String,
    path: String,
    title: Option<String>,
    snippet: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    root: String,
    workspace_id: String,
    notes: Vec<NoteSummary>,
    indexed: usize,
    broken_links: usize,
    orphans: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDocument {
    note_id: String,
    path: String,
    title: Option<String>,
    source: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineHeading {
    level: u8,
    text: String,
    line: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlink {
    note_id: String,
    path: String,
    title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContext {
    note_id: String,
    outline: Vec<OutlineHeading>,
    backlinks: Vec<Backlink>,
}

pub fn open_workspace_at(root: &Path) -> Result<WorkspaceSummary, String> {
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("workspace path cannot be opened: {error}"))?;
    let manifest = load_manifest(&canonical).map_err(|error| error.to_string())?;
    let report = rebuild(&canonical, &IndexConfig::default()).map_err(|error| error.to_string())?;
    let dump = logical_dump(index_path(&canonical)).map_err(|error| error.to_string())?;
    let notes = dump
        .notes
        .into_iter()
        .map(|note| NoteSummary {
            note_id: note.note_id.to_string(),
            path: note.path,
            title: note.title,
        })
        .collect();
    Ok(WorkspaceSummary {
        root: canonical.to_string_lossy().into_owned(),
        workspace_id: manifest.workspace_id.to_string(),
        notes,
        indexed: report.indexed,
        broken_links: report.broken_links,
        orphans: report.orphans,
    })
}

pub fn search_workspace_at(root: &Path, query: &str) -> Result<Vec<SearchResult>, String> {
    load_manifest(root).map_err(|error| error.to_string())?;
    let database = IndexDatabase::open(index_path(root)).map_err(|error| error.to_string())?;
    database
        .search(&SearchQuery::new(query))
        .map_err(|error| error.to_string())
        .map(|hits| {
            hits.into_iter()
                .map(|hit| SearchResult {
                    note_id: hit.note_id.to_string(),
                    path: hit.path,
                    title: hit.title,
                    snippet: hit.snippet,
                })
                .collect()
        })
}

pub fn read_note_at(root: &Path, path: &str) -> Result<NoteDocument, String> {
    let workspace = WorkspaceRoot::open(root).map_err(|error| error.to_string())?;
    load_manifest(workspace.canonical_path()).map_err(|error| error.to_string())?;
    let path = WorkspacePath::new(path).map_err(|error| error.to_string())?;
    let database = IndexDatabase::open(index_path(workspace.canonical_path()))
        .map_err(|error| error.to_string())?;
    let summary = database
        .note_by_path(path.as_str())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("note is not present in the workspace index: {path}"))?;
    let source = fs::read_to_string(
        workspace
            .resolve(&path)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("note cannot be read: {error}"))?;

    Ok(NoteDocument {
        note_id: summary.note_id.to_string(),
        path: summary.path,
        title: summary.title,
        source,
    })
}

pub fn note_context_at(root: &Path, note_id: &str) -> Result<NoteContext, String> {
    let workspace = WorkspaceRoot::open(root).map_err(|error| error.to_string())?;
    load_manifest(workspace.canonical_path()).map_err(|error| error.to_string())?;
    let note_id = note_id
        .parse::<NoteId>()
        .map_err(|error| format!("invalid note id: {error}"))?;
    let database = IndexDatabase::open(index_path(workspace.canonical_path()))
        .map_err(|error| error.to_string())?;
    database
        .note_by_id(note_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("note is not present in the workspace index: {note_id}"))?;
    let outline = database
        .headings(note_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|heading| OutlineHeading {
            level: heading.level,
            text: heading.text,
            line: heading.line,
        })
        .collect();
    let backlinks = database
        .backlinks(note_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|link| Backlink {
            note_id: link.note_id.expect("backlinks are resolved").to_string(),
            path: link.path.expect("backlinks have a current path"),
            title: link.title,
        })
        .collect();
    Ok(NoteContext {
        note_id: note_id.to_string(),
        outline,
        backlinks,
    })
}

#[tauri::command]
async fn open_workspace(root: PathBuf) -> Result<WorkspaceSummary, String> {
    tauri::async_runtime::spawn_blocking(move || open_workspace_at(&root))
        .await
        .map_err(|error| format!("workspace task failed: {error}"))?
}

#[tauri::command]
async fn search_workspace(root: PathBuf, query: String) -> Result<Vec<SearchResult>, String> {
    tauri::async_runtime::spawn_blocking(move || search_workspace_at(&root, &query))
        .await
        .map_err(|error| format!("search task failed: {error}"))?
}

#[tauri::command]
async fn read_note(root: PathBuf, path: String) -> Result<NoteDocument, String> {
    tauri::async_runtime::spawn_blocking(move || read_note_at(&root, &path))
        .await
        .map_err(|error| format!("note read task failed: {error}"))?
}

#[tauri::command]
async fn note_context(root: PathBuf, note_id: String) -> Result<NoteContext, String> {
    tauri::async_runtime::spawn_blocking(move || note_context_at(&root, &note_id))
        .await
        .map_err(|error| format!("note context task failed: {error}"))?
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            open_workspace,
            search_workspace,
            read_note,
            note_context
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed");
}

#[cfg(test)]
mod tests {
    use std::fs;

    use secondbrain_vault::initialize_workspace;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn workspace_browser_uses_the_real_manifest_index_and_search_contracts() {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        fs::write(
            directory.path().join("welcome.md"),
            "---\ntitle: Welcome\n---\n# Welcome\nphase-one-canary\n",
        )
        .unwrap();

        let summary = open_workspace_at(directory.path()).unwrap();
        assert_eq!(summary.indexed, 1);
        assert_eq!(summary.notes[0].title.as_deref(), Some("Welcome"));

        let hits = search_workspace_at(directory.path(), "phase-one-canary").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "welcome.md");

        let note = read_note_at(directory.path(), "welcome.md").unwrap();
        assert_eq!(note.note_id, summary.notes[0].note_id);
        assert_eq!(note.title.as_deref(), Some("Welcome"));
        assert_eq!(
            note.source,
            "---\ntitle: Welcome\n---\n# Welcome\nphase-one-canary\n"
        );
    }

    #[test]
    fn unopened_directory_is_rejected_without_initializing_it() {
        let directory = tempdir().unwrap();
        assert!(open_workspace_at(directory.path()).is_err());
        assert!(!directory.path().join(".secondbrain").exists());
    }

    #[test]
    fn note_read_rejects_unindexed_and_escaping_paths() {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        open_workspace_at(directory.path()).unwrap();

        assert!(read_note_at(directory.path(), "missing.md").is_err());
        assert!(read_note_at(directory.path(), "../outside.md").is_err());
    }

    #[test]
    fn note_context_uses_indexed_outline_and_resolved_backlinks() {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        fs::write(
            directory.path().join("target.md"),
            "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\ntitle: Target\n---\n# Target\n\n### Detail\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("z-source.md"),
            "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAW\ntitle: Zed\n---\n[[Target]]\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("a-source.md"),
            "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAX\ntitle: Alpha\n---\n[[Target]]\n",
        )
        .unwrap();
        open_workspace_at(directory.path()).unwrap();

        let context = note_context_at(directory.path(), "01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();

        assert_eq!(context.note_id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(
            context
                .outline
                .iter()
                .map(|heading| (heading.level, heading.text.as_str(), heading.line))
                .collect::<Vec<_>>(),
            [(1, "Target", 5), (3, "Detail", 7)]
        );
        assert_eq!(
            context
                .backlinks
                .iter()
                .map(|link| (
                    link.note_id.as_str(),
                    link.path.as_str(),
                    link.title.as_deref()
                ))
                .collect::<Vec<_>>(),
            [
                ("01ARZ3NDEKTSV4RRFFQ69G5FAX", "a-source.md", Some("Alpha")),
                ("01ARZ3NDEKTSV4RRFFQ69G5FAW", "z-source.md", Some("Zed")),
            ]
        );
    }

    #[test]
    fn note_context_has_empty_collections_when_no_context_exists() {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        fs::write(
            directory.path().join("plain.md"),
            "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n---\nPlain text.\n",
        )
        .unwrap();
        open_workspace_at(directory.path()).unwrap();

        let context = note_context_at(directory.path(), "01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();

        assert!(context.outline.is_empty());
        assert!(context.backlinks.is_empty());
    }

    #[test]
    fn note_context_rejects_invalid_and_missing_note_ids() {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        open_workspace_at(directory.path()).unwrap();

        let invalid = note_context_at(directory.path(), "not-a-note-id").unwrap_err();
        let missing = note_context_at(directory.path(), "01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap_err();

        assert!(invalid.starts_with("invalid note id:"), "{invalid}");
        assert!(
            missing.contains("note is not present in the workspace index"),
            "{missing}"
        );
    }
}
