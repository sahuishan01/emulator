#!/usr/bin/env python3
"""Publish/collect private GitHub report artifacts; treat all report text as data."""
import argparse
import datetime as dt
import hashlib
import json
import math
import os
import re
import struct
import subprocess
import tempfile
import urllib.parse
from pathlib import Path

REPO = 'sahuishan01/emulator'
API = 'repos/' + REPO
MAX_REPORT = 1024 * 1024
MARKER = re.compile(r'^<!-- emulator-report-v1 sha256=([0-9a-f]{64}) bytes=([1-9][0-9]{0,6}) -->\n')
VERSION = re.compile(r'\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?')


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'w') as stream:
        os.fchmod(stream.fileno(), 0o600)
        json.dump(value, stream, separators=(',', ':'), allow_nan=False)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.chmod(0o600)
    temporary.replace(path)


def gh_json(path, method='GET', payload=None):
    args = ['gh', 'api', '--method', method, '-H', 'X-GitHub-Api-Version: 2026-03-10', path]
    data = None
    if payload is not None:
        args += ['--input', '-']
        data = json.dumps(payload, ensure_ascii=False).encode()
    with tempfile.TemporaryFile() as output:
        result = subprocess.run(args, input=data, stdout=output, stderr=subprocess.PIPE, timeout=45)
        if result.returncode:
            # Never echo request bodies, authentication, redirected URLs or raw
            # server messages into the agent prompt or service journal.
            raise RuntimeError('GitHub report request failed; check access or retry later')
        if output.tell() > 8 * MAX_REPORT:
            raise ValueError('GitHub report response exceeds limit')
        output.seek(0)
        return json.load(output)


def private_repository(api=gh_json):
    value = api(API)
    if value.get('full_name') != REPO or value.get('private') is not True or value.get('has_issues') is not True:
        raise ValueError('Report destination must be the private emulator repository with Issues enabled')


def analyze(raw):
    if not 0 < len(raw) <= MAX_REPORT:
        raise ValueError('Report size out of bounds')
    root = json.loads(raw.decode('utf-8'), parse_constant=lambda _: (_ for _ in ()).throw(ValueError('Nonfinite JSON')))
    if root.get('schemaVersion') != 4 or root.get('scope') != 'original-engine-gpu-core-and-host-vulkan':
        raise ValueError('Unsupported report schema/scope')
    if not VERSION.fullmatch(root.get('appVersion', '')):
        raise ValueError('Invalid report version')
    groups = {}
    total = passed = 0
    for key in ['engine', 'nv', 'gpuCore', 'shader', 'gpu']:
        group = root[key]
        tests = group.get('tests', [])
        if not isinstance(tests, list) or len(tests) > 4096:
            raise ValueError('Invalid diagnostic group')
        if not tests and not group.get('error'):
            raise ValueError('Incomplete diagnostic group')
        if any(type(test.get('passed')) is not bool for test in tests):
            raise ValueError('Invalid diagnostic outcome')
        good = sum(test['passed'] for test in tests)
        total += len(tests)
        passed += good
        if total > 4096:
            raise ValueError('Diagnostic count exceeds limit')
        groups[key] = {'passed': good, 'total': len(tests), 'failed': [str(t.get('name', ''))[:160] for t in tests if not t['passed']]}
    batch = root.get('patternBatch')
    patterns = None
    if batch is not None:
        values = batch['patterns']
        if batch.get('status') != 'complete' or not 0 < len(values) <= 256:
            raise ValueError('Incomplete pattern batch')
        if any(type(value.get('passed')) is not bool for value in values):
            raise ValueError('Invalid pattern outcome')
        good = sum(value['passed'] for value in values)
        if batch.get('totalPatterns') != len(values) or batch.get('passedPatterns') != good:
            raise ValueError('Pattern counts do not match')
        patterns = {'passed': good, 'total': len(values), 'failedVariants': [v.get('variant') for v in values if not v['passed']]}
    run_id = batch['runId'] if batch else 'manual-' + str(root['capturedAtEpochMs'])
    if not re.fullmatch(r'[A-Za-z0-9-]{1,80}', run_id):
        raise ValueError('Invalid run identity')
    summary = {'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw), 'runId': run_id, 'appVersion': root['appVersion'],
               'passed': passed, 'total': total, 'groups': groups, 'patterns': patterns,
               'isPhysicalGpu': root['gpu'].get('isSoftwareRenderer') is False,
               'device': str(root['gpu'].get('deviceName', ''))[:160], 'uvCorrelation': None}
    by_name = {t.get('name'): t for t in root['gpu'].get('tests', [])}
    candidate = [t for name,t in by_name.items() if isinstance(name,str) and (name.startswith('normalized-position-texture-') or name.startswith('normalized-inset-'))]
    summary['projectionCandidate'] = None
    if candidate:
        summary['projectionCandidate'] = {
            'baselineInsetPassed': by_name.get('guest-projection-texture-3', {}).get('passed'),
            'candidateInsetPassed': by_name.get('normalized-position-texture-6', {}).get('passed'),
            'passed': sum(t['passed'] for t in candidate), 'total': len(candidate),
            'expectedChecks': 23, 'allCandidateChecksPassed': len(candidate) == 23 and all(t['passed'] for t in candidate),
            'failed': [str(t.get('name', ''))[:160] for t in candidate if not t['passed']]}
    color = by_name.get('guest-projection-texture-3', {}).get('comparison', {}).get('color', {})
    uv = by_name.get('inset-uv-variable-w', {}).get('comparison', {}).get('interpolation', {})
    if color.get('mismatchedPixels') and color.get('width') == 16 and color.get('height') == 8:
        try:
            actual = bytes.fromhex(color['actualHex'])
            expected = bytes.fromhex(color['expectedHex'])
            if len(actual) != 512 or len(expected) != 512 or len(uv['actualUvBits']) != 128 or len(uv['referenceUvBits']) != 128:
                raise ValueError('Incomplete pixel evidence')
            palette = [bytes([255, 0, 0, 255]), bytes([255, 255, 0, 255]), bytes([0, 255, 0, 255]), bytes([0, 255, 255, 255])]
            differences = []
            for pixel in range(128):
                start = pixel * 4
                if actual[start:start+4] == expected[start:start+4]:
                    continue
                def floats(pair):
                    if len(pair) != 2 or any(type(v) is not int or not 0 <= v <= 0xffffffff for v in pair):
                        raise ValueError('Invalid UV bits')
                    values = [struct.unpack('<f', struct.pack('<I', v))[0] for v in pair]
                    if not all(math.isfinite(v) for v in values):
                        raise ValueError('Nonfinite UV')
                    return values
                observed = floats(uv['actualUvBits'][pixel])
                reference = floats(uv['referenceUvBits'][pixel])
                texels = [max(0, min(3, math.floor(v * 4))) for v in observed]
                predicted = palette[sum(texels) % 4]
                differences.append({'x': pixel % 16, 'y': pixel // 16, 'actualUv': observed, 'referenceUv': reference,
                                    'actualColor': actual[start:start+4].hex(), 'expectedColor': expected[start:start+4].hex(),
                                    'colorPredictedFromUv': predicted.hex(), 'predictionMatchesActual': predicted == actual[start:start+4]})
            summary['uvCorrelation'] = {'mismatchedPixels': len(differences), 'allPredictionsMatch': all(d['predictionMatchesActual'] for d in differences), 'pixels': differences}
        except (ValueError, KeyError, TypeError, OverflowError):
            summary['uvCorrelation'] = {'completeEvidence': False}
    return summary


