#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_diagnostics::{RecoveryReport, WorkspaceReport};
use secondbrain_index::{
    ImportApplyOutcome, ImportPreview, IndexConfig, IndexDatabase, SearchQuery, WorkspaceGraph,
    apply_import, ensure_index, index_path, logical_dump, preview_import,
};
use secondbrain_markdown::{PropertyEdit, PropertyValue};
use secondbrain_transaction::{
    ApplyPreviewOutcome, DailyDate, DailyNote, NoteCreateOutcome, NoteCreatePreview,
    PropertyPreview, TransactionPreview, apply_note_creation, apply_preview,
    apply_property_preview, open_or_preview_daily_note, preview_property, preview_transaction,
    read_note_properties,
};
use secondbrain_vault::{BaseSnapshotStore, WorkspaceRoot, load_manifest};
use serde::Serialize;

const DESKTOP_ACTOR: &str = "secondbrain-desktop";
const DESKTOP_DEVICE: &str = "local-desktop";

fn desktop_identities() -> (ActorId, DeviceId) {
    (
        ActorId::new(DESKTOP_ACTOR).expect("desktop actor identity is valid"),
        DeviceId::new(DESKTOP_DEVICE).expect("desktop device identity is valid"),
    )
}

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
    version: u64,
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
    let report = ensure_index(&canonical, &IndexConfig::default())
        .map_err(|error| error.to_string())?
        .report;
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

pub fn import_preview_at(root: &Path) -> Result<ImportPreview, String> {
    preview_import(root).map_err(|error| error.to_string())
}

pub fn import_apply_at(root: &Path, preview: &ImportPreview) -> Result<ImportApplyOutcome, String> {
    apply_import(root, preview).map_err(|error| error.to_string())
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
    let version = BaseSnapshotStore::new(&workspace)
        .load(summary.note_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("note has no converged base: {}", summary.note_id))?
        .version
        .get();

    Ok(NoteDocument {
        note_id: summary.note_id.to_string(),
        path: summary.path,
        title: summary.title,
        source,
        version,
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

pub fn workspace_graph_at(root: &Path) -> Result<WorkspaceGraph, String> {
    let workspace = WorkspaceRoot::open(root).map_err(|error| error.to_string())?;
    load_manifest(workspace.canonical_path()).map_err(|error| error.to_string())?;
    IndexDatabase::open(index_path(workspace.canonical_path()))
        .map_err(|error| error.to_string())?
        .workspace_graph()
        .map_err(|error| error.to_string())
}

pub fn transaction_preview_at(
    root: &Path,
    path: &str,
    proposed_source: &str,
) -> Result<TransactionPreview, String> {
    let (actor, device) = desktop_identities();
    preview_transaction(root, path, proposed_source, actor, device)
        .map_err(|error| error.to_string())
}

pub fn transaction_apply_at(
    root: &Path,
    preview: &TransactionPreview,
) -> Result<ApplyPreviewOutcome, String> {
    let (actor, device) = desktop_identities();
    if preview.actor != actor || preview.device != device {
        return Err("transaction preview does not use local desktop identities".to_owned());
    }
    apply_preview(root, preview).map_err(|error| error.to_string())
}

pub fn properties_read_at(
    root: &Path,
    path: &str,
) -> Result<std::collections::BTreeMap<String, PropertyValue>, String> {
    read_note_properties(root, path).map_err(|error| error.to_string())
}

pub fn property_preview_at(
    root: &Path,
    path: &str,
    edit: PropertyEdit,
) -> Result<PropertyPreview, String> {
    let (actor, device) = desktop_identities();
    preview_property(root, path, edit, actor, device).map_err(|error| error.to_string())
}

pub fn property_apply_at(
    root: &Path,
    preview: &PropertyPreview,
) -> Result<ApplyPreviewOutcome, String> {
    let (actor, device) = desktop_identities();
    if preview.transaction.actor != actor || preview.transaction.device != device {
        return Err("property preview does not use local desktop identities".to_owned());
    }
    apply_property_preview(root, preview).map_err(|error| error.to_string())
}

pub fn daily_note_at(root: &Path, date: &str) -> Result<DailyNote, String> {
    let (actor, device) = desktop_identities();
    open_or_preview_daily_note(
        root,
        DailyDate::new(date).map_err(|e| e.to_string())?,
        actor,
        device,
    )
    .map_err(|e| e.to_string())
}

pub fn note_create_apply_at(
    root: &Path,
    preview: &NoteCreatePreview,
) -> Result<NoteCreateOutcome, String> {
    let (actor, device) = desktop_identities();
    if preview.actor != actor || preview.device != device {
        return Err("note creation preview does not use local desktop identities".into());
    }
    apply_note_creation(root, preview).map_err(|e| e.to_string())
}

pub fn inspect_workspace_at(root: &Path) -> Result<WorkspaceReport, String> {
    secondbrain_diagnostics::inspect_workspace(root).map_err(|error| error.to_string())
}

pub fn recover_workspace_at(root: &Path) -> Result<RecoveryReport, String> {
    secondbrain_diagnostics::recover_workspace(root).map_err(|error| error.to_string())
}

#[tauri::command]
async fn open_workspace(root: PathBuf) -> Result<WorkspaceSummary, String> {
    tauri::async_runtime::spawn_blocking(move || open_workspace_at(&root))
        .await
        .map_err(|error| format!("workspace task failed: {error}"))?
}

#[tauri::command]
async fn import_preview(root: PathBuf) -> Result<ImportPreview, String> {
    tauri::async_runtime::spawn_blocking(move || import_preview_at(&root))
        .await
        .map_err(|error| format!("import preview task failed: {error}"))?
}

#[tauri::command]
async fn import_apply(root: PathBuf, preview: ImportPreview) -> Result<ImportApplyOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || import_apply_at(&root, &preview))
        .await
        .map_err(|error| format!("import apply task failed: {error}"))?
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

#[tauri::command]
async fn workspace_graph(root: PathBuf) -> Result<WorkspaceGraph, String> {
    tauri::async_runtime::spawn_blocking(move || workspace_graph_at(&root))
        .await
        .map_err(|error| format!("workspace graph task failed: {error}"))?
}

#[tauri::command]
async fn transaction_preview(
    root: PathBuf,
    path: String,
    proposed_source: String,
) -> Result<TransactionPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        transaction_preview_at(&root, &path, &proposed_source)
    })
    .await
    .map_err(|error| format!("transaction preview task failed: {error}"))?
}

