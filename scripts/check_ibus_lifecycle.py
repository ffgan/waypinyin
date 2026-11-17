#!/usr/bin/env python3
"""Verify IBus engine destruction on a private bus and isolated user data."""
import argparse
from pathlib import Path

from ibus_perf import run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("image", type=Path)
    parser.add_argument("probe", type=Path)
    args = parser.parse_args()
    output, _ = run(args.binary.resolve(), args.probe.resolve(), args.image.resolve(), "lifecycle")
    print(output.strip())


if __name__ == "__main__":
    main()
