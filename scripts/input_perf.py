#!/usr/bin/env python3
"""Compare release input_latency probes using identical, isolated user data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile


def run(binary, mode, env):
    return subprocess.check_output([str(binary), mode], env=env, text=True, timeout=180)


def metrics(output):
    result = {}
    for line in output.splitlines():
        if line.startswith("slowest "):
            continue
        group, fields = line.split(" ", 1)
        result[group] = {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", fields)}
    return result


def compare_traces(baselines, after):
    old = [text.splitlines() for text in baselines]
    new = after.splitlines()
    if any(len(lines) != len(new) for lines in old):
        raise RuntimeError("trace lengths differ")
    unstable = []
    for offset, candidate in enumerate(new):
        variants = sorted({lines[offset] for lines in old})
        if candidate not in variants:
            raise RuntimeError(f"unobserved trace variant at line {offset + 1}:\n"
                               f"{variants[0][:600]}\n{candidate[:600]}")
        if len(variants) > 1:
            unstable.append({"line": offset + 1, "case": candidate.split("\t", 2)[:2],
                             "before": variants, "after": candidate})
    return {"lines": len(new), "stable_lines": len(new) - len(unstable),
            "baseline_variants": unstable}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--baseline-traces", type=int, default=3)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.runs < 1 or args.baseline_traces < 2:
        parser.error("runs must be positive and baseline-traces must be at least two")
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    with tempfile.TemporaryDirectory(prefix="waypinyin-input-") as temporary:
        root = Path(temporary)
        config = root / "config/waypinyin"
        data = root / "data/waypinyin/dictionaries"
        config.mkdir(parents=True)
        data.mkdir(parents=True)
        (config / "config.toml").write_text("enabled = true\n")
        (data / "probe.tsv").write_text("nihao\t你好呀\t0\nceshi\t自定义测试短语\t1\nzan\t测试音组\t0\n")
        env = dict(os.environ, WAYPINYIN_LEXICON=str(args.image.resolve()),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        baselines = [run(binaries["before"], "trace", env) for _ in range(args.baseline_traces)]
        after = run(binaries["after"], "trace", env)
        comparison = compare_traces(baselines, after)
        print(f"trace: {comparison['stable_lines']} stable lines, "
              f"{len(comparison['baseline_variants'])} observed baseline variants", flush=True)
        # Prime code/data caches in both variants, outside measured samples.
        for binary in binaries.values():
            run(binary, "bench", env)
        samples = {name: [] for name in binaries}
        for iteration in range(args.runs):
            order = list(binaries) if iteration % 2 == 0 else list(reversed(binaries))
            for name in order:
                output = run(binaries[name], "bench", env)
                samples[name].append({"metrics": metrics(output), "output": output})
            print(f"completed pair {iteration + 1}/{args.runs}", flush=True)
        medians = {
            name: {group: {field: statistics.median(s["metrics"][group][field] for s in rows)
                           for field in rows[0]["metrics"][group]}
                   for group in rows[0]["metrics"]}
            for name, rows in samples.items()
        }
        result = {"runs": args.runs, "baseline_traces": args.baseline_traces, "trace": comparison, "median": medians, "samples": samples,
                  "image_sha256": hashlib.sha256(args.image.read_bytes()).hexdigest(),
                  "probe_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest()
                                   for name, path in binaries.items()}}
        if args.output:
            args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
        print(json.dumps(medians, indent=2))


if __name__ == "__main__":
    main()
