//! Full Phase 0 workflow through the real `secondbrain` process.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use secondbrain_core::hash::ContentHash;
use secondbrain_core::id::NoteId;
use secondbrain_core::path::WorkspacePath;
use secondbrain_index::{index_path, logical_dump};
use secondbrain_transaction::TransactionEngine;
use secondbrain_vault::{BaseSnapshotStore, IdentityMap, WorkspaceRoot, load_manifest};
use serde_json::Value;
use tempfile::TempDir;

const NOTE: &str = "projects/phase-zero.md";
const RUNBOOK: &str = "reference/recovery-runbook.md";
const NOTE_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

fn secondbrain() -> Command {
    Command::cargo_bin("secondbrain").expect("the secondbrain binary builds")
}

fn run_json(arguments: &[&str]) -> (i32, Value) {
    let output = secondbrain()
        .args(arguments)
        .arg("--json")
        .output()
        .expect("run secondbrain");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let value = if stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{error}: {stdout}"))
    };
    (
        output
            .status
            .code()
            .expect("ordinary command exits normally"),
        value,
    )
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create destination");
    for entry in fs::read_dir(source).expect("read fixture") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn hashes(root: &Path) -> BTreeMap<PathBuf, ContentHash> {
    fn collect(root: &Path, directory: &Path, output: &mut BTreeMap<PathBuf, ContentHash>) {
        for entry in fs::read_dir(directory).expect("read directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) != Some(".secondbrain") {
                    collect(root, &path, output);
                }
            } else {
                output.insert(
                    path.strip_prefix(root).expect("below root").to_path_buf(),
                    ContentHash::digest(fs::read(path).expect("read file")),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    collect(root, root, &mut output);
    output
}

fn write_plan(vault: &Path, scratch: &Path, name: &str, incoming: &str) -> PathBuf {
    let incoming_path = scratch.join(format!("{name}.md"));
    let plan_path = scratch.join(format!("{name}.json"));
    fs::write(&incoming_path, incoming).expect("write incoming source");
    let (code, report) = run_json(&[
        "diff",
        vault.to_str().expect("UTF-8 vault"),
        NOTE,
        incoming_path.to_str().expect("UTF-8 incoming"),
        "--out",
        plan_path.to_str().expect("UTF-8 plan"),
    ]);
    assert_eq!(code, 0, "{report}");
    plan_path
}

fn crash_after_durable_operations(vault: &Path, plan: &Path) {
    let output = secondbrain()
        .args([
            "transaction",
            "apply",
            vault.to_str().expect("UTF-8 vault"),
            plan.to_str().expect("UTF-8 plan"),
        ])
        .env("SECONDBRAIN_TEST_FAILPOINT", "after_operations_durable")
        .env("SECONDBRAIN_TEST_FAILPOINT_ABORT", "1")
        .output()
        .expect("spawn crashing binary");
    assert!(
        !output.status.success(),
        "the hard-crash boundary must terminate the process"
    );
}

#[test]
fn phase_zero_cli_survives_external_merge_and_crash_recovery() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown/obsidian-vault");
    let vault = TempDir::new().expect("temporary vault");
    let scratch = TempDir::new().expect("temporary scratch");
    copy_tree(&fixture, vault.path());
    let original_hashes = hashes(vault.path());
    let vault_arg = vault.path().to_str().expect("UTF-8 vault");

    assert_eq!(run_json(&["init", vault_arg]).0, 0);
    assert_eq!(
        hashes(vault.path()),
        original_hashes,
        "init may not touch fixture bytes"
    );
    let (code, rebuilt) = run_json(&["index", "rebuild", vault_arg]);
    assert_eq!(code, 0, "{rebuilt}");
    assert_eq!(rebuilt["indexed"], 3, "{rebuilt}");
    assert_eq!(
        hashes(vault.path()),
        original_hashes,
        "indexing may not touch fixture bytes"
    );

    let (code, search) = run_json(&["search", vault_arg, "durability-canary"]);
    assert_eq!(code, 0, "{search}");
    assert_eq!(search["hits"][0]["path"], NOTE, "{search}");
    let (code, runbook) = run_json(&["note", "inspect", vault_arg, RUNBOOK]);
    assert_eq!(code, 0, "{runbook}");
    assert_eq!(runbook["backlinks"][0]["path"], NOTE, "{runbook}");

    let base = fs::read_to_string(vault.path().join(NOTE)).expect("read note");
    let internal = base.replace(
        "The durability-canary links to [[Recovery Runbook]].",
        "The durability-canary links to [[Recovery Runbook]] for launch.",
    );
    let pending = write_plan(vault.path(), scratch.path(), "pending-merge", &internal);
    crash_after_durable_operations(vault.path(), &pending);
    assert_eq!(fs::read_to_string(vault.path().join(NOTE)).unwrap(), base);

    let external = base.replace(
        "The rollout paragraph remains independently editable.",
        "The rollout paragraph was revised by an Obsidian-compatible editor.",
    );
    fs::write(vault.path().join(NOTE), &external).expect("external whole-file write");
    let (code, reconciled) = run_json(&["reconcile", vault_arg]);
    assert_eq!(code, 0, "{reconciled}");
    assert_eq!(reconciled["merged"], 1, "{reconciled}");
    let merged = fs::read_to_string(vault.path().join(NOTE)).unwrap();
    assert!(
        merged.contains("for launch"),
        "internal edit survives: {merged}"
    );
    assert!(
        merged.contains("revised by an Obsidian-compatible editor"),
        "external edit survives: {merged}"
    );

    let recovered_source = merged.replace(
        "- [ ] Verify recovery",
        "- [x] Verify recovery after a hard process crash",
    );
    let recovery_plan = write_plan(
        vault.path(),
        scratch.path(),
        "pending-recovery",
        &recovered_source,
    );
    crash_after_durable_operations(vault.path(), &recovery_plan);
    assert_eq!(fs::read_to_string(vault.path().join(NOTE)).unwrap(), merged);

    let (code, recovered) = run_json(&["recovery", "check", vault_arg]);
    assert_eq!(code, 0, "{recovered}");
    assert_eq!(recovered["index_repairs"], 1, "{recovered}");
    assert!(
        recovered["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["path"] == NOTE && action["action"] == "index_repair")
    );
    let final_source = fs::read_to_string(vault.path().join(NOTE)).unwrap();
    assert_eq!(final_source, recovered_source);

    let root = WorkspaceRoot::open(vault.path()).expect("workspace root");
    let note_id: NoteId = NOTE_ID.parse().expect("fixture note id");
    let path = WorkspacePath::new(NOTE).expect("workspace path");
    let identity = IdentityMap::open(&root).expect("identity map");
    assert_eq!(identity.note_at(&path), Some(note_id));
    let snapshot = BaseSnapshotStore::new(&root)
        .load(note_id)
        .expect("load snapshot")
        .expect("snapshot exists");
    assert_eq!(snapshot.source, final_source);
    assert_eq!(
        snapshot.source_hash,
        ContentHash::digest(final_source.as_bytes())
    );
    let engine = TransactionEngine::new(root, load_manifest(vault.path()).unwrap().workspace_id);
    assert!(
        engine.transactions().unwrap().iter().all(|transaction| {
            transaction.state == "COMMITTED" || transaction.state == "ABORTED"
        })
    );

    let before = logical_dump(index_path(vault.path())).expect("logical dump");
    fs::remove_file(index_path(vault.path())).expect("delete derived index");
    assert_eq!(run_json(&["index", "rebuild", vault_arg]).0, 0);
    let after = logical_dump(index_path(vault.path())).expect("rebuilt dump");
    assert_eq!(
        after, before,
        "deleting SQLite may not change logical state"
    );

    let (code, doctor) = run_json(&["doctor", vault_arg]);
    assert_eq!(code, 0, "{doctor}");
    assert_eq!(doctor["transactions"]["pending"], 0, "{doctor}");
    assert_eq!(
        doctor["transactions"]["index_repairs_outstanding"], 0,
        "{doctor}"
    );
    assert!(
        doctor["problems"].as_array().unwrap().is_empty(),
        "{doctor}"
    );
}
