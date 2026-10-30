#!/usr/bin/env python3
"""Collect read-only Android GPU evidence; never infer game compatibility."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import sys


PROPERTIES = (
    "ro.product.manufacturer", "ro.product.model", "ro.soc.manufacturer",
    "ro.soc.model", "ro.hardware", "ro.board.platform", "ro.product.cpu.abilist",
    "ro.build.version.release", "ro.build.version.sdk", "ro.build.version.incremental",
)
FEATURES = (
    "textureCompressionBC", "textureCompressionASTC_LDR", "textureCompressionETC2",
    "geometryShader", "tessellationShader", "logicOp", "multiViewport",
    "shaderInt16", "shaderInt64", "fragmentStoresAndAtomics",
    "vertexPipelineStoresAndAtomics", "shaderStorageImageReadWithoutFormat",
    "shaderStorageImageWriteWithoutFormat",
)
EXTENSIONS = (
    "VK_EXT_transform_feedback", "VK_EXT_custom_border_color",
    "VK_EXT_robustness2", "VK_EXT_extended_dynamic_state",
    "VK_EXT_extended_dynamic_state2", "VK_EXT_extended_dynamic_state3",
    "VK_EXT_vertex_input_dynamic_state",
    "VK_EXT_vertex_attribute_divisor", "VK_KHR_shader_float16_int8",
    "VK_KHR_synchronization2",
)


def capture(adb: list[str], arguments: list[str]) -> dict:
    try:
        result = subprocess.run(adb + arguments, capture_output=True, text=True,
                                errors="replace", timeout=20, check=False)
        return {"returncode": result.returncode, "stdout": result.stdout.strip(),
                "stderr": result.stderr.strip()}
    except subprocess.TimeoutExpired:
        return {"returncode": None, "stdout": "", "stderr": "Probe timed out after 20s"}


def choose_device(adb: str, serial: str | None) -> list[str]:
    result = capture([adb], ["devices"])
    if result["returncode"] != 0:
        raise ValueError("ADB device enumeration failed: " + result["stderr"])
    states = {}
    for line in result["stdout"].splitlines():
        fields = line.split()
        if len(fields) >= 2 and not line.startswith("List of devices"):
            states[fields[0]] = fields[1]
    if serial:
        if states.get(serial) != "device":
            raise ValueError("Selected Android device is not connected and authorized")
        return [adb, "-s", serial]
    available = [identifier for identifier, state in states.items() if state == "device"]
    if not available:
        raise ValueError("No authorized Android device is connected. Connect and authorize one first.")
    if len(available) > 1:
        raise ValueError("Multiple Android devices are connected; select one using --serial")
    return [adb, "-s", available[0]]


def version_string(value: object) -> str | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return f"{(value >> 22) & 127}.{(value >> 12) & 1023}.{value & 4095}"


def summarize_vulkan(document: object) -> list[dict]:
    if not isinstance(document, dict) or not isinstance(document.get("devices"), list):
        raise ValueError("Vulkan JSON must contain a devices array (Android cmd gpu vkjson format)")
    summaries = []
    for device in document["devices"]:
        if not isinstance(device, dict):
            continue
        properties = device.get("properties", {})
        features = device.get("features", {})
        if not isinstance(properties, dict) or not properties.get("deviceName"):
            continue
        if not isinstance(features, dict):
            features = {}
        extensions = device.get("extensions")
        extension_names = None
        if isinstance(extensions, list) and all(isinstance(item, dict)
                and isinstance(item.get("extensionName"), str) for item in extensions):
            extension_names = {item["extensionName"] for item in extensions
                               if isinstance(item, dict)
                               and isinstance(item.get("extensionName"), str)}
        reported_features = {}
        for name in FEATURES:
            value = features.get(name)
            reported_features[name] = bool(value) if type(value) in (bool, int) else None
        summaries.append({
            "device_name": properties["deviceName"],
            "vendor_id": properties.get("vendorID"),
            "device_id": properties.get("deviceID"),
            "device_type": properties.get("deviceType"),
            "api_version": version_string(properties.get("apiVersion")),
            "driver_version_raw": properties.get("driverVersion"),
            "features": reported_features,
            "extensions": {name: None if extension_names is None else name in extension_names
                           for name in EXTENSIONS},
        })
    if not summaries:
        raise ValueError("Vulkan JSON contains no physical device with a deviceName")
    return summaries


def collect_live(args: argparse.Namespace) -> tuple[dict, object | None, str]:
    adb = choose_device(args.adb, args.serial)
    probes = {name: capture(adb, ["shell", "getprop", name]) for name in PROPERTIES}
    probes["memory"] = capture(adb, ["shell", "cat", "/proc/meminfo"])
    vulkan = capture(adb, ["shell", "cmd", "gpu", "vkjson"])
    probes["vulkan"] = vulkan
    report = {"device_properties": {name: probes[name]["stdout"]
                                    if probes[name]["returncode"] == 0 else None
                                    for name in PROPERTIES},
              "probes": probes, "warnings": []}
    document = None
    if vulkan["returncode"] == 0:
        try:
            document = json.loads(vulkan["stdout"])
        except json.JSONDecodeError:
            report["warnings"].append("System Vulkan probe did not return valid JSON")
    else:
        report["warnings"].append("System Vulkan probe failed; inspect the probe error")
    if args.package:
        report["package"] = args.package
        package_info = capture(adb, ["shell", "dumpsys", "package", args.package])
        package_info["stdout"] = "\n".join(line.strip()
            for line in package_info["stdout"].splitlines()
            if re.match(r"\s*version(?:Code|Name)=", line))
        probes["package_version"] = package_info
        probes["package_memory"] = capture(adb, ["shell", "dumpsys", "meminfo", args.package])
        probes["package_exit_history"] = capture(adb, ["shell", "dumpsys", "activity",
                                                       "exit-info", args.package])
        if args.logcat:
            pid = capture(adb, ["shell", "pidof", "-s", args.package])
            if pid["returncode"] == 0 and re.fullmatch(r"[1-9][0-9]*", pid["stdout"]):
                probes["package_logcat"] = capture(adb, ["logcat", "-d", "-v", "threadtime",
                    "--pid=" + pid["stdout"], "-t", "1000"])
            else:
                report["warnings"].append("Emulator process is not running; logcat was not collected")
    return report, document, vulkan["stdout"]


def markdown_report(report: dict) -> str:
    lines = ["# Android GPU evidence", "", f"Captured: {report['captured_at']}", "",
             "This is reported capability data, not a gameplay test. The system Vulkan",
             "probe may differ from a custom driver loaded by the emulator.",
             "An absent extension name does not rule out equivalent core functionality.", ""]
    if "package" in report:
        lines += [f"Package: {report['package']}", "",
                  "Package version, process memory and Android exit history are in report.json.",
                  "Exit history can contain older runs; match its timestamps to the failure.", ""]
    for name, value in report.get("device_properties", {}).items():
        lines.append(f"- {name}: {value or 'not reported'}")
    for device in report["vulkan_devices"]:
        lines += ["", f"## {device['device_name']}", "",
                  f"- Vulkan: {device['api_version'] or 'not reported'}",
                  f"- Raw driver version: {device['driver_version_raw']}"]
        if device["device_type"] == 4:
            lines.append("- Software/CPU device: cannot validate a physical Mali GPU fix.")
        lines += ["", "| Feature / extension | Reported support |", "| --- | --- |"]
        for name, value in (device["features"] | device["extensions"]).items():
            state = "unknown" if value is None else "yes" if value else "no"
            lines.append(f"| {name} | {state} |")
    for warning in report["warnings"]:
        lines += ["", f"Warning: {warning}"]
    for name, probe in report.get("probes", {}).items():
        if probe["returncode"] != 0:
            lines += ["", f"Probe failed: {name}. See report.json for details."]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", default="adb", help="Path to Android platform-tools adb")
    parser.add_argument("--serial", help="Select an authorized device when several are connected")
    parser.add_argument("--package", help="Emulator Android package name")
    parser.add_argument("--logcat", action="store_true", help="Include recent logs for the running package PID")
    parser.add_argument("--vkjson", type=Path, help="Analyze existing Vulkan JSON without contacting a device")
    parser.add_argument("--output", required=True, type=Path, help="New report directory; never overwritten")
    args = parser.parse_args()
    if args.package and not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+", args.package):
        parser.error("--package must be a valid Android package name")
    if args.logcat and not args.package:
        parser.error("--logcat requires --package; full-device logs are not collected")
    if args.vkjson and (args.serial or args.package or args.logcat):
        parser.error("--vkjson is offline and cannot be combined with device or package options")
    try:
        if args.output.exists():
            raise ValueError("Output directory already exists; choose a new --output path")
        if args.vkjson:
            raw = args.vkjson.read_text(encoding="utf-8")
            document = json.loads(raw)
            report = {"warnings": [], "mode": "offline"}
            report["vulkan_devices"] = summarize_vulkan(document)
        else:
            report, document, raw = collect_live(args)
            report["mode"] = "device"
            report["vulkan_devices"] = []
            if document is not None:
                try:
                    report["vulkan_devices"] = summarize_vulkan(document)
                except ValueError as error:
                    report["warnings"].append(str(error))
        report["schema_version"] = 1
        report["captured_at"] = datetime.now(timezone.utc).isoformat()
        args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        (args.output / "summary.md").write_text(markdown_report(report), encoding="utf-8")
        (args.output / "vulkan-probe.txt").write_text(raw + "\n", encoding="utf-8")
        print(f"Report saved to {args.output}")
        if not report["vulkan_devices"]:
            print("Vulkan capabilities are unavailable; this is a partial report.", file=sys.stderr)
            return 3
        return 0
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
