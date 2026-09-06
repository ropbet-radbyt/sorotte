from __future__ import annotations

import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import yaml

from scripts import native_required as native


class API:
    repository = 'owner/repo'
    source = 'a' * 40

    def __init__(self):
        self.run = {'id': 12, 'run_attempt': 2, 'head_sha': self.source, 'path': native.WORKFLOW,
                    'repository': {'full_name': self.repository}, 'head_repository': {'full_name': self.repository},
                    'event': 'workflow_dispatch', 'head_branch': 'reviewed-candidate', 'status': 'completed',
                    'conclusion': 'success', 'actor': {'login': 'maintainer'}, 'triggering_actor': {'login': 'maintainer'}}
        self.runs = [self.run]
        self.job = {'id': 34, 'run_id': 12, 'head_sha': self.source, 'status': 'completed', 'conclusion': 'success',
                    'labels': sorted(native.LABELS), 'runner_name': 'sorotte-sandbox-12345678-1234-1234-1234-123456789012',
                    'runner_id': 56, 'completed_at': '2026-09-06T10:00:00Z'}
        self.jobs = [self.job]
        self.permission = {'user': {'login': 'maintainer'}, 'permission': 'write'}

    def get(self, path):
        if path.startswith('collaborators/'):
            return self.permission
        if path == 'actions/runs/12':
            return self.run
        raise AssertionError(path)

    def pages(self, path, field):
        if field == 'workflow_runs':
            if 'head_sha=' + self.source not in path:
                raise AssertionError('missing exact source filter')
            return self.runs
        if path != 'actions/runs/12/attempts/2/jobs':
            raise AssertionError('missing exact attempt binding')
        return self.jobs