#[tauri::command]
async fn transaction_apply(
    root: PathBuf,
    preview: TransactionPreview,
) -> Result<ApplyPreviewOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || transaction_apply_at(&root, &preview))
        .await
        .map_err(|error| format!("transaction apply task failed: {error}"))?
}

#[tauri::command]
async fn properties_read(
    root: PathBuf,
    path: String,
) -> Result<std::collections::BTreeMap<String, PropertyValue>, String> {
    tauri::async_runtime::spawn_blocking(move || properties_read_at(&root, &path))
        .await
        .map_err(|error| format!("property read task failed: {error}"))?
}

#[tauri::command]
async fn property_preview(
    root: PathBuf,
    path: String,
    edit: PropertyEdit,
) -> Result<PropertyPreview, String> {
    tauri::async_runtime::spawn_blocking(move || property_preview_at(&root, &path, edit))
        .await
        .map_err(|error| format!("property preview task failed: {error}"))?
}

#[tauri::command]
async fn property_apply(
    root: PathBuf,
    preview: PropertyPreview,
) -> Result<ApplyPreviewOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || property_apply_at(&root, &preview))
        .await
        .map_err(|error| format!("property apply task failed: {error}"))?
}

#[tauri::command]
async fn daily_note(root: PathBuf, date: String) -> Result<DailyNote, String> {
    tauri::async_runtime::spawn_blocking(move || daily_note_at(&root, &date))
        .await
        .map_err(|e| format!("daily note task failed: {e}"))?
}

#[tauri::command]
async fn note_create_apply(
    root: PathBuf,
    preview: NoteCreatePreview,
) -> Result<NoteCreateOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || note_create_apply_at(&root, &preview))
        .await
        .map_err(|e| format!("note creation task failed: {e}"))?
}

#[tauri::command]
async fn inspect_workspace(root: PathBuf) -> Result<WorkspaceReport, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_workspace_at(&root))
        .await
        .map_err(|error| format!("workspace inspection task failed: {error}"))?
}

#[tauri::command]
async fn recover_workspace(root: PathBuf) -> Result<RecoveryReport, String> {
    tauri::async_runtime::spawn_blocking(move || recover_workspace_at(&root))
        .await
        .map_err(|error| format!("workspace recovery task failed: {error}"))?
}

