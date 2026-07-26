//! Read-only preview and fail-closed in-place adoption of an existing Markdown vault.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use blake3::Hasher;
use secondbrain_core::id::WorkspaceId;
use secondbrain_markdown::extract::extract;
use secondbrain_markdown::{SourceDocument, parse_metadata};
use secondbrain_vault::{WorkspaceRoot, initialize_workspace, load_manifest};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{IndexConfig, IndexError, IndexReport, rebuild};

pub const IMPORT_PREVIEW_FORMAT: &str = "sb-obsidian-import-preview-v1";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportInventory {
    pub markdown: Vec<String>,
    pub attachments: Vec<String>,
    pub obsidian_config: Vec<String>,
    pub ignored: Vec<String>,
    pub unsupported: Vec<String>,
    pub symlinks: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportIssue {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedWrites {
    pub markdown: usize,
    pub attachments: usize,
    pub obsidian_config: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportPreview {
    pub format: String,
    pub root: String,
    pub fingerprint: String,
    pub already_initialized: bool,
    pub workspace_id: Option<WorkspaceId>,
    pub manifest_format_version: Option<u32>,
    pub manifest_fingerprint: Option<String>,
    pub inventory: ImportInventory,
    pub parse_errors: Vec<ImportIssue>,
    pub duplicate_ids: Vec<ImportIssue>,
    pub portable_collisions: Vec<ImportIssue>,
    pub broken_links: Vec<ImportIssue>,
    pub ambiguous_links: Vec<ImportIssue>,
    pub planned_writes: PlannedWrites,
    pub can_apply: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportApplyOutcome {
    pub format: String,
    pub status: String,
    pub root: String,
    pub workspace_id: WorkspaceId,
    pub fingerprint: String,
    pub index: IndexReportWire,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexReportWire {
    pub indexed: usize,
    pub skipped: usize,
    pub broken_links: usize,
    pub orphans: usize,
}

impl From<IndexReport> for IndexReportWire {
    fn from(value: IndexReport) -> Self {
        Self {
            indexed: value.indexed,
            skipped: value.skipped,
            broken_links: value.broken_links,
            orphans: value.orphans,
        }
    }
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("import I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("import preview format is {found}, expected {expected}")]
    Format {
        expected: &'static str,
        found: String,
    },
    #[error("import preview belongs to {preview_root}, not {actual_root}")]
    Root {
        preview_root: String,
        actual_root: String,
    },
    #[error("vault changed after preview (expected {expected}, found {actual}); preview again")]
    FingerprintChanged { expected: String, actual: String },
    #[error("import preview field was modified after review: {0}")]
    PreviewModified(&'static str),
    #[error("import cannot apply while preview reports unsafe or invalid vault entries")]
    Blocked,
    #[error(transparent)]
    Core(#[from] secondbrain_core::Error),
    #[error(transparent)]
    Index(#[from] IndexError),
}

struct ParsedNote {
    path: String,
    document: SourceDocument,
    title: Option<String>,
    aliases: Vec<String>,
}

/// Inventories a vault without creating, removing, or modifying any entry.
pub fn preview_import(root: impl AsRef<Path>) -> Result<ImportPreview, ImportError> {
    let workspace = WorkspaceRoot::open(root)?;
    let root = workspace.canonical_path();
    let mut inventory = ImportInventory::default();
    let mut hashed = Vec::new();
    walk(root, root, &mut inventory, &mut hashed)?;
    sort_inventory(&mut inventory);
    hashed.sort_by(|a, b| a.0.cmp(&b.0));
    let fingerprint = fingerprint(&hashed);

    let mut parse_errors = Vec::new();
    let mut duplicate_ids = Vec::new();
    let mut portable_collisions = Vec::new();
    let mut ids = BTreeMap::new();
    let mut portable = BTreeMap::new();
    let mut notes = Vec::new();
    for path in &inventory.markdown {
        check_portable(path, &mut portable, &mut portable_collisions);
        let bytes = fs::read(root.join(path)).map_err(|source| ImportError::Io {
            path: root.join(path),
            source,
        })?;
        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(error) => {
                parse_errors.push(issue(path, format!("Markdown is not UTF-8: {error}")));
                continue;
            }
        };
        let metadata = match parse_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) => {
                parse_errors.push(issue(path, error.to_string()));
                continue;
            }
        };
        let document = match SourceDocument::parse(&source) {
            Ok(document) => document,
            Err(error) => {
                parse_errors.push(issue(path, error.to_string()));
                continue;
            }
        };
        if let Some(id) = metadata.id
            && let Some(first) = ids.insert(id, path.clone())
        {
            duplicate_ids.push(issue(
                path,
                format!("duplicate note ID {id}; first used by {first}"),
            ));
        }
        let aliases =
            metadata
                .properties
                .get("aliases")
                .map_or_else(Vec::new, |value| match value {
                    serde_yaml::Value::String(value) => vec![value.clone()],
                    serde_yaml::Value::Sequence(values) => values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                    _ => Vec::new(),
                });
        notes.push(ParsedNote {
            path: path.clone(),
            document,
            title: metadata.title,
            aliases,
        });
    }
    for path in inventory
        .attachments
        .iter()
        .chain(&inventory.obsidian_config)
    {
        check_portable(path, &mut portable, &mut portable_collisions);
    }
    let (broken_links, ambiguous_links) = link_issues(&notes);
    let manifest_path = root.join(".secondbrain/manifest.toml");
    let (already_initialized, workspace_id, manifest_format_version, manifest_fingerprint) =
        if manifest_path.exists() {
            let bytes = fs::read(&manifest_path).map_err(|source| ImportError::Io {
                path: manifest_path,
                source,
            })?;
            let manifest = load_manifest(root)?;
            (
                true,
                Some(manifest.workspace_id),
                Some(manifest.format_version),
                Some(blake3::hash(&bytes).to_hex().to_string()),
            )
        } else {
            (false, None, None, None)
        };
    let can_apply = inventory.symlinks.is_empty()
        && inventory.unsupported.is_empty()
        && parse_errors.is_empty()
        && duplicate_ids.is_empty()
        && portable_collisions.is_empty();
    Ok(ImportPreview {
        format: IMPORT_PREVIEW_FORMAT.into(),
        root: root.to_string_lossy().into_owned(),
        fingerprint,
        already_initialized,
        workspace_id,
        manifest_format_version,
        manifest_fingerprint,
        inventory,
        parse_errors,
        duplicate_ids,
        portable_collisions,
        broken_links,
        ambiguous_links,
        planned_writes: PlannedWrites::default(),
        can_apply,
    })
}

/// Revalidates a reviewed preview, initializes only internal state, and rebuilds the derived index.
pub fn apply_import(
    root: impl AsRef<Path>,
    preview: &ImportPreview,
) -> Result<ImportApplyOutcome, ImportError> {
    if preview.format != IMPORT_PREVIEW_FORMAT {
        return Err(ImportError::Format {
            expected: IMPORT_PREVIEW_FORMAT,
            found: preview.format.clone(),
        });
    }
    let current = preview_import(root)?;
    if preview.root != current.root {
        return Err(ImportError::Root {
            preview_root: preview.root.clone(),
            actual_root: current.root,
        });
    }
    if preview.fingerprint != current.fingerprint {
        return Err(ImportError::FingerprintChanged {
            expected: preview.fingerprint.clone(),
            actual: current.fingerprint,
        });
    }
    for (matches, field) in [
        (
            preview.already_initialized == current.already_initialized,
            "alreadyInitialized",
        ),
        (preview.workspace_id == current.workspace_id, "workspaceId"),
        (
            preview.manifest_format_version == current.manifest_format_version,
            "manifestFormatVersion",
        ),
        (
            preview.manifest_fingerprint == current.manifest_fingerprint,
            "manifestFingerprint",
        ),
        (preview.inventory == current.inventory, "inventory"),
        (preview.parse_errors == current.parse_errors, "parseErrors"),
        (
            preview.duplicate_ids == current.duplicate_ids,
            "duplicateIds",
        ),
        (
            preview.portable_collisions == current.portable_collisions,
            "portableCollisions",
        ),
        (preview.broken_links == current.broken_links, "brokenLinks"),
        (
            preview.ambiguous_links == current.ambiguous_links,
            "ambiguousLinks",
        ),
        (
            preview.planned_writes == current.planned_writes,
            "plannedWrites",
        ),
        (preview.can_apply == current.can_apply, "canApply"),
    ] {
        if !matches {
            return Err(ImportError::PreviewModified(field));
        }
    }
    if !current.can_apply {
        return Err(ImportError::Blocked);
    }
    let status = if current.already_initialized {
        "already_initialized"
    } else {
        "initialized"
    };
    let manifest = initialize_workspace(Path::new(&current.root))?;
    let report = rebuild(&current.root, &IndexConfig::default())?;
    Ok(ImportApplyOutcome {
        format: IMPORT_PREVIEW_FORMAT.into(),
        status: status.into(),
        root: current.root,
        workspace_id: manifest.workspace_id,
        fingerprint: current.fingerprint,
        index: report.into(),
    })
}

fn walk(
    root: &Path,
    directory: &Path,
    inventory: &mut ImportInventory,
    hashed: &mut Vec<(String, u8, Vec<u8>)>,
) -> Result<(), ImportError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| ImportError::Io {
            path: directory.into(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ImportError::Io {
            path: directory.into(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("walk is confined")
            .to_str()
            .ok_or_else(|| ImportError::Io {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, "path is not UTF-8"),
            })?
            .replace('\\', "/");
        let kind = entry.file_type().map_err(|source| ImportError::Io {
            path: path.clone(),
            source,
        })?;
        if kind.is_symlink() {
            inventory.symlinks.push(relative.clone());
            let target = fs::read_link(&path).map_err(|source| ImportError::Io {
                path: path.clone(),
                source,
            })?;
            hashed.push((relative, b'l', target.to_string_lossy().as_bytes().to_vec()));
        } else if relative == ".secondbrain" {
            continue;
        } else if relative == ".git" {
            inventory.ignored.push(relative);
            continue;
        } else if kind.is_dir() {
            hashed.push((relative, b'd', Vec::new()));
            walk(root, &path, inventory, hashed)?;
        } else if kind.is_file() {
            let bytes = fs::read(&path).map_err(|source| ImportError::Io {
                path: path.clone(),
                source,
            })?;
            hashed.push((relative.clone(), b'f', bytes));
            if relative.starts_with(".obsidian/") {
                inventory.obsidian_config.push(relative);
            } else if is_markdown(&path) {
                inventory.markdown.push(relative);
            } else {
                inventory.attachments.push(relative);
            }
        } else {
            inventory.unsupported.push(relative.clone());
            hashed.push((relative, b'u', Vec::new()));
        }
    }
    Ok(())
}

fn fingerprint(entries: &[(String, u8, Vec<u8>)]) -> String {
    let mut hash = Hasher::new();
    hash.update(b"secondbrain-obsidian-import-fingerprint-v1\0");
    for (path, kind, bytes) in entries {
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(&[*kind]);
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.finalize().to_hex().to_string()
}

fn check_portable(path: &str, seen: &mut BTreeMap<String, String>, issues: &mut Vec<ImportIssue>) {
    let key = path.to_lowercase();
    if let Some(first) = seen.insert(key, path.into()).filter(|first| first != path) {
        issues.push(issue(
            path,
            format!("case-insensitive path collision with {first}"),
        ));
    }
    for component in path.split('/') {
        let stem = component
            .trim_end_matches([' ', '.'])
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let reserved = matches!(
            stem.as_str(),
            "con"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        );
        if component.ends_with([' ', '.'])
            || component
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || reserved
        {
            issues.push(issue(
                path,
                format!("path component is not portable: {component}"),
            ));
            break;
        }
    }
}

fn link_issues(notes: &[ParsedNote]) -> (Vec<ImportIssue>, Vec<ImportIssue>) {
    let mut lookup: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for note in notes {
        for value in [
            Some(note.path.as_str()),
            note.path
                .strip_suffix(".md")
                .or_else(|| note.path.strip_suffix(".markdown")),
            Path::new(&note.path).file_stem().and_then(|v| v.to_str()),
            note.title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(note.aliases.iter().map(String::as_str))
        {
            lookup
                .entry(value.to_lowercase())
                .or_default()
                .insert(note.path.clone());
        }
    }
    let mut broken = Vec::new();
    let mut ambiguous = Vec::new();
    for note in notes {
        for link in extract(&note.document).links {
            match lookup.get(&link.target.to_lowercase()) {
                None => broken.push(issue(&note.path, format!("broken link: {}", link.target))),
                Some(values) if values.len() > 1 => ambiguous.push(issue(
                    &note.path,
                    format!(
                        "ambiguous link {}: {}",
                        link.target,
                        values.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                )),
                _ => {}
            }
        }
    }
    (broken, ambiguous)
}

fn issue(path: &str, message: String) -> ImportIssue {
    ImportIssue {
        path: path.into(),
        message,
    }
}
fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown")
    )
}
fn sort_inventory(value: &mut ImportInventory) {
    value.markdown.sort();
    value.attachments.sort();
    value.obsidian_config.sort();
    value.ignored.sort();
    value.unsupported.sort();
    value.symlinks.sort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_collision_detection_is_case_insensitive_on_every_platform() {
        let mut seen = BTreeMap::new();
        let mut issues = Vec::new();
        check_portable("Notes/Alpha.md", &mut seen, &mut issues);
        check_portable("notes/alpha.md", &mut seen, &mut issues);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("Notes/Alpha.md"));
    }

    #[test]
    fn portable_names_reject_windows_devices_and_trailing_dots() {
        for path in ["CON.md", "notes/aux.txt", "trailing./note.md"] {
            let mut seen = BTreeMap::new();
            let mut issues = Vec::new();
            check_portable(path, &mut seen, &mut issues);
            assert_eq!(issues.len(), 1, "{path}");
        }
    }
}
