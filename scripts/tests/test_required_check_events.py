"""Required check names are publication authority, including skipped jobs.

Render only the two reviewed name/group expression shapes. This is deliberately
not a general GitHub expression evaluator; unknown syntax needs review.
"""
from __future__ import annotations

import copy
import json
from pathlib import Path
import re
import unittest
from unittest import mock

import yaml

from scripts.tests.test_merge_gate import FakeAPI, SHA, gate


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github/workflows"
REQUIRED = {
    "verification-required": ("rust-ci.yml", "verification_required"),
    "merge-required": ("rust-ci.yml", "merge-required"),
    "mutation-required": ("rust-mutation.yml", "mutation-required"),
    "fuzz-required": ("rust-fuzz.yml", "fuzz-required"),
    "dependency-required": ("dependency-policy.yml", "dependency-required"),
    "package-required": ("package-ci.yml", "package-required"),
    "native-required": ("native-required.yml", "native-required"),
}
AUTHORITY = {name: ".github/workflows/" + entry[0] for name, entry in REQUIRED.items()}
EVENTS = {
    name: {"pull_request", "schedule", "workflow_dispatch"}
    for name in ("rust-ci.yml", "rust-mutation.yml", "rust-fuzz.yml", "dependency-policy.yml")
}
EVENTS.update({"package-ci.yml": {"pull_request", "workflow_dispatch"},
               "native-required.yml": {"pull_request"}})
GROUPS = {
    "rust-ci.yml": ("sorotte-ci-", "pr-or-ref"),
    "rust-mutation.yml": ("sorotte-mutation-", "ref"),
    "rust-fuzz.yml": ("sorotte-protocol-fuzz-", "ref"),
    "dependency-policy.yml": ("dependencies-", "ref"),
    "package-ci.yml": ("package-verification-", "pr-or-ref"),
}
CANCEL = {
    "rust-ci.yml": "${{ github.event_name != 'schedule' && github.event_name != 'workflow_dispatch' }}",
    "rust-mutation.yml": "true",
    "rust-fuzz.yml": "${{ github.event_name != 'schedule' && github.event_name != 'workflow_dispatch' }}",
    "dependency-policy.yml": "true",
    "package-ci.yml": "${{ github.event_name == 'pull_request' }}",
}
NAME_EXPRESSION = re.compile(
    r"\$\{\{\s*\(\s*github\.event_name\s*==\s*'(?P<first>[a-z_]+)'\s*"
    r"\|\|\s*github\.event_name\s*==\s*'(?P<second>[a-z_]+)'\s*\)\s*"
    r"&&\s*'(?P<required>[a-z-]+)'\s*\|\|\s*"
    r"format\('(?P<other>[a-z-]+-\{0\})',\s*github\.event_name\)\s*\}\}"
)
GROUP_EXPRESSION = re.compile(r"\$\{\{\s*(.*?)\s*\}\}")


def workflow(name: str) -> dict:
    return yaml.load((WORKFLOWS / name).read_text(encoding="utf-8"), Loader=yaml.BaseLoader)


def render_name(value: str, event: str) -> str:
    if not isinstance(value, str) or not value:
        raise AssertionError("check name must be nonempty text")
    if "${{" not in value:
        return value
    match = NAME_EXPRESSION.fullmatch(value)
    if match is None:
        raise AssertionError("check name has an unreviewed expression")
    if event in (match["first"], match["second"]):
        return match["required"]
    return match["other"].replace("{0}", event)


def render_group(value: str, event: str, ref: str, pr: int | None = None) -> str:
    def replace(match: re.Match) -> str:
        expressions = {"github.event_name": event, "github.ref": ref,
                       "github.event.pull_request.number || github.ref": str(pr) if pr else ref}
        if match[1] not in expressions:
            raise AssertionError("concurrency group has an unreviewed expression")
        return expressions[match[1]]
    result = GROUP_EXPRESSION.sub(replace, value)
    if "${{" in result:
        raise AssertionError("malformed concurrency expression")
    return result


def configured_events(value: dict, name: str) -> set[str]:
    triggers = value.get("on")
    if not isinstance(triggers, dict) or set(triggers) != EVENTS[name]:
        raise AssertionError("required-workflow events changed; review every emitted check name")
    if "push" in triggers:
        raise AssertionError("application qualification must finish before merge")
    if triggers["pull_request"] != "":
        raise AssertionError("required PR checks cannot acquire an unreviewed trigger filter")
    return set(triggers)


def check_name(name: str, event: str, value: dict | None = None) -> str:
    path, job_id = REQUIRED[name]
    value = workflow(path) if value is None else value
    return render_name(value["jobs"][job_id].get("name", job_id), event)