class NativeAuthorityTests(unittest.TestCase):
    def test_exact_maintainer_dispatch_and_owned_job_pass(self):
        api = API()
        receipt = native.observe(api, api.source)
        self.assertEqual((receipt['run_id'], receipt['run_attempt'], receipt['job_id']), (12, 2, 34))

    def test_main_push_and_schedule_are_independent_of_release_authorization(self):
        for event in ('push', 'schedule'):
            api = API()
            api.run.update(event=event, head_branch='main')
            self.assertEqual(native.observe(api, api.source)['event'], event)

    def test_foreign_source_workflow_fork_or_untrusted_trigger_fails(self):
        for field, value in (('head_sha', 'b' * 40), ('path', '.github/workflows/forged.yml'),
                             ('repository', {'full_name': 'other/repo'}), ('head_repository', {'full_name': 'fork/repo'}),
                             ('event', 'pull_request'), ('actor', {'login': 'github-actions[bot]'})):
            api = API()
            api.run[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                native.observe(api, api.source)

    def test_only_current_maintainers_can_authorize_original_or_rerun(self):
        for permission in ('read', 'triage', 'none'):
            api = API()
            api.permission['permission'] = permission
            with self.subTest(permission=permission), self.assertRaisesRegex(ValueError, 'write authority'):
                native.observe(api, api.source)
        api = API()
        api.run['triggering_actor']['login'] = 'foreign-user'
        with self.assertRaisesRegex(ValueError, 'write authority'):
            native.observe(api, api.source)

    def test_newer_failure_cannot_fall_back_to_older_green(self):
        api = API()
        api.runs.insert(0, {**api.run, 'id': 11, 'conclusion': 'success'})
        for conclusion in ('failure', 'cancelled', 'skipped', 'timed_out'):
            api.run['conclusion'] = conclusion
            with self.subTest(conclusion=conclusion), self.assertRaisesRegex(ValueError, 'latest native run'):
                native.observe(api, api.source)

    def test_missing_or_queued_producer_is_pending(self):
        api = API()
        api.runs = []
        with self.assertRaises(native.NativePending):
            native.observe(api, api.source)
        api = API()
        api.run['status'] = 'queued'
        with self.assertRaises(native.NativePending):
            native.observe(api, api.source)

    def test_native_skip_missing_duplicate_wrong_source_or_runner_is_not_physical_evidence(self):
        for variant in ('skip', 'missing', 'duplicate', 'source', 'static-runner', 'hosted-labels'):
            api = API()
            if variant == 'skip': api.job['conclusion'] = 'skipped'
            if variant == 'missing': api.jobs = []
            if variant == 'duplicate': api.jobs.append(copy.deepcopy(api.job))
            if variant == 'source': api.job['head_sha'] = 'b' * 40
            if variant == 'static-runner': api.job['runner_name'] = 'personal-desktop'
            if variant == 'hosted-labels': api.job['labels'] = ['ubuntu-24.04']
            with self.subTest(variant=variant), self.assertRaises(ValueError):
                native.observe(api, api.source)

    def test_rerun_started_during_lookup_invalidates_observation(self):
        api = API()
        snapshot = copy.deepcopy(api.run)
        api.run.update(run_attempt=3, status='queued', conclusion=None)
        with self.assertRaises(native.NativePending):
            native.validate_producer(api, snapshot, api.source)


class NativeSelectionTests(unittest.TestCase):
    def plan(self, selected=True):
        return {'schema_version': 1, 'kind': 'verification-plan', 'base_sha': 'b' * 40,
                'source_sha': 'a' * 40, 'policy_sha256': 'c' * 64, 'paths': ['crates/sorotte-gui/src/lib.rs'],
                'lanes': [{'id': 'native', 'selected': selected}], 'required_checks': {'native-required': '.github/workflows/native-required.yml'}}

    def test_tampered_plan_cannot_replace_external_base_or_obligations(self):
        original = self.plan()
        for field, value in (('base_sha', 'a' * 40), ('source_sha', 'b' * 40), ('paths', []),
                             ('lanes', [{'id': 'native', 'selected': False}]), ('required_checks', {})):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / 'plan.json'
                path.write_text(json.dumps({**original, field: value}))
                with mock.patch.object(native, 'git', return_value='a' * 40), mock.patch.object(native.verify, 'plan', return_value=original):
                    with self.assertRaises(ValueError):
                        native.validate_plan(path, base='b' * 40, source='a' * 40)

    def test_valid_noop_does_not_request_or_invent_native_authority(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'plan.json'
            path.write_text(json.dumps(self.plan(False)))
            output = Path(folder) / 'required.json'
            with mock.patch.object(native, 'validate_plan', return_value=self.plan(False)), mock.patch.object(native, 'GitHub') as api:
                code = native.main(['--base-sha', 'b' * 40, '--source-sha', 'a' * 40, '--plan', str(path), '--output', str(output)])
            api.assert_not_called()
            self.assertEqual(code, 0)
            self.assertIsNone(json.loads(output.read_text())['producer'])

    def test_missing_capability_retains_explicit_failure_after_bounded_wait(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'plan.json'
            path.write_text(json.dumps(self.plan()))
            output = Path(folder) / 'required.json'
            api = API()
            api.runs = []
            with mock.patch.object(native, 'validate_plan', return_value=self.plan()), mock.patch.object(native, 'GitHub', return_value=api):
                code = native.main(['--base-sha', 'b' * 40, '--source-sha', 'a' * 40, '--plan', str(path), '--output', str(output)])
            receipt = json.loads(output.read_text())
            self.assertEqual((code, receipt['status']), (1, 'failed'))
            self.assertIn('unavailable', receipt['reason'])
            self.assertIsNone(receipt['producer'])

    def test_required_workflow_is_always_present_and_never_exposes_pr_code_to_native_runner(self):
        root = Path(__file__).resolve().parents[2]
        workflow = yaml.load((root / '.github/workflows/native-required.yml').read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(workflow['on'], {'pull_request': '', 'push': {'branches': ['main']}})
        job = workflow['jobs']['native-required']
        self.assertNotIn('if', job)
        self.assertNotIn('continue-on-error', job)
        self.assertEqual(job['runs-on'], 'ubuntu-24.04')
        self.assertEqual(workflow['permissions'], {'contents': 'read', 'actions': 'read'})
        command = next(step for step in job['steps'] if 'scripts/native_required.py' in step.get('run', ''))
        self.assertEqual(command['if'], 'always()')
        self.assertIn('--base-sha "$VERIFICATION_BASE" --source-sha "$VERIFICATION_SHA"', command['run'])
        self.assertIn('--wait-seconds 5400', command['run'])
        source = (root / 'scripts/native-runner-qualify.ps1').read_text()
        self.assertIn('refCommit.sha -cne $SourceSha', source)
        self.assertIn('native-runner-sandbox.ps1', source)
        self.assertLess(source.index('workflow run gui-native-interactive.yml'), source.index('-BundleDirectory $BundleDirectory -SourceSha $SourceSha'))


if __name__ == '__main__':
    unittest.main()
