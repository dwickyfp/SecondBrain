//! Where one workspace's durable transaction files live.
//!
//! Transaction markers and review descriptors share
//! `.secondbrain/transactions/`, and both are JSON. A marker is told apart from
//! everything else in that directory by its filename being a transaction id and
//! nothing more — so `<id>.json` is a marker and `<id>.conflict.json` is not.
//!
//! That convention couples the code that writes a descriptor to the code that
//! filters markers out of the same directory: if either side spelled it out for
//! itself, a descriptor could be parsed as a marker and fail a whole recovery
//! pass. It is therefore stated once, here.

use std::path::{Path, PathBuf};

use secondbrain_core::id::TransactionId;

/// The workspace-relative directory holding markers and review descriptors.
const TRANSACTION_DIR: &str = ".secondbrain/transactions";

/// The directory holding every transaction marker and review descriptor.
pub(crate) fn transactions_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(TRANSACTION_DIR)
}

/// The durable state marker of one transaction.
pub(crate) fn marker_path(workspace_root: &Path, transaction_id: TransactionId) -> PathBuf {
    transactions_dir(workspace_root).join(format!("{transaction_id}.json"))
}

/// The review descriptor filed under one transaction.
pub(crate) fn review_descriptor_path(
    workspace_root: &Path,
    transaction_id: TransactionId,
) -> PathBuf {
    transactions_dir(workspace_root).join(format!("{transaction_id}.conflict.json"))
}

/// Whether `path` names a transaction marker rather than another file sharing
/// the directory, such as a review descriptor.
pub(crate) fn is_marker(path: &Path) -> bool {
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return false;
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.parse::<TransactionId>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_is_recognized_and_a_review_descriptor_is_not() {
        let root = Path::new("/workspace");
        let transaction_id = TransactionId::new();

        assert!(is_marker(&marker_path(root, transaction_id)));
        assert!(
            !is_marker(&review_descriptor_path(root, transaction_id)),
            "a descriptor parsed as a marker would fail a whole recovery pass"
        );
    }

    #[test]
    fn markers_and_descriptors_share_one_directory() {
        let root = Path::new("/workspace");
        let transaction_id = TransactionId::new();

        assert_eq!(
            marker_path(root, transaction_id).parent(),
            review_descriptor_path(root, transaction_id).parent()
        );
        assert_eq!(
            marker_path(root, transaction_id).parent(),
            Some(transactions_dir(root).as_path())
        );
    }
}