def full_api() -> FakeAPI:
    api = FakeAPI()
    template = copy.deepcopy(api.runs[42])
    api.runs, api.checks = {}, []
    api.protection["required_status_checks"]["contexts"] = list(AUTHORITY)
    run_ids = {path: index + 42 for index, path in enumerate(dict.fromkeys(AUTHORITY.values()))}
    for index, (name, path) in enumerate(AUTHORITY.items()):
        run_id = run_ids[path]
        api.runs[run_id] = {**template, "id": run_id, "path": path, "check_suite_id": run_id + 100,
                           "html_url": f"https://github.com/{api.repository}/actions/runs/{run_id}"}
        api.checks.append({"id": index + 1000, "name": name, "head_sha": SHA,
            "status": "completed", "conclusion": "success", "app": {"slug": "github-actions"},
            "check_suite": {"id": run_id + 100},
            "details_url": api.runs[run_id]["html_url"] + f"/job/{index + 1000}",
            "completed_at": "2026-09-07T10:30:00Z"})
    return api


def auxiliary_check(api: FakeAPI, name: str, event: str, conclusion: str | None) -> dict:
    index = list(REQUIRED).index(name)
    run_id = 500 + index
    original = next(row for row in api.checks if row["name"] == name)
    run = copy.deepcopy(api.runs[int(original["details_url"].split("/runs/")[1].split("/")[0])])
    status = "in_progress" if conclusion is None else "completed"
    run.update(id=run_id, event=event, run_attempt=1, check_suite_id=run_id + 100,
               status=status, conclusion=conclusion,
               html_url=f"https://github.com/{api.repository}/actions/runs/{run_id}")
    api.runs[run_id] = run
    return {**copy.deepcopy(original), "id": run_id + 1000, "name": check_name(name, event),
            "check_suite": {"id": run_id + 100}, "status": status, "conclusion": conclusion,
            "details_url": run["html_url"] + f"/job/{run_id + 1000}",
            "completed_at": None if conclusion is None else "2026-09-07T10:41:24Z"}


class RequiredCheckEventTests(unittest.TestCase):
    def test_every_declared_required_workflow_event_is_reviewed(self):
        policy = json.loads((ROOT / "coverage/verification-lanes.json").read_text(encoding="utf-8"))
        self.assertEqual(policy["required_checks"], AUTHORITY)
        for name in EVENTS:
            with self.subTest(workflow=name):
                self.assertEqual(configured_events(workflow(name), name), EVENTS[name])

    def test_new_event_and_tag_or_feature_push_triggers_require_review(self):
        for name in EVENTS:
            for variant in ("new-event", "tags", "feature-branch", "unfiltered-push", "filtered-pr"):
                value = workflow(name)
                if variant == "new-event": value["on"]["workflow_run"] = {}
                if variant == "tags": value["on"]["push"] = {"tags": ["v*"]}
                if variant == "feature-branch": value["on"]["push"] = {"branches": ["feature"]}
                if variant == "unfiltered-push": value["on"]["push"] = ""
                if variant == "filtered-pr": value["on"]["pull_request"] = {"paths": ["docs/**"]}
                with self.subTest(workflow=name, variant=variant), self.assertRaises(AssertionError):
                    configured_events(value, name)

    def test_pr_emits_exactly_the_seven_unique_reserved_contexts(self):
        for event in ("pull_request",):
            names = [check_name(name, event) for name in REQUIRED]
            with self.subTest(event=event):
                self.assertEqual(len(names), len(set(names)))
                self.assertEqual(set(names), set(REQUIRED))
                self.assertEqual(names, list(REQUIRED))

    def test_all_scheduled_and_manual_aggregate_names_are_nonreserved_even_when_skipped(self):
        for required, (path, job_id) in REQUIRED.items():
            value = workflow(path)
            for event in configured_events(value, path) - {"push", "pull_request"}:
                with self.subTest(workflow=path, job=job_id, event=event):
                    # Do not discard jobs whose `if` is false: GitHub emits their skipped checks.
                    actual = check_name(required, event, value)
                    self.assertNotIn(actual, REQUIRED)
                    self.assertEqual(actual, required.removesuffix("-required") + "-" + event)

    def test_unknown_expression_shapes_cannot_silently_pass_the_renderer(self):
        for value in ("${{ github.event_name }}", "${{ true && 'verification-required' }}",
                      "${{ (github.event_name == 'push') && 'verification-required' || '' }}"):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                render_name(value, "schedule")
        with self.assertRaises(AssertionError):
            render_group("ci-${{ github.run_id }}", "push", "refs/heads/main")

    def test_event_groups_are_disjoint_at_the_same_ref(self):
        for path, (prefix, _scope) in GROUPS.items():
            value = workflow(path)
            events = sorted(configured_events(value, path))
            groups = {event: render_group(value["concurrency"]["group"], event, "refs/heads/main")
                      for event in events}
            with self.subTest(workflow=path):
                self.assertEqual(len(set(groups.values())), len(events))
                self.assertEqual(groups, {event: prefix + event + "-refs/heads/main" for event in events})
                self.assertEqual(value["concurrency"]["cancel-in-progress"], CANCEL[path])

    def test_same_event_scope_and_supersession_policy_are_preserved(self):
        for path, (prefix, scope) in GROUPS.items():
            value = workflow(path)
            group = value["concurrency"]["group"]
            with self.subTest(workflow=path):
                for number in (47, 48):
                    ref = f"refs/pull/{number}/merge"
                    expected = str(number) if scope == "pr-or-ref" else ref
                    # A new SHA/attempt in one PR retains its group; the next PR has another.
                    self.assertEqual(render_group(group, "pull_request", ref, number), prefix + "pull_request-" + expected)
                self.assertNotEqual(render_group(group, "pull_request", "refs/pull/47/merge", 47),
                                    render_group(group, "pull_request", "refs/pull/48/merge", 48))
                self.assertEqual(render_group(group, "push", "refs/heads/main"), prefix + "push-refs/heads/main")
                self.assertEqual(value["concurrency"]["cancel-in-progress"], CANCEL[path])
        native = workflow("native-required.yml")
        self.assertEqual(native["concurrency"], {
            "group": "native-required-${{ github.event.pull_request.number || github.ref }}",
            "cancel-in-progress": "${{ github.event_name == 'pull_request' }}"})
        self.assertEqual(workflow("gui-native-interactive.yml")["concurrency"], {
            "group": "sorotte-native-interactive", "cancel-in-progress": "false"})


