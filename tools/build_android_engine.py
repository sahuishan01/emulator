#!/usr/bin/env python3
"""Build the original Rust reference engine for the two Android lab ABIs."""
import os
from pathlib import Path
import platform
import shutil
import subprocess

TOOLCHAIN = "1.85.1"
NDK_VERSION = "28.2.13676358"


def main():
    if platform.system() != "Linux":
        raise SystemExit("This cross-build helper currently supports Linux hosts.")
    root = Path(__file__).resolve().parent.parent
    sdk = Path(os.environ.get("ANDROID_SDK_ROOT", os.environ.get("ANDROID_HOME", str(Path.home() / "Android/Sdk"))))
    bin_dir = sdk / "ndk" / NDK_VERSION / "toolchains/llvm/prebuilt/linux-x86_64/bin"
    if not shutil.which("rustup"):
        raise SystemExit("rustup is required for the pinned Android Rust toolchain")
    subprocess.run(["rustup", "run", TOOLCHAIN, "rustc", "--version"], check=True)
    compiler = subprocess.check_output(["rustup", "which", "--toolchain", TOOLCHAIN, "rustc"], text=True).strip()
    output = root / "target/android-engine"
    for abi, target, clang in [("arm64-v8a", "aarch64-linux-android", "aarch64-linux-android31-clang"),
                               ("x86_64", "x86_64-linux-android", "x86_64-linux-android31-clang")]:
        linker = bin_dir / clang
        if not linker.exists():
            raise SystemExit(f"Missing NDK linker: {linker}")
        env = os.environ.copy()
        # Hosts can have Debian's rustc ahead of the rustup shims in PATH.
        # Pin the compiler as well as Cargo so Android std is found correctly.
        env["RUSTC"] = compiler
        env["CARGO_TARGET_" + target.upper().replace("-", "_") + "_LINKER"] = str(linker)
        subprocess.run(["rustup", "run", TOOLCHAIN, "cargo", "build", "--offline", "--release", "--jobs", "2",
                        "--target", target, "--target-dir", str(output / "cargo"), "-p", "emulator-android-ffi"],
                       cwd=root, env=env, check=True)
        destination = output / abi
        destination.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(output / "cargo" / target / "release/libemulator_android_ffi.a",
                        destination / "libemulator_android_ffi.a")
        print(f"Original Rust engine built for {abi}.", flush=True)


if __name__ == "__main__":
    main()
