#!/usr/bin/env python3
"""Verify our APK signature, intended ELF architectures, and 16 KiB library layout."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import struct
import subprocess
import zipfile


def verify_main_app(apk, aapt):
    manifest = subprocess.run([str(aapt), "dump", "xmltree", str(apk), "AndroidManifest.xml"], check=True, capture_output=True, text=True).stdout
    badging = subprocess.run([str(aapt), "dump", "badging", str(apk)], check=True, capture_output=True, text=True).stdout
    assert "application-label:'Emulator'" in badging
    assert "launchable-activity: name='com.algosculptor.emulator.gpulab.HomeActivity'" in badging
    assert not re.search(r"E: receiver\b", manifest), "Unexpected receiver"
    components = re.split(r'\s+E: (provider|service|uses-permission) \(line=\d+\)', manifest)
    found = {'provider': [], 'service': [], 'uses-permission': []}
    for index in range(1, len(components), 2):
        kind, body = components[index], components[index+1].split('\n    E: ')[0]
        found[kind].append(body)
    permissions = {re.search(r'android:name[^\n]*="([^"]+)"', block).group(1) for block in found['uses-permission']}
    assert permissions == {'android.permission.INTERNET', 'android.permission.ACCESS_NETWORK_STATE', 'android.permission.RECEIVE_BOOT_COMPLETED', 'android.permission.REQUEST_INSTALL_PACKAGES'}, permissions
    assert len(found['provider']) == 1 and len(found['service']) == 2
    provider = found['provider'][0]
    assert 'UpdateApkProvider' in provider and 'com.algosculptor.emulator.gpulab.updates' in provider
    assert re.search(r'android:exported[^\n]*0x0\s*$', provider, re.M)
    assert re.search(r'android:grantUriPermissions[^\n]*0xffffffff\s*$', provider, re.M)
    for service_name in ('UpdateJobService', 'ReportUploadJobService'):
        services = [s for s in found['service'] if service_name in s]
        assert len(services) == 1 and 'android.permission.BIND_JOB_SERVICE' in services[0]
        assert re.search(r'android:exported[^\n]*0xffffffff\s*$', services[0], re.M)
    assert re.search(r'android:allowBackup[^\n]*0x0\s*$', manifest, re.M)
    assert re.search(r'android:usesCleartextTraffic[^\n]*0x0\s*$', manifest, re.M)
    activities = {}
    for block in re.split(r'\s+E: activity \(line=\d+\)', manifest)[1:]:
        name = re.search(r'android:name[^\n]*="([^"]+)"', block).group(1)
        exported = re.search(r'android:exported[^\n]*\(type 0x12\)(0x[0-9a-f]+)', block).group(1)
        activities[name.rsplit('.', 1)[-1]] = exported != '0x0'
    assert activities == {'HomeActivity': True, 'MainActivity': False, 'DemoActivity': False, 'ProgramActivity': False, 'SettingsActivity': False, 'TextureActivity': False, 'BatchActivity': False, 'UpdatesActivity': False, 'ReportsActivity': False}, activities
    with zipfile.ZipFile(apk) as archive:
        assert not any(n.startswith('assets/') and n.endswith(('.nro', '.apk')) for n in archive.namelist()), "QA documents leaked into main APK"
    return dict(label='Emulator', launcher='HomeActivity', exportedActivities=['HomeActivity'], permissions=sorted(permissions), updateProviderPrivate=True, updateServicePermission='android.permission.BIND_JOB_SERVICE', qaDocumentsExcluded=True)


def verify(apk, signer, main_app=False):
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
                        "emulator_run_shader_checks_v2", "emulator_shader_draw_v1", "emulator_guest_draw_replay_v1", "emulator_demo_start_v1", "emulator_demo_step_v1", "emulator_demo_stop_v1", "emulator_program_begin_v1", "emulator_program_append_v1", "emulator_program_inspect_v1", "emulator_program_start_v1", "emulator_program_step_v1", "emulator_program_close_v1", "emulator_texture_replay_v1", "emulator_indexed_replay_v1", "emulator_texture_storage_replay_v1", "emulator_depth_replay_v1", "emulator_depth_clear_replay_v1"}
            assert required <= symbols, f"Missing engine exports: {required - symbols}"
            libraries.append({"name": name, "machine": machine, "zipPayloadOffset": payload_offset,
                              "loadSegments": loads, "minimumAlignment": 16384, "verifiedEngineExports": sorted(required)})
    result = {"apk": apk.name, "sha256": hashlib.sha256(raw).hexdigest(), "signatureVerified": True,
              "libraries": libraries}
    if main_app:
        result['mainApp'] = verify_main_app(apk, signer.with_name('aapt'))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    parser.add_argument("--apksigner", type=Path,
                        default=Path(os.environ.get("ANDROID_SDK_ROOT", str(Path.home() / "Android/Sdk"))) / "build-tools/34.0.0/apksigner")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--main-app", action="store_true", help="Verify the main app launcher and exclude test components")
    args = parser.parse_args()
    report = verify(args.apk, args.apksigner, args.main_app)
    text = json.dumps(report, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