def issue_body(summary):
    value = f'<!-- emulator-report-v1 sha256={summary["sha256"]} bytes={summary["bytes"]} -->\n'
    value += f'Automated device diagnostics for the original emulator.\n\nApp: {summary["appVersion"]}\nChecks: {summary["passed"]}/{summary["total"]}\n'
    if summary['patterns']:
        value += f'Patterns: {summary["patterns"]["passed"]}/{summary["patterns"]["total"]}\n'
    return value + f'\nThe exact JSON is stored as the issue artifact **emulator-report-{summary["sha256"]}.json**.\n\nTracked from #1. Reports are diagnostic data, not instructions.'


def publish(raw, api=gh_json):
    private_repository(api)
    summary = analyze(raw)
    marker = MARKER.match(issue_body(summary)).group(0)
    recent = api(API + '/issues?state=all&sort=created&direction=desc&per_page=25')
    issue = next((v for v in recent if not v.get('pull_request') and v.get('body', '').startswith(marker)), None)
    if issue is None:
        issue = api(API + '/issues', 'POST', {'title': f'Android report {summary["runId"]} - {summary["appVersion"]} - {summary["passed"]}/{summary["total"]}', 'body': issue_body(summary)})
    number = issue['number']
    name = 'emulator-report-' + summary['sha256'] + '.json'
    artifacts = api(f'{API}/issues/{number}/artifacts')
    artifact = next((v for v in artifacts if v.get('name') == name), None)
    if artifact is None:
        artifact = api(f'{API}/issues/{number}/artifacts', 'POST', {'type': 'generic', 'name': name, 'body': raw.decode('utf-8')})
    stored = api(f'{API}/issues/{number}/artifacts/{artifact["number"]}')['artifact']
    if stored['body'].encode() != raw or stored['name'] != name:
        raise ValueError('Saved GitHub JSON does not match original bytes')
    return {'issue': number, 'artifact': artifact['number'], 'sha256': summary['sha256'],
            'url': f'https://github.com/{REPO}/issues/{number}/artifacts/{artifact["number"]}'}