class RequiredCheckGateInteractionTests(unittest.TestCase):
    def test_rendered_auxiliary_checks_cannot_displace_successful_main_push_authority(self):
        for event in ("schedule", "workflow_dispatch"):
            for conclusion in ("skipped", "success", "failure", None):
                for prepend in (False, True):
                    api = full_api()
                    auxiliary = [auxiliary_check(api, name, event, conclusion) for name, (path, _) in REQUIRED.items()
                                 if event in EVENTS[path]]
                    api.checks = auxiliary + api.checks if prepend else api.checks + auxiliary
                    with self.subTest(event=event, conclusion=conclusion, prepend=prepend), mock.patch.object(gate.time, "sleep") as sleep:
                        gate.wait_checks(api, SHA, AUTHORITY, wait_seconds=0, poll_seconds=1)
                        result = gate.authorize(api, SHA, AUTHORITY)
                        self.assertEqual([row["name"] for row in result["producers"]], list(REQUIRED))
                        self.assertTrue(all(row["run_id"] < 500 for row in result["producers"]))
                        self.assertFalse(any(path.startswith("actions/runs/5") for path in api.calls))
                        sleep.assert_not_called()

    def test_true_reserved_duplicate_is_rejected_before_waiting_or_selecting_newest(self):
        for event in ("push", "schedule", "workflow_dispatch"):
            for conclusion in ("skipped", "success", "failure", None):
                for prepend in (False, True):
                    api = full_api()
                    extra = auxiliary_check(api, "verification-required", event, conclusion)
                    extra["name"] = "verification-required"
                    api.checks.insert(0 if prepend else len(api.checks), extra)
                    with self.subTest(event=event, conclusion=conclusion, prepend=prepend), mock.patch.object(gate.time, "sleep") as sleep:
                        with self.assertRaisesRegex(gate.GateError, "expected one latest check, found 2"):
                            gate.wait_checks(api, SHA, AUTHORITY, wait_seconds=0, poll_seconds=1)
                        with self.assertRaisesRegex(gate.GateError, "expected one latest check, found 2"):
                            gate.authorize(api, SHA, AUTHORITY)
                        sleep.assert_not_called()
                        self.assertFalse(any(path.startswith("actions/runs/") for path in api.calls))

    def test_required_label_cannot_authorize_nonpush_foreign_source_or_untrusted_app(self):
        for variant in ("schedule", "workflow_dispatch", "pull_request", "source", "app", "repository"):
            api = full_api()
            check = api.checks[0]
            if variant in ("schedule", "workflow_dispatch", "pull_request"):
                api.runs[42]["event"] = variant
            if variant == "source": check["head_sha"] = "b" * 40
            if variant == "app": check["app"]["slug"] = "untrusted-app"
            if variant == "repository": api.runs[42]["head_repository"] = {"full_name": "other/repo"}
            with self.subTest(variant=variant), mock.patch.object(gate.time, "sleep") as sleep:
                with self.assertRaises(gate.GateError):
                    gate.wait_checks(api, SHA, AUTHORITY, wait_seconds=0, poll_seconds=1)
                with self.assertRaises(gate.GateError):
                    gate.authorize(api, SHA, AUTHORITY)
                sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
