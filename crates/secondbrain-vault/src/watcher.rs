//! Filesystem event normalization.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use secondbrain_core::hash::ContentHash;
use secondbrain_core::path::WorkspacePath;
use thiserror::Error;

use crate::WorkspaceRoot;
use crate::event::WorkspaceEvent;

/// Platform-neutral raw event accepted by the normalization worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawEventKind {
    Create,
    Modify,
    Remove,
    Rename,
}

/// Rename detail retained from the platform watcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawRenameMode {
    From,
    To,
    Both,
    Any,
    Other,
}

/// Minimal data copied from an OS watcher callback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawEvent {
    pub kind: RawEventKind,
    pub rename_mode: Option<RawRenameMode>,
    pub tracker: Option<usize>,
    pub paths: Vec<PathBuf>,
}

impl RawEvent {
    #[must_use]
    pub fn from_notify(event: notify::Event) -> Self {
        let rename_mode = match event.kind {
            notify::EventKind::Modify(notify::event::ModifyKind::Name(mode)) => Some(match mode {
                notify::event::RenameMode::From => RawRenameMode::From,
                notify::event::RenameMode::To => RawRenameMode::To,
                notify::event::RenameMode::Both => RawRenameMode::Both,
                notify::event::RenameMode::Any => RawRenameMode::Any,
                _ => RawRenameMode::Other,
            }),
            _ => None,
        };
        let kind = match event.kind {
            notify::EventKind::Create(_) => RawEventKind::Create,
            notify::EventKind::Remove(_) => RawEventKind::Remove,
            notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => RawEventKind::Rename,
            _ => RawEventKind::Modify,
        };
        Self {
            kind,
            rename_mode,
            tracker: event.attrs.tracker(),
            paths: event.paths,
        }
    }
}

/// Errors while observing and confining changed paths.
#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("filesystem watcher reported: {errors:?}")]
    Notify { errors: Vec<notify::Error> },
    #[error("filesystem watcher worker stopped")]
    WorkerStopped,
}

fn convert_debouncer_message(
    message: notify_debouncer_full::DebounceEventResult,
) -> Result<Vec<notify_debouncer_full::DebouncedEvent>, WatcherError> {
    message.map_err(|errors| WatcherError::Notify { errors })
}

/// Running OS watcher whose callback only forwards debounced events to a worker.
pub struct WorkspaceWatcher {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
    events: Receiver<Result<Vec<WorkspaceEvent>, WatcherError>>,
    commands: SyncSender<WorkerCommand>,
    _worker: JoinHandle<()>,
}

