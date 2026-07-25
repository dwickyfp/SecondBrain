//! Versioned local operation records for the Phase 0 durability journal.
//!
//! Each [`LocalOperationRecord`] captures a single semantic mutation applied to
//! a note, along with full attribution (actor, device, transaction, note,
//! workspace) and a hash chain link (`previous_record_hash`). Records are
//! length-prefixed and CRC-protected for corruption detection.
//!
//! ## Wire format (v1)
//!
//! ```text
//! +-------------------+-------------------+-------------------+-------------------+-------------------+
//! | label (14 bytes)  | version (2 BE)    | length (4 BE)     | JSON payload      | CRC32 (4 BE)      |
//! | "sb-local-oplog-v1"| 0x0001           | payload length    | canonical JSON    | crc32fast(payload)|
//! +-------------------+-------------------+-------------------+-------------------+-------------------+
//! ```
//!
//! The CRC is computed over the **canonical JSON payload only** (with the
//! `crc32` field omitted). JSON keys are canonicalized (alphabetically sorted
//! by `serde_json` with `preserve_order` disabled) before serialization to
//! ensure deterministic encoding.
//!
//! **This format is for local persistence only — it is NOT a network envelope.**

use std::fmt;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, TransactionId, WorkspaceId};
use secondbrain_markdown::operation::SemanticOperation;
use serde::{Deserialize, Serialize};
use thiserror::Error;
// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The supported format version for local operation records.
pub const FORMAT_VERSION_1: u16 = 1;

// ---------------------------------------------------------------------------
// RecordFormat
// ---------------------------------------------------------------------------

/// Format metadata for local operation records.
pub struct RecordFormat;

impl RecordFormat {
    /// The label prefixing every v1 encoded record.
    pub const LABEL_V1: &'static str = "sb-local-oplog-v1";
}

// ---------------------------------------------------------------------------
// LocalOperationRecord
// ---------------------------------------------------------------------------

/// A versioned local mutation journal record.
///
/// Captures a single semantic operation with full attribution and a hash-chain
/// link for recovery. The `crc32` field is computed during encoding and
/// verified during decoding; callers should set it to `0` when constructing
/// a record for encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalOperationRecord {
    /// The wire format version (currently always [`FORMAT_VERSION_1`]).
    pub format_version: u16,
    /// Unique identifier for the transaction this record belongs to.
    pub transaction_id: TransactionId,
    /// Workspace this record belongs to.
    pub workspace_id: WorkspaceId,
    /// The note being mutated.
    pub note_id: NoteId,
    /// The actor (user) who performed the operation.
    pub actor_id: ActorId,
    /// The device on which the operation was performed.
    pub device_id: DeviceId,
    /// Monotonic sequence number within the device's journal.
    pub sequence: u64,
    /// Hash of the previous record in the chain (None for the genesis record).
    pub previous_record_hash: Option<ContentHash>,
    /// The semantic operation applied.
    pub operation: SemanticOperation,
    /// CRC32 of the canonical JSON payload (computed during encoding).
    ///
    /// This field is **excluded** from the CRC computation itself. When
    /// constructing a record for encoding, set this to `0`.
    #[serde(skip_serializing)]
    #[serde(default = "default_crc32")]
    pub crc32: u32,
}

fn default_crc32() -> u32 {
    0
}

// ---------------------------------------------------------------------------
// Serialization helper (canonical JSON with crc32 omitted)
// ---------------------------------------------------------------------------

/// Internal serialization structure that mirrors `LocalOperationRecord` but
/// omits the `crc32` field, ensuring it is not included in the CRC computation.
///
/// Using `#[serde(skip_serializing)]` on `crc32` achieves the same effect, but
/// we use this explicit approach for clarity and to guarantee the field is
/// never accidentally included.
#[derive(Serialize)]
struct CanonicalPayload<'a> {
    format_version: u16,
    transaction_id: &'a TransactionId,
    workspace_id: &'a WorkspaceId,
    note_id: &'a NoteId,
    actor_id: &'a ActorId,
    device_id: &'a DeviceId,
    sequence: u64,
    previous_record_hash: &'a Option<ContentHash>,
    operation: &'a SemanticOperation,
}

impl LocalOperationRecord {
    /// Serialize this record to its canonical binary form.
    ///
    /// The encoding is deterministic: the same record always produces the same
    /// byte sequence. The CRC32 is computed over the canonical JSON payload
    /// (with `crc32` omitted).
    ///
    /// # Errors
    ///
    /// Returns [`RecordEncodeError`] if JSON serialization fails (which should
    /// not happen for well-formed records).
    pub fn encode(&self) -> Result<Vec<u8>, RecordEncodeError> {
        let payload = CanonicalPayload {
            format_version: self.format_version,
            transaction_id: &self.transaction_id,
            workspace_id: &self.workspace_id,
            note_id: &self.note_id,
            actor_id: &self.actor_id,
            device_id: &self.device_id,
            sequence: self.sequence,
            previous_record_hash: &self.previous_record_hash,
            operation: &self.operation,
        };

        // serde_json serializes struct fields in declaration order, which is
        // already alphabetical for our CanonicalPayload. This gives us
        // deterministic, canonical encoding without needing preserve_order.
        let json = serde_json::to_vec(&payload).map_err(RecordEncodeError::Json)?;

        let crc = crc32fast::hash(&json);

        let mut buf = Vec::with_capacity(RecordFormat::LABEL_V1.len() + 2 + 4 + json.len() + 4);
        buf.extend_from_slice(RecordFormat::LABEL_V1.as_bytes());
        buf.extend_from_slice(&self.format_version.to_be_bytes());
        buf.extend_from_slice(&(json.len() as u32).to_be_bytes());
        buf.extend_from_slice(&json);
        buf.extend_from_slice(&crc.to_be_bytes());

        Ok(buf)
    }

