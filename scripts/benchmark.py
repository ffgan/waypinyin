#!/usr/bin/env python3
"""Single-version benchmark suite. Uses private XDG directories and private buses.

Build release examples first. No desktop IBus interaction or global cache flush.
All samples are retained; summaries are medians of per-process statistics, not
pooled event percentiles. Run suites sequentially to avoid self-interference.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import tempfile
import time

from ibus_perf import run as ibus_run
from input_perf import metrics

ROOT = Path(__file__).resolve().parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fields(text):
    return {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", text)}


def summarize(rows):
    return {group: {field: {
        "median": statistics.median(row[group][field] for row in rows),
        "min": min(row[group][field] for row in rows),
        "max": max(row[group][field] for row in rows),
    } for field in rows[0][group]} for group in rows[0]}


def diagnose(probe, image, out, runs):
    result = {}
    with tempfile.TemporaryDirectory(prefix="waypinyin-diagnose-") as tmp:
        root = Path(tmp)
        (root / "config/waypinyin").mkdir(parents=True)
        (root / "config/waypinyin/config.toml").write_text("enabled = true\n")
        (root / "data/waypinyin/dictionaries").mkdir(parents=True)
        (root / "data/waypinyin/user.txt").touch(mode=0o600)
        env = dict(os.environ, WAYPINYIN_LEXICON=str(image),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        env.pop("WAYPINYIN_TRACE_LATENCY", None)
        for name, pattern in [("full_pinyin", "jintianwomenyiqiqugongyuan"),
                              ("ambiguous", "sh"),
                              ("abbreviation", "mtxwsdwmzgsmkjmtlyxgzjh")]:
            rows = []
            for i in range(runs):
                text = subprocess.check_output([str(probe), "diagnose", pattern], env=env, text=True, timeout=120)
                (out / f"diagnose-{name}-{i}.txt").write_text(text)
                rows.append([fields(line) for line in text.splitlines()])
                print(f"diagnose {name} {i+1}/{runs}", flush=True)
            result[name] = {"pattern": pattern, "samples_by_prefix": rows,
                "worst_wall": max((dict(row, prefix=index + 1, run=i) for i, rows_ in enumerate(rows)
                                   for index, row in enumerate(rows_)), key=lambda row: row["wall_ns"]),
                "worst_cpu": max((dict(row, prefix=index + 1, run=i) for i, rows_ in enumerate(rows)
                                  for index, row in enumerate(rows_)), key=lambda row: row["cpu_ns"])}
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--image", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--runs", type=int, default=7)
    p.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    p.add_argument("--skip-ibus", action="store_true")
    p.add_argument("--diagnostics-only", action="store_true", help="only collect per-key wall/thread CPU timings")
    args = p.parse_args()
    if args.runs < 3:
        p.error("use at least three independent process runs")
    image, binaries = args.image.resolve(), args.bin_dir.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    out = args.output.resolve()
    names = ["kernel_perf", "input_latency", "first_screen", "quality",
             "sentence_quality", "stress_latency", "ibus_latency"]
    probes = {name: binaries / "examples" / name for name in names}
    engine = binaries / "waypinyin"
    result = {
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "working_tree": subprocess.check_output(["git", "status", "--short"], cwd=ROOT, text=True),
        "runs": args.runs, "platform": platform.platform(),
        "cpu": subprocess.check_output(["lscpu"], text=True),
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
        "load_start": os.getloadavg(), "image_bytes": image.stat().st_size,
        "image_sha256": sha(image),
        "binaries": {k: {"path": str(v), "sha256": sha(v), "bytes": v.stat().st_size}
                     for k, v in dict(probes, engine=engine).items()},
        "source_sha256": {str(path.relative_to(ROOT)): sha(path) for path in
                          [*sorted((ROOT / "src").glob("*.rs")),
                           *sorted((ROOT / "examples").glob("*.rs")),
                           ROOT / "scripts/benchmark.py", ROOT / "Cargo.lock"]},
        "suites": {},
    }

    def save():
        (out / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")

    def log(msg):
        print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)

    if args.diagnostics_only:
        result["long_key_diagnostics"] = diagnose(probes["stress_latency"], image, out, args.runs)
        result["finished_utc"] = datetime.now(timezone.utc).isoformat()
        save()
        return

    def measured(command, env, label):
        # GNU time measures the child process including startup, unlike per-key timers.
        resource = out / f"{label}.resource.txt"
        started = time.monotonic()
        completed = subprocess.run(
            ["/usr/bin/time", "-f", "wall_s=%e user_s=%U system_s=%S maxrss_kib=%M minor_faults=%R major_faults=%F", "-o", str(resource), *map(str, command)],
            env=env, text=True, capture_output=True, check=True, timeout=300)
        (out / f"{label}.stdout.txt").write_text(completed.stdout)
        (out / f"{label}.stderr.txt").write_text(completed.stderr)
        return completed, {"wall_s": time.monotonic() - started,
                           **{k: float(v) for k, v in re.findall(r"(\w+)=([\d.]+)", resource.read_text())}}

    with tempfile.TemporaryDirectory(prefix="benchmark-", dir=ROOT / "target") as tmp:
        root = Path(tmp)
        (root / "config/waypinyin").mkdir(parents=True)
        (root / "config/waypinyin/config.toml").write_text("enabled = true\n")
        imports = root / "data/waypinyin/dictionaries"
        imports.mkdir(parents=True)
        (imports.parent / "user.txt").touch(mode=0o600)
        env = dict(os.environ, WAYPINYIN_LEXICON=str(image),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        env.pop("WAYPINYIN_TRACE_LATENCY", None)
        for name in ("quality", "sentence_quality", "input_latency", "stress_latency"):
            log(name)
            rows, resources = [], []
            # Explicit warmup outside retained latency samples.
            subprocess.run([str(probes[name])], env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, check=True, timeout=300)
            for i in range(args.runs):
                cp, usage = measured([probes[name]], env, f"{name}-{i}")
                rows.append({"quality": fields(cp.stdout)} if name == "quality" else
                            {"quality": fields(cp.stderr)} if name == "sentence_quality" else metrics(cp.stdout))
                resources.append(usage)
                log(f"{name} {i+1}/{args.runs}")
            result["suites"][name] = {"summary": summarize(rows), "samples": rows, "resources": resources}
            save()
        # Check repeatability separately; do not discard any observed variants.
        log("candidate traces")
        traces = [subprocess.check_output([str(probes["input_latency"]), "trace"], env=env, text=True, timeout=300)
                  for _ in range(3)]
        for i, trace in enumerate(traces):
            (out / f"candidate-trace-{i}.txt").write_text(trace)
        lines = [t.splitlines() for t in traces]
        result["candidate_trace"] = {"line_counts": [len(t) for t in lines],
            "variant_lines": [i + 1 for i in range(min(map(len, lines)))
                              if len({t[i] for t in lines}) > 1]}
        for gap in (0, 2):
            log(f"first page gap={gap}ms")
            rows, resources = [], []
            for i in range(args.runs):
                cp, usage = measured([probes["first_screen"], gap], env, f"first_screen-{gap}-{i}")
                rows.append(metrics(cp.stdout))
                resources.append(usage)
                log(f"first page gap={gap}ms {i+1}/{args.runs}")
            result["suites"][f"first_screen_{gap}ms"] = {"summary": summarize(rows), "samples": rows, "resources": resources}
            save()
        # A private on-disk copy receives DONTNEED. Source/global caches untouched.
        private = root / "lexicon.bin"
        shutil.copyfile(image, private)
        env["WAYPINYIN_LEXICON"] = str(private)
        with private.open("rb") as f:
            os.fsync(f.fileno())
            for phase in ("warm", "cold"):
                rows = []
                for i in range(args.runs):
                    if phase == "cold":
                        os.posix_fadvise(f.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
                    else:
                        f.seek(0)
                        while f.read(1024 * 1024):
                            pass
                    cp, _ = measured([probes["kernel_perf"], "startup"], env, f"startup-{phase}-{i}")
                    rows.append({"startup": fields(cp.stdout)})
                result["suites"][f"startup_{phase}"] = {"summary": summarize(rows), "samples": rows}
                log(f"startup {phase} complete")
                save()
        env["WAYPINYIN_LEXICON"] = str(image)
        for count in (0, 100, 1000):
            # File-count scalability; each file has only one original record.
            for index in range(count):
                (imports / f"{index}.tsv").write_text("ni\t你\t0\n")
            rows = []
            for i in range(args.runs):
                cp, _ = measured([probes["kernel_perf"], "input", 1000], env, f"imports-{count}-{i}")
                rows.append({"input": fields(cp.stdout)})
            result["suites"][f"imports_{count}"] = {"summary": summarize(rows), "samples": rows}
            log(f"import files={count} complete")
            save()
    if not args.skip_ibus:
        log("IBus signal verification and 100-engine lifecycle")
        result["ibus_verification"] = ibus_run(engine, probes["ibus_latency"], image, "verify")[0]
        result["ibus_lifecycle"] = ibus_run(engine, probes["ibus_latency"], image, "lifecycle")[0]
        for routed in (False, True):
            name = "ibus_routed" if routed else "ibus_direct"
            rows = []
            traces = []
            for i in range(2):
                trace, _ = ibus_run(engine, probes["ibus_latency"], image, "trace", routed=routed)
                (out / f"{name}-trace-{i}.txt").write_text(trace)
                traces.append(trace)
            ibus_run(engine, probes["ibus_latency"], image, "bench", routed=routed)
            for i in range(args.runs):
                text, _ = ibus_run(engine, probes["ibus_latency"], image, "bench", routed=routed)
                (out / f"{name}-{i}.stdout.txt").write_text(text)
                rows.append(metrics(text))
                log(f"{name} {i+1}/{args.runs}")
            result["suites"][name] = {"summary": summarize(rows), "samples": rows,
                "trace_lines": len(traces[0].splitlines()), "traces_equal": traces[0] == traces[1]}
            save()
        _, diagnostics = ibus_run(engine, probes["ibus_latency"], image, "bench", diagnostic=True)
        result["handler_diagnostics"] = diagnostics
    result["long_key_diagnostics"] = diagnose(probes["stress_latency"], image, out, args.runs)
    result.update(finished_utc=datetime.now(timezone.utc).isoformat(), load_end=os.getloadavg())
    save()
    log(f"saved {out / 'results.json'}")


if __name__ == "__main__":
    main()
