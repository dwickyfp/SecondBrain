//! Durable append-only, per-note local mutation journals.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::NoteId;
use thiserror::Error;

use crate::record::{LocalOperationRecord, RecordFormat};

/// Storage boundary used to make successful appends explicitly durable.
pub trait AppendStorage: Send + Sync {
    /// Append bytes, flush userspace buffers, and synchronize them to stable storage.
    fn append_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
}

#[derive(Debug, Default)]
struct FileStorage;

impl AppendStorage for FileStorage {
    fn append_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()
    }
}

/// A report describing bytes removed from the active journal but preserved in quarantine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorruptionReport {
    /// Byte offset where corruption begins.
    pub offset: u64,
    /// Number of invalid bytes preserved.
    pub length: u64,
    /// Deterministic path containing the exact invalid bytes.
    pub quarantine_path: PathBuf,
    /// Human-readable validation failure.
    pub reason: String,
}

/// Result of replaying a journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    /// Valid records preceding any corruption.
    pub records: Vec<LocalOperationRecord>,
    /// Corruption details when an invalid suffix was found.
    pub corruption: Option<CorruptionReport>,
}

/// Journal operation failure.
#[derive(Debug, Error)]
pub enum OplogError {
    /// Filesystem operation failed.
    #[error("journal I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Record encoding failed.
    #[error("record encoding failed: {0}")]
    Encode(#[from] crate::record::RecordEncodeError),
    /// Record belongs to a different note.
    #[error("record note {actual} does not match journal note {expected}")]
    NoteMismatch { expected: NoteId, actual: NoteId },
    /// Sequence is not strictly increasing.
    #[error("record sequence {next} must be greater than {previous}")]
    Sequence { previous: u64, next: u64 },
    /// Previous-record hash does not match the finalized prior record.
    #[error("record {sequence} has previous hash {actual:?}, expected {expected:?}")]
    HashChain {
        sequence: u64,
        expected: Option<ContentHash>,
        actual: Option<ContentHash>,
    },
    /// Existing journal corruption must be handled before append.
    #[error("cannot append while journal corruption is present: {0}")]
    Corrupt(String),
}

/// A durable mutation journal isolated to one note.
pub struct LocalMutationLog {
    note_id: NoteId,
    path: PathBuf,
    quarantine_dir: PathBuf,
    storage: Box<dyn AppendStorage>,
}

impl LocalMutationLog {
    /// Open or create the required journal path below `root`.
    pub fn open(root: impl AsRef<Path>, note_id: NoteId) -> Result<Self, OplogError> {
        Self::open_with_storage(root, note_id, FileStorage)
    }

