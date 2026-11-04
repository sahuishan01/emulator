#!/usr/bin/env python3
"""Verify discoverable manual updates and disconnected access feedback via adb."""
import argparse
import json
import re
import sqlite3
import subprocess
import time
import xml.etree.ElementTree as ET
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--version', default='0.27.3-dev')
    args = parser.parse_args()
    out = args.output
    out.mkdir(parents=True, exist_ok=True)
    adb = [str(Path.home() / 'Android/Sdk/platform-tools/adb'), '-s', 'emulator-5580']
    package = 'com.algosculptor.emulator.gpulab'
    checks = []
    notes = []

    def call(*commands, timeout=35):
        return subprocess.check_output(adb + list(commands), stderr=subprocess.PIPE, timeout=timeout)

    def bounds(node):
        return tuple(map(int, re.findall(r'\d+', node.get('bounds'))))

    def ui(phase):
        for attempt in range(4):
            remote = f'/sdcard/update-controls-{phase}-{attempt}.xml'
            if b'dumped to:' not in call('shell', 'uiautomator', 'dump', remote):
                continue
            raw = call('exec-out', 'cat', remote)
            (out / f'{phase}-{attempt}.xml').write_bytes(raw)
            root = ET.fromstring(raw)
            if any(n.get('text') == "System UI isn't responding" for n in root.iter('node')):
                node = next(n for n in root.iter('node') if n.get('text') == 'Wait')
                x, y, x2, y2 = bounds(node)
                call('shell', 'input', 'touchscreen', 'swipe', str((x+x2)//2), str((y+y2)//2), str((x+x2)//2), str((y+y2)//2), '80')
                notes.append('Dismissed unrelated System UI startup dialog with Wait')
                continue
            assert not any("isn't responding" in n.get('text', '') for n in root.iter('node'))
            return root
        raise RuntimeError('Fresh UI unavailable: ' + phase)

    def find(label, phase):
        for attempt in range(6):
            root = ui(phase + '-' + str(attempt))
            nodes = [n for n in root.iter('node') if n.get('text', '').casefold() == label.casefold()]
            if nodes:
                return root, nodes[0]
            scroll = next((n for n in root.iter('node') if n.get('scrollable') == 'true'), None)
            if scroll is None:
                time.sleep(.2)
                continue
            x, y, x2, y2 = bounds(scroll)
            call('shell', 'input', 'swipe', str((x+x2)//2), str(y2-70), str((x+x2)//2), str(y+70), '250')
        raise RuntimeError('Missing control: ' + label)

    def tap(label, phase):
        root, node = find(label, phase)
        x, y, x2, y2 = bounds(node)
        assert node.get('enabled') == 'true' and node.get('clickable') == 'true'
        call('shell', 'input', 'touchscreen', 'swipe', str((x+x2)//2), str((y+y2)//2), str((x+x2)//2), str((y+y2)//2), '80')
        return root

    def shot(phase):
        (out / f'{phase}.png').write_bytes(call('exec-out', 'screencap', '-p'))

    def verified(name):
        checks.append(name)
        print('Verified', name, flush=True)

    def rows(phase):
        path = out / (phase + '.db')
        path.write_bytes(call('exec-out', 'run-as', package, 'cat', 'databases/library.db'))
        with sqlite3.connect(path) as database:
            return database.execute('select * from titles order by id').fetchall()

    try:
        assert call('shell', 'getprop', 'sys.boot_completed').strip() == b'1'
        call('shell', 'am', 'force-stop', package)
        from_version = re.search(r'versionName=([^\s]+)', call('shell', 'dumpsys', 'package', package).decode()).group(1)
        before = rows('before-update')
        report = call('exec-out', 'run-as', package, 'cat', 'files/last-report.json')
        json.loads(report)
        (out / 'before-update-report.json').write_bytes(report)
        call('install', '-r', str(args.apk), timeout=60)
        assert rows('after-update') == before
        assert call('exec-out', 'run-as', package, 'cat', 'files/last-report.json') == report
        assert f'versionName={args.version}' in call('shell', 'dumpsys', 'package', package).decode()
        verified('Signed update preserves library and existing diagnostics')
        call('logcat', '-c')
        call('shell', 'am', 'start', '-W', '-n', package + '/.HomeActivity')
        find('Updates', 'home-updates')
        shot('home-updates')
        tap('Updates', 'open-updates')
        find('GitHub: Not connected', 'disconnected-status')
        for label in ['Check for updates', 'Test GitHub access']:
            _, node = find(label, 'disconnected-' + label.split()[0])
            assert node.get('enabled') == 'true'
        find('Last successful check: Never', 'no-success-history')
        shot('disconnected-updates')
        verified('Library exposes Updates and both manual actions without a token')
        for label, phase in [('Check for updates', 'manual-connect'), ('Test GitHub access', 'access-connect')]:
            tap(label, phase)
            root, _ = find('Verify and save', phase + '-dialog')
            edit = next(n for n in root.iter('node') if n.get('class') == 'android.widget.EditText')
            assert edit.get('password') == 'true'
            tap('Verify and save', phase + '-empty')
            find('Connect private updates', phase + '-invalid-retained')
            tap('Cancel', phase + '-cancel')
        verified('Both disconnected actions open masked verification and reject an empty token')
        missing = subprocess.run(adb + ['shell', 'run-as', package, 'test', '-f', 'no_backup/updates-access.bin'], capture_output=True)
        assert missing.returncode != 0
        verified('Cancelled or empty verification stores no credential')
        call('shell', 'input', 'keyevent', '4')
        tap('Settings', 'settings-entry')
        root, node = find('App updates', 'settings-updates')
        player = next(n for n in root.iter('node') if n.get('text') == 'PLAYER')
        assert bounds(node)[1] < bounds(player)[1]
        shot('settings-updates')
        tap('App updates', 'settings-open-updates')
        verified('Settings puts App updates before player preferences')
        call('shell', 'settings', 'put', 'system', 'accelerometer_rotation', '0')
        call('shell', 'settings', 'put', 'system', 'user_rotation', '1')
        for label in ['Check for updates', 'Test GitHub access']:
            find(label, 'landscape-' + label.split()[0])
        shot('landscape-updates')
        call('shell', 'settings', 'put', 'system', 'user_rotation', '0')
        verified('Manual controls remain reachable in landscape')
        call('shell', 'am', 'force-stop', package)
        call('shell', 'am', 'start', '-W', '-n', package + '/.HomeActivity')
        tap('Updates', 'cold-updates')
        find('GitHub: Not connected', 'cold-disconnected')
        find('Check for updates', 'cold-manual')
        assert call('exec-out', 'run-as', package, 'cat', 'files/last-report.json') == report
        shot('cold-updates')
        verified('Cold launch preserves disconnected controls and immutable diagnostics')
        assert not call('logcat', '-b', 'crash', '-d').strip()
        assert not re.search(rb'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab', call('logcat', '-d'))
        result = {'passed': True, 'checks': checks, 'notes': notes, 'fromVersion': from_version, 'version': args.version}
        (out / 'verification.json').write_text(json.dumps(result, indent=2))
        print(json.dumps(result), flush=True)
    finally:
        for name, commands in [('logcat.txt', ['logcat', '-d']), ('crash-buffer.txt', ['logcat', '-b', 'crash', '-d'])]:
            try:
                (out / name).write_bytes(call(*commands))
            except subprocess.SubprocessError:
                pass
        call('shell', 'settings', 'put', 'system', 'user_rotation', '0')
        call('shell', 'settings', 'put', 'system', 'accelerometer_rotation', '1')


if __name__ == '__main__':
    main()
