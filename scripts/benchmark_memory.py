#!/usr/bin/env python3
"""Snapshot a private engine after corpus input or 100 create/destroy cycles.

Observation happens after all probe calls, outside latency measurements.
RSS/PSS are Linux /proc values in KiB; file mappings are not private heap.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import statistics

from ibus_perf import run


def snapshot(pid):
    root = Path(f"/proc/{pid}")
    status = dict(re.findall(r"^(\w+):\s+(\d+)", (root / "status").read_text(), re.M))
    rollup = dict(re.findall(r"^(\w+):\s+(\d+) kB", (root / "smaps_rollup").read_text(), re.M))
    return {**{key + "_kib": int(status[key]) for key in ("VmRSS", "VmHWM", "RssAnon", "RssFile")},
            **{key + "_kib": int(rollup[key]) for key in
               ("Pss", "Pss_Anon", "Pss_File", "Private_Clean", "Private_Dirty")},
            "threads": int(status["Threads"]), "fds": len(list((root / "fd").iterdir()))}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--engine", type=Path, required=True)
    p.add_argument("--probe", type=Path, required=True)
    p.add_argument("--image", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--runs", type=int, default=7)
    args = p.parse_args()
    if args.runs < 3:
        p.error("use at least three independent processes")
    result = {"runs": args.runs, "sha256": {key: hashlib.sha256(path.read_bytes()).hexdigest()
              for key, path in {"engine": args.engine, "probe": args.probe, "image": args.image,
                                "driver": Path(__file__), "ibus_driver": Path(__file__).with_name("ibus_perf.py")}.items()}, "scenarios": {}}
    for mode in ("bench", "lifecycle"):
        rows = []
        for i in range(args.runs):
            run(args.engine.resolve(), args.probe.resolve(), args.image.resolve(), mode,
                observe=lambda pid: rows.append(snapshot(pid)))
            print(f"{mode} {i+1}/{args.runs}", flush=True)
        result["scenarios"][mode] = {"samples": rows, "summary": {key: {
            "median": statistics.median(row[key] for row in rows),
            "min": min(row[key] for row in rows), "max": max(row[key] for row in rows)} for key in rows[0]}}
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
