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
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::{NoteId, NoteVersion, TransactionId};
use secondbrain_core::path::WorkspacePath;
use secondbrain_markdown::SourceDocument;
use secondbrain_markdown::diff::diff_documents;
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
        let vault = Self::empty();
        vault.write(ALPHA, ALPHA_SOURCE);
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
    let vault = Vault::indexed();
    vault.write(ALPHA, "Duplicate.\n\nSome middle.\n\nDuplicate.\n");
    secondbrain()
        .args(["index", "rebuild", vault.arg()])
        .assert()
        .success();
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

// ---------------------------------------------------------------------------
// transaction apply
// ---------------------------------------------------------------------------

#[test]
fn transaction_apply_materializes_the_plan_and_refreshes_the_index() {
    let vault = Vault::indexed();
    let incoming = vault.scratch("incoming.md");
    let updated = "# Alpha\n\nAlpha links to [[beta]] for the launch.\n";
    fs::write(&incoming, updated).expect("write");
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
        .success();

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
    let incoming = vault.scratch("incoming.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    )
    .expect("write");
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
        .success();

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
    let vault = Vault::indexed();
    vault.write(ALPHA, "Duplicate.\n\nSome middle.\n\nDuplicate.\n");
    secondbrain()
        .args(["index", "rebuild", vault.arg()])
        .assert()
        .success();
    let incoming = vault.scratch("incoming.md");
    fs::write(&incoming, "Duplicate.\n\nSome middle.\n\nChanged.\n").expect("write");
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
        .code(REVIEW_REQUIRED);

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
fn transaction_apply_refuses_a_plan_from_another_workspace() {
    let vault = Vault::indexed();
    let other = Vault::indexed();
    let incoming = vault.scratch("incoming.md");
    fs::write(
        &incoming,
        "# Alpha\n\nAlpha links to [[beta]] for the launch.\n",
    )
    .expect("write");
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
        .success();

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
fn doctor_reports_a_workspace_that_was_never_initialized() {
    let vault = Vault::empty();

    secondbrain()
        .args(["doctor", vault.arg()])
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
