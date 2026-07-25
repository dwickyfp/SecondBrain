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
///
/// Public for the same reason [`is_review_descriptor`] is: anything that has to
/// name one — including a test standing a pending review up — asks here rather
/// than spelling `<id>.conflict.json` out for itself and drifting from the side
/// that reads it.
#[must_use]
pub fn review_descriptor_path(workspace_root: &Path, transaction_id: TransactionId) -> PathBuf {
    transactions_dir(workspace_root).join(format!("{transaction_id}.conflict.json"))
}

/// Whether `path` names a transaction marker rather than another file sharing
/// the directory, such as a review descriptor.
///
/// Public so that anything reading the directory — including tests — asks this
/// rather than restating the convention and drifting from it.
#[must_use]
pub fn is_marker(path: &Path) -> bool {
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return false;
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.parse::<TransactionId>().is_ok())
}

/// Whether `path` names a review descriptor rather than a marker.
///
/// The counterpart to [`is_marker`], stated here for the same reason: anything
/// that has to tell the two apart while reading the shared directory asks
/// rather than re-deriving `<id>.conflict.json` for itself.
#[must_use]
pub fn is_review_descriptor(path: &Path) -> bool {
    review_descriptor_transaction(path).is_some()
}

/// The transaction a review descriptor is filed under, or `None` when `path` is
/// not one.
///
/// The filename *is* the identity, so recognizing a descriptor and naming the
/// transaction it belongs to are one question, answered once.
pub(crate) fn review_descriptor_transaction(path: &Path) -> Option<TransactionId> {
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return None;
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_suffix(".conflict"))
        .and_then(|stem| stem.parse::<TransactionId>().ok())
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
        assert!(is_review_descriptor(&review_descriptor_path(
            root,
            transaction_id
        )));
        assert!(
            !is_review_descriptor(&marker_path(root, transaction_id)),
            "a marker counted as a pending review would report a human is needed when none is"
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
