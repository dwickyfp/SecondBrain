#!/usr/bin/env python3
"""Generate and benchmark deterministic Obsidian-compatible Phase 0 vaults."""

import argparse
import hashlib
import json
import subprocess
import tempfile
import time
from pathlib import Path


def run(binary, *args):
    started = time.perf_counter()
    completed = subprocess.run([str(binary), *map(str, args), "--json"], capture_output=True, check=False)
    elapsed_ms = (time.perf_counter() - started) * 1000
    payload = json.loads(completed.stdout) if completed.stdout else None
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.decode() or repr(payload))
    response_bytes = json.dumps(payload, sort_keys=True).encode()
    summary = payload
    if isinstance(payload, dict) and "notes" in payload:
        summary = {
            key: payload.get(key)
            for key in [
                "considered", "adopted", "merged", "reviews_required", "absent",
                "unchanged", "index_refreshed", "indexed", "broken_links",
            ]
            if key in payload
        }
    return {
        "elapsed_ms": elapsed_ms,
        "response_sha256": hashlib.sha256(response_bytes).hexdigest(),
        "response_summary": summary,
    }


def source_hash(root):
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        if path.is_file() and ".secondbrain" not in path.parts:
            digest.update(path.relative_to(root).as_posix().encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()


def generate(root, count):
    (root / ".obsidian").mkdir(parents=True)
    (root / ".obsidian/app.json").write_text('{"showLineNumber":true}\n')
    notes = root / "notes"
    notes.mkdir()
    for index in range(count):
        target = (index + 1) % count
        (notes / f"note-{index:05}.md").write_text(
            f"# Note {index}\n\nbenchmark-canary-{index} links to [[note-{target:05}]].\n\n- [ ] task {index}\n"
        )


def benchmark(binary, count):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        generate(root, count)
        original = source_hash(root)
        stages = {
            "init": run(binary, "init", root),
            "index": run(binary, "index", "rebuild", root),
            "search": run(binary, "search", root, f"benchmark-canary-{count // 2}"),
        }
        note = root / "notes/note-00000.md"
        note.write_text(note.read_text().replace("task 0", "task 0 externally edited"))
        stages["reconcile"] = run(binary, "reconcile", root)
        stages["reconcile_noop"] = run(binary, "reconcile", root)
        index_path = root / ".secondbrain/index.sqlite"
        index_bytes = index_path.stat().st_size
        index_path.unlink()
        stages["rebuild_after_delete"] = run(binary, "index", "rebuild", root)
        return {
            "notes": count,
            "source_hash_before": original,
            "source_hash_after": source_hash(root),
            "index_bytes": index_bytes,
            "stages": stages,
        }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--counts", type=int, nargs="+", default=[1000, 10000])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    evidence = {"schema": "secondbrain-phase0-benchmark-v1", "runs": []}
    for count in args.counts:
        evidence["runs"].append(benchmark(args.binary, count))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
