#![forbid(unsafe_code)]

//! SecondBrain Transaction: versioned local mutation journal records.
//!
//! Phase 0 durability layer with corruption detection. Provides deterministic
//! encoding of local operation records for the pre-CRDT recovery plumbing.
//! This format is **not** a network envelope — it is for local persistence only.

pub mod create;
pub mod daily;
pub mod engine;
pub mod external_edit;
pub mod failpoint;
mod marker;
pub mod oplog;
pub mod paths;
pub mod preview;
pub mod property;
pub mod record;
pub mod recovery;
pub mod state;

pub use create::{
    NOTE_CREATE_PREVIEW_FORMAT_V1, NoteCreateError, NoteCreateOutcome, NoteCreatePreview,
    apply_note_creation, preview_note_creation,
};
pub use daily::{DailyDate, DailyNote, DailyNoteError, open_or_preview_daily_note};
pub use engine::{
    CommitOutcome, CreateTransactionRequest, TransactionEngine, TransactionError,
    TransactionRequest,
};
pub use external_edit::{
    ExternalEditCoordinator, ExternalEditError, ExternalEditOutcome, IndexRefresh,
    InternalWriteReceipts, NoWatcher,
};
pub use preview::{
    ApplyPreviewOutcome, LegacyTransactionPreview, TRANSACTION_PREVIEW_FORMAT_V1,
    TransactionPreview, TransactionPreviewError, apply_legacy_preview, apply_preview,
    preview_transaction,
};
pub use property::{
    PROPERTY_PREVIEW_FORMAT_V1, PropertyPreview, apply_property_preview, preview_property,
    read_note_properties,
};
pub use recovery::{AbandonedReason, PendingReview, RecoveryAction, TransactionSummary};
pub use state::{StateTransitionError, TransactionState};

pub use record::{
    FORMAT_VERSION_1, LocalOperationRecord, RecordDecodeError, RecordEncodeError, RecordFormat,
};