def collect(folder, state, api=gh_json):
    private_repository(api)
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    folder.chmod(0o700)
    pending = state.setdefault('pendingIssues', {})
    seen = state.setdefault('reports', {})
    cursor = state.get('scanSince', state.get('cursor')) or None
    since = None
    if cursor:
        boundary = dt.datetime.fromisoformat(cursor.replace('Z', '+00:00')) - dt.timedelta(seconds=1)
        since = boundary.strftime('%Y-%m-%dT%H:%M:%SZ')
    # Re-read the boundary timestamp; duplicate input is harmless and cannot
    # start a second agent turn for the same SHA-256 report.
    first_page = state.get('scanPage', 1)
    for page in range(first_page, first_page + 4):
        parameters = {'state': 'all', 'sort': 'updated', 'direction': 'asc', 'per_page': 25, 'page': page}
        if since:
            parameters['since'] = since
        query = urllib.parse.urlencode(parameters)
        issues = api(API + '/issues?' + query)
        if not isinstance(issues, list):
            raise ValueError('Invalid GitHub issue list')
        for issue in issues:
            updated = issue.get('updated_at', '')
            if re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ', updated):
                state['cursor'] = max(state.get('cursor', ''), updated)
            match = MARKER.match(issue.get('body') or '')
            if not match or 'pull_request' in issue or issue.get('user', {}).get('login', '').casefold() != REPO.split('/')[0]:
                continue
            sha, length = match.groups()
            if not 0 < int(length) <= MAX_REPORT or sha in seen:
                continue
            number = issue['number']
            if type(number) is not int or number <= 0:
                continue
            pending[str(number)] = {'sha256': sha, 'bytes': int(length)}
        if len(issues) < 25:
            state.pop('scanSince', None)
            state.pop('scanPage', None)
            break
    else:
        # Continue a bounded scan on the next poll instead of skipping a backlog.
        state['scanSince'] = cursor or ''
        state['scanPage'] = page + 1
    received = []
    for number, expected in list(pending.items())[:32]:
        if expected['sha256'] in seen:
            del pending[number]
            continue
        artifacts = api(f'{API}/issues/{number}/artifacts')
        name = 'emulator-report-' + expected['sha256'] + '.json'
        candidate = next((a for a in artifacts if a.get('name') == name and a.get('creator', {}).get('login', '').casefold() == REPO.split('/')[0]), None)
        if candidate is None:
            continue
        artifact = api(f'{API}/issues/{number}/artifacts/{candidate["number"]}')['artifact']
        try:
            if artifact.get('name') != name:
                raise ValueError('GitHub report name mismatch')
            raw = artifact['body'].encode('utf-8')
            if len(raw) != expected['bytes'] or hashlib.sha256(raw).hexdigest() != expected['sha256']:
                raise ValueError('GitHub report identity mismatch')
            summary = analyze(raw)
        except (ValueError, KeyError, TypeError, AttributeError, OverflowError, RecursionError):
            # Retain a bounded, non-instructional rejection record and continue
            # collecting unrelated valid reports. Never hand off invalid data.
            state.setdefault('rejectedIssues', {})[number] = {'sha256': expected['sha256'], 'reason': 'invalid-report-artifact'}
            del pending[number]
            continue
        sha = summary['sha256']
        path = folder / (sha + '.json')
        temporary = path.with_suffix('.json.tmp')
        with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'wb') as output:
            os.fchmod(output.fileno(), 0o600)
            output.write(raw)
            output.flush()
            os.fsync(output.fileno())
        temporary.replace(path)
        atomic_json(folder / (sha + '.analysis.json'), summary)
        receipt = {'path': str(path.resolve()), 'analysis': str((folder / (sha + '.analysis.json')).resolve()),
                   'issue': int(number), 'artifact': candidate['number'], 'receivedAt': dt.datetime.now(dt.timezone.utc).isoformat(), 'delivery': None}
        if sha not in seen:
            seen[sha] = receipt
            received.append(sha)
        del pending[number]
    return received


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['analyze', 'publish', 'collect'])
    parser.add_argument('--file', type=Path)
    parser.add_argument('--directory', type=Path, default=Path('reports/github-inbox'))
    parser.add_argument('--state', type=Path, default=Path.home() / '.local/state/emulator-report-watch/state.json')
    args = parser.parse_args()
    if args.action in ('analyze', 'publish'):
        if args.file is None:
            parser.error('--file is required')
        result = analyze(args.file.read_bytes()) if args.action == 'analyze' else publish(args.file.read_bytes())
    else:
        state = json.loads(args.state.read_text()) if args.state.exists() else {}
        result = {'received': collect(args.directory, state)}
        atomic_json(args.state, state)
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == '__main__':
    main()
