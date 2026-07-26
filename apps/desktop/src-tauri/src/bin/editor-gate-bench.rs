#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::hint::black_box;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use secondbrain_desktop::{open_workspace_at, transaction_apply_at, transaction_preview_at};
use secondbrain_vault::initialize_workspace;

fn main() {
    let samples = env::args()
        .nth(1)
        .expect("sample count")
        .parse::<usize>()
        .expect("integer sample count");
    assert!(samples >= 20, "at least 20 samples are required");

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "secondbrain-editor-gate-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).expect("create benchmark workspace");
    initialize_workspace(&root).expect("initialize benchmark workspace");
    let note = "benchmark.md";
    let mut source = "# Editor gate\n\nMaterialization baseline.\n".to_owned();
    fs::write(root.join(note), &source).expect("write benchmark note");
    open_workspace_at(&root).expect("index benchmark workspace");

    for index in 0..samples {
        source.push_str(&format!("\nSample {index}.\n"));
        let preview_started = Instant::now();
        let preview = transaction_preview_at(&root, note, black_box(&source)).expect("preview");
        let preview_us = preview_started.elapsed().as_micros();
        let materialization_started = Instant::now();
        let outcome = transaction_apply_at(&root, black_box(&preview)).expect("materialize");
        let materialization_us = materialization_started.elapsed().as_micros();
        assert!(outcome.changed, "benchmark edit must materialize");
        println!("{preview_us},{materialization_us}");
    }

    fs::remove_dir_all(root).expect("remove benchmark workspace");
}
