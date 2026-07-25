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
