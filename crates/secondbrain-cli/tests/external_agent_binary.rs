//! An OpenCode-compatible external process uses only the binary contract.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const NOTE: &str = "projects/phase-zero.md";

fn secondbrain() -> Command {
    Command::cargo_bin("secondbrain").expect("binary builds")
}

fn run_json(arguments: &[&str]) -> (i32, Value) {
    let output = secondbrain()
        .args(arguments)
        .arg("--json")
        .output()
        .unwrap();
    let value = if output.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout)))
    };
    (output.status.code().unwrap(), value)
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn preview(vault: &Path, scratch: &Path, source: &str) -> PathBuf {
    let incoming = scratch.join("incoming.md");
    let plan = scratch.join("plan.json");
    fs::write(&incoming, source).unwrap();
    let (code, report) = run_json(&[
        "diff",
        vault.to_str().unwrap(),
        NOTE,
        incoming.to_str().unwrap(),
        "--out",
        plan.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{report}");
    plan
}

#[test]
fn external_agent_previews_applies_retries_and_reconciles_safely() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown/obsidian-vault");
    let vault = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    copy_tree(&fixture, vault.path());
    let vault_arg = vault.path().to_str().unwrap();

    let help = secondbrain().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(!help.stdout.contains(&0x1b));
    assert_eq!(run_json(&["init", vault_arg]).0, 0);
    assert_eq!(run_json(&["index", "rebuild", vault_arg]).0, 0);
    assert_eq!(
        run_json(&["search", vault_arg, "durability-canary"]).1["hits"][0]["path"],
        NOTE
    );

    let before = fs::read_to_string(vault.path().join(NOTE)).unwrap();
    let proposed = before.replace(
        "remains independently editable",
        "is safely edited by OpenCode",
    );
    let plan = preview(vault.path(), scratch.path(), &proposed);
    assert_eq!(
        fs::read_to_string(vault.path().join(NOTE)).unwrap(),
        before,
        "preview is read-only"
    );
    let plan_json: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(plan_json["format"], "sb-transaction-plan-v1");
    assert!(!plan_json["operations"].as_array().unwrap().is_empty());

    let (code, applied) = run_json(&["transaction", "apply", vault_arg, plan.to_str().unwrap()]);
    assert_eq!(code, 0, "{applied}");
    assert_eq!(
        fs::read_to_string(vault.path().join(NOTE)).unwrap(),
        proposed
    );
    let (code, stale) = run_json(&["transaction", "apply", vault_arg, plan.to_str().unwrap()]);
    assert_eq!(code, 1, "a retried stale plan must fail closed: {stale}");
    assert_eq!(
        fs::read_to_string(vault.path().join(NOTE)).unwrap(),
        proposed
    );

    let external = proposed.replace("durability-canary", "agent-reconcile-canary");
    fs::write(vault.path().join(NOTE), &external).unwrap();
    let (code, reconciled) = run_json(&["reconcile", vault_arg]);
    assert_eq!(code, 0, "{reconciled}");
    assert_eq!(reconciled["adopted"], 1, "{reconciled}");
    assert_eq!(
        run_json(&["search", vault_arg, "agent-reconcile-canary"]).1["hits"][0]["path"],
        NOTE
    );
    let (code, doctor) = run_json(&["doctor", vault_arg]);
    assert_eq!(code, 0, "{doctor}");
    assert!(doctor["problems"].as_array().unwrap().is_empty());
}

#[test]
fn external_agent_uses_typed_property_binary_contract_end_to_end() {
    let vault = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let note = "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\ntitle: Agent note # keep\n---\nBody bytes\n";
    fs::write(vault.path().join("agent.md"), note).unwrap();
    let vault_arg = vault.path().to_str().unwrap();
    assert_eq!(run_json(&["init", vault_arg]).0, 0);
    assert_eq!(run_json(&["index", "rebuild", vault_arg]).0, 0);

    let (code, read) = run_json(&["property", "read", vault_arg, "agent.md"]);
    assert_eq!(code, 0, "{read}");
    assert_eq!(read["properties"]["title"], "Agent note");
    assert!(read["properties"].get("id").is_none());

    let plan = scratch.path().join("property.json");
    let (code, _) = run_json(&[
        "property",
        "set",
        vault_arg,
        "agent.md",
        "tags",
        "[\"rust\",2,true]",
        "--out",
        plan.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(vault.path().join("agent.md")).unwrap(),
        note
    );
    let preview: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(preview["format"], "sb-property-preview-v1");
    assert_eq!(
        preview["properties"]["tags"],
        serde_json::json!(["rust", 2, true])
    );
    assert_eq!(
        preview["transaction"]["format"],
        "sb-transaction-preview-v1"
    );

    let (code, applied) = run_json(&["property", "apply", vault_arg, plan.to_str().unwrap()]);
    assert_eq!(code, 0, "{applied}");
    assert_eq!(applied["changed"], true);
    let changed = fs::read_to_string(vault.path().join("agent.md")).unwrap();
    assert!(changed.contains("title: Agent note # keep\n"));
    assert!(changed.contains("tags:\n- rust\n- 2\n- true\n---\nBody bytes\n"));

    let (code, stale) = run_json(&["property", "apply", vault_arg, plan.to_str().unwrap()]);
    assert_eq!(code, 1, "{stale}");
    assert_eq!(
        fs::read_to_string(vault.path().join("agent.md")).unwrap(),
        changed
    );
}

#[test]
fn external_agent_previews_applies_and_retries_daily_note_creation() {
    let vault = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let vault_arg = vault.path().to_str().unwrap();
    assert_eq!(run_json(&["init", vault_arg]).0, 0);
    assert_eq!(run_json(&["index", "rebuild", vault_arg]).0, 0);
    let preview_file = scratch.path().join("daily.json");
    let (code, _) = run_json(&[
        "note",
        "daily",
        vault_arg,
        "2026-07-26",
        "--out",
        preview_file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert!(
        !vault.path().join("Daily").exists(),
        "preview must not create directories"
    );
    let preview: Value = serde_json::from_slice(&fs::read(&preview_file).unwrap()).unwrap();
    assert_eq!(preview["format"], "sb-note-create-preview-v1");
    assert_eq!(preview["path"], "Daily/2026-07-26.md");
    assert_eq!(preview["actor"], "cli");

    let (code, applied) = run_json(&[
        "note",
        "apply-create",
        vault_arg,
        preview_file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{applied}");
    assert_eq!(applied["created"], true);
    let source = fs::read_to_string(vault.path().join("Daily/2026-07-26.md")).unwrap();
    assert!(source.contains("# 2026-07-26"));
    assert!(source.contains(preview["note_id"].as_str().unwrap()));
    let (code, retried) = run_json(&[
        "note",
        "apply-create",
        vault_arg,
        preview_file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{retried}");
    assert_eq!(retried["created"], false);
    let (code, opened) = run_json(&["note", "daily", vault_arg, "2026-07-26"]);
    assert_eq!(code, 0, "{opened}");
    assert_eq!(opened["status"], "existing");
    assert_eq!(opened["note_id"], preview["note_id"]);

    let (code, invalid) = run_json(&["note", "daily", vault_arg, "2026-02-30"]);
    assert_eq!(code, 1, "{invalid}");
}

#[test]
fn external_agent_adopts_obsidian_vault_with_versioned_read_only_import_contract() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown/obsidian-vault");
    let vault = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    copy_tree(&fixture, vault.path());
    let before = fs::read(vault.path().join(NOTE)).unwrap();
    let preview_file = scratch.path().join("import.json");
    let vault_arg = vault.path().to_str().unwrap();

    let (code, _) = run_json(&[
        "import",
        "preview",
        vault_arg,
        "--out",
        preview_file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert!(!vault.path().join(".secondbrain").exists());
    let preview: Value = serde_json::from_slice(&fs::read(&preview_file).unwrap()).unwrap();
    assert_eq!(preview["format"], "sb-obsidian-import-preview-v1");
    assert_eq!(
        preview["plannedWrites"],
        serde_json::json!({
            "markdown": 0, "attachments": 0, "obsidianConfig": 0
        })
    );

    let (code, applied) = run_json(&["import", "apply", vault_arg, preview_file.to_str().unwrap()]);
    assert_eq!(code, 0, "{applied}");
    assert_eq!(applied["status"], "initialized");
    assert_eq!(fs::read(vault.path().join(NOTE)).unwrap(), before);
    let workspace_id = applied["workspaceId"].clone();
    let (code, _) = run_json(&[
        "import",
        "preview",
        vault_arg,
        "--out",
        preview_file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let (code, retry) = run_json(&["import", "apply", vault_arg, preview_file.to_str().unwrap()]);
    assert_eq!(code, 0, "{retry}");
    assert_eq!(retry["status"], "already_initialized");
    assert_eq!(retry["workspaceId"], workspace_id);
}

#[test]
fn external_agent_consumes_deterministic_versioned_graph_json() {
    let vault = TempDir::new().unwrap();
    fs::write(
        vault.path().join("a.md"),
        "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAV\ntitle: A\n---\n[[B]] [[B]] [[Missing]]\n",
    )
    .unwrap();
    fs::write(
        vault.path().join("b.md"),
        "---\nid: 01ARZ3NDEKTSV4RRFFQ69G5FAW\ntitle: B\n---\n[[B]]\n",
    )
    .unwrap();
    let root = vault.path().to_str().unwrap();
    assert_eq!(run_json(&["init", root]).0, 0);
    assert_eq!(run_json(&["index", "rebuild", root]).0, 0);

    let (code, graph) = run_json(&["graph", root]);
    let (_, repeated) = run_json(&["graph", root]);

    assert_eq!(code, 0, "{graph}");
    assert_eq!(graph, repeated);
    assert_eq!(graph["format"], "sb-workspace-graph-v1");
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"][0]["occurrences"], 2);
    assert_eq!(graph["edges"][1]["self_link"], true);
    assert_eq!(graph["broken_links"][0]["target"], "Missing");
}
