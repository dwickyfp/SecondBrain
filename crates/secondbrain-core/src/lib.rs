#![forbid(unsafe_code)]

//! Strongly typed primitives prevent identity domains from being mixed.
//!
//! ```compile_fail
//! use secondbrain_core::id::{TransactionId, WorkspaceId};
//!
//! let transaction_id = TransactionId::new();
//! let _: WorkspaceId = transaction_id;
//! ```

pub mod actor;
pub mod hash;
pub mod id;
pub mod path;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::path::Path;
    use std::str::FromStr;

    use crate::actor::{ActorId, DeviceId, IdentityError, MAX_IDENTITY_LEN};
    use crate::hash::ContentHash;
    use crate::id::{NoteId, NoteVersion, TransactionId, WorkspaceEpoch, WorkspaceId};
    use crate::path::{WorkspacePath, WorkspacePathError};

    fn assert_id_api_types(
        workspace_id: WorkspaceId,
        transaction_id: TransactionId,
    ) -> (WorkspaceId, TransactionId) {
        (workspace_id, transaction_id)
    }

    const CANONICAL_ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    const OVERFLOW_ULID_ALIAS: &str = "80000000000000000000000000";

    fn assert_json_round_trip<T>(value: &T, expected_json: &str)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Eq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("value serializes");
        assert_eq!(json, expected_json);
        let decoded: T = serde_json::from_str(&json).expect("value deserializes");
        assert_eq!(&decoded, value);
    }

    fn noncanonical_ulid_forms() -> [String; 4] {
        [
            CANONICAL_ULID.to_ascii_lowercase(),
            CANONICAL_ULID[..25].to_owned(),
            format!("{CANONICAL_ULID}0"),
            OVERFLOW_ULID_ALIAS.to_owned(),
        ]
    }

    fn assert_parse_rejects_noncanonical_ulids<T>()
    where
        T: FromStr,
    {
        for candidate in noncanonical_ulid_forms() {
            assert!(
                candidate.parse::<T>().is_err(),
                "noncanonical ULID must be rejected: {candidate:?}"
            );
        }
    }

    fn assert_serde_rejects_noncanonical_ulids<T>()
    where
        T: serde::de::DeserializeOwned,
    {
        for candidate in noncanonical_ulid_forms() {
            let json = serde_json::to_string(&candidate).expect("candidate serializes");
            assert!(
                serde_json::from_str::<T>(&json).is_err(),
                "noncanonical ULID JSON must be rejected: {json}"
            );
        }
    }

    macro_rules! rejects_noncanonical_ulids {
        ($parse_test:ident, $serde_test:ident, $id:ty) => {
            #[test]
            fn $parse_test() {
                assert_parse_rejects_noncanonical_ulids::<$id>();
            }

            #[test]
            fn $serde_test() {
                assert_serde_rejects_noncanonical_ulids::<$id>();
            }
        };
    }

    rejects_noncanonical_ulids!(
        workspace_id_rejects_noncanonical_text,
        workspace_id_serde_rejects_noncanonical_text,
        WorkspaceId
    );
    rejects_noncanonical_ulids!(
        note_id_rejects_noncanonical_text,
        note_id_serde_rejects_noncanonical_text,
        NoteId
    );
    rejects_noncanonical_ulids!(
        transaction_id_rejects_noncanonical_text,
        transaction_id_serde_rejects_noncanonical_text,
        TransactionId
    );

    #[test]
    fn note_id_new_uses_canonical_ulid_text_and_round_trips() {
        let id = NoteId::new();
        let text = id.to_string();

        assert_eq!(text.len(), 26);
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        );
        assert_eq!(NoteId::from_str(&text), Ok(id));
    }

    #[test]
    fn workspace_and_transaction_ids_are_distinct_round_tripping_types() {
        let workspace_id = WorkspaceId::new();
        let transaction_id = TransactionId::new();
        let (workspace_id, transaction_id) = assert_id_api_types(workspace_id, transaction_id);

        assert_eq!(workspace_id.to_string().parse(), Ok(workspace_id));
        assert_eq!(transaction_id.to_string().parse(), Ok(transaction_id));

        let mut workspace_map = HashMap::<WorkspaceId, &'static str>::new();
        workspace_map.insert(workspace_id, "workspace");
        let mut transaction_map = HashMap::<TransactionId, &'static str>::new();
        transaction_map.insert(transaction_id, "transaction");
        assert_eq!(workspace_map.get(&workspace_id), Some(&"workspace"));
        assert_eq!(transaction_map.get(&transaction_id), Some(&"transaction"));
    }

    #[test]
    fn invalid_ulid_returns_a_typed_parse_error_without_panicking() {
        let error = "not-a-ulid"
            .parse::<NoteId>()
            .expect_err("invalid ULID must fail");
        let _: crate::id::IdParseError = error;
    }

    #[test]
    fn ids_serialize_as_stable_ulid_strings() {
        let note = NoteId::new();
        let workspace = WorkspaceId::new();
        let transaction = TransactionId::new();

        assert_json_round_trip(&note, &format!("\"{note}\""));
        assert_json_round_trip(&workspace, &format!("\"{workspace}\""));
        assert_json_round_trip(&transaction, &format!("\"{transaction}\""));
    }

    #[test]
    fn workspace_path_accepts_normalized_portable_utf8_relative_paths() {
        let workspace_path = WorkspacePath::new("Notes/日本語.md").expect("valid workspace path");

        assert_eq!(workspace_path.as_path(), Path::new("Notes/日本語.md"));
        assert_eq!(workspace_path.to_string(), "Notes/日本語.md");
        assert_json_round_trip(&workspace_path, "\"Notes/日本語.md\"");
    }

    #[test]
    fn workspace_path_rejects_empty_absolute_traversal_and_reserved_paths() {
        let invalid = [
            ("", WorkspacePathError::Empty),
            ("/Notes/a.md", WorkspacePathError::Absolute),
            ("C:/Notes/a.md", WorkspacePathError::Absolute),
            ("C:Notes/a.md", WorkspacePathError::Absolute),
            ("//server/share/a.md", WorkspacePathError::Absolute),
            ("../a.md", WorkspacePathError::ParentTraversal),
            ("a/../b.md", WorkspacePathError::ParentTraversal),
            ("a\0b.md", WorkspacePathError::Nul),
            (".", WorkspacePathError::NotNormalized),
            ("././", WorkspacePathError::NotNormalized),
            (".secondbrain", WorkspacePathError::Reserved),
            (".secondbrain/config", WorkspacePathError::Reserved),
        ];

        for (candidate, expected) in invalid {
            assert_eq!(
                WorkspacePath::new(candidate),
                Err(expected),
                "candidate: {candidate:?}"
            );
        }
    }

    #[test]
    fn workspace_path_rejects_ambiguous_or_non_normalized_separators() {
        let invalid = [
            (r"Notes\a.md", WorkspacePathError::Backslash),
            (r"C:\Notes\a.md", WorkspacePathError::Backslash),
            (r"\\server\share\a.md", WorkspacePathError::Backslash),
            ("Notes//a.md", WorkspacePathError::NotNormalized),
            ("Notes/./a.md", WorkspacePathError::NotNormalized),
            ("Notes/", WorkspacePathError::NotNormalized),
        ];

        for (candidate, expected) in invalid {
            assert_eq!(
                WorkspacePath::new(candidate),
                Err(expected),
                "candidate: {candidate:?}"
            );
        }
    }

    #[test]
    fn workspace_path_only_reserves_exact_secondbrain_first_component() {
        for candidate in [".secondbrain-notes/a.md", "notes/.secondbrain/a.md"] {
            assert!(
                WorkspacePath::new(candidate).is_ok(),
                "candidate: {candidate:?}"
            );
        }
    }

    #[test]
    fn content_hash_is_exact_blake3_and_changes_with_input_bytes() {
        let first = ContentHash::digest(b"second brain");
        let second = ContentHash::digest(b"second brain!");

        assert_eq!(first.as_bytes(), blake3::hash(b"second brain").as_bytes());
        assert_ne!(first, second);
    }

    #[test]
    fn content_hash_has_canonical_hex_parse_and_serde_round_trips() {
        let hash = ContentHash::digest("日本語".as_bytes());
        let text = hash.to_string();

        assert_eq!(text.len(), 64);
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(ContentHash::from_str(&text), Ok(hash));
        assert_json_round_trip(&hash, &format!("\"{text}\""));
    }

    #[test]
    fn content_hash_rejects_malformed_length_and_hex() {
        assert!(matches!(
            ContentHash::from_str("00"),
            Err(crate::hash::ContentHashParseError::InvalidLength { actual: 2 })
        ));
        assert!(matches!(
            ContentHash::from_str(&"g".repeat(64)),
            Err(crate::hash::ContentHashParseError::InvalidHex { index: 0, .. })
        ));
    }

    #[test]
    fn actors_and_devices_accept_ordinary_unicode_without_normalizing() {
        let actor = ActorId::new("開発者-01").expect("valid actor");
        let device = DeviceId::new("Dwicky’s MacBook").expect("valid device");

        assert_eq!(actor.as_str(), "開発者-01");
        assert_eq!(device.as_str(), "Dwicky’s MacBook");
        assert_eq!(actor.to_string(), "開発者-01");
        assert_eq!(device.to_string(), "Dwicky’s MacBook");
        assert_json_round_trip(&actor, "\"開発者-01\"");
        assert_json_round_trip(&device, "\"Dwicky’s MacBook\"");
    }

    #[test]
    fn actors_and_devices_reject_whitespace_controls_and_excessive_length() {
        for candidate in [
            "",
            " ",
            "\t",
            " actor",
            "actor ",
            "actor\nname",
            "actor\u{0085}name",
        ] {
            assert!(
                ActorId::new(candidate).is_err(),
                "actor candidate: {candidate:?}"
            );
            assert!(
                DeviceId::new(candidate).is_err(),
                "device candidate: {candidate:?}"
            );
        }

        assert_eq!(
            ActorId::new("a".repeat(MAX_IDENTITY_LEN + 1)),
            Err(IdentityError::TooLong {
                max: MAX_IDENTITY_LEN,
                actual: MAX_IDENTITY_LEN + 1,
            })
        );
        assert!(ActorId::new("a".repeat(MAX_IDENTITY_LEN)).is_ok());
    }

    #[test]
    fn identity_length_limit_counts_unicode_scalar_values() {
        assert!(ActorId::new("界".repeat(MAX_IDENTITY_LEN)).is_ok());
        assert_eq!(
            DeviceId::new("界".repeat(MAX_IDENTITY_LEN + 1)),
            Err(IdentityError::TooLong {
                max: MAX_IDENTITY_LEN,
                actual: MAX_IDENTITY_LEN + 1,
            })
        );
    }

    #[test]
    fn workspace_epoch_and_note_version_are_checked_value_types() {
        let epoch = WorkspaceEpoch::new(41);
        let version = NoteVersion::new(7);

        assert_eq!(epoch.get(), 41);
        assert_eq!(epoch.checked_increment(), Some(WorkspaceEpoch::new(42)));
        assert_eq!(WorkspaceEpoch::new(u64::MAX).checked_increment(), None);
        assert_eq!(version.get(), 7);
        assert_eq!(version.checked_increment(), Some(NoteVersion::new(8)));
        assert_eq!(NoteVersion::new(u64::MAX).checked_increment(), None);

        let mut epochs = BTreeMap::new();
        epochs.insert(epoch, "epoch");
        assert_eq!(epochs.get(&WorkspaceEpoch::new(41)), Some(&"epoch"));
    }

    #[test]
    fn epochs_and_versions_use_stable_numeric_json_forms() {
        assert_json_round_trip(&WorkspaceEpoch::new(41), "41");
        assert_json_round_trip(&NoteVersion::new(7), "7");
    }
}
