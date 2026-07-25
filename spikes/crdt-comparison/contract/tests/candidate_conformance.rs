use std::{env, path::Path};

use crdt_spike_contract::{assert_fixture, fixture_paths, invoke, load_fixture};

#[test]
fn candidate_executes_every_mandatory_fixture() {
    let Ok(binary) = env::var("CRDT_CANDIDATE_BIN") else {
        eprintln!("SKIP: CRDT_CANDIDATE_BIN is absent; dedicated candidate evidence must set it");
        return;
    };
    let binary = Path::new(&binary);
    let binary = if binary.is_absolute() {
        binary.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(binary)
    };
    let mut executed = 0;
    for path in fixture_paths() {
        let fixture = load_fixture(&path).unwrap();
        let invocation = invoke(&binary, &fixture.request).unwrap();
        assert_fixture(&fixture, &invocation.response)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        executed += 1;
    }
    assert_eq!(executed, 10, "all mandatory scenarios must execute");
    println!("executed_mandatory={executed}");
}
