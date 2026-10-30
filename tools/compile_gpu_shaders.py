#!/usr/bin/env python3
"""Compile and validate our diagnostic shaders; generate aligned SPIR-V words."""
import argparse
from pathlib import Path
import struct
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent / "native" / "gpu-lab"
    lines = ["// Generated from our GLSL sources by tools/compile_gpu_shaders.py.",
             "#pragma once", "#include <array>", "#include <cstdint>", "namespace shaders {"]
    with tempfile.TemporaryDirectory(prefix="emulator-spirv-") as temporary:
        for source in sorted((root / "shaders").iterdir()):
            output = Path(temporary) / "shader.spv"
            subprocess.run(["glslangValidator", "-V", "--target-env", "vulkan1.1", str(source), "-o", str(output)], check=True)
            subprocess.run(["spirv-val", "--target-env", "vulkan1.1", str(output)], check=True)
            data = output.read_bytes()
            words = struct.unpack("<" + "I" * (len(data) // 4), data)
            name = source.name.replace(".", "_")
            lines.append(f"inline constexpr std::array<std::uint32_t, {len(words)}> {name} = {{")
            for offset in range(0, len(words), 8):
                lines.append("    " + ", ".join(f"0x{word:08x}" for word in words[offset:offset + 8]) + ",")
            lines.append("};")
    lines.append("} // namespace shaders")
    text = "\n".join(lines) + "\n"
    target = root / "generated" / "shaders.h"
    if args.check:
        if not target.exists() or target.read_text() != text:
            raise SystemExit("Generated shader header differs; regenerate it with this tool")
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
    print("Shader compilation and SPIR-V validation passed.")


if __name__ == "__main__":
    main()