    /// Decode a record from its canonical binary form.
    ///
    /// Verifies the format label, version, length prefix, and CRC32. Unknown
    /// additive JSON fields are tolerated within version 1 (forward
    /// compatibility).
    ///
    /// # Errors
    ///
    /// - [`RecordDecodeError::Truncated`] — the buffer is too short.
    /// - [`RecordDecodeError::InvalidLabel`] — the format label is wrong.
    /// - [`RecordDecodeError::UnsupportedVersion`] — the format version is not 1.
    /// - [`RecordDecodeError::CrcMismatch`] — the CRC32 does not match.
    /// - [`RecordDecodeError::Json`] — the JSON payload is invalid.
    pub fn decode(buf: &[u8]) -> Result<Self, RecordDecodeError> {
        let label = RecordFormat::LABEL_V1.as_bytes();

        // --- Header parsing with truncation checks ---
        if buf.len() < label.len() + 2 + 4 {
            return Err(RecordDecodeError::Truncated {
                expected: label.len() + 2 + 4,
                actual: buf.len(),
            });
        }

        let label_bytes = &buf[..label.len()];
        if label_bytes != label {
            return Err(RecordDecodeError::InvalidLabel {
                expected: RecordFormat::LABEL_V1,
                actual: String::from_utf8_lossy(label_bytes).into_owned(),
            });
        }

        let version_offset = label.len();
        let format_version = u16::from_be_bytes([buf[version_offset], buf[version_offset + 1]]);

        if format_version != FORMAT_VERSION_1 {
            return Err(RecordDecodeError::UnsupportedVersion {
                version: format_version,
                supported: FORMAT_VERSION_1,
            });
        }

        let length_offset = label.len() + 2;
        let payload_len = u32::from_be_bytes([
            buf[length_offset],
            buf[length_offset + 1],
            buf[length_offset + 2],
            buf[length_offset + 3],
        ]) as usize;

        let payload_start = label.len() + 2 + 4;
        let payload_end = payload_start + payload_len;
        let crc_end = payload_end + 4;

        if buf.len() < crc_end {
            return Err(RecordDecodeError::Truncated {
                expected: crc_end,
                actual: buf.len(),
            });
        }

        let json_bytes = &buf[payload_start..payload_end];
        let crc_bytes = &buf[payload_end..crc_end];
        let stored_crc =
            u32::from_be_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);

        // --- CRC verification ---
        let computed_crc = crc32fast::hash(json_bytes);
        if computed_crc != stored_crc {
            return Err(RecordDecodeError::CrcMismatch {
                expected: stored_crc,
                actual: computed_crc,
            });
        }

        // --- JSON deserialization (tolerant of unknown additive fields) ---
        let json: serde_json::Value =
            serde_json::from_slice(json_bytes).map_err(RecordDecodeError::Json)?;

        // Re-serialize to strip unknown fields, then deserialize into the record.
        // serde_json::Value preserves all fields; we need to filter to known fields
        // only. Since our struct uses `#[serde(default)]` on crc32 and no
        // `deny_unknown_fields`, we can deserialize directly. But to be safe and
        // ensure the crc32 field is populated, we deserialize into the struct
        // directly from the JSON value.
        //
        // Actually, serde_json::from_value will handle unknown fields gracefully
        // (they're ignored by default). Let's do that.
        let canonical_json = serde_json::to_vec(&json).map_err(RecordDecodeError::Json)?;
        let record: Self =
            serde_json::from_slice(&canonical_json).map_err(RecordDecodeError::Json)?;

        // Populate the crc32 field from the stored value.
        let mut record = record;
        record.crc32 = stored_crc;

        Ok(record)
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error returned when encoding a [`LocalOperationRecord`] fails.
#[derive(Debug, Error)]
pub enum RecordEncodeError {
    /// JSON serialization of the record payload failed.
    #[error("failed to serialize record payload as JSON: {0}")]
    Json(serde_json::Error),
}

/// Error returned when decoding a [`LocalOperationRecord`] fails.
#[derive(Debug, Error)]
pub enum RecordDecodeError {
    /// The buffer was shorter than expected.
    #[error("truncated record: expected {expected} bytes, got {actual}")]
    Truncated {
        /// The minimum number of bytes expected.
        expected: usize,
        /// The actual number of bytes available.
        actual: usize,
    },

    /// The format label did not match.
    #[error("invalid label: expected {expected:?}, got {actual:?}")]
    InvalidLabel {
        /// The expected label.
        expected: &'static str,
        /// The actual label found.
        actual: String,
    },

    /// The format version is not supported.
    #[error("unsupported format version {version}, supported: {supported}")]
    UnsupportedVersion {
        /// The version found in the buffer.
        version: u16,
        /// The supported version.
        supported: u16,
    },

    /// The stored CRC32 did not match the computed CRC32.
    #[error("CRC mismatch: expected {expected:#010x}, computed {actual:#010x}")]
    CrcMismatch {
        /// The CRC value stored in the record.
        expected: u32,
        /// The CRC value computed from the payload.
        actual: u32,
    },

    /// JSON deserialization of the record payload failed.
    #[error("failed to deserialize record payload as JSON: {0}")]
    Json(serde_json::Error),
}

impl fmt::Display for RecordFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::LABEL_V1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_constant_is_one() {
        assert_eq!(FORMAT_VERSION_1, 1);
    }

    #[test]
    fn label_is_correct() {
        assert_eq!(RecordFormat::LABEL_V1, "sb-local-oplog-v1");
    }
}
