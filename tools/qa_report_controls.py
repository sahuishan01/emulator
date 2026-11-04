#!/usr/bin/env python3
"""Drive the shipped report screen with fresh XML-derived controls, no credentials."""
import argparse
import json
import re
import subprocess
import time
import xml.etree.ElementTree as ET
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    adb = [str(Path.home()/'Android/Sdk/platform-tools/adb'), '-s', 'emulator-5580']
    package = 'com.algosculptor.emulator.gpulab'
    checks, notes = [], []

    def call(*parts):
        return subprocess.check_output(adb+list(parts), stderr=subprocess.PIPE, timeout=40)

    def bounds(node):
        return tuple(map(int, re.findall(r'\d+', node.get('bounds'))))

    def tap_node(node):
        x, y, x2, y2 = bounds(node)
        assert node.get('enabled') == 'true' and node.get('clickable') == 'true'
        call('shell', 'input', 'tap', str((x+x2)//2), str((y+y2)//2))

    def ui(phase):
        for attempt in range(4):
            remote = '/sdcard/report-controls.xml'
            if b'dumped to:' not in call('shell', 'uiautomator', 'dump', remote):
                continue
            raw = call('exec-out', 'cat', remote)
            (args.output/f'{phase}-{attempt}.xml').write_bytes(raw)
            root = ET.fromstring(raw)
            if any(n.get('text') == "System UI isn't responding" for n in root.iter('node')):
                tap_node(next(n for n in root.iter('node') if n.get('text') == 'Wait'))
                notes.append('Handled exact System UI startup Wait')
                continue
            assert not any("isn't responding" in n.get('text', '') for n in root.iter('node'))
            return root
        raise RuntimeError('Missing fresh UI: '+phase)

    def find(label, phase, click=False):
        for attempt in range(6):
            root = ui(phase+'-'+str(attempt))
            node = next((n for n in root.iter('node') if n.get('text') == label), None)
            if node is not None:
                if click:
                    tap_node(node)
                return node
            scroll = next((n for n in root.iter('node') if n.get('scrollable') == 'true'), None)
            if scroll is not None:
                x, y, x2, y2 = bounds(scroll)
                call('shell', 'input', 'swipe', str((x+x2)//2), str(y2-70), str((x+x2)//2), str(y+70), '250')
            else:
                time.sleep(.2)
        raise RuntimeError('Missing '+label)

    try:
        call('shell', 'am', 'force-stop', package)
        call('shell', 'am', 'start', '-W', '-n', package+'/.HomeActivity')
        find('Settings','settings',True); find('GitHub reports','reports-entry',True)
        switch = find('Upload completed reports automatically','off-default')
        assert switch.get('checked') == 'false'
        for label in ('Test report upload access', 'Upload latest JSON', 'Cancel pending uploads'):
            assert find(label,'manual-control').get('enabled') == 'true'
        checks.append('Settings discovers reports; automatic uploads default off; manual controls enabled')
        (args.output/'reports-default.png').write_bytes(call('exec-out','screencap','-p'))
        find('Test report upload access','test-no-access',True)
        expected = 'Connect GitHub in App updates, then give the token Issues: read and write for emulator.'
        find(expected,'missing-access')
        checks.append('Disconnected manual access test gives explicit Issues permission guidance')
        find('Upload completed reports automatically','enable-no-access',True)
        time.sleep(.2)
        assert find('Upload completed reports automatically','still-off').get('checked') == 'false'
        checks.append('Enabling fails safely without a token and returns switch to off')
        find('Manage GitHub connection','connection',True)
        find('Test GitHub access','update-screen')
        checks.append('Manage connection opens existing updater access controls')
        call('shell','input','keyevent','4')
        call('shell','settings','put','system','accelerometer_rotation','0')
        call('shell','settings','put','system','user_rotation','1')
        find('GitHub reports','landscape')
        assert find('Upload completed reports automatically','landscape-off').get('checked') == 'false'
        (args.output/'reports-landscape.png').write_bytes(call('exec-out','screencap','-p'))
        checks.append('Landscape recreation preserves off state and report controls')
        call('shell','settings','put','system','user_rotation','0')
        call('shell','am','force-stop',package)
        call('shell','am','start','-W','-n',package+'/.HomeActivity')
        find('Settings','cold-settings',True); find('GitHub reports','cold-reports',True)
        assert find('Upload completed reports automatically','cold-off').get('checked') == 'false'
        find(expected,'cold-access-result')
        checks.append('Cold restart preserves disabled uploads and accurate failed-access result')
        logs = call('logcat','-d').decode(errors='replace')
        (args.output/'logcat.txt').write_text(logs)
        assert not re.search(r'FATAL EXCEPTION|Fatal signal|ANR in com\.algosculptor\.emulator\.gpulab',logs)
        result = {'passed':True,'checks':checks,'notes':notes}
        (args.output/'result.json').write_text(json.dumps(result,indent=2))
        print(json.dumps(result,indent=2))
    finally:
        call('shell','settings','put','system','user_rotation','0')


if __name__ == '__main__':
    main()
