//! Deterministic daily-note open-or-create contracts.

use std::path::Path;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::id::NoteId;
use secondbrain_index::{IndexDatabase, index_path};
use secondbrain_vault::WorkspaceRoot;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{NoteCreateError, NoteCreatePreview, preview_note_creation};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DailyDate(String);

impl DailyDate {
    pub fn new(value: impl Into<String>) -> Result<Self, DailyNoteError> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.len() != 10
            || bytes[4] != b'-'
            || bytes[7] != b'-'
            || !bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
        {
            return Err(DailyNoteError::InvalidDate(value));
        }
        let year: u32 = value[0..4]
            .parse()
            .map_err(|_| DailyNoteError::InvalidDate(value.clone()))?;
        let month: u32 = value[5..7]
            .parse()
            .map_err(|_| DailyNoteError::InvalidDate(value.clone()))?;
        let day: u32 = value[8..10]
            .parse()
            .map_err(|_| DailyNoteError::InvalidDate(value.clone()))?;
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        if year == 0 || day == 0 || day > days {
            return Err(DailyNoteError::InvalidDate(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DailyNote {
    Existing { note_id: NoteId, path: String },
    Create { preview: NoteCreatePreview },
}

#[derive(Debug, Error)]
pub enum DailyNoteError {
    #[error("invalid daily-note date {0}; expected a real YYYY-MM-DD date")]
    InvalidDate(String),
    #[error("{0}")]
    Create(#[from] NoteCreateError),
    #[error("{0}")]
    Core(#[from] secondbrain_core::Error),
    #[error("{0}")]
    Index(#[from] secondbrain_index::Error),
}

pub fn open_or_preview_daily_note(
    workspace_root: &Path,
    date: DailyDate,
    actor: ActorId,
    device: DeviceId,
) -> Result<DailyNote, DailyNoteError> {
    let path = format!("Daily/{}.md", date.as_str());
    let root = WorkspaceRoot::open(workspace_root)?;
    let database = IndexDatabase::open(index_path(root.canonical_path()))?;
    if let Some(note) = database.note_by_path(&path)? {
        return Ok(DailyNote::Existing {
            note_id: note.note_id,
            path,
        });
    }
    let source = format!("# {}\n", date.as_str());
    Ok(DailyNote::Create {
        preview: preview_note_creation(workspace_root, &path, &source, actor, device)?,
    })
}
