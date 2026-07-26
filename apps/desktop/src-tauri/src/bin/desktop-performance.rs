#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use secondbrain_desktop::{open_workspace_at, read_note_at, search_workspace_at};
use secondbrain_vault::initialize_workspace;
use serde::Serialize;

const SCHEMA: &str = "secondbrain-desktop-performance-v1";
const GENERATOR: &str = "secondbrain-deterministic-vault-v1";

#[derive(Serialize)]
struct Evidence {
    schema: &'static str,
    generated_at_unix_seconds: u64,
    classification: String,
    provenance: Provenance,
    command: CommandEvidence,
    fixture: Fixture,
    environment: Environment,
    methodology: Methodology,
    measurements: Measurements,
}

#[derive(Serialize)]
struct CommandEvidence {
    program: &'static str,
    args: Vec<String>,
    environment: Vec<(&'static str, String)>,
}

#[derive(Serialize)]
struct Provenance {
    git_revision: String,
    dirty: bool,
    diff_blake3: String,
    evidence_mutability: &'static str,
    cargo_lock_blake3: String,
    npm_lock_blake3: String,
    performance_budget_blake3: String,
    source_blake3: String,
}

#[derive(Serialize)]
struct Fixture {
    generator: &'static str,
    notes: usize,
    bytes: u64,
    content_blake3: String,
}

#[derive(Serialize)]
struct Environment {
    label: String,
    os: &'static str,
    arch: &'static str,
    rustc: String,
    cpu: String,
    logical_cpus: usize,
    tool_blake3: String,
    environment_blake3: String,
}

#[derive(Serialize)]
struct Methodology {
    timing_clock: &'static str,
    percentile: &'static str,
    first_index: &'static str,
    indexed_startup: &'static str,
    search: &'static str,
    open_note: &'static str,
    memory: &'static str,
    native_app_measured: bool,
}

#[derive(Serialize)]
struct Measurements {
    first_index_us: Distribution,
    indexed_startup_us: Distribution,
    search_us: Distribution,
    open_note_us: Distribution,
    backend_process_rss_bytes: Option<Distribution>,
}

#[derive(Serialize)]
struct Distribution {
    unit: &'static str,
    samples: Vec<u64>,
    p50: u64,
    p95: u64,
    p99: u64,
}

fn main() {
    let mut args = env::args().skip(1);
    let notes = number(&mut args, "note count");
    let first_samples = number(&mut args, "first-index sample count");
    let operation_samples = number(&mut args, "operation sample count");
    assert!(notes > 0, "note count must be positive");
    assert!(
        first_samples >= 2,
        "at least 2 first-index samples are required"
    );
    assert!(
        operation_samples >= 20,
        "at least 20 operation samples are required"
    );
    assert!(args.next().is_none(), "unexpected arguments");

    let label = env::var("SECONDBRAIN_BENCH_ENV").unwrap_or_else(|_| "unlabeled-local".into());
    let classification = env::var("SECONDBRAIN_BENCH_CLASSIFICATION")
        .unwrap_or_else(|_| "local-non-reference".into());
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let git_revision = command_in(&repository, "git", &["rev-parse", "HEAD"]);
    let git_status = command_in(&repository, "git", &["status", "--porcelain"]);
    let git_diff = command_bytes(&repository, "git", &["diff", "--binary"]);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let parent = env::temp_dir().join(format!(
        "secondbrain-performance-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&parent).expect("create benchmark directory");

    let mut first_index = Vec::with_capacity(first_samples);
    let mut rss = Vec::new();
    for sample in 0..first_samples {
        let root = parent.join(format!("first-{sample}"));
        generate_fixture(&root, notes);
        first_index.push(timed(|| {
            black_box(open_workspace_at(black_box(&root)).expect("first index"));
        }));
        sample_rss(&mut rss);
        fs::remove_dir_all(root).expect("remove first-index fixture");
    }

    let root = parent.join("indexed");
    let fixture = generate_fixture(&root, notes);
    open_workspace_at(&root).expect("prepare indexed fixture");
    let mut indexed_startup = Vec::with_capacity(operation_samples);
    let mut search = Vec::with_capacity(operation_samples);
    let mut open_note = Vec::with_capacity(operation_samples);
    for sample in 0..operation_samples {
        indexed_startup.push(timed(|| {
            black_box(open_workspace_at(black_box(&root)).expect("indexed startup"));
        }));
        let query = format!("canary{:05}", sample % notes);
        search.push(timed(|| {
            let hits = search_workspace_at(black_box(&root), black_box(&query)).expect("search");
            assert_eq!(hits.len(), 1, "fixture query must have one hit");
        }));
        let path = format!("notes/note-{:05}.md", sample % notes);
        open_note.push(timed(|| {
            let note = read_note_at(black_box(&root), black_box(&path)).expect("open note");
            black_box(note);
        }));
        sample_rss(&mut rss);
    }

    let executable = env::current_exe().expect("current executable");
    let rustc = command_output("rustc", &["--version"]);
    let cpu = cpu_name();
    let environment_material = format!(
        "{}|{}|{}|{}|{}|{}",
        label,
        env::consts::OS,
        env::consts::ARCH,
        rustc,
        cpu,
        std::thread::available_parallelism().map_or(1, usize::from)
    );
    let evidence = Evidence {
        schema: SCHEMA,
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        classification: classification.clone(),
        provenance: Provenance {
            git_revision,
            dirty: !git_status.is_empty(),
            diff_blake3: hash_bytes(&git_diff),
            evidence_mutability: if git_status.is_empty() {
                "immutable-source"
            } else {
                "mutable-local-non-immutable-not-completion"
            },
            cargo_lock_blake3: hash_file(&repository.join("Cargo.lock")),
            npm_lock_blake3: hash_file(&repository.join("apps/desktop/package-lock.json")),
            performance_budget_blake3: hash_file(
                &repository.join("apps/desktop/performance-budgets.json"),
            ),
            source_blake3: hash_file(
                &repository.join("apps/desktop/src-tauri/src/bin/desktop-performance.rs"),
            ),
        },
        command: CommandEvidence {
            program: "cargo",
            args: vec![
                "run".into(),
                "--release".into(),
                "--locked".into(),
                "--manifest-path".into(),
                "apps/desktop/src-tauri/Cargo.toml".into(),
                "--features".into(),
                "benchmark-binaries".into(),
                "--bin".into(),
                "desktop-performance".into(),
                "--".into(),
                notes.to_string(),
                first_samples.to_string(),
                operation_samples.to_string(),
            ],
            environment: vec![
                ("SECONDBRAIN_BENCH_ENV", label.clone()),
                ("SECONDBRAIN_BENCH_CLASSIFICATION", classification.clone()),
            ],
        },
        fixture,
        environment: Environment {
            label,
            os: env::consts::OS,
            arch: env::consts::ARCH,
            rustc,
            cpu,
            logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
            tool_blake3: hash_bytes(&fs::read(executable).expect("read benchmark executable")),
            environment_blake3: hash_bytes(environment_material.as_bytes()),
        },
        methodology: Methodology {
            timing_clock: "std::time::Instant wall time; fixture generation excluded",
            percentile: "nearest-rank: sorted[ceil(p*n)-1]",
            first_index: "fresh initialized deterministic vault per raw sample; production open_workspace_at includes missing-index health check and rebuild",
            indexed_startup: "repeated production open_workspace_at on unchanged valid index; includes full correctness-preserving path and content-hash health validation",
            search: "production search_workspace_at; opens SQLite and executes FTS5 query with exactly one expected hit",
            open_note: "production read_note_at; opens SQLite, resolves indexed identity, reads Markdown and converged-base version",
            memory: "RSS sampled after operations for this release benchmark/backend process, including loaded Rust backend and fixture-processing allocations; Linux /proc/self/status VmRSS, macOS ps -o rss= -p PID, Windows PowerShell WorkingSet64; not idle and not a packaged native application",
            native_app_measured: false,
        },
        measurements: Measurements {
            first_index_us: distribution("microseconds", first_index),
            indexed_startup_us: distribution("microseconds", indexed_startup),
            search_us: distribution("microseconds", search),
            open_note_us: distribution("microseconds", open_note),
            backend_process_rss_bytes: (!rss.is_empty()).then(|| distribution("bytes", rss)),
        },
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).expect("serialize evidence")
    );
    fs::remove_dir_all(parent).expect("remove benchmark directory");
}

fn number(args: &mut impl Iterator<Item = String>, name: &str) -> usize {
    args.next()
        .unwrap_or_else(|| panic!("missing {name}"))
        .parse()
        .unwrap_or_else(|_| panic!("invalid {name}"))
}

fn generate_fixture(root: &Path, notes: usize) -> Fixture {
    fs::create_dir_all(root.join("notes")).expect("create fixture");
    initialize_workspace(root).expect("initialize fixture");
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0;
    for index in 0..notes {
        let path = format!("notes/note-{index:05}.md");
        let next = (index + 1) % notes;
        let source = format!(
            "---\ntitle: Note {index:05}\ntags: [benchmark]\n---\n# Note {index:05}\n\nDeterministic canary{index:05} benchmark content.\n\nLinks to [[note-{next:05}]].\n"
        );
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        hasher.update(source.as_bytes());
        bytes += source.len() as u64;
        fs::write(root.join(path), source).expect("write fixture note");
    }
    Fixture {
        generator: GENERATOR,
        notes,
        bytes,
        content_blake3: hasher.finalize().to_hex().to_string(),
    }
}

fn timed(operation: impl FnOnce()) -> u64 {
    let started = Instant::now();
    operation();
    started.elapsed().as_micros().try_into().unwrap_or(u64::MAX)
}

fn distribution(unit: &'static str, samples: Vec<u64>) -> Distribution {
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    Distribution {
        unit,
        p50: percentile(&sorted, 50),
        p95: percentile(&sorted, 95),
        p99: percentile(&sorted, 99),
        samples,
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    sorted[(percentile * sorted.len()).div_ceil(100).saturating_sub(1)]
}

fn sample_rss(samples: &mut Vec<u64>) {
    if let Some(value) = process_rss_bytes() {
        samples.push(value);
    }
}

fn process_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        let kb = status
            .lines()
            .find(|line| line.starts_with("VmRSS:"))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()?;
        return kb.checked_mul(1024);
    }
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id().to_string();
        return command_output("ps", &["-o", "rss=", "-p", &pid])
            .trim()
            .parse::<u64>()
            .ok()?
            .checked_mul(1024);
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!("(Get-Process -Id {}).WorkingSet64", std::process::id());
        return command_output("powershell", &["-NoProfile", "-Command", &script])
            .trim()
            .parse()
            .ok();
    }
    #[allow(unreachable_code)]
    None
}

fn cpu_name() -> String {
    #[cfg(target_os = "macos")]
    {
        return command_output("sysctl", &["-n", "machdep.cpu.brand_string"]);
    }
    #[cfg(target_os = "linux")]
    {
        return fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("model name\t: ").map(str::to_owned))
            })
            .unwrap_or_else(|| "unknown".into());
    }
    #[cfg(target_os = "windows")]
    {
        return env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".into());
    }
    #[allow(unreachable_code)]
    "unknown".into()
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}

fn command_in(directory: &Path, program: &str, arguments: &[&str]) -> String {
    String::from_utf8(command_bytes(directory, program, arguments))
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|_| "unknown".into())
}

fn command_bytes(directory: &Path, program: &str, arguments: &[&str]) -> Vec<u8> {
    Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(|| b"unknown".to_vec(), |output| output.stdout)
}

fn hash_file(path: &Path) -> String {
    hash_bytes(&fs::read(path).unwrap_or_else(|_| b"unavailable".to_vec()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