    /// Open with an injectable append storage boundary.
    pub fn open_with_storage<S: AppendStorage + 'static>(
        root: impl AsRef<Path>,
        note_id: NoteId,
        storage: S,
    ) -> Result<Self, OplogError> {
        let note_dir = root
            .as_ref()
            .join(".secondbrain")
            .join("oplog")
            .join(note_id.to_string());
        let quarantine_dir = note_dir.join("quarantine");
        fs::create_dir_all(&quarantine_dir)?;
        let path = note_dir.join("local-mutations.log");
        OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            note_id,
            path,
            quarantine_dir,
            storage: Box::new(storage),
        })
    }

    /// Return the active journal path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Validate and durably append one record.
    pub fn append(&mut self, record: &LocalOperationRecord) -> Result<(), OplogError> {
        if record.note_id != self.note_id {
            return Err(OplogError::NoteMismatch {
                expected: self.note_id,
                actual: record.note_id,
            });
        }
        let replay = self.replay()?;
        if let Some(report) = replay.corruption {
            return Err(OplogError::Corrupt(report.reason));
        }
        if let Some(previous) = replay.records.last() {
            if record.sequence <= previous.sequence {
                return Err(OplogError::Sequence {
                    previous: previous.sequence,
                    next: record.sequence,
                });
            }
            let expected = Some(record_hash(previous)?);
            if record.previous_record_hash != expected {
                return Err(OplogError::HashChain {
                    sequence: record.sequence,
                    expected,
                    actual: record.previous_record_hash,
                });
            }
        } else if record.previous_record_hash.is_some() {
            return Err(OplogError::HashChain {
                sequence: record.sequence,
                expected: None,
                actual: record.previous_record_hash,
            });
        }
        let encoded = record.encode()?;
        self.storage.append_and_sync(&self.path, &encoded)?;
        Ok(())
    }

    /// Replay the valid prefix and quarantine any invalid suffix without dropping bytes.
    pub fn replay(&self) -> Result<ReplayOutcome, OplogError> {
        let bytes = fs::read(&self.path)?;
        let mut records: Vec<LocalOperationRecord> = Vec::new();
        let mut offset = 0usize;
        let mut reason = None;
        while offset < bytes.len() {
            let remaining = &bytes[offset..];
            let frame_len = match encoded_record_len(remaining) {
                Ok(length) => length,
                Err(message) => {
                    reason = Some(message);
                    break;
                }
            };
            let record = match LocalOperationRecord::decode(&remaining[..frame_len]) {
                Ok(record) => record,
                Err(error) => {
                    reason = Some(error.to_string());
                    break;
                }
            };
            if record.note_id != self.note_id {
                reason = Some(format!("record belongs to note {}", record.note_id));
                break;
            }
            if let Some(previous) = records.last() {
                if record.sequence <= previous.sequence {
                    reason = Some(format!(
                        "sequence {} is not greater than {}",
                        record.sequence, previous.sequence
                    ));
                    break;
                }
                let expected = record_hash(previous)?;
                if record.previous_record_hash != Some(expected) {
                    reason = Some(format!("hash chain break at sequence {}", record.sequence));
                    break;
                }
            } else if record.previous_record_hash.is_some() {
                reason = Some("genesis record has a previous hash".to_owned());
                break;
            }
            records.push(record);
            offset += frame_len;
        }

        let corruption = match reason {
            None => None,
            Some(reason) => Some(self.quarantine_suffix(&bytes, offset, reason)?),
        };
        Ok(ReplayOutcome {
            records,
            corruption,
        })
    }

    fn quarantine_suffix(
        &self,
        bytes: &[u8],
        offset: usize,
        reason: String,
    ) -> Result<CorruptionReport, OplogError> {
        let invalid = &bytes[offset..];
        let digest = ContentHash::digest(invalid);
        let name = format!("offset-{offset:020}-{digest}.bin");
        let quarantine_path = self.quarantine_dir.join(name);
        let mut quarantine = File::create(&quarantine_path)?;
        quarantine.write_all(invalid)?;
        quarantine.flush()?;
        quarantine.sync_all()?;

        let file = OpenOptions::new().write(true).open(&self.path)?;
        file.set_len(offset as u64)?;
        file.sync_all()?;
        Ok(CorruptionReport {
            offset: offset as u64,
            length: invalid.len() as u64,
            quarantine_path,
            reason,
        })
    }
}

fn record_hash(
    record: &LocalOperationRecord,
) -> Result<ContentHash, crate::record::RecordEncodeError> {
    Ok(ContentHash::digest(record.encode()?))
}

fn encoded_record_len(bytes: &[u8]) -> Result<usize, String> {
    let header = RecordFormat::LABEL_V1.len() + 2 + 4;
    if bytes.len() < header {
        return Err(format!("truncated record header: {} bytes", bytes.len()));
    }
    let length_offset = RecordFormat::LABEL_V1.len() + 2;
    let payload_length = u32::from_be_bytes(
        bytes[length_offset..length_offset + 4]
            .try_into()
            .expect("four-byte length slice"),
    ) as usize;
    header
        .checked_add(payload_length)
        .and_then(|length| length.checked_add(4))
        .filter(|length| *length <= bytes.len())
        .ok_or_else(|| "truncated record payload".to_owned())
}
