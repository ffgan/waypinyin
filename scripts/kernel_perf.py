#!/usr/bin/env python3
"""Compare kernel_perf binaries with isolated settings and a private image copy.

Only the private copy receives POSIX_FADV_DONTNEED; global caches are untouched.
Timings are descriptive local samples, not an accuracy or performance gate.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, help="disk-backed directory for cold-cache tests")
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--iterations", type=int, default=1000)
    parser.add_argument("--dictionaries", type=int, default=100)
    args = parser.parse_args()
    if args.runs < 1 or args.iterations < 1 or args.dictionaries < 0:
        parser.error("runs/iterations must be positive; dictionaries cannot be negative")
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    with tempfile.TemporaryDirectory(prefix="waypinyin-perf-", dir=args.work_dir) as temporary:
        root = Path(temporary).resolve()
        config = root / "config/waypinyin"
        dictionaries = root / "data/waypinyin/dictionaries"
        config.mkdir(parents=True)
        dictionaries.mkdir(parents=True)
        (config / "config.toml").write_text("enabled = true\n")
        for index in range(args.dictionaries):
            (dictionaries / f"{index}.tsv").write_text("ni\t你\t0\n")
        image = root / "lexicon.bin"
        shutil.copyfile(args.image, image)
        env = dict(os.environ, WAYPINYIN_LEXICON=str(image),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        results = {phase: {name: [] for name in binaries}
                   for phase in ("warm_startup", "cold_startup", "input")}
        with image.open("rb") as file:
            os.fsync(file.fileno())
            for phase in results:
                for run in range(args.runs):
                    # Alternate order to reduce drift bias.
                    order = list(binaries) if run % 2 == 0 else list(reversed(binaries))
                    for name in order:
                        if phase == "cold_startup":
                            os.posix_fadvise(file.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
                        elif phase == "warm_startup":
                            # Prime the page cache outside the timed child.
                            while file.read(1024 * 1024):
                                pass
                            file.seek(0)
                        mode = "input" if phase == "input" else "startup"
                        output = subprocess.check_output(
                            [str(binaries[name]), mode, str(args.iterations)],
                            env=env, text=True, timeout=120)
                        row = {key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", output)}
                        results[phase][name].append(row)
        summary = {
            phase: {
                name: {key: statistics.median(row[key] for row in rows)
                       for key in rows[0]}
                for name, rows in variants.items()
            }
            for phase, variants in results.items()
        }
        print(json.dumps({"runs": args.runs, "image_bytes": image.stat().st_size,
                          "dictionaries": args.dictionaries,
                          "cold_faults_observed": all(row["major_faults"] > 0
                              for rows in results["cold_startup"].values() for row in rows),
                          "median": summary, "samples": results}, indent=2))


if __name__ == "__main__":
    main()
