//! Canonical per-note CRDT state and durable persistence.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use loro::{ExportMode, LoroDoc};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion};
use secondbrain_core::path::WorkspacePath;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAGIC: &[u8; 8] = b"SBCRDT01";
const FORMAT_VERSION: u16 = 1;
const MARKDOWN_TEXT: &str = "markdown";
const ENGINE: &str = "loro-1.13.7";
const LEGACY_FORMAT: &str = "sb-base-snapshot-v1";

/// The canonical materialized view of one note's Loro document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteState {
    pub note_id: NoteId,
    pub path: WorkspacePath,
    pub version: NoteVersion,
    pub source_hash: ContentHash,
    pub markdown: String,
}

impl NoteState {
    #[must_use]
    pub fn describes(&self, source_hash: ContentHash) -> bool {
        self.source_hash == source_hash
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("CRDT state I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("CRDT state metadata failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CRDT state write failed: {0}")]
    Core(#[from] secondbrain_core::Error),
    #[error("CRDT state for {note_id} is corrupt: {reason}")]
    Corrupt { note_id: NoteId, reason: String },
    #[error("CRDT state for {note_id} uses unsupported format {format}")]
    UnsupportedFormat { note_id: NoteId, format: String },
    #[error("Loro state failed: {0}")]
    Loro(String),
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    format: u16,
    engine: String,
    note_id: NoteId,
    path: WorkspacePath,
    version: NoteVersion,
    source_hash: ContentHash,
}

#[derive(Deserialize)]
struct LegacySnapshot {
    format: String,
    note_id: NoteId,
    path: WorkspacePath,
    version: NoteVersion,
    source_hash: ContentHash,
    source: String,
}

/// Per-workspace canonical note store.
pub struct NoteStore {
    directory: PathBuf,
    legacy_directory: PathBuf,
}

impl NoteStore {
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            directory: root.join(".secondbrain").join("crdt"),
            legacy_directory: root.join(".secondbrain").join("snapshots"),
        }
    }

    /// Loads canonical state, migrating a legacy base snapshot if needed.
    pub fn load(&self, note_id: NoteId) -> Result<Option<NoteState>, Error> {
        match fs::read(self.state_path(note_id)) {
            Ok(bytes) => self.decode(note_id, &bytes).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.migrate(note_id),
            Err(error) => Err(error.into()),
        }
    }

    /// Returns every canonical or legacy note, migrating legacy records once.
    pub fn list(&self) -> Result<Vec<NoteState>, Error> {
        let mut ids = BTreeSet::new();
        self.collect_ids(&self.directory, "sbcrdt", &mut ids)?;
        self.collect_ids(&self.legacy_directory, "json", &mut ids)?;
        let mut states = ids
            .into_iter()
            .map(|id| {
                self.load(id)?.ok_or_else(|| Error::Corrupt {
                    note_id: id,
                    reason: "state disappeared while listing".into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        states.sort_by(|left, right| {
            left.path
                .as_str()
                .cmp(right.path.as_str())
                .then_with(|| left.note_id.cmp(&right.note_id))
        });
        Ok(states)
    }

    /// Advances the note's LoroText to `markdown` and atomically persists it.
    pub fn save(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        version: NoteVersion,
        markdown: &str,
    ) -> Result<NoteState, Error> {
        let doc = match fs::read(self.state_path(note_id)) {
            Ok(bytes) => self.decode_doc(note_id, &bytes)?.0,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.migrate(note_id)?.is_some() {
                    let bytes = fs::read(self.state_path(note_id))?;
                    self.decode_doc(note_id, &bytes)?.0
                } else {
                    LoroDoc::new()
                }
            }
            Err(error) => return Err(error.into()),
        };
        replace_markdown(&doc, markdown)?;
        doc.commit();
        let snapshot = doc
            .export(ExportMode::Snapshot)
            .map_err(|error| Error::Loro(error.to_string()))?;
        let metadata = Metadata {
            format: FORMAT_VERSION,
            engine: ENGINE.into(),
            note_id,
            path: path.clone(),
            version,
            source_hash: ContentHash::digest(markdown.as_bytes()),
        };
        self.persist(note_id, &metadata, &snapshot)?;
        Ok(NoteState {
            note_id,
            path: path.clone(),
            version,
            source_hash: metadata.source_hash,
            markdown: markdown.into(),
        })
    }

    /// Changes only the materialized path metadata, preserving Loro history.
    pub fn update_path(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
    ) -> Result<Option<NoteState>, Error> {
        let Some(mut state) = self.load(note_id)? else {
            return Ok(None);
        };
        if state.path == *path {
            return Ok(Some(state));
        }
        let bytes = fs::read(self.state_path(note_id))?;
        let (_, mut metadata, snapshot) = self.decode_doc(note_id, &bytes)?;
        metadata.path = path.clone();
        self.persist(note_id, &metadata, &snapshot)?;
        state.path = path.clone();
        Ok(Some(state))
    }

    #[must_use]
    pub fn state_path(&self, note_id: NoteId) -> PathBuf {
        self.directory.join(format!("{note_id}.sbcrdt"))
    }

    fn migrate(&self, note_id: NoteId) -> Result<Option<NoteState>, Error> {
        let path = self.legacy_directory.join(format!("{note_id}.json"));
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let legacy: LegacySnapshot = serde_json::from_slice(&bytes)?;
        if legacy.format != LEGACY_FORMAT {
            return Err(Error::UnsupportedFormat {
                note_id,
                format: legacy.format,
            });
        }
        if legacy.note_id != note_id
            || legacy.source_hash != ContentHash::digest(legacy.source.as_bytes())
        {
            return Err(Error::Corrupt {
                note_id,
                reason: "legacy identity or source hash mismatch".into(),
            });
        }
        self.save_fresh(note_id, &legacy.path, legacy.version, &legacy.source)
            .map(Some)
    }

    fn save_fresh(
        &self,
        note_id: NoteId,
        path: &WorkspacePath,
        version: NoteVersion,
        markdown: &str,
    ) -> Result<NoteState, Error> {
        let doc = LoroDoc::new();
        replace_markdown(&doc, markdown)?;
        doc.commit();
        let snapshot = doc
            .export(ExportMode::Snapshot)
            .map_err(|e| Error::Loro(e.to_string()))?;
        let metadata = Metadata {
            format: FORMAT_VERSION,
            engine: ENGINE.into(),
            note_id,
            path: path.clone(),
            version,
            source_hash: ContentHash::digest(markdown.as_bytes()),
        };
        self.persist(note_id, &metadata, &snapshot)?;
        Ok(NoteState {
            note_id,
            path: path.clone(),
            version,
            source_hash: metadata.source_hash,
            markdown: markdown.into(),
        })
    }

    fn decode(&self, note_id: NoteId, bytes: &[u8]) -> Result<NoteState, Error> {
        let (doc, metadata, _) = self.decode_doc(note_id, bytes)?;
        let markdown = doc.get_text(MARKDOWN_TEXT).to_string();
        if ContentHash::digest(markdown.as_bytes()) != metadata.source_hash {
            return Err(Error::Corrupt {
                note_id,
                reason: "materialized Markdown hash mismatch".into(),
            });
        }
        Ok(NoteState {
            note_id,
            path: metadata.path,
            version: metadata.version,
            source_hash: metadata.source_hash,
            markdown,
        })
    }

    fn decode_doc(
        &self,
        note_id: NoteId,
        bytes: &[u8],
    ) -> Result<(LoroDoc, Metadata, Vec<u8>), Error> {
        let minimum = MAGIC.len() + 2 + 4 + 8 + 4;
        if bytes.len() < minimum || &bytes[..MAGIC.len()] != MAGIC {
            return Err(Error::Corrupt {
                note_id,
                reason: "invalid or truncated frame header".into(),
            });
        }
        let crc_offset = bytes.len() - 4;
        let expected_crc = u32::from_be_bytes(bytes[crc_offset..].try_into().expect("CRC slice"));
        if crc32fast::hash(&bytes[..crc_offset]) != expected_crc {
            return Err(Error::Corrupt {
                note_id,
                reason: "CRC mismatch".into(),
            });
        }
        let format = u16::from_be_bytes(bytes[8..10].try_into().expect("format slice"));
        if format != FORMAT_VERSION {
            return Err(Error::UnsupportedFormat {
                note_id,
                format: format.to_string(),
            });
        }
        let metadata_len =
            u32::from_be_bytes(bytes[10..14].try_into().expect("metadata length")) as usize;
        let snapshot_len =
            u64::from_be_bytes(bytes[14..22].try_into().expect("snapshot length")) as usize;
        let metadata_end = 22usize
            .checked_add(metadata_len)
            .ok_or_else(|| Error::Corrupt {
                note_id,
                reason: "frame length overflow".into(),
            })?;
        let snapshot_end =
            metadata_end
                .checked_add(snapshot_len)
                .ok_or_else(|| Error::Corrupt {
                    note_id,
                    reason: "frame length overflow".into(),
                })?;
        if snapshot_end != crc_offset {
            return Err(Error::Corrupt {
                note_id,
                reason: "frame length mismatch".into(),
            });
        }
        let metadata: Metadata = serde_json::from_slice(&bytes[22..metadata_end])?;
        if metadata.format != FORMAT_VERSION
            || metadata.engine != ENGINE
            || metadata.note_id != note_id
        {
            return Err(Error::Corrupt {
                note_id,
                reason: "metadata format, engine, or identity mismatch".into(),
            });
        }
        let snapshot = bytes[metadata_end..snapshot_end].to_vec();
        let doc =
            LoroDoc::from_snapshot(&snapshot).map_err(|error| Error::Loro(error.to_string()))?;
        Ok((doc, metadata, snapshot))
    }

    fn persist(&self, note_id: NoteId, metadata: &Metadata, snapshot: &[u8]) -> Result<(), Error> {
        let metadata = serde_json::to_vec(metadata)?;
        let metadata_len = u32::try_from(metadata.len()).map_err(|_| Error::Corrupt {
            note_id,
            reason: "metadata too large".into(),
        })?;
        let snapshot_len = u64::try_from(snapshot.len()).map_err(|_| Error::Corrupt {
            note_id,
            reason: "snapshot too large".into(),
        })?;
        let mut frame = Vec::with_capacity(22 + metadata.len() + snapshot.len() + 4);
        frame.extend_from_slice(MAGIC);
        frame.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
        frame.extend_from_slice(&metadata_len.to_be_bytes());
        frame.extend_from_slice(&snapshot_len.to_be_bytes());
        frame.extend_from_slice(&metadata);
        frame.extend_from_slice(snapshot);
        frame.extend_from_slice(&crc32fast::hash(&frame).to_be_bytes());
        fs::create_dir_all(&self.directory)?;
        atomic_write(&self.state_path(note_id), &frame)?;
        Ok(())
    }

    fn collect_ids(
        &self,
        directory: &Path,
        extension: &str,
        ids: &mut BTreeSet<NoteId>,
    ) -> Result<(), Error> {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some(extension) {
                continue;
            }
            if let Some(id) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .and_then(|value| value.parse().ok())
            {
                ids.insert(id);
            }
        }
        Ok(())
    }
}

fn replace_markdown(doc: &LoroDoc, markdown: &str) -> Result<(), Error> {
    let text = doc.get_text(MARKDOWN_TEXT);
    let len = text.len_unicode();
    if len > 0 {
        text.delete(0, len)
            .map_err(|error| Error::Loro(error.to_string()))?;
    }
    if !markdown.is_empty() {
        text.insert(0, markdown)
            .map_err(|error| Error::Loro(error.to_string()))?;
    }
    Ok(())
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("CRDT target has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    #[cfg(unix)]
    if let Ok(directory) = fs::File::open(parent) {
        directory.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> WorkspacePath {
        WorkspacePath::new("notes/a.md").unwrap()
    }

    #[test]
    fn framed_state_round_trips_markdown_and_rejects_crc_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let store = NoteStore::new(directory.path());
        let note_id = NoteId::new();
        store
            .save(note_id, &path(), NoteVersion::new(2), "# Hello\n")
            .unwrap();
        assert_eq!(store.load(note_id).unwrap().unwrap().markdown, "# Hello\n");

        let state_path = store.state_path(note_id);
        let mut bytes = fs::read(&state_path).unwrap();
        bytes[22] ^= 1;
        fs::write(state_path, bytes).unwrap();
        assert!(matches!(store.load(note_id), Err(Error::Corrupt { .. })));
    }

    #[test]
    fn legacy_snapshot_migration_is_idempotent_and_canonical_wins() {
        let directory = tempfile::tempdir().unwrap();
        let note_id = NoteId::new();
        let legacy_dir = directory.path().join(".secondbrain/snapshots");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = serde_json::json!({
            "format": LEGACY_FORMAT,
            "note_id": note_id,
            "path": path(),
            "version": NoteVersion::new(4),
            "source_hash": ContentHash::digest(b"legacy\n"),
            "source": "legacy\n"
        });
        fs::write(
            legacy_dir.join(format!("{note_id}.json")),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let store = NoteStore::new(directory.path());

        assert_eq!(store.load(note_id).unwrap().unwrap().markdown, "legacy\n");
        store
            .save(note_id, &path(), NoteVersion::new(5), "canonical\n")
            .unwrap();
        assert_eq!(
            store.load(note_id).unwrap().unwrap().markdown,
            "canonical\n"
        );
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn path_updates_do_not_change_markdown_or_version() {
        let directory = tempfile::tempdir().unwrap();
        let store = NoteStore::new(directory.path());
        let note_id = NoteId::new();
        store
            .save(note_id, &path(), NoteVersion::new(7), "body\n")
            .unwrap();
        let moved = WorkspacePath::new("archive/a.md").unwrap();
        let state = store.update_path(note_id, &moved).unwrap().unwrap();
        assert_eq!(state.path, moved);
        assert_eq!(state.version, NoteVersion::new(7));
        assert_eq!(state.markdown, "body\n");
    }
}
