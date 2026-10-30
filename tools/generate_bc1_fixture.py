#!/usr/bin/env python3
"""Generate/check GPU fixtures using the original tested Rust BC1 reference."""
import argparse
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    result = subprocess.run(["cargo", "run", "--offline", "--quiet", "-p", "emulator-texture",
                             "--example", "bc1_fixture"], cwd=root, capture_output=True, check=True)
    target = root / "native/gpu-lab/generated/bc1_fixture.h"
    if args.check:
        if not target.exists() or target.read_bytes() != result.stdout:
            raise SystemExit("BC1 fixture differs; regenerate with tools/generate_bc1_fixture.py")
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(result.stdout)
    print("BC1 GPU fixture matches the Rust reference decoder.")


if __name__ == "__main__":
    main()
