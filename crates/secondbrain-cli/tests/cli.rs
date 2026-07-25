//! The `secondbrain` binary, exercised as a real process.
//!
//! Every case here spawns the actual executable against a real temporary
//! workspace and asserts on its exit code and its output. Nothing is mocked:
//! the point of this crate is that the CLI drives the same library paths the
//! desktop app, the MCP server, and the local API will drive, so a test that
//! substituted a fake for any of them would verify nothing about that claim.
//!
//! Workspace state that no command can create on its own — an interrupted
//! transaction, for instance — is built with the production libraries and then
//! handed to the binary, which is the only thing under test.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::diff::diff_documents;
use secondbrain_transaction::oplog::LocalMutationLog;
use secondbrain_transaction::{TransactionEngine, TransactionRequest, failpoint};
use secondbrain_vault::{WorkspaceRoot, load_manifest};
use serde_json::Value;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// The exit-code contract
// ---------------------------------------------------------------------------
//
// These mirror the table in README.md. An operator scripting against this
// binary branches on them, so they are asserted here rather than left to
// whatever the implementation happens to return.

/// The command completed and nothing needs attention.
const OK: i32 = 0;
/// The command could not complete.
const FAILED: i32 = 1;
/// The command line itself was invalid.
const USAGE: i32 = 2;
/// The change is ambiguous and a human must decide.
const REVIEW_REQUIRED: i32 = 3;
/// The command completed and reported workspace problems.
const DIAGNOSTICS: i32 = 4;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The note every link and search case revolves around.
const ALPHA: &str = "notes/alpha.md";

/// The note `alpha` links to.
const BETA: &str = "notes/beta.md";

const ALPHA_SOURCE: &str = "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n";
const BETA_SOURCE: &str = "# Beta\n\nBeta describes the migration timeline.\n";

fn secondbrain() -> Command {
    Command::cargo_bin("secondbrain").expect("the secondbrain binary builds")
}

/// A temporary workspace on disk, with a scratch area beside it.
struct Vault {
    directory: TempDir,
    /// Incoming files and plan files live here, deliberately outside the
    /// workspace: an operator artifact dropped into the vault would be indexed
    /// as a note, and a test that did that would be measuring its own fixture.
    scratch: TempDir,
}

impl Vault {
    /// An empty directory that no command has touched yet.
    fn empty() -> Self {
        Self {
            directory: TempDir::new().expect("temporary directory"),
            scratch: TempDir::new().expect("temporary scratch directory"),
        }
    }

    /// An initialized, indexed workspace holding two linked notes.
    fn indexed() -> Self {
        Self::indexed_with(ALPHA_SOURCE)
    }

    /// [`Vault::indexed`], with `ALPHA` holding `contents` from the start.
    ///
    /// A test that needs the workspace to have converged on something other
    /// than [`ALPHA_SOURCE`] writes it before indexing rather than over an
    /// indexed note: indexing is where the workspace takes responsibility for a
    /// note, and a write to a note it already tracks is an external edit that
    /// `reconcile`, not a rebuild, is what answers.
    fn indexed_with(contents: &str) -> Self {
        let vault = Self::empty();
        vault.write(ALPHA, contents);
        vault.write(BETA, BETA_SOURCE);
        secondbrain().args(["init", vault.arg()]).assert().success();
        secondbrain()
            .args(["index", "rebuild", vault.arg()])
            .assert()
            .success();
        vault
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    /// The workspace path as a command-line argument.
    fn arg(&self) -> &str {
        self.path().to_str().expect("temporary paths are UTF-8")
    }

    fn write(&self, relative: &str, contents: &str) {
        let target = self.path().join(relative);
        fs::create_dir_all(target.parent().expect("note has a parent")).expect("create directory");
        fs::write(&target, contents).expect("write note");
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path().join(relative)).expect("read note")
    }

    /// A path for a plan file or an incoming file, outside the workspace.
    fn scratch(&self, name: &str) -> PathBuf {
        self.scratch.path().join(name)
    }

    fn root(&self) -> WorkspaceRoot {
        WorkspaceRoot::open(self.path()).expect("open workspace root")
    }

    fn engine(&self) -> TransactionEngine {
        let manifest = load_manifest(self.path()).expect("load manifest");
        TransactionEngine::new(self.root(), manifest.workspace_id)
    }
}

/// Runs the binary and returns its exit code and parsed JSON payload.
fn run_json(arguments: &[&str]) -> (i32, Value) {
    let output = secondbrain()
        .args(arguments)
        .arg("--json")
        .output()
        .expect("run");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(
        !stdout.contains('\u{1b}'),
        "--json is a machine contract and must never carry ANSI: {stdout:?}"
    );
    let value = if stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{error}: {stdout}"))
    };
    (
        output.status.code().expect("process exited normally"),
        value,
    )
}

/// The note id the index assigned to a workspace path.
fn note_id_of(vault: &Vault, path: &str) -> NoteId {
    let (code, report) = run_json(&["note", "inspect", vault.arg(), path]);
    assert_eq!(code, OK, "{report}");
    report["note_id"]
        .as_str()
        .expect("inspect reports a note id")
        .parse()
        .expect("a canonical note id")
}

