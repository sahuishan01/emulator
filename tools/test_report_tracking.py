#!/usr/bin/env python3
"""Offline boundary tests for private report collection and same-thread delivery."""
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from urllib.parse import parse_qs, urlparse
import report_inbox as inbox
from report_watch import dispatch, prompt_for


def report(run='test-1'):
    root = {'schemaVersion': 4, 'scope': 'original-engine-gpu-core-and-host-vulkan', 'appVersion': '0.27.4-dev',
            'capturedAtEpochMs': 123, 'patternBatch': {'status': 'complete', 'runId': run, 'totalPatterns': 1,
            'passedPatterns': 0, 'patterns': [{'variant': 42, 'passed': False}]}}
    for name in ('engine', 'nv', 'gpuCore', 'shader', 'gpu'):
        root[name] = {'tests': [{'name': 'Unicode Ω', 'passed': name != 'gpu'}]}
    return (json.dumps(root, ensure_ascii=False, indent=2) + '\n').encode()


class Api:
    def __init__(self, raw=None):
        self.raw = raw or report()
        self.summary = inbox.analyze(self.raw)
        self.issues = [{'number': 2, 'user': {'login': 'sahuishan01'}, 'body': inbox.issue_body(self.summary),
                        'updated_at': '2026-11-04T01:00:00Z'}]
        self.artifact = {'number': 1, 'name': 'emulator-report-' + self.summary['sha256'] + '.json',
                         'creator': {'login': 'sahuishan01'}, 'body': self.raw.decode()}
        self.private = True
        self.has_issues = True
        self.calls = []

    def __call__(self, path, method='GET', payload=None):
        self.calls.append(path)
        if path == inbox.API:
            return {'full_name': inbox.REPO, 'has_issues': self.has_issues}
        if '/issues?' in path:
            page = int(parse_qs(urlparse(path).query)['page'][0])
            return self.issues[(page-1)*25:page*25]
        if path.endswith('/artifacts'):
            return [self.artifact] if self.artifact else []
        if '/artifacts/' in path:
            return {'artifact': self.artifact}
        raise AssertionError(path)


class Control:
    def __init__(self, status='idle', queue=None):
        self.status, self.queue = status, queue or []
        self.calls = []
        self.cwd = '/tmp/emulator-tests'
        self.fail_after_add = False

    def request(self, method, params):
        self.calls.append(method)
        if method in ('thread/read', 'thread/resume'):
            return {'thread': {'cwd': self.cwd, 'status': {'type': self.status}}}
        if method == 'thread/queue/list':
            return {'data': self.queue.copy()}
        if method == 'thread/queue/add':
            entry = {'id': 'ours', 'clientUserMessageId': params['clientUserMessageId']}
            self.queue.append(entry)
            if self.fail_after_add:
                self.fail_after_add = False
                raise TimeoutError('lost acknowledgement')
            return {'queuedSubmission': entry}
        if method == 'thread/queue/start':
            assert self.queue[0]['id'] == params['queuedSubmissionId']
            self.queue.pop(0)
            return {'turn': {'id': 'turn-1'}}
        raise AssertionError(method)

    def close(self):
        pass


