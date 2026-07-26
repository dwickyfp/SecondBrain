use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use secondbrain_index::{ImportError, apply_import, preview_import};
use secondbrain_vault::load_manifest;

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn user_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if relative == ".secondbrain" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), result);
            } else {
                result.insert(relative, fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

#[test]
fn real_obsidian_vault_preview_and_apply_preserve_all_user_bytes_and_retry_identity() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown/obsidian-vault");
    let vault = tempfile::tempdir().unwrap();
    copy_tree(&fixture, vault.path());
    fs::create_dir_all(vault.path().join("assets")).unwrap();
    fs::write(vault.path().join("assets/diagram.png"), [0, 1, 2, 255]).unwrap();
    let before = user_bytes(vault.path());

    let preview = preview_import(vault.path()).unwrap();
    assert_eq!(preview.format, "sb-obsidian-import-preview-v1");
    assert_eq!(preview.inventory.markdown.len(), 3);
    assert_eq!(preview.inventory.attachments, ["assets/diagram.png"]);
    assert_eq!(preview.inventory.obsidian_config, [".obsidian/app.json"]);
    assert_eq!(preview.planned_writes.markdown, 0);
    assert!(preview.can_apply);
    assert_eq!(user_bytes(vault.path()), before, "preview is read-only");

    let first = apply_import(vault.path(), &preview).unwrap();
    assert_eq!(first.status, "initialized");
    assert_eq!(first.index.indexed, 3);
    assert_eq!(user_bytes(vault.path()), before);
    let identity = load_manifest(vault.path()).unwrap().workspace_id;
    let retry_preview = preview_import(vault.path()).unwrap();
    let retry = apply_import(vault.path(), &retry_preview).unwrap();
    assert_eq!(retry.status, "already_initialized");
    assert_eq!(retry.workspace_id, identity);
    assert_eq!(user_bytes(vault.path()), before);
}

#[test]
fn apply_binds_manifest_existence_format_bytes_and_workspace_identity() {
    let vault = tempfile::tempdir().unwrap();
    fs::write(vault.path().join("note.md"), "# Note\n").unwrap();

    let absent = preview_import(vault.path()).unwrap();
    secondbrain_vault::initialize_workspace(vault.path()).unwrap();
    assert!(matches!(
        apply_import(vault.path(), &absent),
        Err(ImportError::PreviewModified("alreadyInitialized"))
    ));

    let original = preview_import(vault.path()).unwrap();
    let manifest_path = vault.path().join(".secondbrain/manifest.toml");
    let original_bytes = fs::read(&manifest_path).unwrap();
    fs::remove_file(&manifest_path).unwrap();
    assert!(matches!(
        apply_import(vault.path(), &original),
        Err(ImportError::PreviewModified("alreadyInitialized"))
    ));

    fs::write(&manifest_path, &original_bytes).unwrap();
    let original = preview_import(vault.path()).unwrap();
    let mut changed_bytes = String::from_utf8(original_bytes.clone()).unwrap();
    changed_bytes.push('\n');
    fs::write(&manifest_path, changed_bytes).unwrap();
    assert!(matches!(
        apply_import(vault.path(), &original),
        Err(ImportError::PreviewModified("manifestFingerprint"))
    ));

    fs::write(&manifest_path, &original_bytes).unwrap();
    let original = preview_import(vault.path()).unwrap();
    let old_id = original.workspace_id.unwrap().to_string();
    let new_id = secondbrain_core::id::WorkspaceId::new().to_string();
    let changed_id = String::from_utf8(original_bytes.clone())
        .unwrap()
        .replace(&old_id, &new_id);
    fs::write(&manifest_path, changed_id).unwrap();
    assert!(matches!(
        apply_import(vault.path(), &original),
        Err(ImportError::PreviewModified("workspaceId"))
    ));

    fs::write(&manifest_path, &original_bytes).unwrap();
    let original = preview_import(vault.path()).unwrap();
    let wrong_format = String::from_utf8(original_bytes)
        .unwrap()
        .replace("format_version = 1", "format_version = 2");
    fs::write(&manifest_path, wrong_format).unwrap();
    assert!(apply_import(vault.path(), &original).is_err());
}

#[test]
fn apply_rejects_tamper_and_case_collisions_before_initialization() {
    let vault = tempfile::tempdir().unwrap();
    fs::write(vault.path().join("note.md"), "# Note\n").unwrap();
    let preview = preview_import(vault.path()).unwrap();
    let mut edited_preview = preview.clone();
    edited_preview.planned_writes.markdown = 1;
    assert!(matches!(
        apply_import(vault.path(), &edited_preview),
        Err(ImportError::PreviewModified("plannedWrites"))
    ));
    assert!(!vault.path().join(".secondbrain").exists());
    fs::write(vault.path().join("note.md"), "# Changed\n").unwrap();
    assert!(matches!(
        apply_import(vault.path(), &preview),
        Err(ImportError::FingerprintChanged { .. })
    ));
    assert!(!vault.path().join(".secondbrain").exists());

    fs::write(vault.path().join("Note.md"), "# Collision\n").unwrap();
    let collision = preview_import(vault.path()).unwrap();
    if collision.inventory.markdown.len() == 2 {
        assert!(!collision.can_apply);
        assert_eq!(collision.portable_collisions.len(), 1);
        assert!(matches!(
            apply_import(vault.path(), &collision),
            Err(ImportError::Blocked)
        ));
    } else {
        assert_eq!(
            collision.inventory.markdown,
            ["note.md"],
            "case-insensitive filesystem aliases both names"
        );
    }
    assert!(!vault.path().join(".secondbrain").exists());
}

#[test]
fn preview_reports_parse_errors_duplicate_ids_and_link_diagnostics_deterministically() {
    let vault = tempfile::tempdir().unwrap();
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    fs::write(
        vault.path().join("a.md"),
        format!("---\nid: {id}\ntitle: Same\n---\n[[Same]] [[Missing]]\n"),
    )
    .unwrap();
    fs::write(
        vault.path().join("b.md"),
        format!("---\nid: {id}\ntitle: Same\n---\n# B\n"),
    )
    .unwrap();
    fs::write(vault.path().join("bad.md"), "---\ninvalid: [\n---\n").unwrap();
    let preview = preview_import(vault.path()).unwrap();
    assert_eq!(preview.parse_errors.len(), 1);
    assert_eq!(preview.duplicate_ids.len(), 1);
    assert_eq!(preview.broken_links.len(), 1);
    assert_eq!(preview.ambiguous_links.len(), 1);
    assert!(!preview.can_apply);
}

#[cfg(unix)]
#[test]
fn symlinks_outside_cycles_and_internal_directory_links_are_reported_without_following() {
    use std::os::unix::fs::symlink;

    for name in ["outside", "cycle", "internal"] {
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "# Secret\n").unwrap();
        match name {
            "outside" => symlink(outside.path(), vault.path().join("linked")).unwrap(),
            "cycle" => symlink(vault.path(), vault.path().join("cycle")).unwrap(),
            "internal" => symlink(outside.path(), vault.path().join(".secondbrain")).unwrap(),
            _ => unreachable!(),
        }
        let preview = preview_import(vault.path()).unwrap();
        assert_eq!(preview.inventory.symlinks.len(), 1);
        assert!(!preview.can_apply);
        assert!(matches!(
            apply_import(vault.path(), &preview),
            Err(ImportError::Blocked)
        ));
        assert!(!outside.path().join("manifest.toml").exists());
    }
}