fn release_readiness_at(path: Option<&Path>, frontend_version: &str) -> Result<bool, String> {
    let Some(path) = path else {
        return Ok(false);
    };
    let backend_version = env!("CARGO_PKG_VERSION");
    if frontend_version != backend_version {
        return Err(format!(
            "frontend/backend version mismatch: {frontend_version} != {backend_version}"
        ));
    }
    let commit = option_env!("SB_BUILD_COMMIT").unwrap_or("unknown");
    let fixture_sha256 = option_env!("SB_FIXTURE_SHA256").unwrap_or("unavailable");
    if !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) || commit.len() != 40 {
        return Err("release readiness requires a compile-time full Git commit".into());
    }
    if !fixture_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) || fixture_sha256.len() != 64 {
        return Err("release readiness requires a compile-time fixture SHA-256".into());
    }
    let marker = serde_json::json!({
        "schema": "secondbrain.desktop.readiness.v1",
        "version": backend_version,
        "commit": commit,
        "platform": std::env::consts::OS,
        "fixture_sha256": fixture_sha256,
        "diagnostics": "frontend-page-loaded-and-backend-ready"
    });
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&marker).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write readiness marker: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish readiness marker: {error}"))?;
    Ok(true)
}

#[tauri::command]
fn release_readiness(
    window: tauri::WebviewWindow,
    frontend_version: String,
) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("release readiness is restricted to the main webview".into());
    }
    release_readiness_at(
        env::var_os("SB_READINESS_MARKER").as_deref().map(Path::new),
        &frontend_version,
    )
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            release_readiness,
            open_workspace,
            import_preview,
            import_apply,
            search_workspace,
            read_note,
            note_context,
            workspace_graph,
            transaction_preview,
            transaction_apply,
            properties_read,
            property_preview,
            property_apply,
            daily_note,
            note_create_apply,
            inspect_workspace,
            recover_workspace
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use secondbrain_core::id::NoteVersion;
    use secondbrain_index::rebuild;
    use secondbrain_index::{IndexDatabase, IndexHealth, SearchQuery, index_health, index_path};
    use secondbrain_vault::initialize_workspace;
    use tempfile::tempdir;

    #[test]
    fn import_commands_adopt_an_unopened_vault_without_changing_user_bytes_then_open_it() {
        let vault = tempdir().unwrap();
        fs::create_dir_all(vault.path().join(".obsidian")).unwrap();
        let note = b"# Imported\n\n[[Missing note]]\n";
        let config = br#"{"useMarkdownLinks":false}"#;
        let attachment = [0_u8, 1, 2, 255];
        fs::write(vault.path().join("imported.md"), note).unwrap();
        fs::write(vault.path().join(".obsidian/app.json"), config).unwrap();
        fs::write(vault.path().join("diagram.png"), attachment).unwrap();

        let preview = import_preview_at(vault.path()).unwrap();
        assert!(!preview.already_initialized);
        assert_eq!(preview.broken_links.len(), 1);
        assert_eq!(preview.planned_writes.markdown, 0);
        assert!(!vault.path().join(".secondbrain").exists());
        let outcome = import_apply_at(vault.path(), &preview).unwrap();
        assert_eq!(outcome.status, "initialized");
        assert_eq!(fs::read(vault.path().join("imported.md")).unwrap(), note);
        assert_eq!(
            fs::read(vault.path().join(".obsidian/app.json")).unwrap(),
            config
        );
        assert_eq!(
            fs::read(vault.path().join("diagram.png")).unwrap(),
            attachment
        );
        let opened = open_workspace_at(vault.path()).unwrap();
        assert_eq!(opened.workspace_id, outcome.workspace_id.to_string());
        assert_eq!(opened.indexed, 1);
    }

    use super::*;

    #[test]
    fn readiness_is_inert_without_a_marker_and_rejects_frontend_mismatch() {
        assert!(!release_readiness_at(None, env!("CARGO_PKG_VERSION")).unwrap());
        let directory = tempdir().unwrap();
        let error = release_readiness_at(Some(&directory.path().join("readiness.json")), "wrong")
            .unwrap_err();
        assert!(error.contains("frontend/backend version mismatch"));
        assert!(!directory.path().join("readiness.json").exists());
    }

    const EDIT_NOTE: &str = "edit.md";
    const EDIT_BASE: &str = "# Editing\n\nOriginal desktop text.\n\n## Original context\n";

    fn editable_workspace() -> tempfile::TempDir {
        editable_workspace_with(EDIT_BASE)
    }

    fn editable_workspace_with(source: &str) -> tempfile::TempDir {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        fs::write(directory.path().join(EDIT_NOTE), source).unwrap();
        open_workspace_at(directory.path()).unwrap();
        directory
    }

    fn files_below(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let mut entries = fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }

        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

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
        assert_eq!(note.version, 0);
        assert_eq!(
            note.source,
            "---\ntitle: Welcome\n---\n# Welcome\nphase-one-canary\n"
        );
    }

    #[test]
    fn workspace_open_reuses_a_valid_index_and_rebuilds_stale_content() {
        let directory = tempdir().unwrap();
        initialize_workspace(directory.path()).unwrap();
        let note = directory.path().join("welcome.md");
        fs::write(&note, "# Welcome\n\nfirst value\n").unwrap();
        let first = open_workspace_at(directory.path()).unwrap();
        assert_eq!(
            index_health(directory.path(), &IndexConfig::default()).unwrap(),
            IndexHealth::Valid
        );

        let reused = open_workspace_at(directory.path()).unwrap();
        assert_eq!(reused.notes, first.notes);

        fs::write(&note, "# Welcome\n\nsecond value\n").unwrap();
        assert_eq!(
            index_health(directory.path(), &IndexConfig::default()).unwrap(),
            IndexHealth::Stale
        );
        open_workspace_at(directory.path()).unwrap();
        assert_eq!(
            search_workspace_at(directory.path(), "second value")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn desktop_diagnostics_commands_use_shared_real_workspace_reports() {
        let workspace = editable_workspace();
        let report = inspect_workspace_at(workspace.path()).unwrap();
        assert_eq!(report.format_version, 1);
        assert_eq!(report.status, "healthy");
        let recovery = recover_workspace_at(workspace.path()).unwrap();
        assert_eq!(recovery.status, "nothing_to_recover");
        assert!(recovery.actions.is_empty());
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

    #[test]
    fn transaction_preview_is_read_only_and_uses_desktop_identities() {
        let workspace = editable_workspace();
        let before = files_below(workspace.path());

        let preview = transaction_preview_at(
            workspace.path(),
            EDIT_NOTE,
            "# Editing\n\nProposed desktop text.\n\n## Original context\n",
        )
        .unwrap();

        assert_eq!(preview.actor.as_str(), DESKTOP_ACTOR);
        assert_eq!(preview.device.as_str(), DESKTOP_DEVICE);
        assert!(!preview.operations.is_empty());
        assert_eq!(files_below(workspace.path()), before);
    }

    #[test]
    fn transaction_apply_materializes_and_refreshes_shared_read_and_index_contracts() {
        let workspace = editable_workspace();
        let proposed = "# Editing\n\nDesktop apply searchable canary.\n\n## Context\n";
        let preview = transaction_preview_at(workspace.path(), EDIT_NOTE, proposed).unwrap();

        let outcome = transaction_apply_at(workspace.path(), &preview).unwrap();

        assert!(outcome.changed);
        assert!(outcome.index_refreshed);
        assert_eq!(outcome.version, NoteVersion::new(1));
        assert_eq!(
            fs::read_to_string(workspace.path().join(EDIT_NOTE)).unwrap(),
            proposed
        );

        let note = read_note_at(workspace.path(), EDIT_NOTE).unwrap();
        assert_eq!(note.note_id, outcome.note_id.to_string());
        assert_eq!(note.source, proposed);
        assert_eq!(note.version, outcome.version.get());
        let context = note_context_at(workspace.path(), &note.note_id).unwrap();
        assert_eq!(
            context
                .outline
                .iter()
                .map(|heading| (heading.level, heading.text.as_str()))
                .collect::<Vec<_>>(),
            [(1, "Editing"), (2, "Context")]
        );

        let database = IndexDatabase::open(index_path(workspace.path())).unwrap();
        let hits = database
            .search(&SearchQuery::new("searchable canary"))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, EDIT_NOTE);
    }

    #[test]
    fn transaction_apply_rejects_a_stale_preview() {
        let workspace = editable_workspace();
        let preview = transaction_preview_at(
            workspace.path(),
            EDIT_NOTE,
            "# Editing\n\nProposed desktop text.\n\n## Original context\n",
        )
        .unwrap();
        let external = "# Editing\n\nExternal edit won.\n";
        fs::write(workspace.path().join(EDIT_NOTE), external).unwrap();

        let error = transaction_apply_at(workspace.path(), &preview).unwrap_err();

        assert!(error.contains("stale note hash"), "{error}");
        assert_eq!(
            fs::read_to_string(workspace.path().join(EDIT_NOTE)).unwrap(),
            external
        );
    }

    #[test]
    fn transaction_apply_rejects_non_desktop_attribution() {
        let workspace = editable_workspace();
        let mut preview = transaction_preview_at(
            workspace.path(),
            EDIT_NOTE,
            "# Editing\n\nProposed desktop text.\n\n## Original context\n",
        )
        .unwrap();
        preview.actor = ActorId::new("caller-supplied-actor").unwrap();

        let error = transaction_apply_at(workspace.path(), &preview).unwrap_err();

        assert!(error.contains("local desktop identities"), "{error}");
        assert_eq!(
            fs::read_to_string(workspace.path().join(EDIT_NOTE)).unwrap(),
            EDIT_BASE
        );
    }

    #[test]
    fn transaction_apply_rejects_needs_review() {
        let base = "Duplicate.\n\nMiddle.\n\nDuplicate.\n";
        let workspace = editable_workspace_with(base);
        let preview = transaction_preview_at(
            workspace.path(),
            EDIT_NOTE,
            "Duplicate.\n\nMiddle.\n\nChanged.\n",
        )
        .unwrap();
        assert!(preview.review_required);

        let error = transaction_apply_at(workspace.path(), &preview).unwrap_err();

        assert!(error.contains("requires human review"), "{error}");
        assert_eq!(
            fs::read_to_string(workspace.path().join(EDIT_NOTE)).unwrap(),
            base
        );
    }

    #[test]
    fn no_op_apply_does_not_churn_version_or_workspace_files() {
        let workspace = editable_workspace();
        let preview = transaction_preview_at(workspace.path(), EDIT_NOTE, EDIT_BASE).unwrap();
        let before = files_below(workspace.path());

        let outcome = transaction_apply_at(workspace.path(), &preview).unwrap();

        assert!(!outcome.changed);
        assert!(!outcome.index_refreshed);
        assert_eq!(outcome.version, preview.expected_version);
        assert_eq!(outcome.version, NoteVersion::new(0));
        assert_eq!(files_below(workspace.path()), before);
    }

    #[test]
    fn property_commands_use_fixed_attribution_and_shared_apply_contracts() {
        let workspace =
            editable_workspace_with("---\ntitle: Old # preserved\n---\n# Editing\n\nBody\n");
        let before = files_below(workspace.path());
        let preview = property_preview_at(
            workspace.path(),
            EDIT_NOTE,
            PropertyEdit::Set {
                key: "rating".into(),
                value: serde_json::json!(5),
            },
        )
        .unwrap();
        assert_eq!(preview.transaction.actor.as_str(), DESKTOP_ACTOR);
        assert_eq!(preview.transaction.device.as_str(), DESKTOP_DEVICE);
        assert_eq!(preview.properties["rating"], serde_json::json!(5));
        assert_eq!(files_below(workspace.path()), before);

        let outcome = property_apply_at(workspace.path(), &preview).unwrap();
        assert!(outcome.changed);
        assert_eq!(
            properties_read_at(workspace.path(), EDIT_NOTE).unwrap()["rating"],
            serde_json::json!(5)
        );
        assert!(
            fs::read_to_string(workspace.path().join(EDIT_NOTE))
                .unwrap()
                .contains("title: Old # preserved\nrating: 5\n---\n# Editing")
        );

        let mut foreign = property_preview_at(
            workspace.path(),
            EDIT_NOTE,
            PropertyEdit::Remove {
                key: "rating".into(),
            },
        )
        .unwrap();
        foreign.transaction.actor = ActorId::new("caller-supplied").unwrap();
        assert!(
            property_apply_at(workspace.path(), &foreign)
                .unwrap_err()
                .contains("local desktop identities")
        );
    }

    #[test]
    fn graph_refreshes_after_title_and_alias_property_edits() {
        let workspace = editable_workspace_with(
            "---\ntitle: Old title\naliases: [Old alias]\n---\n# Editing\n",
        );
        fs::write(
            workspace.path().join("source.md"),
            "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n---\n[[Old alias]]\n",
        )
        .unwrap();
        open_workspace_at(workspace.path()).unwrap();
        let before = workspace_graph_at(workspace.path()).unwrap();
        assert_eq!(before.edges.len(), 1);

        for edit in [
            PropertyEdit::Set {
                key: "title".into(),
                value: serde_json::json!("New title"),
            },
            PropertyEdit::Set {
                key: "aliases".into(),
                value: serde_json::json!(["New alias"]),
            },
        ] {
            let preview = property_preview_at(workspace.path(), EDIT_NOTE, edit).unwrap();
            property_apply_at(workspace.path(), &preview).unwrap();
        }

        let after = workspace_graph_at(workspace.path()).unwrap();
        let edited = after
            .nodes
            .iter()
            .find(|node| node.path == EDIT_NOTE)
            .unwrap();
        assert_eq!(edited.title.as_deref(), Some("New title"));
        assert!(after.edges.is_empty());
        assert_eq!(after.broken_links[0].target, "Old alias");
    }

    #[test]
    fn daily_note_commands_preview_read_only_apply_with_desktop_attribution_and_then_open() {
        let workspace = tempdir().unwrap();
        initialize_workspace(workspace.path()).unwrap();
        rebuild(workspace.path(), &IndexConfig::default()).unwrap();
        let before = files_below(workspace.path());
        let DailyNote::Create { preview } = daily_note_at(workspace.path(), "2026-07-26").unwrap()
        else {
            panic!("daily note must be absent")
        };
        assert_eq!(preview.actor.as_str(), DESKTOP_ACTOR);
        assert_eq!(preview.device.as_str(), DESKTOP_DEVICE);
        assert_eq!(files_below(workspace.path()), before);
        assert!(!workspace.path().join("Daily").exists());

        let outcome = note_create_apply_at(workspace.path(), &preview).unwrap();
        assert!(outcome.created && outcome.index_refreshed);
        let graph = workspace_graph_at(workspace.path()).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].path, "Daily/2026-07-26.md");
        assert!(
            matches!(daily_note_at(workspace.path(), "2026-07-26").unwrap(), DailyNote::Existing { note_id, .. } if note_id == preview.note_id)
        );
        let mut foreign = preview;
        foreign.actor = ActorId::new("caller-supplied").unwrap();
        assert!(
            note_create_apply_at(workspace.path(), &foreign)
                .unwrap_err()
                .contains("local desktop identities")
        );
    }

    #[test]
    fn markdown_corpus_survives_desktop_preview_and_apply() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("fixtures/markdown");
        let mut fixtures = Vec::new();
        fn collect(directory: &Path, fixtures: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(&path, fixtures);
                } else if path.extension().is_some_and(|extension| extension == "md") {
                    fixtures.push(path);
                }
            }
        }
        collect(&corpus, &mut fixtures);
        fixtures.sort();
        assert!(!fixtures.is_empty());
        let expected_index_rejections = ["frontmatter/duplicate_id.md", "frontmatter/malformed.md"];
        let mut rejected = Vec::new();

        for fixture in fixtures {
            let source = fs::read(&fixture).unwrap();
            let source_text = String::from_utf8(source.clone()).unwrap();
            let workspace = tempdir().unwrap();
            initialize_workspace(workspace.path()).unwrap();
            fs::write(workspace.path().join(EDIT_NOTE), &source).unwrap();
            if let Err(error) = open_workspace_at(workspace.path()) {
                let name = fixture
                    .strip_prefix(&corpus)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                assert!(
                    expected_index_rejections.contains(&name.as_str()),
                    "{} unexpectedly failed to index: {error}",
                    fixture.display()
                );
                assert_eq!(fs::read(workspace.path().join(EDIT_NOTE)).unwrap(), source);
                rejected.push(name);
                continue;
            }
            let before = files_below(workspace.path());

            let preview = transaction_preview_at(workspace.path(), EDIT_NOTE, &source_text)
                .unwrap_or_else(|error| panic!("{} failed to preview: {error}", fixture.display()));
            assert!(preview.operations.is_empty(), "{}", fixture.display());
            let outcome = transaction_apply_at(workspace.path(), &preview)
                .unwrap_or_else(|error| panic!("{} failed to apply: {error}", fixture.display()));

            assert!(!outcome.changed, "{}", fixture.display());
            assert!(!outcome.index_refreshed, "{}", fixture.display());
            assert_eq!(
                fs::read(workspace.path().join(EDIT_NOTE)).unwrap(),
                source,
                "{}",
                fixture.display()
            );
            assert_eq!(
                files_below(workspace.path()),
                before,
                "{}",
                fixture.display()
            );
        }
        assert_eq!(rejected, expected_index_rejections);
    }
}
