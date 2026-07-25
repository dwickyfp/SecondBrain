#!/usr/bin/env python3
"""Run isolated candidate processes and retain raw per-repetition evidence."""

import argparse
import hashlib
import json
import platform
import subprocess
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

CASES = [
    ("text-1k-r2", "text", 1_000, 2),
    ("text-10k-r2", "text", 10_000, 2),
    ("text-100k-r2", "text", 100_000, 2),
    ("list-move-10k-r10", "list_move", 10_000, 10),
    ("properties-10k-r100", "properties", 10_000, 100),
    ("offline-1k-r2", "offline_merge", 1_000, 2),
    ("offline-10k-r10", "offline_merge", 10_000, 10),
    ("offline-100k-r100", "offline_merge", 100_000, 100),
    ("snapshot-100k-r2", "snapshot_restore", 100_000, 2),
    ("incremental-10k-r2", "incremental_update", 10_000, 2),
    ("compacted-100k-r2", "compacted_restore", 100_000, 2),
]


def summarize(response):
    canonical = json.dumps(response, sort_keys=True, separators=(",", ":")).encode()
    return {
        "response_sha256": hashlib.sha256(canonical).hexdigest(),
        "response_summary": {key: value for key, value in response.items() if key != "observations"},
    }


def peak_rss(pid, done, result):
    peak = 0
    while not done.is_set():
        sample = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True, check=False)
        try:
            peak = max(peak, int(sample.stdout.strip()) * 1024)
        except ValueError:
            pass
        done.wait(0.002)
    result.append(peak)


def invoke(binary, request):
    started = time.perf_counter_ns()
    process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    done, rss = threading.Event(), []
    sampler = threading.Thread(target=peak_rss, args=(process.pid, done, rss), daemon=True)
    sampler.start()
    stdout, stderr = process.communicate(json.dumps(request, separators=(",", ":")).encode())
    done.set()
    sampler.join()
    record = {"wall_time_ns": time.perf_counter_ns() - started, "peak_rss_bytes": rss[0] if rss else 0, "stdout_bytes": len(stdout), "exit_code": process.returncode}
    if process.returncode == 0:
        try:
            record.update(summarize(json.loads(stdout)))
        except json.JSONDecodeError as error:
            record["protocol_error"] = str(error)
    else:
        record["stderr"] = stderr.decode(errors="replace")
    return record


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release", action="store_true", required=True)
    parser.add_argument("--repetitions", type=int, default=10)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=764_230_027)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    subprocess.run(["cargo", "build", "--release", "-p", "crdt-comparison-bench", "-p", "loro-candidate"], cwd=root, check=True)
    yrs_env = {**__import__("os").environ, "RUSTC_BOOTSTRAP": "1", "RUSTFLAGS": "-Zcrate-attr=feature(if_let_guard)"}
    yrs_manifest = root / "spikes/crdt-comparison/yrs-candidate/Cargo.toml"
    subprocess.run(["cargo", "build", "--release", "--manifest-path", str(yrs_manifest)], cwd=root, env=yrs_env, check=True)
    generator = root / "target/release/crdt-comparison-bench"
    candidates = {"loro": root / "target/release/loro-candidate", "yrs": root / "spikes/crdt-comparison/yrs-candidate/target/release/yrs-candidate"}
    runs = []
    for case_index, (name, workload, operations, replicas) in enumerate(CASES):
        seed = args.seed + case_index
        request = json.loads(subprocess.check_output([str(generator), name, workload, str(operations), str(replicas), str(seed)]))
        for candidate, binary in candidates.items():
            for repetition in range(args.repetitions):
                runs.append({"candidate": candidate, "case": name, "workload": workload, "operations": operations, "replicas": replicas, "seed": seed, "repetition": repetition, **invoke(binary, request)})
    evidence = {"schema": "secondbrain-crdt-benchmark-v1", "generated_at": datetime.now(timezone.utc).isoformat(), "repetitions": args.repetitions, "base_seed": args.seed, "host": {"platform": platform.platform(), "machine": platform.machine(), "python": platform.python_version()}, "cases": [{"name": n, "workload": w, "operations": o, "replicas": r} for n, w, o, r in CASES], "runs": runs}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
