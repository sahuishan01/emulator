#!/usr/bin/env python3
"""Verify our APK signature, intended ELF architectures, and 16 KiB library layout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import zipfile


def verify(apk, signer):
    subprocess.run([str(signer), "verify", str(apk)], check=True)
    raw = apk.read_bytes()
    expected = {"lib/arm64-v8a/libemulator_gpu_lab.so": 183,
                "lib/x86_64/libemulator_gpu_lab.so": 62}
    libraries = []
    with zipfile.ZipFile(apk) as archive:
        assert {name for name in archive.namelist() if name.endswith(".so")} == set(expected), "Unexpected native libraries"
        for name, machine in expected.items():
            entry = archive.getinfo(name)
            assert entry.compress_type == zipfile.ZIP_STORED, "Native library must be uncompressed"
            assert entry.file_size <= 64 * 1024 * 1024, "Unexpected library size"
            offset = entry.header_offset
            assert raw[offset:offset + 4] == b"PK\x03\x04", "Invalid local ZIP header"
            filename_size, extra_size = struct.unpack_from("<HH", raw, offset + 26)
            payload_offset = offset + 30 + filename_size + extra_size
            assert payload_offset % 16384 == 0, "Native library ZIP payload is not 16 KiB aligned"
            data = archive.read(name)
            assert len(data) >= 64 and data[:6] == b"\x7fELF\x02\x01", "Expected ELF64 little-endian library"
            assert struct.unpack_from("<H", data, 16)[0] == 3, "Expected shared object"
            assert struct.unpack_from("<H", data, 18)[0] == machine, "Incorrect native architecture"
            table = struct.unpack_from("<Q", data, 32)[0]
            stride, count = struct.unpack_from("<HH", data, 54)
            assert stride >= 56 and table + stride * count <= len(data), "Invalid program header table"
            loads = 0
            for index in range(count):
                kind, _, file_offset, address, _, file_size, memory_size, alignment = struct.unpack_from("<IIQQQQQQ", data, table + index * stride)
                if kind != 1:
                    continue
                loads += 1
                assert alignment >= 16384 and alignment & (alignment - 1) == 0, "ELF LOAD alignment is too small"
                assert file_offset % 16384 == address % 16384, "ELF LOAD alignment congruence failed"
                assert file_offset + file_size <= len(data) and memory_size >= file_size, "Invalid ELF LOAD bounds"
            assert loads, "No ELF load segments"
            section_table = struct.unpack_from("<Q", data, 40)[0]
            section_stride, section_count = struct.unpack_from("<HH", data, 58)
            assert section_stride >= 64 and section_table + section_stride * section_count <= len(data)
            symbols = set()
            for section_index in range(section_count):
                section = struct.unpack_from("<IIQQQQIIQQ", data, section_table + section_stride * section_index)
                if section[1] != 11:  # SHT_DYNSYM
                    continue
                _, _, _, _, symbol_offset, symbol_size, string_index, _, _, symbol_stride = section
                assert string_index < section_count and symbol_stride >= 24 and symbol_size % symbol_stride == 0
                string_section = struct.unpack_from("<IIQQQQIIQQ", data, section_table + section_stride * string_index)
                string_offset, string_size = string_section[4:6]
                assert symbol_offset + symbol_size <= len(data) and string_offset + string_size <= len(data)
                for entry_offset in range(symbol_offset, symbol_offset + symbol_size, symbol_stride):
                    name_offset, info, other, defined, _, _ = struct.unpack_from("<IBBHQQ", data, entry_offset)
                    if info >> 4 not in (1, 2) or other & 3 or defined == 0:
                        continue
                    assert name_offset < string_size
                    start = string_offset + name_offset
                    end = data.index(b"\0", start, string_offset + string_size)
                    symbols.add(data[start:end].decode("ascii"))
            required = {"emulator_run_reference_guests_v2", "emulator_run_gpu_core_checks_v2",
                        "emulator_run_nv_checks_v2", "emulator_guest_clear_replay_v1",
                        "emulator_run_shader_checks_v2", "emulator_shader_draw_v1", "emulator_guest_draw_replay_v1", "emulator_demo_start_v1", "emulator_demo_step_v1", "emulator_demo_stop_v1"}
            assert required <= symbols, f"Missing engine exports: {required - symbols}"
            libraries.append({"name": name, "machine": machine, "zipPayloadOffset": payload_offset,
                              "loadSegments": loads, "minimumAlignment": 16384, "verifiedEngineExports": sorted(required)})
    return {"apk": apk.name, "sha256": hashlib.sha256(raw).hexdigest(), "signatureVerified": True,
            "libraries": libraries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    parser.add_argument("--apksigner", type=Path,
                        default=Path(os.environ.get("ANDROID_SDK_ROOT", str(Path.home() / "Android/Sdk"))) / "build-tools/34.0.0/apksigner")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    report = verify(args.apk, args.apksigner)
    text = json.dumps(report, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
