#!/usr/bin/env python3
"""Run one complete Android batch, export and cold-restore flow on an owned AVD."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
import xml.etree.ElementTree as ET


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--workload', type=int, required=True)
    parser.add_argument('--previous-workload', type=int, required=True)
    parser.add_argument('--schema', type=int, default=4)
    parser.add_argument('--gpu-tests', type=int, default=6)
    parser.add_argument('--gpu-workload', type=int, default=3)
    parser.add_argument('--engine-reference', type=Path, required=True)
    parser.add_argument('--nv-reference', type=Path, required=True)
    parser.add_argument('--core-reference', type=Path, required=True)
    parser.add_argument('--shader-reference', type=Path)
    parser.add_argument('--serial', default='emulator-5580')
    parser.add_argument('--resume', action='store_true', help='Resume export/restore after a recorded successful batch')
    parser.add_argument('--shutdown', action='store_true', help='Stop this explicitly owned AVD after QA')
    args = parser.parse_args()
    if not args.serial.startswith('emulator-'):
        parser.error('This runner is limited to an explicitly owned Android emulator')
    sdk = Path(os.environ.get('ANDROID_SDK_ROOT', str(Path.home() / 'Android/Sdk')))
    adb = [str(sdk / 'platform-tools/adb'), '-s', args.serial]
    package = 'com.algosculptor.emulator.gpulab'
    out = args.output
    out.mkdir(parents=True, exist_ok=True)
    notes = []

    def call(*command, timeout=25):
        return subprocess.run(adb + list(command), check=True, capture_output=True, timeout=timeout).stdout

    def dump(phase):
        for retry in range(3):
            remote = f'/sdcard/batch-{args.workload}-{phase}-{retry}.xml'
            try:
                result = call('shell', 'uiautomator', 'dump', remote)
                if b'dumped to:' not in result:
                    raise ValueError(f'No dump completion: {result[:160]!r}')
                raw = call('exec-out', 'cat', remote)
                if not raw.lstrip().startswith(b'<?xml'):
                    raise ValueError('Dump is not fresh XML')
                root = ET.fromstring(raw)
                (out / f'{phase}-{retry}.xml').write_bytes(raw)
                return root
            except (subprocess.SubprocessError, ValueError, ET.ParseError) as error:
                print(f'UI retry {phase}: {error}', flush=True)
        raise RuntimeError(f'No valid fresh UI for {phase}')

    def tap(node):
        x1, y1, x2, y2 = map(int, re.findall(r'\d+', node.attrib['bounds']))
        call('shell', 'input', 'tap', str((x1+x2)//2), str((y1+y2)//2))

    def settled(phase):
        root = dump(phase)
        if any(n.get('text', '') == "System UI isn't responding" for n in root.iter('node')):
            tap(next(n for n in root.iter('node') if n.get('text', '').lower() == 'wait'))
            notes.append(f'System UI startup ANR Wait at {phase}')
            root = dump(phase + '-after-system-wait')
        assert not any("isn't responding" in n.get('text', '') for n in root.iter('node')), 'Unexpected ANR dialog'
        return root

    def button(root, label, enabled=True):
        return next(n for n in root.iter('node') if n.get('text', '').lower() == label and n.get('enabled') == str(enabled).lower())

    def wait_button(phase, label, enabled=True):
        deadline = time.monotonic() + 20
        attempt = 0
        while True:
            root = settled(f'{phase}-{attempt}')
            try:
                button(root, label, enabled)
                return root
            except StopIteration:
                if time.monotonic() >= deadline:
                    raise RuntimeError(f'Expected {label} after {phase}') from None
                attempt += 1
                time.sleep(0.25)

    def launch_diagnostics(phase):
        call('shell', 'am', 'start', '-W', '-n', package + '/.HomeActivity')
        root = wait_button(phase + '-library', 'settings')
        tap(button(root, 'settings'))
        for attempt in range(3):
            root = settled(f'{phase}-settings-{attempt}')
            try:
                tap(button(root, 'diagnostics'))
                return
            except StopIteration:
                scroll = next(n for n in root.iter('node') if n.get('scrollable') == 'true')
                x1,y1,x2,y2 = map(int,re.findall(r'\d+',scroll.attrib['bounds']))
                call('shell','input','swipe',str((x1+x2)//2),str(y2-100),str((x1+x2)//2),str(y1+100),'300')
        raise RuntimeError('Diagnostics navigation unavailable')

    def saved():
        return call('exec-out', 'run-as', package, 'cat', 'files/last-report.json')

    def logs():
        for filename, command in [
            ('logcat.txt', ['logcat', '-d']),
            ('crash-buffer.txt', ['logcat', '-b', 'crash', '-d']),
            ('exit-info.txt', ['shell', 'dumpsys', 'activity', 'exit-info', package]),
        ]:
            try:
                (out / filename).write_bytes(call(*command))
            except subprocess.SubprocessError as error:
                notes.append(f'Capture {filename} failed: {error}')

    def downloads():
        return set(call('shell', 'ls', '/sdcard/Download').decode().splitlines())

    try:
        if args.resume:
            old = (out / 'before-upgrade-report.json').read_bytes()
            old_report = json.loads(old)
            assert old_report['workloadVersion'] == args.previous_workload and old_report['allPassed']
            call('logcat', '-c')
            call('shell', 'am', 'force-stop', package)
            launch_diagnostics('open-diagnostics')
            wait_button('resumed', 'export report')
            notes.append('Export/restore validation resumed from an existing successful current-workload report')
            print('Resuming the recorded successful batch', flush=True)
        else:
            old = saved()
            old_report = json.loads(old)
            assert old_report['workloadVersion'] == args.previous_workload and old_report['allPassed']
            (out / 'before-upgrade-report.json').write_bytes(old)
            call('install', '-r', str(args.apk), timeout=45)
            call('logcat', '-c')
            launch_diagnostics('open-diagnostics')
            root = wait_button('upgraded', 'export report', enabled=False)
            tap(button(root, 'run all checks'))
            print('Upgrade invalidated the prior report; full batch started', flush=True)
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            raw = saved()
            report = json.loads(raw)
            if report.get('workloadVersion') == args.workload:
                break
            time.sleep(1)
        else:
            raise RuntimeError('Full batch did not finish')
        assert (report['schemaVersion'], report['appVersion'], report['allPassed']) == (args.schema, args.version, True), report
        groups = {'engine': args.engine_reference, 'nv': args.nv_reference, 'gpuCore': args.core_reference}
        if args.shader_reference:
            groups['shader'] = args.shader_reference
        counts = {}
        for group, reference in groups.items():
            expected = json.loads(reference.read_text())
            actual = dict(report[group])
            assert actual.pop('hostArchitecture') == 'x86_64'
            expected.pop('hostArchitecture')
            assert actual == expected, f'{group} differs from reference'
            counts[group] = len(actual['tests'])
        assert report['gpu']['allPassed'] and len(report['gpu']['tests']) == args.gpu_tests
        assert report['gpu']['workloadVersion'] == args.gpu_workload
        assert all(t['passed'] for t in report['gpu']['tests'])
        counts['gpu'] = args.gpu_tests
        total = sum(counts.values())
        (out / 'private-report.json').write_bytes(raw)
        root = wait_button('results', 'export report')
        assert any(f'All {total} checks passed' in n.get('text', '') for n in root.iter('node'))
        button(root, 'export report')
        (out / 'results.png').write_bytes(call('exec-out', 'screencap', '-p'))
        print(f'All {total} cases passed: {counts}', flush=True)
        before = downloads()
        tap(button(root, 'export report'))
        root = wait_button('picker', 'save')
        tap(button(root, 'save'))
        root = wait_button('after-save', 'export report')
        button(root, 'export report')
        new = [name for name in downloads() - before if name.startswith('emulator-gpu-report') and name.endswith('.json')]
        assert len(new) == 1, new
        call('pull', '/sdcard/Download/' + new[0], str(out / 'exported-report.json'))
        assert (out / 'exported-report.json').read_bytes() == raw
        call('shell', 'am', 'force-stop', package)
        launch_diagnostics('open-diagnostics')
        root = wait_button('restored', 'export report')
        button(root, 'export report')
        assert any(f'All {total} checks passed' in n.get('text', '') for n in root.iter('node'))
        assert saved() == raw
        (out / 'restored.png').write_bytes(call('exec-out', 'screencap', '-p'))
        # Exercise AtomicFile recovery from an interrupted older-style backup,
        # using the just-executed report bytes, not a fabricated passing result.
        call('shell', 'am', 'force-stop', package)
        backups = ['last-report.json']
        if 'demo' in report:
            backups.append('last-demo-report.json')
        if 'textureScene' in report:
            backups.append('last-texture-report.json')
        backup_bytes = {name: call('exec-out', 'run-as', package, 'cat', 'files/' + name) for name in backups}
        for name in backups:
            call('shell', 'run-as', package, 'mv', 'files/' + name, 'files/' + name + '.bak')
        launch_diagnostics('backup-recovery')
        root = wait_button('backup-recovered', 'export report')
        assert any(f'All {total} checks passed' in n.get('text', '') for n in root.iter('node'))
        assert saved() == raw
        for name in backups:
            assert call('exec-out', 'run-as', package, 'cat', 'files/' + name) == backup_bytes[name]
            assert call('shell', 'run-as', package, 'test', '!', '-e', 'files/' + name + '.bak') == b''
        (out / 'backup-recovered.png').write_bytes(call('exec-out', 'screencap', '-p'))
        logs()
        assert not (out / 'crash-buffer.txt').read_bytes().strip()
        assert not re.search(r'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab', (out / 'logcat.txt').read_text())
        verification = dict(version=args.version, checks=total, groups=counts, allPassed=report['allPassed'],
                            upgradeInvalidatedWorkload=args.previous_workload, exportByteExact=True,
                            exportedFilename=new[0], coldRestoreByteExact=True, atomicBackupRecoveryByteExact=backups, summaryVisible=True, notes=notes)
        (out / 'verification.json').write_text(json.dumps(verification, indent=2) + '\n')
        print(json.dumps(verification), flush=True)
    except Exception as error:
        logs()
        (out / 'qa-error.json').write_text(json.dumps(dict(error=f'{type(error).__name__}: {error}', notes=notes), indent=2) + '\n')
        raise
    finally:
        if args.shutdown:
            try:
                call('emu', 'kill', timeout=10)
            except subprocess.SubprocessError:
                pass


if __name__ == '__main__':
    main()
