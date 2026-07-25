//! Tests for versioned local operation records (Phase 0 durability journal).
//!
//! These tests verify the encode/decode contract, deterministic encoding,
//! version rejection, CRC corruption detection, truncation detection, and
//! additive JSON field tolerance for the local mutation journal.

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, TransactionId, WorkspaceId};
use secondbrain_markdown::operation::{NodeAnchor, SemanticOperation, StructuralPath};
use secondbrain_transaction::record::{
    FORMAT_VERSION_1, LocalOperationRecord, RecordDecodeError, RecordFormat,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a sample record with all attribution fields populated.
fn sample_record() -> LocalOperationRecord {
    LocalOperationRecord {
        format_version: FORMAT_VERSION_1,
        transaction_id: TransactionId::new(),
        workspace_id: WorkspaceId::new(),
        note_id: NoteId::new(),
        actor_id: ActorId::new("alice").unwrap(),
        device_id: DeviceId::new("macbook-01").unwrap(),
        sequence: 42,
        previous_record_hash: Some(ContentHash::digest(b"prior")),
        operation: SemanticOperation::SetProperty {
            key: "status".to_string(),
            value: "done".to_string(),
        },
        crc32: 0, // filled by encode
    }
}

fn sample_delete_record() -> LocalOperationRecord {
    let content_hash = ContentHash::digest(b"# Heading");
    let anchor = NodeAnchor::new(
        StructuralPath::root(0),
        secondbrain_markdown::SemanticKind::Heading,
        content_hash,
    );
    LocalOperationRecord {
        format_version: FORMAT_VERSION_1,
        transaction_id: TransactionId::new(),
        workspace_id: WorkspaceId::new(),
        note_id: NoteId::new(),
        actor_id: ActorId::new("bob").unwrap(),
        device_id: DeviceId::new("phone-02").unwrap(),
        sequence: 7,
        previous_record_hash: None,
        operation: SemanticOperation::DeleteNode { anchor },
        crc32: 0,
    }
}

// ---------------------------------------------------------------------------
// Round-trip tests
// ---------------------------------------------------------------------------

#[test]
fn encode_decode_round_trip_preserves_all_fields() {
    let original = sample_record();
    let encoded = original.encode().expect("encode must succeed");
    let decoded = LocalOperationRecord::decode(&encoded).expect("decode must succeed");

    assert_eq!(decoded.format_version, original.format_version);
    assert_eq!(decoded.transaction_id, original.transaction_id);
    assert_eq!(decoded.workspace_id, original.workspace_id);
    assert_eq!(decoded.note_id, original.note_id);
    assert_eq!(decoded.actor_id, original.actor_id);
    assert_eq!(decoded.device_id, original.device_id);
    assert_eq!(decoded.sequence, original.sequence);
    assert_eq!(decoded.previous_record_hash, original.previous_record_hash);
    assert_eq!(decoded.operation, original.operation);
    // crc32 is computed during encode; verify it's non-zero and matches
    // the re-computed CRC (the original.crc32 is 0 before encoding).
    assert_ne!(decoded.crc32, 0, "CRC must be computed during encode");
    let re_encoded = original.encode().expect("re-encode");
    let re_decoded = LocalOperationRecord::decode(&re_encoded).expect("re-decode");
    assert_eq!(decoded.crc32, re_decoded.crc32, "CRC must be deterministic");
}

#[test]
fn encode_decode_round_trip_with_delete_operation() {
    let original = sample_delete_record();
    let encoded = original.encode().expect("encode must succeed");
    let decoded = LocalOperationRecord::decode(&encoded).expect("decode must succeed");

    assert_eq!(decoded.operation, original.operation);
    assert_eq!(decoded.actor_id, original.actor_id);
    assert_eq!(decoded.device_id, original.device_id);
    assert_eq!(decoded.note_id, original.note_id);
    assert_eq!(decoded.transaction_id, original.transaction_id);
}

#[test]
fn encode_decode_round_trip_with_none_previous_hash() {
    let mut record = sample_record();
    record.previous_record_hash = None;
    let encoded = record.encode().expect("encode must succeed");
    let decoded = LocalOperationRecord::decode(&encoded).expect("decode must succeed");
    // Compare all fields except crc32 (which is 0 on the pre-encode record).
    assert_eq!(decoded.format_version, record.format_version);
    assert_eq!(decoded.transaction_id, record.transaction_id);
    assert_eq!(decoded.workspace_id, record.workspace_id);
    assert_eq!(decoded.note_id, record.note_id);
    assert_eq!(decoded.actor_id, record.actor_id);
    assert_eq!(decoded.device_id, record.device_id);
    assert_eq!(decoded.sequence, record.sequence);
    assert_eq!(decoded.previous_record_hash, record.previous_record_hash);
    assert_eq!(decoded.operation, record.operation);
    assert_ne!(decoded.crc32, 0);
}

// ---------------------------------------------------------------------------
// Deterministic encoding
// ---------------------------------------------------------------------------

#[test]
fn encoding_is_deterministic_across_calls() {
    let record = sample_record();
    let first = record.encode().expect("first encode");
    let second = record.encode().expect("second encode");
    assert_eq!(
        first, second,
        "encoding must be byte-for-byte deterministic"
    );
}

#[test]
fn encoding_is_deterministic_across_different_record_values() {
    let record_a = sample_record();
    let mut record_b = sample_record();
    record_b.sequence = 43;

    let encoded_a = record_a.encode().expect("encode a");
    let encoded_b = record_b.encode().expect("encode b");

    // They must differ (different sequence) and both be deterministic.
    assert_ne!(encoded_a, encoded_b);
    assert_eq!(record_a.encode().unwrap(), encoded_a);
    assert_eq!(record_b.encode().unwrap(), encoded_b);
}

// ---------------------------------------------------------------------------
// Version rejection
// ---------------------------------------------------------------------------

#[test]
fn unsupported_format_version_is_rejected() {
    let record = sample_record();
    let encoded = record.encode().expect("encode must succeed");
    let mut bytes = encoded;

    // Mutate the format_version field to an unsupported version (e.g., 999).
    // The format_version is in the header; we need to find and corrupt it.
    // Since we control the format, we can decode the header to find the offset,
    // but for this test, we'll construct a record with an unsupported version
    // and try to encode+decode it via raw manipulation.
    //
    // Strategy: parse the label, then the version bytes, then corrupt them.
    let label = RecordFormat::LABEL_V1.as_bytes();
    assert!(bytes.starts_with(label));

    // The version follows the label. Find it and corrupt.
    let version_offset = label.len();
    // Overwrite the 2-byte big-endian format_version with 999.
    bytes[version_offset] = 0x03;
    bytes[version_offset + 1] = 0xE7; // 999 = 0x03E7

    let result = LocalOperationRecord::decode(&bytes);
    assert!(
        matches!(
            result,
            Err(RecordDecodeError::UnsupportedVersion { version: 999, .. })
        ),
        "expected UnsupportedVersion(999), got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// CRC corruption detection
// ---------------------------------------------------------------------------

#[test]
fn crc_corruption_is_rejected() {
    let record = sample_record();
    let mut encoded = record.encode().expect("encode must succeed");

    // Flip a byte in the JSON payload (after the header+length prefix, before CRC).
    // The layout is: label | format_version(2) | length(u32) | json_payload | crc32(4)
    let label_len = RecordFormat::LABEL_V1.len();
    let header_len = label_len + 2 + 4; // label + version + length
    let crc_offset = encoded.len() - 4;

    // Flip a byte somewhere in the JSON payload.
    let payload_mid = header_len + 10;
    if payload_mid < crc_offset {
        encoded[payload_mid] ^= 0xFF;
    }

    let result = LocalOperationRecord::decode(&encoded);
    assert!(
        matches!(result, Err(RecordDecodeError::CrcMismatch { .. })),
        "expected CrcMismatch, got {result:?}"
    );
}

#[test]
fn crc_tampering_with_wrong_crc_value_is_rejected() {
    let record = sample_record();
    let mut encoded = record.encode().expect("encode must succeed");

    // Corrupt the CRC bytes directly (last 4 bytes).
    let len = encoded.len();
    encoded[len - 1] ^= 0x01;

    let result = LocalOperationRecord::decode(&encoded);
    assert!(
        matches!(result, Err(RecordDecodeError::CrcMismatch { .. })),
        "expected CrcMismatch, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Truncation detection
// ---------------------------------------------------------------------------

#[test]
fn truncated_record_is_rejected() {
    let record = sample_record();
    let encoded = record.encode().expect("encode must succeed");

    // Truncate by removing the last 10 bytes (cuts into CRC and payload).
    let truncated = &encoded[..encoded.len() - 10];
    let result = LocalOperationRecord::decode(truncated);
    assert!(
        matches!(result, Err(RecordDecodeError::Truncated { .. })),
        "expected Truncated, got {result:?}"
    );
}

#[test]
fn empty_buffer_is_rejected_as_truncated() {
    let result = LocalOperationRecord::decode(&[]);
    assert!(
        matches!(result, Err(RecordDecodeError::Truncated { .. })),
        "expected Truncated for empty buffer, got {result:?}"
    );
}

#[test]
fn buffer_shorter_than_header_is_rejected() {
    let result = LocalOperationRecord::decode(b"sb");
    assert!(
        matches!(result, Err(RecordDecodeError::Truncated { .. })),
        "expected Truncated for too-short buffer, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Unknown additive JSON field tolerance within version 1
// ---------------------------------------------------------------------------

#[test]
fn unknown_additive_json_field_is_tolerated_in_v1() {
    let record = sample_record();
    let encoded = record.encode().expect("encode must succeed");

    // Decode the structure to find the JSON payload, add an unknown field,
    // recompute CRC, and re-encode. The decoder should tolerate the extra field.
    let label_len = RecordFormat::LABEL_V1.len();
    let _header_len = label_len + 2 + 4;
    let _crc_offset = encoded.len() - 4;

    // Use the public decode to get the payload boundaries, then reconstruct.
    // Actually, let's build a modified buffer manually:
    // label | version(2) | length(4) | modified_json | new_crc(4)
    let label = RecordFormat::LABEL_V1.as_bytes();
    let version_bytes = &encoded[label_len..label_len + 2];
    let length_bytes = &encoded[label_len + 2..label_len + 6];
    let original_length = u32::from_be_bytes([
        length_bytes[0],
        length_bytes[1],
        length_bytes[2],
        length_bytes[3],
    ]) as usize;
    let payload_start = label_len + 6;
    let payload_end = payload_start + original_length;
    let json_bytes = &encoded[payload_start..payload_end];

    // Parse JSON, add an unknown field, re-serialize.
    let mut json: serde_json::Value =
        serde_json::from_slice(json_bytes).expect("payload is valid JSON");
    if let serde_json::Value::Object(ref mut map) = json {
        map.insert(
            "future_extension_field".to_string(),
            serde_json::Value::String("forward-compatible-value".to_string()),
        );
    }
    let modified_json = serde_json::to_vec(&json).expect("re-serialize JSON");

    // Recompute CRC over the modified payload.
    let crc = crc32fast::hash(&modified_json);

    // Reconstruct the buffer.
    let mut modified = Vec::new();
    modified.extend_from_slice(label);
    modified.extend_from_slice(version_bytes);
    modified.extend_from_slice(&(modified_json.len() as u32).to_be_bytes());
    modified.extend_from_slice(&modified_json);
    modified.extend_from_slice(&crc.to_be_bytes());

    let result = LocalOperationRecord::decode(&modified);
    assert!(
        result.is_ok(),
        "unknown additive field must be tolerated, got: {result:?}"
    );

    let decoded = result.unwrap();
    assert_eq!(decoded.transaction_id, record.transaction_id);
    assert_eq!(decoded.sequence, record.sequence);
    assert_eq!(decoded.operation, record.operation);
}

// ---------------------------------------------------------------------------
// Attribution retention
// ---------------------------------------------------------------------------

#[test]
fn actor_device_note_transaction_attribution_retained() {
    let original = sample_record();
    let encoded = original.encode().expect("encode");
    let decoded = LocalOperationRecord::decode(&encoded).expect("decode");

    assert_eq!(decoded.actor_id.as_str(), "alice");
    assert_eq!(decoded.device_id.as_str(), "macbook-01");
    assert_eq!(decoded.note_id, original.note_id);
    assert_eq!(decoded.transaction_id, original.transaction_id);
    assert_eq!(decoded.workspace_id, original.workspace_id);
}

// ---------------------------------------------------------------------------
// Format label
// ---------------------------------------------------------------------------

#[test]
fn encoded_record_starts_with_v1_label() {
    let record = sample_record();
    let encoded = record.encode().expect("encode");
    assert!(
        encoded.starts_with(RecordFormat::LABEL_V1.as_bytes()),
        "encoded record must start with v1 label"
    );
}