/// Every note id the workspace has filed durable state under, deduplicated and
/// in deterministic order.
///
/// A forked identity is precisely a note id nobody asked for, so the honest way
/// to say a note did not fork is to name every id the workspace knows — its
/// identity record, its converged base and its journal alike, read from the
/// record files rather than from the index that is derived from them.
fn tracked_ids(vault: &Vault) -> Vec<String> {
    let mut ids = Vec::new();
    for directory in ["identity-map", "snapshots", "oplog"] {
        let Ok(entries) = fs::read_dir(vault.path().join(".secondbrain").join(directory)) else {
            // A workspace that has journaled nothing has no oplog directory,
            // and that is a workspace with no ids filed there.
            continue;
        };
        for entry in entries {
            let path = entry.expect("directory entry").path();
            let name = path.file_stem().expect("a named entry").to_string_lossy();
            if let Ok(id) = name.parse::<NoteId>() {
                ids.push(id.to_string());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Derives a plan turning `ALPHA` into `contents`, and returns the plan file.
///
/// Both the incoming file and the plan live in the scratch directory outside
/// the workspace, for the reason [`Vault::scratch`] gives.
fn plan_for(vault: &Vault, contents: &str) -> PathBuf {
    plan_for_expecting(vault, contents, OK)
}

/// [`plan_for`], for a diff that is expected to end with `code` — the ambiguous
/// case exits [`REVIEW_REQUIRED`] and still writes the plan it could not decide.
fn plan_for_expecting(vault: &Vault, contents: &str, code: i32) -> PathBuf {
    let incoming = vault.scratch("incoming.md");
    fs::write(&incoming, contents).expect("write incoming file");
    let plan = vault.scratch("plan.json");
    secondbrain()
        .args([
            "diff",
            vault.arg(),
            ALPHA,
            incoming.to_str().expect("UTF-8"),
            "--out",
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .code(code);
    plan
}

/// Journals an edit to `ALPHA` and dies at `boundary`, leaving the workspace in
/// whatever durable state that boundary produces.
fn interrupted_edit(vault: &Vault, incoming: &str, boundary: &str) -> TransactionId {
    let note_id = note_id_of(vault, ALPHA);
    let base = vault.read(ALPHA);
    let operations = diff_documents(
        &SourceDocument::parse(&base).expect("parse base"),
        &SourceDocument::parse(incoming).expect("parse incoming"),
    );
    assert!(
        !operations.is_empty(),
        "the fixture edit must change something"
    );
    let transaction_id = TransactionId::new();
    let request = TransactionRequest {
        id: transaction_id,
        actor: ActorId::new("alice").expect("actor"),
        device: DeviceId::new("laptop").expect("device"),
        note_id,
        path: WorkspacePath::new(ALPHA).expect("workspace path"),
        expected_hash: ContentHash::digest(base.as_bytes()),
        expected_version: NoteVersion::new(0),
        operations,
    };

    failpoint::set(Some(boundary));
    let error = vault.engine().commit(request);
    failpoint::set(None);
    error.expect_err("the failpoint interrupts the commit");
    transaction_id
}

/// Journals an edit to `ALPHA` and then damages the tail of its journal, which
/// is what a torn write at a power failure leaves behind.
fn interrupted_edit_with_a_damaged_journal(vault: &Vault) -> TransactionId {
    let transaction_id = interrupted_edit(
        vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
        "after_append_before_state",
    );
    let note_directory = fs::read_dir(vault.path().join(".secondbrain/oplog"))
        .expect("the journal directory exists")
        .next()
        .expect("one note was journaled")
        .expect("read the entry")
        .path();
    fs::OpenOptions::new()
        .append(true)
        .open(note_directory.join("local-mutations.log"))
        .expect("open the journal")
        .write_all(b"truncated")
        .expect("append bytes that are not a record");
    transaction_id
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

#[test]
fn help_lists_every_command_without_ansi() {
    let assertion = secondbrain().arg("--help").assert().success();
    let help = String::from_utf8(assertion.get_output().stdout.clone()).expect("UTF-8 help");

    for command in [
        "init",
        "validate",
        "index",
        "search",
        "note",
        "diff",
        "transaction",
        "recovery",
        "reconcile",
        "doctor",
    ] {
        assert!(help.contains(command), "help omits `{command}`: {help}");
    }
    assert!(
        !help.contains('\u{1b}'),
        "help must carry no ANSI when stdout is not a TTY: {help:?}"
    );
}

#[test]
fn an_unrecognized_command_exits_with_the_usage_code() {
    secondbrain()
        .arg("summon")
        .assert()
        .code(USAGE)
        .stderr(predicate::str::contains('\u{1b}').not());
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

#[test]
fn init_reports_the_workspace_identity_and_is_idempotent() {
    let vault = Vault::empty();

    let (code, first) = run_json(&["init", vault.arg()]);
    assert_eq!(code, OK, "{first}");
    assert_eq!(first["format_version"], 1);
    assert!(
        vault.path().join(".secondbrain/manifest.toml").exists(),
        "init must create the manifest"
    );

    let (code, second) = run_json(&["init", vault.arg()]);
    assert_eq!(code, OK, "{second}");
    assert_eq!(
        first["workspace_id"], second["workspace_id"],
        "re-initializing must not mint a second identity"
    );
}

#[test]
fn init_of_a_missing_directory_fails() {
    let vault = Vault::empty();
    let missing = vault.scratch("nowhere");

    secondbrain()
        .args(["init", missing.to_str().expect("UTF-8")])
        .assert()
        .code(FAILED);
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

#[test]
fn validate_accepts_a_healthy_workspace() {
    let vault = Vault::indexed();

    let (code, report) = run_json(&["validate", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["notes_checked"], 2);
    assert_eq!(report["problems"].as_array().expect("problems").len(), 0);
}

#[test]
fn validate_reports_notes_that_declare_the_same_identity() {
    let vault = Vault::indexed();
    let shared = NoteId::new();
    vault.write(ALPHA, &format!("---\nid: {shared}\n---\n\n# Alpha\n"));
    vault.write(BETA, &format!("---\nid: {shared}\n---\n\n# Beta\n"));

    let (code, report) = run_json(&["validate", vault.arg()]);

    assert_eq!(code, DIAGNOSTICS, "{report}");
    let problems = report["problems"].as_array().expect("problems");
    assert!(
        problems
            .iter()
            .any(|problem| problem["code"] == "SB-NOTE-DUPLICATE-ID"),
        "{report}"
    );
}

#[test]
fn validate_of_an_uninitialized_directory_fails() {
    let vault = Vault::empty();

    secondbrain()
        .args(["validate", vault.arg()])
        .assert()
        .code(FAILED);
}

// ---------------------------------------------------------------------------
// index rebuild and search
// ---------------------------------------------------------------------------

#[test]
fn index_rebuild_reports_what_it_indexed() {
    let vault = Vault::empty();
    vault.write(ALPHA, ALPHA_SOURCE);
    vault.write(BETA, BETA_SOURCE);
    secondbrain().args(["init", vault.arg()]).assert().success();

    let (code, report) = run_json(&["index", "rebuild", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["indexed"], 2);
    assert_eq!(report["broken_links"], 0);
}

/// A note renamed outside the workspace keeps its identity across a rebuild.
///
/// A rename and a copy are the same bytes at a path no record claims, so the
/// content alone cannot tell them apart. Guessing "copy" mints a second
/// identity for one note and leaves the first record claiming a path nothing
/// occupies — and the note's backlinks, converged base and journal all stay
/// with the dead identity, so the file the operator can still open becomes a
/// stranger to its own history.
///
/// The move keeps the file stem, so `[[beta]]` still names this note: what is
/// under test is the identity of the file, not link rewriting.
#[test]
fn a_rebuild_after_an_external_rename_moves_the_note_rather_than_forking_it() {
    let vault = Vault::indexed();
    let beta = note_id_of(&vault, BETA);
    let known_before = tracked_ids(&vault);
    let moved = "archive/beta.md";
    fs::create_dir_all(vault.path().join("archive")).expect("create directory");
    fs::rename(vault.path().join(BETA), vault.path().join(moved))
        .expect("an external tool renames the note");

    let (code, report) = run_json(&["index", "rebuild", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["indexed"], 2, "{report}");
    assert_eq!(
        tracked_ids(&vault),
        known_before,
        "one note that moved is one note: a rebuild may not mint a second \
         identity, nor file a base or a journal under one"
    );

    let (code, inspected) = run_json(&["note", "inspect", vault.arg(), moved]);
    assert_eq!(code, OK, "{inspected}");
    assert_eq!(
        inspected["note_id"],
        beta.to_string(),
        "the identity has to follow the bytes: {inspected}"
    );
    let backlinks = inspected["backlinks"].as_array().expect("backlinks");
    assert_eq!(backlinks.len(), 1, "{inspected}");
    assert_eq!(
        backlinks[0]["path"], ALPHA,
        "the link that pointed at this note must still arrive — losing it is \
         what a forked identity costs an operator: {inspected}"
    );

    // The record now describes where the file actually is.
    let root = vault.root();
    let map = secondbrain_vault::IdentityMap::open(&root).expect("identity map");
    let record = map
        .lookup(&beta)
        .expect("look the record up")
        .expect("the note kept its record");
    assert_eq!(record.current_path.as_str(), moved, "{record:?}");
    assert!(
        record
            .historical_paths
            .contains(&WorkspacePath::new(BETA).expect("path")),
        "the path the note came from is history, not a claim: {record:?}"
    );
    assert_eq!(
        map.note_at(&WorkspacePath::new(BETA).expect("path")),
        None,
        "no record may go on claiming a path that holds no file"
    );

    // The derived state the identity anchors came with it.
    let base = secondbrain_vault::BaseSnapshotStore::new(&root)
        .load(beta)
        .expect("load the converged base")
        .expect("the note kept the base it converged at");
    assert_eq!(base.source, BETA_SOURCE, "{base:?}");

    // And not one byte of a note was rewritten to achieve any of it.
    assert_eq!(
        (vault.read(moved), vault.read(ALPHA)),
        (BETA_SOURCE.to_owned(), ALPHA_SOURCE.to_owned()),
        "recovering an identity is workspace state; it may not touch a note"
    );
}

/// A copy is still a copy: the second file is a second note.
///
/// This is the other half of the rename rule and the reason it needs the
/// workspace scan. The evidence is identical — same fingerprint, same hash, a
/// path no record claims — and only the original file still being on disk says
/// this is a fork rather than a move.
#[test]
fn a_rebuild_after_an_external_copy_gives_the_copy_its_own_identity() {
    let vault = Vault::indexed();
    let beta = note_id_of(&vault, BETA);
    let copied = "archive/beta.md";
    fs::create_dir_all(vault.path().join("archive")).expect("create directory");
    fs::copy(vault.path().join(BETA), vault.path().join(copied))
        .expect("an external tool copies the note");

    let (code, report) = run_json(&["index", "rebuild", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["indexed"], 3, "{report}");
    assert_eq!(
        note_id_of(&vault, BETA),
        beta,
        "the file that never moved keeps the identity it had"
    );
    let duplicate = note_id_of(&vault, copied);
    assert_ne!(
        duplicate, beta,
        "two files that both exist are two notes, however alike their bytes"
    );

    let root = vault.root();
    let map = secondbrain_vault::IdentityMap::open(&root).expect("identity map");
    assert_eq!(
        map.note_at(&WorkspacePath::new(BETA).expect("path")),
        Some(beta),
        "the original's record must still claim the path it still occupies"
    );
    assert_eq!(
        map.note_at(&WorkspacePath::new(copied).expect("path")),
        Some(duplicate)
    );
    assert_eq!(
        (vault.read(BETA), vault.read(copied)),
        (BETA_SOURCE.to_owned(), BETA_SOURCE.to_owned()),
        "indexing a copy may not touch either file"
    );
}

#[test]
fn search_finds_a_note_by_its_text() {
    let vault = Vault::indexed();

    let (code, report) = run_json(&["search", vault.arg(), "migration"]);

    assert_eq!(code, OK, "{report}");
    let hits = report["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 1, "{report}");
    assert_eq!(hits[0]["path"], BETA);
}

#[test]
fn search_without_an_index_says_so_rather_than_reporting_nothing_found() {
    let vault = Vault::empty();
    secondbrain().args(["init", vault.arg()]).assert().success();

    secondbrain()
        .args(["search", vault.arg(), "migration"])
        .assert()
        .code(FAILED)
        .stderr(predicate::str::contains("index rebuild"));
}

// ---------------------------------------------------------------------------
// note inspect
// ---------------------------------------------------------------------------

#[test]
fn note_inspect_reports_identity_and_both_link_directions() {
    let vault = Vault::indexed();

    let (code, alpha) = run_json(&["note", "inspect", vault.arg(), ALPHA]);
    assert_eq!(code, OK, "{alpha}");
    assert_eq!(alpha["path"], ALPHA);
    assert_eq!(alpha["title"], "Alpha");
    let outgoing = alpha["outgoing_links"].as_array().expect("outgoing links");
    assert_eq!(outgoing.len(), 1, "{alpha}");
    assert_eq!(outgoing[0]["target"], "beta");
    assert_eq!(outgoing[0]["path"], BETA);

    let (code, beta) = run_json(&["note", "inspect", vault.arg(), BETA]);
    assert_eq!(code, OK, "{beta}");
    let backlinks = beta["backlinks"].as_array().expect("backlinks");
    assert_eq!(backlinks.len(), 1, "{beta}");
    assert_eq!(backlinks[0]["path"], ALPHA);
}

#[test]
fn note_inspect_of_an_unindexed_path_fails() {
    let vault = Vault::indexed();

    secondbrain()
        .args(["note", "inspect", vault.arg(), "notes/absent.md"])
        .assert()
        .code(FAILED);
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[test]
fn diff_writes_a_plan_and_leaves_the_note_exactly_as_it_was() {
    let vault = Vault::indexed();
    let incoming = vault.scratch("incoming.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    )
    .expect("write");
    let plan = vault.scratch("plan.json");

    let (code, report) = run_json(&[
        "diff",
        vault.arg(),
        ALPHA,
        incoming.to_str().expect("UTF-8"),
        "--out",
        plan.to_str().expect("UTF-8"),
    ]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(
        vault.read(ALPHA),
        ALPHA_SOURCE,
        "diff previews a change; it never performs one"
    );
    let written: Value =
        serde_json::from_str(&fs::read_to_string(&plan).expect("read plan")).expect("plan is JSON");
    assert_eq!(written["format"], "sb-transaction-plan-v1");
    assert_eq!(written["path"], ALPHA);
    assert_eq!(written["review_required"], false);
    assert!(
        !written["operations"]
            .as_array()
            .expect("operations")
            .is_empty(),
        "{written}"
    );
}

#[test]
fn diff_prints_the_plan_when_no_output_file_is_named() {
    let vault = Vault::indexed();
    let incoming = vault.scratch("incoming.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    )
    .expect("write");

    let (code, plan) = run_json(&[
        "diff",
        vault.arg(),
        ALPHA,
        incoming.to_str().expect("UTF-8"),
    ]);

    assert_eq!(code, OK, "{plan}");
    assert_eq!(plan["format"], "sb-transaction-plan-v1");
    assert_eq!(plan["expected_version"], 0);
}

#[test]
fn diff_of_an_ambiguous_change_requires_review() {
    // The workspace converged on two identical paragraphs, which is the state
    // in which a change to one of them cannot be attributed to either without
    // a person saying which.
    let vault = Vault::indexed_with("Duplicate.\n\nSome middle.\n\nDuplicate.\n");
    let incoming = vault.scratch("incoming.md");
    fs::write(&incoming, "Duplicate.\n\nSome middle.\n\nChanged.\n").expect("write");

    let (code, plan) = run_json(&[
        "diff",
        vault.arg(),
        ALPHA,
        incoming.to_str().expect("UTF-8"),
    ]);

    assert_eq!(
        code, REVIEW_REQUIRED,
        "an operator must be able to tell `needs a human` from `broke`: {plan}"
    );
    assert_eq!(plan["review_required"], true);
}

#[test]
fn diff_refuses_a_note_whose_file_diverged_from_its_converged_base() {
    let vault = Vault::indexed();
    let plan = plan_for(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );
    secondbrain()
        .args([
            "transaction",
            "apply",
            vault.arg(),
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .success();

    // An external editor saved over the note. That edit is a semantic
    // operation the journal has never seen, and the converged base still
    // describes the state before it.
    let external = "# Alpha\n\nAlpha links to [[beta]] after the retro.\n";
    vault.write(ALPHA, external);

    let incoming = vault.scratch("later.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] after the retro, on Tuesday.\n",
    )
    .expect("write");
    let out = vault.scratch("later-plan.json");
    let output = secondbrain()
        .args([
            "diff",
            vault.arg(),
            ALPHA,
            incoming.to_str().expect("UTF-8"),
            "--out",
            out.to_str().expect("UTF-8"),
        ])
        .output()
        .expect("run");

    assert_eq!(
        output.status.code(),
        Some(REVIEW_REQUIRED),
        "planning over an unjournaled external edit would bump the version from a \
         base the file no longer holds, and the oplog could never replay to it: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !out.exists(),
        "a refused diff must not leave a plan an operator could apply"
    );
    assert_eq!(
        vault.read(ALPHA),
        external,
        "diff previews a change; it never performs one"
    );
    let error: Value = serde_json::from_slice(
        &secondbrain()
            .args([
                "diff",
                vault.arg(),
                ALPHA,
                incoming.to_str().expect("UTF-8"),
                "--json",
            ])
            .output()
            .expect("run")
            .stderr,
    )
    .expect("the error is JSON too");
    assert!(
        error["error"]["code"]
            .as_str()
            .expect("code")
            .starts_with("SB-"),
        "{error}"
    );
}

#[test]
fn diff_of_a_converged_note_plans_against_its_recorded_base() {
    let vault = Vault::indexed();
    let applied = "# Alpha\n\nAlpha links to [[beta]] for the launch.\n";
    let plan = plan_for(&vault, applied);
    secondbrain()
        .args([
            "transaction",
            "apply",
            vault.arg(),
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .success();
    assert_eq!(vault.read(ALPHA), applied);

    // Nothing touched the file behind the workspace's back, so the recorded
    // base and the file agree and the next change plans normally.
    let incoming = vault.scratch("later.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] for the launch on Tuesday.\n",
    )
    .expect("write");

    let (code, next) = run_json(&[
        "diff",
        vault.arg(),
        ALPHA,
        incoming.to_str().expect("UTF-8"),
    ]);

    assert_eq!(code, OK, "{next}");
    assert_eq!(
        next["expected_version"], 1,
        "the plan must continue from the version the base records: {next}"
    );
    assert!(
        !next["operations"]
            .as_array()
            .expect("operations")
            .is_empty(),
        "{next}"
    );
}

// ---------------------------------------------------------------------------
// transaction apply
// ---------------------------------------------------------------------------

#[test]
fn transaction_apply_materializes_the_plan_and_refreshes_the_index() {
    let vault = Vault::indexed();
    let updated = "# Alpha\n\nAlpha links to [[beta]] for the launch.\n";
    let plan = plan_for(&vault, updated);

    let (code, report) = run_json(&[
        "transaction",
        "apply",
        vault.arg(),
        plan.to_str().expect("UTF-8"),
    ]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["changed"], true);
    assert_eq!(report["version"], 1);
    assert_eq!(vault.read(ALPHA), updated);

    // The index is derived state the CLI owns keeping current, so the word
    // that only exists in the new content must now be findable.
    let (code, search) = run_json(&["search", vault.arg(), "launch"]);
    assert_eq!(code, OK, "{search}");
    assert_eq!(
        search["hits"].as_array().expect("hits").len(),
        1,
        "{search}"
    );

    // And the marker must say the index repair happened, because it did.
    let (code, doctor) = run_json(&["doctor", vault.arg()]);
    assert_eq!(code, OK, "{doctor}");
    assert_eq!(doctor["transactions"]["index_repairs_outstanding"], 0);
}

#[test]
fn transaction_apply_refuses_a_plan_whose_precondition_moved() {
    let vault = Vault::indexed();
    let plan = plan_for(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );

    // Somebody else saved the note between the preview and the apply.
    vault.write(ALPHA, "# Alpha\n\nSomeone else got here first.\n");

    secondbrain()
        .args([
            "transaction",
            "apply",
            vault.arg(),
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .code(FAILED);
    assert_eq!(
        vault.read(ALPHA),
        "# Alpha\n\nSomeone else got here first.\n",
        "a refused apply must not have written anything"
    );
}

#[test]
fn transaction_apply_refuses_a_plan_that_needs_review() {
    let vault = Vault::indexed_with("Duplicate.\n\nSome middle.\n\nDuplicate.\n");
    let plan = plan_for_expecting(
        &vault,
        "Duplicate.\n\nSome middle.\n\nChanged.\n",
        REVIEW_REQUIRED,
    );

    secondbrain()
        .args([
            "transaction",
            "apply",
            vault.arg(),
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .code(REVIEW_REQUIRED);
    assert_eq!(
        vault.read(ALPHA),
        "Duplicate.\n\nSome middle.\n\nDuplicate.\n"
    );
}

#[test]
fn transaction_apply_names_the_format_of_a_plan_it_cannot_read() {
    let vault = Vault::indexed();
    let plan = plan_for(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );

    // A plan from a newer binary: a format this one has never heard of, and a
    // required field to match. The format tag is the thing that answers this,
    // and it must answer before anything else in the file is parsed.
    let mut newer: Value =
        serde_json::from_str(&fs::read_to_string(&plan).expect("read plan")).expect("plan is JSON");
    let object = newer.as_object_mut().expect("a plan is an object");
    object.insert("format".into(), Value::from("sb-transaction-plan-v2"));
    object.remove("expected_version");
    object.insert("converged_at".into(), Value::from(7));
    fs::write(&plan, serde_json::to_string_pretty(&newer).expect("encode")).expect("write");

    let output = secondbrain()
        .args([
            "transaction",
            "apply",
            vault.arg(),
            plan.to_str().expect("UTF-8"),
            "--json",
        ])
        .output()
        .expect("run");

    assert_eq!(output.status.code(), Some(FAILED));
    let error: Value = serde_json::from_slice(&output.stderr).expect("the error is JSON too");
    assert_eq!(
        error["error"]["code"], "SB-PLAN-INVALID",
        "a newer plan format is not store corruption: {error}"
    );
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("sb-transaction-plan-v2"),
        "the operator must be told which format they handed over: {error}"
    );
    assert_eq!(
        vault.read(ALPHA),
        ALPHA_SOURCE,
        "nothing may have been written"
    );
}

#[test]
fn transaction_apply_refuses_a_plan_from_another_workspace() {
    let vault = Vault::indexed();
    let other = Vault::indexed();
    let plan = plan_for(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );

    secondbrain()
        .args([
            "transaction",
            "apply",
            other.arg(),
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .code(FAILED);
}

// ---------------------------------------------------------------------------
// recovery check
// ---------------------------------------------------------------------------

#[test]
fn recovery_check_finishes_a_committed_transaction_and_refreshes_its_index() {
    let vault = Vault::indexed();
    let updated = "# Alpha\n\nAlpha links to [[beta]] for the launch.\n";
    let transaction_id = interrupted_edit(&vault, updated, "after_commit_before_index");
    assert_eq!(
        vault.read(ALPHA),
        updated,
        "the boundary is after the Markdown write"
    );

    let (code, report) = run_json(&["recovery", "check", vault.arg()]);

    assert_eq!(
        code, OK,
        "an index repair is ordinary work, not a problem: {report}"
    );
    let actions = report["actions"].as_array().expect("actions");
    assert!(
        actions
            .iter()
            .any(|action| action["action"] == "index_repair"
                && action["transaction_id"] == transaction_id.to_string()),
        "a committed transaction whose index was never repaired must be surfaced: {report}"
    );
    assert_eq!(report["index_refreshed"], true);

    // The repair is the point: the index must now hold the recovered content.
    let (code, search) = run_json(&["search", vault.arg(), "launch"]);
    assert_eq!(code, OK, "{search}");
    assert_eq!(
        search["hits"].as_array().expect("hits").len(),
        1,
        "{search}"
    );
}

#[test]
fn recovery_check_reports_an_edit_it_had_to_abandon() {
    let vault = Vault::indexed();
    let internal = "# Alpha\n\nAlpha links to [[beta]] for the launch.\n";
    let transaction_id = interrupted_edit(&vault, internal, "after_operations_durable");

    // An external editor saved over the note while the workspace was down, so
    // the journaled operations can no longer be placed.
    vault.write(ALPHA, "# Alpha\n\nSomething else entirely.\n");

    let (code, report) = run_json(&["recovery", "check", vault.arg()]);

    assert_eq!(
        code, DIAGNOSTICS,
        "a durably journaled edit was lost; that is not a success: {report}"
    );
    let abandoned = report["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .find(|action| action["action"] == "abandoned")
        .unwrap_or_else(|| panic!("the abandoned edit must be reported: {report}"));
    assert_eq!(abandoned["transaction_id"], transaction_id.to_string());
    assert_eq!(abandoned["path"], ALPHA);
    // `AbandonedReason` has a `Display` impl written for this command.
    let explanation = abandoned["explanation"].as_str().expect("explanation");
    assert!(
        explanation.contains("the file on disk is untouched")
            && explanation.contains("edit is lost"),
        "an operator must be told what this means for their data: {explanation}"
    );
    assert_eq!(
        vault.read(ALPHA),
        "# Alpha\n\nSomething else entirely.\n",
        "recovery preserves the file it cannot place an edit into"
    );
}

#[test]
fn recovery_check_that_could_not_rebuild_leaves_the_repair_still_owed() {
    let vault = Vault::indexed();
    let updated = "# Alpha\n\nAlpha links to [[beta]] for the launch.\n";
    interrupted_edit(&vault, updated, "after_commit_before_index");

    // Two notes declaring one identity: the rebuild this command has to perform
    // will refuse, so the repair recovery asked for never happens.
    let shared = NoteId::new();
    vault.write(
        "notes/dup-one.md",
        &format!("---\nid: {shared}\n---\n\n# One\n"),
    );
    vault.write(
        "notes/dup-two.md",
        &format!("---\nid: {shared}\n---\n\n# Two\n"),
    );

    secondbrain()
        .args(["recovery", "check", vault.arg()])
        .assert()
        .code(FAILED);

    // The marker must still owe the repair. Recording it here would be the
    // marker lying: a later `recovery check` would find nothing to do and
    // `doctor` would report a clean workspace over a stale index.
    let (code, doctor) = run_json(&["doctor", vault.arg()]);
    assert_eq!(code, DIAGNOSTICS, "{doctor}");
    assert_eq!(
        doctor["transactions"]["index_repairs_outstanding"], 1,
        "a repair that never ran must still be outstanding: {doctor}"
    );

    // And once the obstruction is gone, the repair is still there to be done.
    fs::remove_file(vault.path().join("notes/dup-one.md")).expect("remove");
    fs::remove_file(vault.path().join("notes/dup-two.md")).expect("remove");
    let (code, report) = run_json(&["recovery", "check", vault.arg()]);
    assert_eq!(code, OK, "{report}");
    assert!(
        report["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .any(|action| action["action"] == "index_repair"),
        "the repair recovery could not finish must still be asked for: {report}"
    );
    let (code, search) = run_json(&["search", vault.arg(), "launch"]);
    assert_eq!(code, OK, "{search}");
    assert_eq!(
        search["hits"].as_array().expect("hits").len(),
        1,
        "{search}"
    );
}

#[test]
fn recovery_check_reports_a_journal_suffix_it_had_to_quarantine() {
    let vault = Vault::indexed();
    let transaction_id = interrupted_edit_with_a_damaged_journal(&vault);

    let (code, report) = run_json(&["recovery", "check", vault.arg()]);

    assert_eq!(
        code, DIAGNOSTICS,
        "records set aside as damaged are not a success: {report}"
    );
    let quarantined = report["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .find(|action| action["action"] == "quarantined")
        .unwrap_or_else(|| panic!("the quarantine must be reported: {report}"));
    assert_eq!(quarantined["transaction_id"], transaction_id.to_string());
    assert_eq!(
        quarantined["path"], ALPHA,
        "every action answers `which note` the same way: {report}"
    );
    assert_eq!(report["quarantined"], 1, "{report}");

    let preserved = quarantined["quarantine_path"]
        .as_str()
        .expect("a quarantine names where the bytes went");
    assert!(
        Path::new(preserved).exists(),
        "the damaged bytes must actually be preserved: {preserved}"
    );
    assert!(
        Path::new(preserved)
            .components()
            .any(|component| component.as_os_str() == ".secondbrain"),
        "a preserved journal suffix is not a note path: {preserved}"
    );
    assert_eq!(
        vault.read(ALPHA),
        ALPHA_SOURCE,
        "a quarantine sets records aside; it never writes Markdown"
    );
}

#[test]
fn recovery_check_tells_a_human_where_the_quarantined_records_went() {
    let vault = Vault::indexed();
    interrupted_edit_with_a_damaged_journal(&vault);

    let assertion = secondbrain()
        .args(["recovery", "check", vault.arg()])
        .assert()
        .code(DIAGNOSTICS);
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("UTF-8");

    // Naming the file is the whole point of preserving it: an operator who is
    // not told the path cannot investigate what was lost.
    assert!(
        stdout.contains("quarantined notes/alpha.md"),
        "the quarantine must name the note: {stdout}"
    );
    let (_, located) = stdout
        .split_once("journal suffix preserved at ")
        .unwrap_or_else(|| panic!("the operator must be told where the bytes went: {stdout}"));
    let preserved = located.lines().next().expect("a path on its own line");
    assert!(
        Path::new(preserved.trim()).exists(),
        "the path printed to a human must be the path on disk: {preserved}"
    );
    assert!(
        stdout.contains("journal set aside"),
        "the summary must say a person is needed: {stdout}"
    );
}

#[test]
fn recovery_check_of_a_clean_workspace_reports_no_actions() {
    let vault = Vault::indexed();

    let (code, report) = run_json(&["recovery", "check", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["actions"].as_array().expect("actions").len(), 0);
    assert_eq!(report["index_refreshed"], false);
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

#[test]
fn doctor_reports_a_healthy_workspace() {
    let vault = Vault::indexed();

    let (code, report) = run_json(&["doctor", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["index"]["present"], true);
    assert_eq!(report["index"]["notes"], 2);
    assert_eq!(report["index"]["broken_links"], 0);
    assert_eq!(report["transactions"]["pending"], 0);
    assert_eq!(report["reviews_pending"], 0);
    assert_eq!(report["problems"].as_array().expect("problems").len(), 0);
}

#[test]
fn doctor_reports_a_transaction_still_awaiting_recovery() {
    let vault = Vault::indexed();
    interrupted_edit(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
        "after_operations_durable",
    );

    let (code, report) = run_json(&["doctor", vault.arg()]);

    assert_eq!(code, DIAGNOSTICS, "{report}");
    assert_eq!(report["transactions"]["pending"], 1);
    assert!(
        report["problems"]
            .as_array()
            .expect("problems")
            .iter()
            .any(|problem| problem["code"] == "SB-TXN-STATE"),
        "{report}"
    );
}

#[test]
fn doctor_reports_a_change_that_is_waiting_on_a_person() {
    let vault = Vault::indexed();
    // A descriptor is filed when a change cannot be integrated without someone
    // deciding what it meant. Its path comes from the library so this fixture
    // cannot drift from the convention the library reads back.
    let descriptor =
        secondbrain_transaction::paths::review_descriptor_path(vault.path(), TransactionId::new());
    fs::create_dir_all(descriptor.parent().expect("a parent directory"))
        .expect("create the transaction directory");
    fs::write(&descriptor, "{}").expect("file the descriptor");

    let (code, report) = run_json(&["doctor", vault.arg()]);

    assert_eq!(code, DIAGNOSTICS, "{report}");
    assert_eq!(report["reviews_pending"], 1, "{report}");
    assert!(
        report["problems"]
            .as_array()
            .expect("problems")
            .iter()
            .any(|problem| problem["code"] == "SB-REVIEW-REQUIRED"),
        "a stale precondition is one cause of a review, not the category: {report}"
    );
    assert_eq!(
        report["transactions"]["total"], 0,
        "a descriptor is not a transaction marker: {report}"
    );
}

#[test]
fn doctor_reports_a_workspace_that_was_never_initialized() {
    let vault = Vault::empty();

    secondbrain()
        .args(["doctor", vault.arg()])
        .assert()
        .code(FAILED);
}

// ---------------------------------------------------------------------------
// reconcile
// ---------------------------------------------------------------------------

/// Applies `contents` to `ALPHA` through the diff/apply pair.
///
/// This is how a test moves a tracked note on from the base indexing recorded
/// for it. Writing the file directly is what an editor outside the workspace
/// does, and the product treats it as exactly that — which is the condition
/// these cases put the note into deliberately, one step later.
fn converge_alpha(vault: &Vault, contents: &str) {
    let plan = plan_for(vault, contents);
    secondbrain()
        .args([
            "transaction",
            "apply",
            vault.arg(),
            plan.to_str().expect("UTF-8"),
        ])
        .assert()
        .success();
}

/// Every review descriptor filed in a workspace, in deterministic order.
///
/// The rule telling a descriptor from a transaction marker is production code's
/// to state, so this asks rather than restating `<id>.conflict.json`.
fn review_descriptors(vault: &Vault) -> Vec<PathBuf> {
    let directory = vault.path().join(".secondbrain/transactions");
    let mut filed = Vec::new();
    let Ok(entries) = fs::read_dir(&directory) else {
        return filed;
    };
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if secondbrain_transaction::paths::is_review_descriptor(&path) {
            filed.push(path);
        }
    }
    filed.sort();
    filed
}

/// The per-note entry `reconcile` reported for one workspace path.
fn reconciled<'a>(report: &'a Value, path: &str) -> &'a Value {
    report["notes"]
        .as_array()
        .expect("reconcile reports one entry per tracked note")
        .iter()
        .find(|note| note["path"] == path)
        .unwrap_or_else(|| panic!("no entry for {path}: {report}"))
}

#[test]
fn reconcile_journals_an_external_edit_and_makes_the_note_diffable_again() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );

    // An editor outside the workspace saves over the note. Nothing has
    // journaled that edit, so the converged base still describes the state
    // before it and `diff` refuses to plan across the gap.
    let external = "# Alpha\n\nAlpha links to [[beta]] after the retro.\n";
    vault.write(ALPHA, external);
    let incoming = vault.scratch("later.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] after the retro, on Tuesday.\n",
    )
    .expect("write incoming file");
    secondbrain()
        .args([
            "diff",
            vault.arg(),
            ALPHA,
            incoming.to_str().expect("UTF-8"),
        ])
        .assert()
        .code(REVIEW_REQUIRED);

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    let alpha = reconciled(&report, ALPHA);
    assert_eq!(alpha["outcome"], "adopted", "{report}");
    assert_eq!(alpha["version"], 2, "{report}");
    assert!(
        alpha["transaction_id"].as_str().is_some(),
        "an adopted edit is a transaction an operator can look up: {report}"
    );
    assert_eq!(report["adopted"], 1, "{report}");
    assert_eq!(report["index_refreshed"], true, "{report}");
    assert_eq!(
        vault.read(ALPHA),
        external,
        "reconcile journals what the editor wrote; it never rewrites the file"
    );

    // The edit is attributed and journaled, so the note can be planned against
    // again — which is the remedy this command exists to provide.
    let (code, plan) = run_json(&[
        "diff",
        vault.arg(),
        ALPHA,
        incoming.to_str().expect("UTF-8"),
    ]);
    assert_eq!(code, OK, "{plan}");
    assert_eq!(plan["expected_version"], 2, "{plan}");

    // And the derived index describes what is on disk now.
    let (code, hits) = run_json(&["search", vault.arg(), "retro"]);
    assert_eq!(code, OK, "{hits}");
    assert_eq!(hits["hits"].as_array().expect("hits").len(), 1, "{hits}");
}

#[test]
fn reconcile_leaves_a_converged_note_exactly_as_it_is() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );
    let before = vault.read(ALPHA);

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(
        reconciled(&report, ALPHA)["outcome"],
        "unchanged",
        "{report}"
    );
    assert_eq!(report["adopted"], 0, "{report}");
    assert_eq!(
        report["index_refreshed"], false,
        "a pass that changed nothing must not claim to have rebuilt anything: {report}"
    );
    assert_eq!(
        vault.read(ALPHA),
        before,
        "an unchanged note is never normalized or rewritten"
    );

    let (code, doctor) = run_json(&["doctor", vault.arg()]);
    assert_eq!(code, OK, "{doctor}");
    assert_eq!(
        doctor["transactions"]["total"], 1,
        "reconciling an unchanged note must journal nothing: {doctor}"
    );
}

#[test]
fn reconcile_merges_an_edit_the_external_write_would_have_clobbered() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nSecond paragraph.\n",
    );
    // A workspace transaction journaled a change to the first paragraph and
    // died before it could write it.
    interrupted_edit(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n\nSecond paragraph.\n",
        "after_operations_durable",
    );
    // The external editor then saved its own change to the second paragraph
    // over the same file, clobbering the journaled one.
    vault.write(
        ALPHA,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nSecond paragraph, revised.\n",
    );

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(reconciled(&report, ALPHA)["outcome"], "merged", "{report}");
    assert_eq!(report["merged"], 1, "{report}");
    assert_eq!(
        vault.read(ALPHA),
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n\nSecond paragraph, revised.\n",
        "both changes must survive the reconciliation"
    );

    let (code, recovery) = run_json(&["recovery", "check", vault.arg()]);
    assert_eq!(code, OK, "{recovery}");
    assert!(
        recovery["actions"].as_array().expect("actions").is_empty(),
        "a merged edit leaves recovery nothing to replay over the merged file: {recovery}"
    );
}

#[test]
fn reconcile_reports_an_absent_note_and_stops_the_index_describing_it() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );
    let converged = vault.read(ALPHA);
    fs::remove_file(vault.path().join(ALPHA)).expect("an external editor deletes the note");

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    let alpha = reconciled(&report, ALPHA);
    assert_eq!(alpha["outcome"], "absent", "{report}");
    assert!(
        alpha["note_id"].as_str().is_some(),
        "an operator must be told which note the vanished file was: {report}"
    );
    assert_eq!(report["absent"], 1, "{report}");

    let (code, hits) = run_json(&["search", vault.arg(), "launch"]);
    assert_eq!(code, OK, "{hits}");
    assert!(
        hits["hits"].as_array().expect("hits").is_empty(),
        "a note whose file is gone must stop being findable: {hits}"
    );

    // Identity and converged base are kept, so a file that comes back is
    // recognized as the same note rather than observed for the first time.
    vault.write(ALPHA, &converged);
    let (code, report) = run_json(&["reconcile", vault.arg()]);
    assert_eq!(code, OK, "{report}");
    assert_eq!(
        reconciled(&report, ALPHA)["outcome"],
        "unchanged",
        "the converged base must survive the deletion: {report}"
    );
}

#[test]
fn reconcile_of_an_ambiguous_edit_files_it_for_review_rather_than_guessing() {
    let vault = Vault::indexed();
    // Converged in two steps, because the base has to end up holding two
    // identical paragraphs: that is the state in which a change to one of them
    // cannot be attributed to either without a person saying which.
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nRepeated.\n",
    );
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nRepeated.\n\nRepeated.\n",
    );
    let external = "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nRepeated.\n\nChanged.\n";
    vault.write(ALPHA, external);

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(
        code, REVIEW_REQUIRED,
        "an operator must be able to tell `needs a human` from `broke`: {report}"
    );
    let alpha = reconciled(&report, ALPHA);
    assert_eq!(alpha["outcome"], "review_required", "{report}");
    let descriptor = alpha["descriptor_path"]
        .as_str()
        .expect("the review is a real artifact a person can open");
    assert!(Path::new(descriptor).exists(), "{report}");
    assert_eq!(report["reviews_required"], 1, "{report}");
    assert_eq!(
        vault.read(ALPHA),
        external,
        "a change awaiting review is left exactly as the editor wrote it"
    );

    let (code, doctor) = run_json(&["doctor", vault.arg()]);
    assert_eq!(code, DIAGNOSTICS, "{doctor}");
    assert_eq!(doctor["reviews_pending"], 1, "{doctor}");
}

/// `reconcile` is a routine catch-up pass, so running it twice must cost
/// nothing.
///
/// An operator who puts this in a loop — or a cron job that runs it hourly —
/// would otherwise file one review descriptor per pass over an ambiguity nobody
/// has resolved, growing `.secondbrain/transactions/` without bound and driving
/// `doctor`'s pending-review count up forever over a single undecided change.
#[test]
fn reconciling_an_unresolved_review_twice_leaves_one_review_waiting() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nRepeated.\n",
    );
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nRepeated.\n\nRepeated.\n",
    );
    vault.write(
        ALPHA,
        "# Alpha\n\nAlpha links to [[beta]] for the rollout.\n\nRepeated.\n\nChanged.\n",
    );

    let (code, first) = run_json(&["reconcile", vault.arg()]);
    assert_eq!(code, REVIEW_REQUIRED, "{first}");
    let descriptor = reconciled(&first, ALPHA)["descriptor_path"]
        .as_str()
        .expect("the review is a real artifact a person can open")
        .to_owned();

    // Nothing resolved the review and nothing touched the note, so the second
    // pass is being told about the very same undecided change.
    let (code, second) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, REVIEW_REQUIRED, "{second}");
    assert_eq!(
        reconciled(&second, ALPHA)["descriptor_path"],
        descriptor.as_str(),
        "a second pass must point at the review already waiting, not open another: {second}"
    );
    // Compared by file name: the report names the descriptor through the
    // canonicalized workspace root, and the temporary directory reaches it
    // through a symlink.
    let filed = review_descriptors(&vault);
    assert_eq!(
        filed
            .iter()
            .map(|path| path.file_name())
            .collect::<Vec<_>>(),
        vec![Path::new(&descriptor).file_name()],
        "one undecided change is one descriptor, however often the pass runs"
    );

    let (code, doctor) = run_json(&["doctor", vault.arg()]);
    assert_eq!(code, DIAGNOSTICS, "{doctor}");
    assert_eq!(
        doctor["reviews_pending"], 1,
        "a review count that climbs on its own stops meaning anything: {doctor}"
    );
}

/// A note moved outside the workspace is absent from where it was, not deleted.
///
/// `reconcile` derives its work from each note's converged base, and a move it
/// never saw leaves nothing at the path that base names. The identity map is
/// equally stale, so the pass can name the note confidently — and reporting
/// that as a *deletion* would tell an operator their note is gone when the file
/// is sitting intact one directory over. Real rename detection is not the
/// remedy; saying what is actually known is.
#[test]
fn a_note_moved_outside_the_workspace_is_reported_absent_rather_than_deleted() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );
    let converged = vault.read(ALPHA);
    let note_id = note_id_of(&vault, ALPHA);
    let moved = "notes/moved.md";
    fs::rename(vault.path().join(ALPHA), vault.path().join(moved))
        .expect("an external tool moves the note");

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    let alpha = reconciled(&report, ALPHA);
    assert_eq!(
        alpha["outcome"], "absent",
        "a file that is not where it was last seen is absent, not deleted: {report}"
    );
    assert_eq!(report["absent"], 1, "{report}");
    assert!(
        alpha["note_id"].as_str().is_some(),
        "an operator must be told which note the path belonged to: {report}"
    );

    // Nothing was lost: the bytes, the identity, and the converged base are all
    // still there, which is exactly why "deleted" was the wrong word.
    assert_eq!(
        fs::read_to_string(vault.path().join(moved)).expect("the moved file"),
        converged,
        "reconcile never touches a note, least of all one it could not find"
    );
    let root = vault.root();
    assert_eq!(
        secondbrain_vault::IdentityMap::open(&root)
            .expect("identity map")
            .note_at(&WorkspacePath::new(ALPHA).expect("path")),
        Some(note_id),
        "the note's identity survives a move the workspace never saw"
    );
    assert!(
        secondbrain_vault::BaseSnapshotStore::new(&root)
            .load(note_id)
            .expect("load the converged base")
            .is_some(),
        "the converged base survives too, so the note stays reconcilable"
    );
}

/// A deletion is an outstanding fact, and a fact does not refresh anything.
///
/// Phase 0 cannot journal a deletion, so the file's absence is reported on every
/// pass — but only the first pass has derived state to clean up. Claiming
/// `index_refreshed` forever would make a routine pass look like work, and it
/// costs a whole index rebuild each time to say nothing new.
#[test]
fn reconciling_a_deleted_note_twice_stops_claiming_it_refreshed_the_index() {
    let vault = Vault::indexed();
    converge_alpha(
        &vault,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    );
    fs::remove_file(vault.path().join(ALPHA)).expect("an external editor deletes the note");

    let (code, first) = run_json(&["reconcile", vault.arg()]);
    assert_eq!(code, OK, "{first}");
    assert_eq!(reconciled(&first, ALPHA)["outcome"], "absent", "{first}");
    assert_eq!(
        first["index_refreshed"], true,
        "the first pass really does stop the index describing the vanished file: {first}"
    );

    let (code, second) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{second}");
    assert_eq!(
        reconciled(&second, ALPHA)["outcome"],
        "absent",
        "the file is still gone, and the pass still says so: {second}"
    );
    assert_eq!(
        second["index_refreshed"], false,
        "a pass that refreshed nothing new must not claim it did: {second}"
    );

    // And the fact it reports is still true afterwards.
    let (code, hits) = run_json(&["search", vault.arg(), "launch"]);
    assert_eq!(code, OK, "{hits}");
    assert!(
        hits["hits"].as_array().expect("hits").is_empty(),
        "a note whose file is gone must stay unfindable: {hits}"
    );
}

/// The external-edit pipeline, driven the way an operator drives it.
///
/// Every other case in this section hands the workspace a converged base built
/// by a transaction first, so none of them can tell whether the product is able
/// to establish the first one at all. This one starts from ordinary Markdown
/// and uses nothing but `init`, `index rebuild` and `reconcile` — which is what
/// "External edits become deterministic semantic operations" has to mean on a
/// real vault.
#[test]
fn indexing_a_plain_vault_makes_a_later_external_edit_reconcilable() {
    let vault = Vault::empty();
    vault.write(ALPHA, ALPHA_SOURCE);
    vault.write(BETA, BETA_SOURCE);

    secondbrain().args(["init", vault.arg()]).assert().success();
    secondbrain()
        .args(["index", "rebuild", vault.arg()])
        .assert()
        .success();

    // Taking responsibility for a note is what gives it a converged base, and
    // it is not something a person can be asked to arrange by hand.
    for path in [ALPHA, BETA] {
        let (code, note) = run_json(&["note", "inspect", vault.arg(), path]);
        assert_eq!(code, OK, "{note}");
        assert_eq!(
            note["converged"], true,
            "indexing must leave {path} with a base an editor can diverge from: {note}"
        );
        assert_eq!(
            note["converged_version"], 0,
            "the first base a note ever gets is its genesis: {note}"
        );
    }
    assert_eq!(
        (vault.read(ALPHA), vault.read(BETA)),
        (ALPHA_SOURCE.to_owned(), BETA_SOURCE.to_owned()),
        "recording a base is workspace state; it may not touch a byte of any note"
    );

    // An editor that is not this workspace saves the whole file over the note.
    let external = "# Alpha\n\nAlpha links to [[beta]] after the retro.\n";
    vault.write(ALPHA, external);

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["considered"], 2, "{report}");
    let alpha = reconciled(&report, ALPHA);
    assert_eq!(alpha["outcome"], "adopted", "{report}");
    assert_eq!(alpha["version"], 1, "{report}");
    let transaction_id: TransactionId = alpha["transaction_id"]
        .as_str()
        .unwrap_or_else(|| panic!("an adopted edit is a transaction: {report}"))
        .parse()
        .expect("a canonical transaction id");
    assert_eq!(report["adopted"], 1, "{report}");
    assert_eq!(
        reconciled(&report, BETA)["outcome"],
        "unchanged",
        "{report}"
    );
    assert_eq!(
        vault.read(ALPHA),
        external,
        "reconcile journals what the editor wrote; it never rewrites the file"
    );
    assert_eq!(
        vault.read(BETA),
        BETA_SOURCE,
        "a note nobody edited comes out of a reconciliation byte-identical"
    );

    // The edit is a semantic operation in the note's own journal, attributed to
    // the device that made it — not an opaque overwrite.
    let note_id = note_id_of(&vault, ALPHA);
    let replay = LocalMutationLog::open(vault.path(), note_id)
        .expect("the note has a journal")
        .replay()
        .expect("replay the journal");
    assert!(replay.corruption.is_none(), "{:?}", replay.corruption);
    let records: Vec<_> = replay
        .records
        .iter()
        .filter(|record| record.transaction_id == transaction_id)
        .collect();
    assert!(
        !records.is_empty(),
        "the adopted edit must be journaled as operations: {:?}",
        replay.records
    );
    for record in &records {
        assert_eq!(
            record.actor_id.as_str(),
            "external:local",
            "an external edit is attributed to the editor, not to the workspace"
        );
    }

    // And a pass over a workspace that nothing has edited since claims nothing.
    let (code, again) = run_json(&["reconcile", vault.arg()]);
    assert_eq!(code, OK, "{again}");
    assert_eq!(again["considered"], 2, "{again}");
    assert_eq!(again["adopted"], 0, "{again}");
    assert_eq!(again["unchanged"], 2, "{again}");
    assert_eq!(
        again["index_refreshed"], false,
        "a pass that changed nothing must not claim to have rebuilt anything: {again}"
    );
}

#[test]
fn reconcile_of_a_workspace_that_converged_nothing_claims_no_work() {
    // Initialized but never indexed: the workspace has taken responsibility for
    // no note yet, so nothing has a converged base an editor could have
    // diverged from — which is the condition this case is about.
    let vault = Vault::empty();
    vault.write(ALPHA, ALPHA_SOURCE);
    vault.write(BETA, BETA_SOURCE);
    secondbrain().args(["init", vault.arg()]).assert().success();

    let (code, report) = run_json(&["reconcile", vault.arg()]);

    assert_eq!(code, OK, "{report}");
    assert_eq!(report["considered"], 0, "{report}");
    assert!(
        report["notes"].as_array().expect("notes").is_empty(),
        "{report}"
    );
    assert_eq!(report["index_refreshed"], false, "{report}");
}

#[test]
fn reconcile_of_an_uninitialized_directory_fails() {
    let vault = Vault::empty();

    secondbrain()
        .args(["reconcile", vault.arg()])
        .assert()
        .code(FAILED);
}

// ---------------------------------------------------------------------------
// Output contract
// ---------------------------------------------------------------------------

#[test]
fn human_output_carries_no_ansi_when_stdout_is_not_a_tty() {
    let vault = Vault::indexed();

    for arguments in [
        vec!["validate", vault.arg()],
        vec!["index", "rebuild", vault.arg()],
        vec!["search", vault.arg(), "migration"],
        vec!["note", "inspect", vault.arg(), ALPHA],
        vec!["recovery", "check", vault.arg()],
        vec!["reconcile", vault.arg()],
        vec!["doctor", vault.arg()],
    ] {
        let assertion = secondbrain().args(&arguments).assert().success();
        let output = assertion.get_output();
        let stdout = String::from_utf8(output.stdout.clone()).expect("UTF-8");
        assert!(
            !stdout.contains('\u{1b}'),
            "`{arguments:?}` emitted ANSI to a pipe: {stdout:?}"
        );
    }
}

#[test]
fn errors_are_reported_on_stderr_with_a_stable_diagnostic_code() {
    let vault = Vault::empty();
    secondbrain().args(["init", vault.arg()]).assert().success();

    let output = secondbrain()
        .args(["search", vault.arg(), "migration", "--json"])
        .output()
        .expect("run");

    assert_eq!(output.status.code(), Some(FAILED));
    assert!(
        output.stdout.is_empty(),
        "a failure writes nothing to stdout"
    );
    let error: Value = serde_json::from_slice(&output.stderr).expect("the error is JSON too");
    assert!(
        error["error"]["code"]
            .as_str()
            .expect("code")
            .starts_with("SB-"),
        "{error}"
    );
}
