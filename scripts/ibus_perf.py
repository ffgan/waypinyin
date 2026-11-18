#!/usr/bin/env python3
"""Compare engines over disposable D-Bus instances, never the desktop IBus bus."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
import signal
import shlex
from pathlib import Path
import statistics
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET

from input_perf import metrics


@contextmanager
def process(command, group=False, **kwargs):
    child = subprocess.Popen(command, start_new_session=group, **kwargs)
    try:
        yield child
    finally:
        if group:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        elif child.poll() is None:
            child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            if group:
                os.killpg(child.pid, signal.SIGKILL)
            else:
                child.kill()
            child.wait(timeout=5)


def run(engine, probe, image, mode, diagnostic=False, routed=False, observe=None):
    with tempfile.TemporaryDirectory(prefix="waypinyin-ibus-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, WAYPINYIN_LEXICON=str(image),
                   XDG_CONFIG_HOME=str(root / "config"),
                   XDG_DATA_HOME=str(root / "data"),
                   XDG_CACHE_HOME=str(root / "cache"),
                   XDG_RUNTIME_DIR=str(root),
                   GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix")
        env.pop("WAYPINYIN_TRACE_LATENCY", None)
        env.pop("WAYPINYIN_PROBE_ROUTED", None)
        if diagnostic:
            env["WAYPINYIN_TRACE_LATENCY"] = "1"
        # No imports or personal history. A new directory for every engine run
        # also ensures that commits in the probe cannot change the next run.
        (root / "config/waypinyin").mkdir(parents=True)
        (root / "config/waypinyin/config.toml").write_text("enabled = true\n")
        # An existing empty destination prevents migration from the real HOME's
        # legacy journal without changing HOME or touching personal records.
        (root / "data/waypinyin").mkdir(parents=True)
        journal = root / "data/waypinyin/user.txt"
        journal.touch(mode=0o600)
        # No service directories: the disposable bus cannot activate desktop
        # portals, GVfs or other services from the user's normal session.
        config = root / "bus.conf"
        config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=' + str(root) +
                          '</listen><auth>EXTERNAL</auth><policy context="default">'
                          '<allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/>'
                          '</policy></busconfig>')
        with process(["dbus-daemon", "--config-file=" + str(config), "--nofork",
                      "--print-address=1"], stdout=subprocess.PIPE, text=True, env=env) as bus:
            address = bus.stdout.readline().strip()
            if not address:
                raise RuntimeError("private dbus-daemon did not provide an address")
            env.update(IBUS_ADDRESS=address, WAYPINYIN_PROBE_BUS=address)
            with (root / "engine.log").open("w+") as log:
                command = [str(engine)]
                if routed:
                    env.update(DBUS_SESSION_BUS_ADDRESS=address,
                               XDG_CACHE_HOME=str(root / "cache"),
                               XDG_RUNTIME_DIR=str(root),
                               WAYPINYIN_PROBE_ROUTED="1")
                    env.pop("DISPLAY", None)
                    env.pop("WAYLAND_DISPLAY", None)
                    components = root / "components"
                    components.mkdir()
                    source = Path(__file__).resolve().parent.parent / "share/waypinyin.xml"
                    component = ET.parse(source)
                    component.find("exec").text = shlex.quote(str(engine))
                    component.find("engines/engine/setup").text = ""
                    component.write(components / "waypinyin.xml")
                    env["IBUS_COMPONENT_PATH"] = str(components)
                    env["IBUS_ADDRESS"] = "unix:path=" + str(root / "ibus.sock")
                    env["WAYPINYIN_PROBE_BUS"] = env["IBUS_ADDRESS"]
                    command = ["ibus-daemon", "--single", "--panel=disable", "--config=disable",
                               "--emoji-extension=disable", "--cache=none",
                               "--address=" + env["IBUS_ADDRESS"]]
                with process(command, group=routed, env=env, stdout=subprocess.DEVNULL,
                             stderr=log) as child:
                    if routed:
                        for _ in range(100):
                            if (root / "ibus.sock").exists() or child.poll() is not None:
                                break
                            time.sleep(0.05)
                    try:
                        output = subprocess.check_output(
                            [str(probe), mode], env=env, text=True, timeout=180)
                    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                        log.seek(0)
                        raise RuntimeError(f"probe failed; engine log:\n{log.read()}") from None
                    if child.poll() is not None:
                        raise RuntimeError(f"engine exited unexpectedly: {child.returncode}")
                    if observe is not None:
                        if routed:
                            raise ValueError("process observation requires the directly launched engine")
                        observe(child.pid)
                    if mode == "lifecycle":
                        # Every destroyed session owned a journal and two watches.
                        # A live process with 100 leaked sessions would exceed this.
                        descriptors = len(list(Path(f"/proc/{child.pid}/fd").iterdir()))
                        if descriptors > 24:
                            raise RuntimeError(f"engine descriptors leaked: {descriptors}")
                        output += f"open descriptors after lifecycle test: {descriptors}\n"
                    log.flush()
                    log.seek(0)
                    diagnostics = [line.strip() for line in log
                                   if line.startswith("waypinyin: latency ")]
        return output, diagnostics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--routed", action="store_true",
                        help="include a real private ibus-daemon and InputContext routing")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    probe, image = args.probe.resolve(), args.image.resolve()
    # Inspect exact engine emission counts directly; ibus-daemon may suppress
    # redundant empty updates before forwarding them to an InputContext.
    verification, _ = run(binaries["after"], probe, image, "verify")
    print(verification.strip(), flush=True)
    lifecycle, _ = run(binaries["after"], probe, image, "lifecycle")
    print(lifecycle.strip(), flush=True)
    traces = {name: run(binary, probe, image, "trace", routed=args.routed)[0]
              for name, binary in binaries.items()}
    old, new = (traces[name].splitlines() for name in binaries)
    if old != new:
        for offset, (a, b) in enumerate(zip(old, new), 1):
            if a != b:
                raise RuntimeError(f"display/commit trace differs at line {offset}:\n{a}\n{b}")
        raise RuntimeError(f"display trace lengths differ: {len(old)} vs {len(new)}")
    print(f"trace: {len(old)} exact display/commit/reply matches", flush=True)
    samples = {name: [] for name in binaries}
    # Startup and dictionary load occur before the probe's per-call timing.
    for binary in binaries.values():
        run(binary, probe, image, "bench", routed=args.routed)
    for iteration in range(args.runs):
        order = list(binaries) if iteration % 2 == 0 else list(reversed(binaries))
        for name in order:
            output, _ = run(binaries[name], probe, image, "bench", routed=args.routed)
            samples[name].append({"metrics": metrics(output), "output": output})
        print(f"completed pair {iteration + 1}/{args.runs}", flush=True)
    medians = {
        name: {group: {field: statistics.median(row["metrics"][group][field] for row in rows)
                       for field in rows[0]["metrics"][group]}
               for group in rows[0]["metrics"]}
        for name, rows in samples.items()
    }
    # IBus may redirect spawned engine stderr. Attribute handler work on a
    # directly launched engine in this separate, untimed diagnostic run.
    _, diagnostics = run(binaries["after"], probe, image, "bench", diagnostic=True)
    result = {
        "runs": args.runs, "routed": args.routed, "verification": verification.strip(),
        "lifecycle": lifecycle.strip(),
        "trace_lines": len(old), "median": medians, "samples": samples,
        "diagnostics": diagnostics, "diagnostics_routed": False,
        "image_sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
        "image_bytes": image.stat().st_size,
        "binary_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest()
                          for name, path in dict(binaries, probe=probe).items()},
        "source_sha256": {
            name: hashlib.sha256((Path(__file__).resolve().parent.parent / name).read_bytes()).hexdigest()
            for name in ("src/ibus.rs", "src/lexicon.rs", "src/latency.rs",
                         "examples/ibus_latency.rs", "scripts/ibus_perf.py", "Cargo.lock")
        },
    }
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(medians, indent=2))


if __name__ == "__main__":
    main()
