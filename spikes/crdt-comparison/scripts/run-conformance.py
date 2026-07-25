#!/usr/bin/env python3
"""Capture every shared fixture response without stopping at the first failure."""

import argparse
import hashlib
import json
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path


def summarize(response):
    canonical = json.dumps(response, sort_keys=True, separators=(",", ":")).encode()
    return {
        "response_sha256": hashlib.sha256(canonical).hexdigest(),
        "response_summary": {key: value for key, value in response.items() if key != "observations"},
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    fixtures = sorted((root / "spikes/crdt-comparison/fixtures").glob("*.json"))
    candidates = {name: root / f"target/release/{name}-candidate" for name in ("loro", "yrs")}
    runs = []
    for name, binary in candidates.items():
        for fixture_path in fixtures:
            request = json.loads(fixture_path.read_text())["request"]
            started = time.perf_counter_ns()
            process = subprocess.run([str(binary)], input=json.dumps(request).encode(), capture_output=True)
            run = {"candidate": name, "fixture": fixture_path.name, "wall_time_ns": time.perf_counter_ns() - started, "exit_code": process.returncode}
            if process.returncode == 0:
                run.update(summarize(json.loads(process.stdout)))
            else:
                run["stderr"] = process.stderr.decode(errors="replace")
            runs.append(run)
    evidence = {"schema": "secondbrain-crdt-conformance-v1", "generated_at": datetime.now(timezone.utc).isoformat(), "fixtures": len(fixtures), "runs": runs}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