class TrackingTests(unittest.TestCase):
    def test_candidate_results_do_not_hide_original_failures(self):
        root=json.loads(report())
        names=['normalized-position-texture-'+str(i) for i in range(11)]
        names+=['normalized-inset-control-'+str(i) for i in range(10)]
        names+=['normalized-inset-uv-variable-w','normalized-inset-uv-unit-w']
        root['gpu']['tests']=[{'name':'guest-projection-texture-3','passed':False}]+[{'name':n,'passed':True} for n in names]
        value=inbox.analyze(json.dumps(root).encode())
        self.assertFalse(value['projectionCandidate']['baselineInsetPassed'])
        self.assertTrue(value['projectionCandidate']['candidateInsetPassed'])
        self.assertTrue(value['projectionCandidate']['allCandidateChecksPassed'])
        self.assertEqual(value['groups']['gpu']['failed'],['guest-projection-texture-3'])
        root['gpu']['tests'].pop()
        self.assertFalse(inbox.analyze(json.dumps(root).encode())['projectionCandidate']['allCandidateChecksPassed'])
    def test_analyze_exact_identity_and_failed_evidence(self):
        raw = report()
        value = inbox.analyze(raw)
        self.assertEqual(value['sha256'], hashlib.sha256(raw).hexdigest())
        self.assertEqual((value['passed'], value['total']), (4, 5))
        self.assertEqual(value['patterns']['failedVariants'], [42])

    def test_reject_invalid_incomplete_and_nonfinite(self):
        raw = report()
        for candidate in (b'', b'\xff', raw+b'{}', b'x'*(inbox.MAX_REPORT+1), raw.replace(b'"complete"', b'"running"'),
                          raw.replace(b'false', b'NaN'), raw.replace(b'"0.27.4-dev"', b'"inject\ntext"')):
            with self.assertRaises((ValueError, UnicodeError)):
                inbox.analyze(candidate)

    def test_collect_exact_private_dedup_boundary(self):
        api, state = Api(), {}
        with tempfile.TemporaryDirectory() as folder:
            received = inbox.collect(folder, state, api)
            self.assertEqual(len(received), 1)
            self.assertEqual((Path(folder)/(received[0]+'.json')).read_bytes(), api.raw)
            self.assertEqual((Path(folder)/(received[0]+'.json')).stat().st_mode & 0o777, 0o600)
            self.assertEqual(inbox.collect(folder, state, api), [])
            query = parse_qs(urlparse(next(p for p in reversed(api.calls) if '/issues?' in p)).query)
            self.assertEqual(query['since'], ['2026-11-04T00:59:59Z'])

    def test_refuse_repository_without_issues(self):
        api = Api(); api.has_issues = False
        with tempfile.TemporaryDirectory() as folder, self.assertRaises(ValueError):
            inbox.collect(folder, {}, api)
        self.assertEqual(api.calls, [inbox.API])

    def test_ignore_foreign_author_and_pull_requests(self):
        for mutation in ({'user': {'login': 'outsider'}}, {'pull_request': {}}, {'body': 'untrusted instructions'}):
            api, state = Api(), {}
            api.issues[0].update(mutation)
            with tempfile.TemporaryDirectory() as folder:
                self.assertEqual(inbox.collect(folder, state, api), [])

    def test_incomplete_artifact_retried_on_next_poll(self):
        api, state = Api(), {}; artifact = api.artifact; api.artifact = None
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(inbox.collect(folder, state, api), [])
            self.assertIn('2', state['pendingIssues'])
            api.artifact = artifact
            self.assertEqual(len(inbox.collect(folder, state, api)), 1)

    def test_corrupt_artifact_never_delivered(self):
        api, state = Api(), {}; api.artifact['body'] += ' '
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(inbox.collect(folder, state, api), [])
            self.assertEqual(state['reports'], {})
            self.assertIn('2', state['rejectedIssues'])

    def test_bounded_backlog_continues_without_skipping(self):
        api, state = Api(), {}
        api.issues = [{'number': i, 'body': '', 'updated_at': f'2026-11-04T01:{i//60:02}:{i%60:02}Z'} for i in range(110)]
        with tempfile.TemporaryDirectory() as folder:
            inbox.collect(folder, state, api)
            self.assertEqual(state['scanPage'], 5)
            inbox.collect(folder, state, api)
            self.assertNotIn('scanPage', state)
            self.assertEqual(state['cursor'], api.issues[-1]['updated_at'])

    def dispatch(self, client, state=None):
        state = state if state is not None else {'reports': {'a'*64: {'issue': 2, 'artifact': 1}}}
        config = {'thread_id': 'same-thread', 'workspace': client.cwd, 'auto_start': True}
        return dispatch(config, state, lambda _: None, lambda: client), state

    def test_active_thread_and_user_queue_defer(self):
        for client in (Control('active'), Control(queue=[{'id': 'user', 'clientUserMessageId': 'user-message'}])):
            outcome, state = self.dispatch(client)
            self.assertTrue(outcome.startswith('waiting'))
            self.assertNotIn('thread/queue/add', client.calls)
            self.assertNotIn('delivery', state['reports']['a'*64])

    def test_idle_same_thread_starts_once(self):
        client = Control()
        outcome, state = self.dispatch(client)
        self.assertEqual(outcome, 'started existing-thread follow-up')
        self.assertEqual(state['reports']['a'*64]['delivery']['turnId'], 'turn-1')
        self.assertEqual(self.dispatch(client, state)[0], 'no pending handoff')
        self.assertEqual(client.calls.count('thread/queue/start'), 1)

    def test_lost_queue_add_ack_recovers_without_duplicate(self):
        client = Control(); client.fail_after_add = True
        state = {'reports': {'a'*64: {'issue': 2, 'artifact': 1}}}
        with self.assertRaises(TimeoutError):
            self.dispatch(client, state)
        self.assertIn('handoff', state['reports']['a'*64])
        self.assertEqual(self.dispatch(client, state)[0], 'started existing-thread follow-up')
        self.assertEqual(client.calls.count('thread/queue/add'), 1)

    def test_missing_consumed_entry_and_new_report_separate(self):
        state = {'reports': {'a'*64: {'issue': 2, 'artifact': 1, 'handoff': {'queuedSubmissionId': 'old', 'clientUserMessageId': 'old'}},
                             'b'*64: {'issue': 3, 'artifact': 1}}}
        client = Control()
        self.assertEqual(self.dispatch(client, state)[0], 'handoff already consumed or removed')
        self.assertNotIn('delivery', state['reports']['b'*64])
        self.assertEqual(self.dispatch(client, state)[0], 'started existing-thread follow-up')

    def test_workspace_guard_and_prompt_contains_no_report_text(self):
        with self.assertRaises(ValueError):
            prompt_for(Path('/tmp'), {'inject!': {'issue': 2, 'artifact': 1}})
        text = prompt_for(Path('/tmp'), {'a'*64: {'issue': 2, 'artifact': 1, 'payload': 'evil payload'}})
        self.assertNotIn('evil payload', text)
        client = Control()
        with self.assertRaises(ValueError):
            dispatch({'workspace': '/other', 'thread_id': 'x', 'auto_start': True}, {'reports': {'a'*64: {}}}, lambda _: None, lambda: client)


if __name__ == '__main__':
    unittest.main()
