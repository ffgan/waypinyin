#!/usr/bin/env python3
"""Compare first_screen probes with zero or a specified inter-key idle gap."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--idle-ms", type=int, default=2)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 1 or args.idle_ms < 1:
        parser.error("runs and idle-ms must be positive")
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    with tempfile.TemporaryDirectory(prefix="waypinyin-first-screen-") as temporary:
        root = Path(temporary)
        (root / "config/waypinyin").mkdir(parents=True)
        (root / "config/waypinyin/config.toml").write_text("enabled = true\n")
        (root / "data/waypinyin/dictionaries").mkdir(parents=True)
        env = dict(os.environ, WAYPINYIN_LEXICON=str(args.image.resolve()),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        samples = {str(gap): {name: [] for name in binaries} for gap in (0, args.idle_ms)}
        for gap in samples:
            for iteration in range(args.runs):
                order = list(binaries) if iteration % 2 == 0 else list(reversed(binaries))
                pair = {}
                for name in order:
                    completed = subprocess.run([str(binaries[name]), gap], env=env,
                        text=True, capture_output=True, check=True, timeout=120)
                    output = completed.stdout
                    metrics = {}
                    for line in output.splitlines():
                        group, fields = line.split(" ", 1)
                        metrics[group] = {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", fields)}
                    pair[name] = metrics
                    samples[gap][name].append({"metrics": metrics, "output": output,
                                               "diagnostics": completed.stderr})
                if pair["before"]["quality"] != pair["after"]["quality"]:
                    raise RuntimeError("first-candidate quality changed")
                print(f"gap={gap}ms completed pair {iteration+1}/{args.runs}", flush=True)
        median = {gap: {name: {group: {
            field: statistics.median(row["metrics"][group][field] for row in rows)
            for field in rows[0]["metrics"][group]}
            for group in rows[0]["metrics"]} for name, rows in variants.items()}
            for gap, variants in samples.items()}
        result = {"runs": args.runs, "idle_ms": args.idle_ms, "median": median, "samples": samples,
                  "image_sha256": hashlib.sha256(args.image.read_bytes()).hexdigest(),
                  "probe_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest()
                                   for name, path in binaries.items()}}
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(median, indent=2))


if __name__ == "__main__":
    main()