impl WorkspaceWatcher {
    /// Starts watching with independently configured debounce, split-rename, and receipt windows.
    ///
    /// A zero `rename_ttl` expires unmatched tracked rename halves on the next worker pass. A zero
    /// `receipt_ttl` makes internal-write receipts immediately ineligible for suppression.
    pub fn start(
        root: WorkspaceRoot,
        exclusions: Vec<WorkspacePath>,
        debounce: Duration,
        rename_ttl: Duration,
        receipt_ttl: Duration,
    ) -> Result<Self, WatcherError> {
        let (raw_tx, raw_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::sync_channel(64);
        let mut debouncer = notify_debouncer_full::new_debouncer(debounce, None, move |result| {
            let _ = raw_tx.send(result);
        })
        .map_err(|source| WatcherError::Io {
            path: PathBuf::from("watcher"),
            source: io::Error::other(source),
        })?;
        debouncer
            .watch(root.canonical_path(), notify::RecursiveMode::Recursive)
            .map_err(|source| WatcherError::Io {
                path: root.canonical_path().to_owned(),
                source: io::Error::other(source),
            })?;
        let worker = thread::spawn(move || {
            let mut state = WorkerState::new(root, exclusions, rename_ttl, receipt_ttl);
            loop {
                if !state.process_pending_commands(&command_rx) {
                    break;
                }
                let message = match raw_rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        match state.on_idle_timeout(Instant::now()) {
                            Ok(events) if events.is_empty() => {}
                            result => {
                                if event_tx.send(result).is_err() {
                                    break;
                                }
                            }
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if !state.process_pending_commands(&command_rx) {
                    break;
                }
                let events = match convert_debouncer_message(message) {
                    Ok(events) => events,
                    Err(error) => {
                        let _ = event_tx.send(Err(error));
                        continue;
                    }
                };
                let raw = events
                    .into_iter()
                    .map(|event| RawEvent::from_notify(event.event));
                if event_tx.send(state.normalize(raw.collect())).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            _debouncer: debouncer,
            events,
            commands: command_tx,
            _worker: worker,
        })
    }

    pub fn recv(&self) -> Result<Vec<WorkspaceEvent>, WatcherError> {
        self.events
            .recv()
            .map_err(|_| WatcherError::WorkerStopped)?
    }

    /// Records an expected internal write for one-shot suppression by the live worker.
    ///
    /// The worker timestamps the receipt and acknowledges it before this method returns,
    /// so filesystem events observed after a successful return see the receipt first.
    pub fn record_internal_write(
        &self,
        path: WorkspacePath,
        hash: ContentHash,
    ) -> Result<(), WatcherError> {
        let (acknowledged, acknowledgement) = mpsc::sync_channel(0);
        self.commands
            .send(WorkerCommand::RecordInternalWrite {
                path,
                hash,
                acknowledged,
            })
            .map_err(|_| WatcherError::WorkerStopped)?;
        acknowledgement
            .recv()
            .map_err(|_| WatcherError::WorkerStopped)
    }
}

enum WorkerCommand {
    RecordInternalWrite {
        path: WorkspacePath,
        hash: ContentHash,
        acknowledged: SyncSender<()>,
    },
}

struct WorkerState {
    normalizer: Normalizer,
    rename_ttl: Duration,
    pending_tracked: VecDeque<PendingTrackedRename>,
    pending_untracked: VecDeque<PendingUntrackedRename>,
    poisoned_trackers: VecDeque<PoisonedTracker>,
}

const MAX_PENDING_TRACKED_RENAMES: usize = 1024;
const MAX_PENDING_UNTRACKED_RENAMES: usize = 1024;
const MAX_POISONED_TRACKERS: usize = 1024;

#[derive(Clone, Debug)]
struct PoisonedTracker {
    tracker: usize,
    expires: Instant,
}

#[derive(Clone, Debug)]
struct PendingTrackedRename {
    tracker: usize,
    mode: RawRenameMode,
    event: RawEvent,
    expires: Instant,
}

#[derive(Clone, Debug)]
struct PendingUntrackedRename {
    mode: RawRenameMode,
    event: RawEvent,
    expires: Instant,
}

impl WorkerState {
    fn new(
        root: WorkspaceRoot,
        exclusions: Vec<WorkspacePath>,
        rename_ttl: Duration,
        receipt_ttl: Duration,
    ) -> Self {
        Self {
            normalizer: Normalizer::new(root, exclusions, receipt_ttl),
            rename_ttl,
            pending_tracked: VecDeque::new(),
            pending_untracked: VecDeque::new(),
            poisoned_trackers: VecDeque::new(),
        }
    }

    fn process_command(&mut self, command: WorkerCommand) {
        match command {
            WorkerCommand::RecordInternalWrite {
                path,
                hash,
                acknowledged,
            } => {
                self.normalizer
                    .record_internal_write(path, hash, Instant::now());
                let _ = acknowledged.send(());
            }
        }
    }

    fn process_pending_commands(&mut self, commands: &Receiver<WorkerCommand>) -> bool {
        loop {
            match commands.try_recv() {
                Ok(command) => self.process_command(command),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn normalize(&mut self, events: Vec<RawEvent>) -> Result<Vec<WorkspaceEvent>, WatcherError> {
        self.normalize_at(events, Instant::now())
    }

    fn normalize_at(
        &mut self,
        events: Vec<RawEvent>,
        now: Instant,
    ) -> Result<Vec<WorkspaceEvent>, WatcherError> {
        let pending_tracked = self.pending_tracked.clone();
        let pending_untracked = self.pending_untracked.clone();
        let poisoned_trackers = self.poisoned_trackers.clone();
        match self.normalize_staged(events, now) {
            Ok(output) => Ok(output),
            Err(error) => {
                self.pending_tracked = pending_tracked;
                self.pending_untracked = pending_untracked;
                self.poisoned_trackers = poisoned_trackers;
                Err(error)
            }
        }
    }

    fn normalize_staged(
        &mut self,
        events: Vec<RawEvent>,
        now: Instant,
    ) -> Result<Vec<WorkspaceEvent>, WatcherError> {
        let mut ready = self.flush_expired(now);
        for event in events {
            if let (Some(mode @ (RawRenameMode::From | RawRenameMode::To)), None) =
                (event.rename_mode, event.tracker)
            {
                if let Some(evicted) = self.defer_untracked(mode, event, now) {
                    ready.push(evicted);
                }
                continue;
            }
            let (Some(mode @ (RawRenameMode::From | RawRenameMode::To)), Some(tracker)) =
                (event.rename_mode, event.tracker)
            else {
                ready.push(event);
                continue;
            };

            if self
                .poisoned_trackers
                .iter()
                .any(|poisoned| poisoned.tracker == tracker && poisoned.expires > now)
            {
                if let Some(evicted) = self.defer_unpairable(tracker, mode, event, now) {
                    ready.push(evicted);
                }
                continue;
            }
            self.poisoned_trackers
                .retain(|poisoned| poisoned.tracker != tracker);

            if self
                .pending_tracked
                .iter()
                .any(|half| half.tracker == tracker && half.mode == mode)
            {
                // Reusing one tracker for multiple same-side halves is ambiguous. Quarantine
                // every half for that tracker until expiry rather than pairing wrong paths.
                ready.extend(self.poison_tracker(tracker, now));
                if let Some(evicted) = self.defer_unpairable(tracker, mode, event, now) {
                    ready.push(evicted);
                }
                continue;
            }

            if let Some(index) = self
                .pending_tracked
                .iter()
                .position(|half| half.tracker == tracker && half.mode != mode)
            {
                let other = self.pending_tracked.remove(index).expect("index exists");
                let (from, to) = if mode == RawRenameMode::From {
                    (event, other.event)
                } else {
                    (other.event, event)
                };
                ready.push(RawEvent {
                    kind: RawEventKind::Rename,
                    rename_mode: Some(RawRenameMode::Both),
                    tracker: Some(tracker),
                    paths: from.paths.into_iter().chain(to.paths).collect(),
                });
            } else if let Some(evicted) = self.defer_unpairable(tracker, mode, event, now) {
                ready.push(evicted);
            }
        }
        if let Some(rename) = self.take_unambiguous_untracked_pair() {
            ready.push(rename);
        }
        self.normalizer.normalize(ready, now)
    }

    fn on_idle_timeout(&mut self, now: Instant) -> Result<Vec<WorkspaceEvent>, WatcherError> {
        self.normalize_at(Vec::new(), now)
    }

    fn defer_unpairable(
        &mut self,
        tracker: usize,
        mode: RawRenameMode,
        event: RawEvent,
        now: Instant,
    ) -> Option<RawEvent> {
        let evicted = if self.pending_tracked.len() == MAX_PENDING_TRACKED_RENAMES {
            self.pending_tracked
                .pop_front()
                .and_then(observe_tracked_to)
        } else {
            None
        };
        self.pending_tracked.push_back(PendingTrackedRename {
            tracker,
            mode,
            event,
            expires: now + self.rename_ttl,
        });
        evicted
    }

    fn poison_tracker(&mut self, tracker: usize, now: Instant) -> Vec<RawEvent> {
        let mut observed = Vec::new();
        if self.poisoned_trackers.len() == MAX_POISONED_TRACKERS {
            let oldest = self
                .poisoned_trackers
                .iter()
                .enumerate()
                .min_by_key(|(_, poisoned)| poisoned.expires)
                .map(|(index, _)| index)
                .expect("non-empty at capacity");
            let evicted = self.poisoned_trackers.remove(oldest).expect("index exists");
            // Remove every corresponding half before tracker reuse. Preserve destinations as
            // content observations; sources have no independently observable destination.
            let mut retained = VecDeque::new();
            while let Some(half) = self.pending_tracked.pop_front() {
                if half.tracker == evicted.tracker {
                    if let Some(event) = observe_tracked_to(half) {
                        observed.push(event);
                    }
                } else {
                    retained.push_back(half);
                }
            }
            self.pending_tracked = retained;
        }
        self.poisoned_trackers.push_back(PoisonedTracker {
            tracker,
            expires: now + self.rename_ttl,
        });
        observed
    }

    fn defer_untracked(
        &mut self,
        mode: RawRenameMode,
        event: RawEvent,
        now: Instant,
    ) -> Option<RawEvent> {
        let evicted = if self.pending_untracked.len() == MAX_PENDING_UNTRACKED_RENAMES {
            self.pending_untracked
                .pop_front()
                .and_then(observe_untracked_to)
        } else {
            None
        };
        self.pending_untracked.push_back(PendingUntrackedRename {
            mode,
            event,
            expires: now + self.rename_ttl,
        });
        evicted
    }

    fn take_unambiguous_untracked_pair(&mut self) -> Option<RawEvent> {
        let from = self
            .pending_untracked
            .iter()
            .position(|half| half.mode == RawRenameMode::From)?;
        if self
            .pending_untracked
            .iter()
            .skip(from + 1)
            .any(|half| half.mode == RawRenameMode::From)
        {
            return None;
        }
        let to = self
            .pending_untracked
            .iter()
            .position(|half| half.mode == RawRenameMode::To)?;
        if self
            .pending_untracked
            .iter()
            .skip(to + 1)
            .any(|half| half.mode == RawRenameMode::To)
        {
            return None;
        }
        let to_event = self
            .pending_untracked
            .remove(to)
            .expect("index exists")
            .event;
        let from = if from > to { from - 1 } else { from };
        let from_event = self
            .pending_untracked
            .remove(from)
            .expect("index exists")
            .event;
        Some(RawEvent {
            kind: RawEventKind::Rename,
            rename_mode: Some(RawRenameMode::Both),
            tracker: None,
            paths: from_event.paths.into_iter().chain(to_event.paths).collect(),
        })
    }

    fn flush_expired(&mut self, now: Instant) -> Vec<RawEvent> {
        let mut ready = Vec::new();
        let mut retained = VecDeque::new();
        while let Some(half) = self.pending_tracked.pop_front() {
            if half.expires <= now {
                if let Some(event) = observe_tracked_to(half) {
                    ready.push(event);
                }
            } else {
                retained.push_back(half);
            }
        }
        self.pending_tracked = retained;
        let mut retained = VecDeque::new();
        while let Some(half) = self.pending_untracked.pop_front() {
            if half.expires <= now {
                if let Some(event) = observe_untracked_to(half) {
                    ready.push(event);
                }
            } else {
                retained.push_back(half);
            }
        }
        self.pending_untracked = retained;
        self.poisoned_trackers
            .retain(|poisoned| poisoned.expires > now);
        ready
    }
}

fn observe_tracked_to(half: PendingTrackedRename) -> Option<RawEvent> {
    (half.mode == RawRenameMode::To).then_some(RawEvent {
        kind: RawEventKind::Modify,
        ..half.event
    })
}

fn observe_untracked_to(half: PendingUntrackedRename) -> Option<RawEvent> {
    (half.mode == RawRenameMode::To).then_some(RawEvent {
        kind: RawEventKind::Modify,
        ..half.event
    })
}

#[derive(Clone, Debug)]
struct Receipt {
    path: WorkspacePath,
    hash: ContentHash,
    expires: Instant,
}

/// Stateful conversion of noisy raw events into deterministic events.
pub struct Normalizer {
    root: WorkspaceRoot,
    exclusions: Vec<WorkspacePath>,
    receipt_ttl: Duration,
    receipts: VecDeque<Receipt>,
    known: HashMap<WorkspacePath, ContentHash>,
}

impl Normalizer {
    #[must_use]
    pub fn new(root: WorkspaceRoot, exclusions: Vec<WorkspacePath>, receipt_ttl: Duration) -> Self {
        Self {
            root,
            exclusions,
            receipt_ttl,
            receipts: VecDeque::new(),
            known: HashMap::new(),
        }
    }

    pub fn normalize(
        &mut self,
        raw_events: impl IntoIterator<Item = RawEvent>,
        now: Instant,
    ) -> Result<Vec<WorkspaceEvent>, WatcherError> {
        let mut receipts = self.receipts.clone();
        let mut known = self.known.clone();
        receipts.retain(|receipt| receipt.expires > now);
        // Renames retain input order; ordinary paths are sorted, while transitions for
        // the same path retain input order. This makes batches deterministic without
        // collapsing a meaningful Remove -> Create lifecycle.
        let mut pending = BTreeMap::<WorkspacePath, Vec<RawEventKind>>::new();
        let mut output = Vec::new();
        let raw_events = correlate_split_renames(raw_events.into_iter().collect());
        for raw in raw_events {
            if raw.kind == RawEventKind::Rename && raw.paths.len() == 2 {
                if let (Some(from), Some(to)) =
                    (self.relative(&raw.paths[0]), self.relative(&raw.paths[1]))
                {
                    if self.confined_absolute(&from).is_none() {
                        continue;
                    }
                    let Some(to_absolute) = self.confined_absolute(&to) else {
                        continue;
                    };
                    if is_atomic_save_temporary(&from) {
                        push_pending(&mut pending, to, RawEventKind::Modify);
                    } else {
                        known.remove(&from);
                        if let Some(hash) = read_rename_destination(&to_absolute)? {
                            known.insert(to.clone(), hash);
                        }
                        output.push(WorkspaceEvent::Renamed { from, to });
                    }
                }
                continue;
            }
            for absolute in raw.paths {
                if let Some(path) = self.relative(&absolute) {
                    push_pending(&mut pending, path, raw.kind);
                }
            }
        }
        for (path, kinds) in pending {
            for kind in kinds {
                match kind {
                    RawEventKind::Remove => {
                        known.remove(&path);
                        output.push(WorkspaceEvent::Deleted { path: path.clone() });
                    }
                    RawEventKind::Create | RawEventKind::Modify | RawEventKind::Rename => {
                        let Some(absolute) = self.confined_absolute(&path) else {
                            continue;
                        };
                        match fs::read(&absolute) {
                            Ok(bytes) => {
                                let hash = ContentHash::digest(bytes);
                                if let Some(index) = receipts.iter().position(|receipt| {
                                    receipt.path == path && receipt.hash == hash
                                }) {
                                    receipts.remove(index);
                                    known.insert(path.clone(), hash);
                                    continue;
                                }
                                if known.get(&path) != Some(&hash) {
                                    known.insert(path.clone(), hash);
                                    output.push(WorkspaceEvent::ContentChanged {
                                        path: path.clone(),
                                        hash,
                                    });
                                }
                            }
                            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                            Err(source) => {
                                return Err(WatcherError::Io {
                                    path: absolute,
                                    source,
                                });
                            }
                        }
                    }
                }
            }
        }
        self.receipts = receipts;
        self.known = known;
        Ok(output)
    }

    fn relative(&self, absolute: &Path) -> Option<WorkspacePath> {
        let relative = absolute.strip_prefix(self.root.canonical_path()).ok()?;
        let portable = relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
        if portable == ".git"
            || portable.starts_with(".git/")
            || portable == ".secondbrain"
            || portable.starts_with(".secondbrain/")
        {
            return None;
        }
        let path = WorkspacePath::new(portable).ok()?;
        if self
            .exclusions
            .iter()
            .any(|excluded| path == *excluded || path.as_path().starts_with(excluded.as_path()))
        {
            return None;
        }
        self.confined_absolute(&path)?;
        Some(path)
    }

    fn confined_absolute(&self, path: &WorkspacePath) -> Option<PathBuf> {
        let absolute = self.root.canonical_path().join(path.as_path());
        let mut existing = absolute.as_path();
        while !existing.exists() {
            existing = existing.parent()?;
        }
        let resolved = existing.canonicalize().ok()?;
        resolved
            .starts_with(self.root.canonical_path())
            .then_some(absolute)
    }

    /// Records one expected internal write for one-shot suppression.
    pub fn record_internal_write(&mut self, path: WorkspacePath, hash: ContentHash, now: Instant) {
        const MAX_RECEIPTS: usize = 1024;
        if self.receipts.len() == MAX_RECEIPTS {
            self.receipts.pop_front();
        }
        self.receipts.push_back(Receipt {
            path,
            hash,
            expires: now + self.receipt_ttl,
        });
    }
}

fn read_rename_destination(path: &Path) -> Result<Option<ContentHash>, WatcherError> {
    read_rename_destination_with(path, |path| fs::read(path))
}

fn read_rename_destination_with(
    path: &Path,
    read: impl FnOnce(&Path) -> io::Result<Vec<u8>>,
) -> Result<Option<ContentHash>, WatcherError> {
    match read(path) {
        Ok(bytes) => Ok(Some(ContentHash::digest(bytes))),
        // The destination can vanish between notification and normalization. In that race,
        // emit the rename without caching an invented destination hash.
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(WatcherError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn correlate_split_renames(mut events: Vec<RawEvent>) -> Vec<RawEvent> {
    let untracked_from = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event.rename_mode == Some(RawRenameMode::From) && event.tracker.is_none()
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let untracked_to = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event.rename_mode == Some(RawRenameMode::To) && event.tracker.is_none()
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if let ([from], [to]) = (untracked_from.as_slice(), untracked_to.as_slice()) {
        let paths = events[*from]
            .paths
            .clone()
            .into_iter()
            .chain(events[*to].paths.clone())
            .collect();
        events[*from] = RawEvent {
            kind: RawEventKind::Rename,
            rename_mode: Some(RawRenameMode::Both),
            tracker: None,
            paths,
        };
        events.remove(*to);
    }

    let mut output = Vec::new();
    let mut from_by_tracker = HashMap::<usize, RawEvent>::new();
    for event in events {
        match (event.rename_mode, event.tracker) {
            (Some(RawRenameMode::From), Some(tracker)) => {
                from_by_tracker.insert(tracker, event);
            }
            (Some(RawRenameMode::To), Some(tracker)) => {
                if let Some(from) = from_by_tracker.remove(&tracker) {
                    output.push(RawEvent {
                        kind: RawEventKind::Rename,
                        rename_mode: Some(RawRenameMode::Both),
                        tracker: Some(tracker),
                        paths: from.paths.into_iter().chain(event.paths).collect(),
                    });
                } else {
                    // An unmatched destination is a safe content observation, never an invented rename.
                    output.push(RawEvent {
                        kind: RawEventKind::Modify,
                        ..event
                    });
                }
            }
            _ => output.push(event),
        }
    }
    // Unmatched sources are intentionally ignored: the path may still exist and guessing a
    // deletion would be destructive. Correlation without trackers is only safe when a future
    // batch-level policy can prove exactly one From and one To.
    output
}

fn push_pending(
    pending: &mut BTreeMap<WorkspacePath, Vec<RawEventKind>>,
    path: WorkspacePath,
    kind: RawEventKind,
) {
    let kinds = pending.entry(path).or_default();
    if kinds.last() != Some(&kind) {
        kinds.push(kind);
    }
}

fn is_atomic_save_temporary(path: &WorkspacePath) -> bool {
    let name = path
        .as_path()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    // Recognize explicit backup/temp suffixes; a leading dot alone is a legitimate filename.
    name.ends_with('~') || name.ends_with(".tmp") || name.ends_with(".swp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked_half(
        root: &WorkspaceRoot,
        mode: RawRenameMode,
        tracker: usize,
        path: &str,
    ) -> RawEvent {
        RawEvent {
            kind: RawEventKind::Rename,
            rename_mode: Some(mode),
            tracker: Some(tracker),
            paths: vec![root.canonical_path().join(path)],
        }
    }

    fn untracked_half(root: &WorkspaceRoot, mode: RawRenameMode, path: &str) -> RawEvent {
        RawEvent {
            kind: RawEventKind::Rename,
            rename_mode: Some(mode),
            tracker: None,
            paths: vec![root.canonical_path().join(path)],
        }
    }

    #[test]
    fn untracked_rename_from_then_to_correlates_across_batches() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("new.md"), "moved").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![untracked_half(&root, RawRenameMode::From, "old.md")],
                    now
                )
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            worker
                .normalize_at(
                    vec![untracked_half(&root, RawRenameMode::To, "new.md")],
                    now + Duration::from_millis(1),
                )
                .unwrap(),
            vec![WorkspaceEvent::Renamed {
                from: WorkspacePath::new("old.md").unwrap(),
                to: WorkspacePath::new("new.md").unwrap(),
            }]
        );
    }

    #[test]
    fn untracked_rename_to_then_from_correlates_across_batches() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("new.md"), "moved").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![untracked_half(&root, RawRenameMode::To, "new.md")],
                    now
                )
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            worker
                .normalize_at(
                    vec![untracked_half(&root, RawRenameMode::From, "old.md")],
                    now + Duration::from_millis(1),
                )
                .unwrap(),
            vec![WorkspaceEvent::Renamed {
                from: WorkspacePath::new("old.md").unwrap(),
                to: WorkspacePath::new("new.md").unwrap(),
            }]
        );
    }

    #[test]
    fn ambiguous_untracked_halves_never_cross_pair() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("new.md"), "moved").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![
                        untracked_half(&root, RawRenameMode::From, "first.md"),
                        untracked_half(&root, RawRenameMode::From, "second.md"),
                    ],
                    now
                )
                .unwrap()
                .is_empty()
        );
        assert!(
            worker
                .normalize_at(
                    vec![untracked_half(&root, RawRenameMode::To, "new.md")],
                    now + Duration::from_millis(1),
                )
                .unwrap()
                .iter()
                .all(|event| !matches!(event, WorkspaceEvent::Renamed { .. }))
        );
    }

    #[test]
    fn idle_timeout_expires_untracked_destination_and_drops_source() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("orphan.md"), "content").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![
                        untracked_half(&root, RawRenameMode::To, "orphan.md"),
                        untracked_half(&root, RawRenameMode::From, "old.md"),
                        untracked_half(&root, RawRenameMode::From, "other.md"),
                    ],
                    now
                )
                .unwrap()
                .is_empty()
        );
        let events = worker
            .on_idle_timeout(now + Duration::from_secs(2))
            .unwrap();
        assert!(
            matches!(events.as_slice(), [WorkspaceEvent::ContentChanged { path, .. }]
            if path.as_str() == "orphan.md")
        );
        assert!(worker.pending_untracked.is_empty());
    }

    #[test]
    fn pending_untracked_halves_are_bounded_and_oldest_destination_is_observed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("oldest.md"), "oldest").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        assert!(
            worker
                .normalize_at(
                    vec![
                        untracked_half(&root, RawRenameMode::To, "oldest.md"),
                        untracked_half(&root, RawRenameMode::To, "second.md"),
                    ],
                    now
                )
                .unwrap()
                .is_empty()
        );

        let overflow = (0..MAX_PENDING_UNTRACKED_RENAMES)
            .map(|index| untracked_half(&root, RawRenameMode::From, &format!("source-{index}.md")))
            .collect();
        let events = worker.normalize_at(overflow, now).unwrap();

        assert_eq!(
            worker.pending_untracked.len(),
            MAX_PENDING_UNTRACKED_RENAMES
        );
        assert!(
            matches!(events.as_slice(), [WorkspaceEvent::ContentChanged { path, .. }]
            if path.as_str() == "oldest.md")
        );
    }

    #[test]
    fn tracked_rename_to_then_from_correlates_across_batches() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("new.md"), "moved").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::To, 7, "new.md")],
                    now,
                )
                .expect("defer destination")
                .is_empty()
        );
        assert_eq!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::From, 7, "old.md")],
                    now + Duration::from_millis(1),
                )
                .expect("correlate rename"),
            vec![WorkspaceEvent::Renamed {
                from: WorkspacePath::new("old.md").expect("from"),
                to: WorkspacePath::new("new.md").expect("to"),
            }]
        );
    }

    #[test]
    fn failed_tracked_rename_read_preserves_pair_for_retry() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let destination = root.canonical_path().join("new.md");
        fs::create_dir(&destination).expect("unreadable rename destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::To, 8, "new.md")],
                    now,
                )
                .expect("defer destination")
                .is_empty()
        );
        let error = worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::From, 8, "old.md")],
                now + Duration::from_millis(1),
            )
            .expect_err("destination read must fail");
        assert!(matches!(
            error,
            WatcherError::Io { path, source }
                if path == destination && source.kind() != io::ErrorKind::NotFound
        ));

        fs::remove_dir(&destination).expect("remove unreadable destination");
        fs::write(&destination, "moved").expect("make destination readable");
        assert_eq!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::From, 8, "old.md")],
                    now + Duration::from_millis(2),
                )
                .expect("retry preserved pair"),
            vec![WorkspaceEvent::Renamed {
                from: WorkspacePath::new("old.md").expect("from"),
                to: WorkspacePath::new("new.md").expect("to"),
            }]
        );
    }

    #[test]
    fn tracked_rename_from_then_to_correlates_across_batches() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("new.md"), "moved").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::From, 9, "old.md")],
                    now,
                )
                .expect("defer source")
                .is_empty()
        );
        assert_eq!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::To, 9, "new.md")],
                    now + Duration::from_millis(1),
                )
                .expect("correlate rename"),
            vec![WorkspaceEvent::Renamed {
                from: WorkspacePath::new("old.md").expect("from"),
                to: WorkspacePath::new("new.md").expect("to"),
            }]
        );
    }

    #[test]
    fn idle_timeout_flushes_expired_destination_and_drops_expired_source() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("orphan.md"), "content").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        assert!(
            worker
                .normalize_at(
                    vec![
                        tracked_half(&root, RawRenameMode::To, 10, "orphan.md"),
                        tracked_half(&root, RawRenameMode::From, 11, "old.md"),
                    ],
                    now,
                )
                .expect("defer unmatched halves")
                .is_empty()
        );

        let events = worker
            .on_idle_timeout(now + Duration::from_secs(2))
            .expect("flush idle timeout");

        assert!(
            matches!(events.as_slice(), [WorkspaceEvent::ContentChanged { path, .. }]
            if path.as_str() == "orphan.md")
        );
        assert!(worker.pending_tracked.is_empty());
    }

    #[test]
    fn expired_tracked_to_becomes_safe_content_observation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("orphan.md"), "content").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        assert!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::To, 11, "orphan.md")],
                    now,
                )
                .unwrap()
                .is_empty()
        );

        let events = worker
            .normalize_at(Vec::new(), now + Duration::from_secs(2))
            .expect("flush expiry");
        assert!(
            matches!(events.as_slice(), [WorkspaceEvent::ContentChanged { path, .. }]
            if path.as_str() == "orphan.md")
        );
    }

    #[test]
    fn failed_expired_destination_read_preserves_half_for_retry() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let destination = root.canonical_path().join("orphan.md");
        fs::create_dir(&destination).expect("unreadable destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::To, 12, "orphan.md")],
                now,
            )
            .expect("defer destination");

        worker
            .on_idle_timeout(now + Duration::from_secs(2))
            .expect_err("expired destination read must fail");
        fs::remove_dir(&destination).expect("remove unreadable destination");
        fs::write(&destination, "content").expect("make destination readable");

        let retried = worker
            .on_idle_timeout(now + Duration::from_secs(2))
            .expect("retry preserved expired destination");
        assert!(matches!(
            retried.as_slice(),
            [WorkspaceEvent::ContentChanged { path, .. }] if path.as_str() == "orphan.md"
        ));
    }

    #[test]
    fn expired_tracked_from_is_silently_dropped() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::From, 12, "old.md")],
                now,
            )
            .unwrap();

        assert!(
            worker
                .normalize_at(Vec::new(), now + Duration::from_secs(2))
                .unwrap()
                .is_empty()
        );
        assert!(worker.pending_tracked.is_empty());
    }

    #[test]
    fn pending_tracked_halves_are_bounded_and_evicted_to_is_observed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("oldest.md"), "oldest").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::To, 0, "oldest.md")],
                now,
            )
            .unwrap();

        let mut overflow = Vec::new();
        for tracker in 1..=MAX_PENDING_TRACKED_RENAMES {
            overflow.push(tracked_half(
                &root,
                RawRenameMode::From,
                tracker,
                &format!("source-{tracker}.md"),
            ));
        }
        let events = worker
            .normalize_at(overflow, now)
            .expect("bounded insertion");

        assert_eq!(worker.pending_tracked.len(), MAX_PENDING_TRACKED_RENAMES);
        assert!(
            matches!(events.as_slice(), [WorkspaceEvent::ContentChanged { path, .. }]
            if path.as_str() == "oldest.md")
        );
    }

    #[test]
    fn failed_capacity_eviction_destination_read_preserves_queue_for_retry() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let destination = root.canonical_path().join("oldest.md");
        fs::create_dir(&destination).expect("unreadable destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();
        worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::To, 0, "oldest.md")],
                now,
            )
            .expect("defer destination");
        let overflow = (1..=MAX_PENDING_TRACKED_RENAMES)
            .map(|tracker| {
                tracked_half(
                    &root,
                    RawRenameMode::From,
                    tracker,
                    &format!("source-{tracker}.md"),
                )
            })
            .collect();

        worker
            .normalize_at(overflow, now)
            .expect_err("evicted destination read must fail");
        assert_eq!(worker.pending_tracked.len(), 1);
        fs::remove_dir(&destination).expect("remove unreadable destination");
        fs::write(&destination, "content").expect("make destination readable");

        let retried = worker
            .normalize_at(
                (1..=MAX_PENDING_TRACKED_RENAMES)
                    .map(|tracker| {
                        tracked_half(
                            &root,
                            RawRenameMode::From,
                            tracker,
                            &format!("source-{tracker}.md"),
                        )
                    })
                    .collect(),
                now,
            )
            .expect("retry preserved queue");
        assert!(matches!(
            retried.as_slice(),
            [WorkspaceEvent::ContentChanged { path, .. }] if path.as_str() == "oldest.md"
        ));
        assert_eq!(worker.pending_tracked.len(), MAX_PENDING_TRACKED_RENAMES);
    }

    #[test]
    fn poisoned_trackers_are_bounded_and_eviction_cannot_pair_reused_tracker() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("destination.md"), "content").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        let now = Instant::now();

        for tracker in 0..MAX_POISONED_TRACKERS {
            worker
                .normalize_at(
                    vec![
                        tracked_half(&root, RawRenameMode::From, tracker, "first.md"),
                        tracked_half(&root, RawRenameMode::From, tracker, "second.md"),
                    ],
                    now,
                )
                .expect("collision is quarantined");
        }
        worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::To, 0, "destination.md")],
                now,
            )
            .expect("poisoned destination is deferred");
        let events = worker
            .normalize_at(
                vec![
                    tracked_half(
                        &root,
                        RawRenameMode::From,
                        MAX_POISONED_TRACKERS,
                        "first.md",
                    ),
                    tracked_half(
                        &root,
                        RawRenameMode::From,
                        MAX_POISONED_TRACKERS,
                        "second.md",
                    ),
                ],
                now,
            )
            .expect("oldest poisoned tracker is evicted");

        assert_eq!(worker.poisoned_trackers.len(), MAX_POISONED_TRACKERS);
        assert!(matches!(
            events.as_slice(),
            [WorkspaceEvent::ContentChanged { path, .. }] if path.as_str() == "destination.md"
        ));

        let reused = worker
            .normalize_at(
                vec![tracked_half(&root, RawRenameMode::To, 0, "destination.md")],
                now,
            )
            .expect("evicted tracker can be reused safely");
        assert!(
            reused
                .iter()
                .all(|event| !matches!(event, WorkspaceEvent::Renamed { .. })),
            "an evicted quarantine must not leave a stale half that can pair"
        );
    }

    #[test]
    fn tracker_collision_never_pairs_wrong_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        fs::write(root.canonical_path().join("destination.md"), "content").expect("destination");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let now = Instant::now();

        let events = worker
            .normalize_at(
                vec![
                    tracked_half(&root, RawRenameMode::From, 13, "first.md"),
                    tracked_half(&root, RawRenameMode::From, 13, "second.md"),
                    tracked_half(&root, RawRenameMode::To, 13, "destination.md"),
                ],
                now,
            )
            .expect("collision is quarantined");
        assert!(events.is_empty());
        assert_eq!(worker.pending_tracked.len(), 3);
        assert!(
            worker
                .normalize_at(Vec::new(), now + Duration::from_secs(2))
                .unwrap()
                .iter()
                .all(|event| !matches!(event, WorkspaceEvent::Renamed { .. }))
        );
    }

    #[test]
    fn failed_batch_does_not_commit_known_content_or_consume_receipts() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let external_absolute = root.canonical_path().join("a-external.md");
        let internal_absolute = root.canonical_path().join("b-internal.md");
        let unreadable_absolute = root.canonical_path().join("z-directory");
        fs::write(&external_absolute, "external").expect("external content");
        fs::write(&internal_absolute, "internal").expect("internal content");
        fs::create_dir(&unreadable_absolute).expect("directory that cannot be read as a file");

        let now = Instant::now();
        let internal_path = WorkspacePath::new("b-internal.md").expect("internal path");
        let mut normalizer = Normalizer::new(root, Vec::new(), Duration::from_secs(60));
        normalizer.record_internal_write(internal_path, ContentHash::digest("internal"), now);
        let event = |path| RawEvent {
            kind: RawEventKind::Modify,
            rename_mode: None,
            tracker: None,
            paths: vec![path],
        };

        let error = normalizer
            .normalize(
                [
                    event(external_absolute.clone()),
                    event(internal_absolute.clone()),
                    event(unreadable_absolute.clone()),
                ],
                now,
            )
            .expect_err("later read failure rejects the entire batch");
        assert!(matches!(
            error,
            WatcherError::Io { path, source }
                if path == unreadable_absolute && source.kind() != io::ErrorKind::NotFound
        ));

        let retried = normalizer
            .normalize([event(external_absolute), event(internal_absolute)], now)
            .expect("retry valid prefix");
        assert!(matches!(
            retried.as_slice(),
            [WorkspaceEvent::ContentChanged { path, .. }] if path.as_str() == "a-external.md"
        ));
    }

    #[test]
    fn rename_destination_read_error_preserves_path_and_source() {
        let destination = PathBuf::from("vault/destination.md");
        let error = read_rename_destination_with(&destination, |_| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            WatcherError::Io { path, source }
                if path == destination && source.kind() == io::ErrorKind::PermissionDenied
        ));
    }

    #[test]
    fn missing_rename_destination_is_an_explicit_safe_race() {
        let destination = PathBuf::from("vault/destination.md");
        let hash = read_rename_destination_with(&destination, |_| {
            Err(io::Error::new(io::ErrorKind::NotFound, "rename raced"))
        })
        .expect("a vanished rename destination is safely ignored");

        assert_eq!(hash, None, "the race must not invent a content hash");
    }

    #[test]
    fn debouncer_error_preserves_notify_context() {
        let notify_error = notify::Error::generic("backend disconnected");
        let error = convert_debouncer_message(Err(vec![notify_error])).unwrap_err();

        assert!(matches!(&error, WatcherError::Notify { errors } if errors.len() == 1));
        assert!(error.to_string().contains("backend disconnected"));
    }

    #[test]
    fn dotfile_to_regular_file_is_a_rename() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let from_absolute = root.canonical_path().join(".draft.md");
        let to_absolute = root.canonical_path().join("draft.md");
        fs::write(&to_absolute, "draft").expect("write destination");
        let mut normalizer = Normalizer::new(root, Vec::new(), Duration::from_secs(2));

        let events = normalizer
            .normalize(
                [RawEvent {
                    kind: RawEventKind::Rename,
                    rename_mode: Some(RawRenameMode::Both),
                    tracker: None,
                    paths: vec![from_absolute, to_absolute],
                }],
                Instant::now(),
            )
            .expect("normalize rename");

        assert_eq!(
            events,
            vec![WorkspaceEvent::Renamed {
                from: WorkspacePath::new(".draft.md").expect("from path"),
                to: WorkspacePath::new("draft.md").expect("to path"),
            }]
        );
    }

    #[test]
    fn rename_ttl_expires_independently_of_receipt_ttl() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let absolute = root.canonical_path().join("note.md");
        fs::write(&absolute, "internal").expect("write");
        let path = WorkspacePath::new("note.md").expect("path");
        let hash = ContentHash::digest("internal");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(1),
            Duration::from_secs(10),
        );
        worker.process_command(WorkerCommand::RecordInternalWrite {
            path,
            hash,
            acknowledged: mpsc::sync_channel(1).0,
        });
        let now = Instant::now();

        assert!(
            worker
                .normalize_at(
                    vec![tracked_half(&root, RawRenameMode::From, 99, "old.md")],
                    now,
                )
                .expect("defer rename source")
                .is_empty()
        );
        assert!(
            worker
                .on_idle_timeout(now + Duration::from_secs(2))
                .expect("expire rename source")
                .is_empty()
        );
        assert!(worker.pending_tracked.is_empty());

        let content_event = RawEvent {
            kind: RawEventKind::Modify,
            rename_mode: None,
            tracker: None,
            paths: vec![absolute],
        };
        assert!(
            worker
                .normalize_at(vec![content_event], now + Duration::from_secs(2))
                .expect("receipt remains live")
                .is_empty()
        );
    }

    #[test]
    fn worker_receipt_command_suppresses_exactly_one_subsequent_event() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot::open(directory.path()).expect("root");
        let absolute = root.canonical_path().join("note.md");
        fs::write(&absolute, "internal").expect("write");
        let path = WorkspacePath::new("note.md").expect("path");
        let hash = ContentHash::digest("internal");
        let mut worker = WorkerState::new(
            root.clone(),
            Vec::new(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );

        worker.process_command(WorkerCommand::RecordInternalWrite {
            path,
            hash,
            acknowledged: mpsc::sync_channel(1).0,
        });
        let event = RawEvent {
            kind: RawEventKind::Modify,
            rename_mode: None,
            tracker: None,
            paths: vec![absolute],
        };

        assert!(worker.normalize(vec![event.clone()]).unwrap().is_empty());
        fs::write(root.canonical_path().join("note.md"), "external").expect("external write");
        assert_eq!(worker.normalize(vec![event]).unwrap().len(), 1);
    }
}
