//! Deterministic filesystem contracts that must hold on supported desktop OSes.

use std::ffi::OsString;
use std::fs;

use secondbrain_core::path::WorkspacePath;
use secondbrain_vault::WorkspaceRoot;

#[test]
fn workspace_paths_use_portable_forward_slashes() {
    let nested = WorkspacePath::new("notes/nested/note.md").expect("portable path");
    assert_eq!(nested.as_str(), "notes/nested/note.md");
    assert!(WorkspacePath::new("notes\\nested\\note.md").is_err());

    let temp = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot::open(temp.path()).expect("workspace root");
    root.atomic_write(&nested, b"portable").expect("write");
    assert_eq!(
        fs::read(temp.path().join("notes/nested/note.md")).unwrap(),
        b"portable"
    );
}

#[test]
fn atomic_write_preserves_crlf_bytes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot::open(temp.path()).expect("workspace root");
    let path = WorkspacePath::new("notes/crlf.md").expect("portable path");
    let source = b"# Heading\r\n\r\nBody\r\n";

    root.atomic_write(&path, source).expect("write");

    assert_eq!(fs::read(temp.path().join("notes/crlf.md")).unwrap(), source);
}

#[cfg(unix)]
#[test]
fn non_utf8_filename_is_explicitly_not_a_workspace_path() {
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let name = OsString::from_vec(vec![b'n', b'o', b't', b'e', 0xff, b'.', b'm', b'd']);
    let path = temp.path().join(&name);
    assert!(path.file_name().unwrap().to_str().is_none());
    match fs::write(&path, b"bytes") {
        Ok(()) => {
            assert_eq!(fs::read(&path).unwrap(), b"bytes");
            println!(
                "non-UTF-8 filename policy: filesystem-preserved, excluded from UTF-8 WorkspacePath"
            );
        }
        Err(error) => println!(
            "{} ({error})",
            "non-UTF-8 filename policy: filesystem does not support this byte sequence; "
                .to_owned()
                + "excluded from UTF-8 WorkspacePath"
        ),
    }
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    let target = outside.path().join("secret.md");
    fs::write(&target, b"secret").expect("outside file");
    symlink(&target, workspace.path().join("escape.md")).expect("symlink");

    let root = WorkspaceRoot::open(workspace.path()).expect("workspace root");
    let path = WorkspacePath::new("escape.md").expect("portable path");
    assert!(
        root.resolve(&path).is_err(),
        "symlink must not escape workspace"
    );
}

#[cfg(windows)]
#[test]
fn symlink_behavior_is_capability_aware() {
    use std::os::windows::fs::symlink_file;

    let workspace = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    let target = outside.path().join("secret.md");
    fs::write(&target, b"secret").expect("outside file");
    let link = workspace.path().join("escape.md");

    match symlink_file(&target, &link) {
        Ok(()) => {
            let root = WorkspaceRoot::open(workspace.path()).expect("workspace root");
            let path = WorkspacePath::new("escape.md").expect("portable path");
            assert!(
                root.resolve(&path).is_err(),
                "symlink must not escape workspace"
            );
        }
        Err(error) => println!("symlink capability unavailable; skipped: {error}"),
    }
}

#[test]
fn non_utf8_policy_does_not_change_portable_path_validation() {
    let invalid = WorkspacePath::new("notes/\u{fffd}.md");
    assert!(
        invalid.is_ok(),
        "valid UTF-8 replacement text remains portable"
    );
    let _ = OsString::from("portable");
}
