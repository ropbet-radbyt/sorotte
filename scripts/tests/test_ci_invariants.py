from __future__ import annotations

import copy
import json
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock

import yaml

from scripts import ci_invariants as invariants
from scripts.tests import test_ci_policy as policy


WORKFLOW = ".github/workflows/first.yml"
OTHER_WORKFLOW = ".github/workflows/second.yml"
LABEL = "Produce required evidence"
PREFIX = "scripts/producer.py run"


def entry(workflow: str = WORKFLOW, job: str = "producer") -> dict:
    return {"workflow": workflow, "job_id": job, "step_id": "evidence", "display_name": LABEL}


def sample_jobs() -> dict:
    return {"producer": {"steps": [{"id": "evidence", "name": LABEL, "run": "python " + PREFIX}]}}


class WorkflowIdentityTests(unittest.TestCase):
    def test_every_named_repository_step_has_one_scoped_reviewed_id(self) -> None:
        expected = {(item["workflow"], item["job_id"], item["step_id"]) for item in invariants.contracts()}
        observed = set()
        paths = sorted((invariants.ROOT / ".github/workflows").glob("*.y*ml"))
        self.assertTrue(paths)
        for path in paths:
            workflow = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
            scope = invariants.workflow_name(path)
            jobs = invariants.canonicalize_labels(workflow["jobs"], path)
            for job_id, job in jobs.items():
                for step in job.get("steps", []):
                    if "name" in step:
                        self.assertIn("id", step, f"{scope}/{job_id}")
                        observed.add((scope, job_id, step["id"]))
        self.assertEqual(observed, expected)

    def test_all_policy_tests_still_pass_after_every_display_step_label_changes(self) -> None:
        # Preserve commands and action-pin comments, replacing only YAML labels.
        # This exercises the existing independently authored policy assertions.
        read_text = Path.read_text

        def renamed(path: Path, *args, **kwargs) -> str:
            text = read_text(path, *args, **kwargs)
            if path.parent == invariants.ROOT / ".github/workflows":
                text = re.sub(r"(?m)^(\s*- name: ).+$", r"\1A display label may change", text)
            return text

        suite = unittest.defaultTestLoader.loadTestsFromTestCase(policy.CiPolicyTests)
        result = unittest.TestResult()
        with mock.patch.object(Path, "read_text", renamed):
            suite.run(result)
        self.assertTrue(result.wasSuccessful(), result.errors + result.failures)
        self.assertGreater(result.testsRun, 0)
        self.assertEqual(result.skipped, [])

    def test_identity_survives_deepcopy_label_changes_and_independent_step_order(self) -> None:
        jobs = sample_jobs()
        jobs["producer"]["steps"].insert(0, {"id": "setup", "run": "echo setup"})
        jobs["producer"]["steps"][-1]["name"] = "New display wording"
        with mock.patch.object(invariants, "contracts", return_value=(entry(),)):
            scoped = invariants.canonicalize_labels(jobs, WORKFLOW)
            copied = copy.deepcopy(scoped)
            copied["producer"]["steps"].reverse()
            step = invariants.by_contract(copied, "producer", LABEL)
        self.assertEqual(copied.workflow, WORKFLOW)
        self.assertEqual(step["run"], "python " + PREFIX)

    def test_missing_duplicate_invalid_and_unregistered_id_changes_fail(self) -> None:
        mutations = (
            lambda jobs: jobs["producer"]["steps"][0].pop("id"),
            lambda jobs: jobs["producer"]["steps"][0].update(id="new_identity"),
            lambda jobs: jobs["producer"]["steps"].append(copy.deepcopy(jobs["producer"]["steps"][0])),
            lambda jobs: jobs["producer"]["steps"][0].update(id="invalid.id"),
            lambda jobs: jobs.pop("producer"),
        )
        with mock.patch.object(invariants, "contracts", return_value=(entry(),)):
            for mutate in mutations:
                with self.subTest(mutation=mutate):
                    jobs = sample_jobs()
                    mutate(jobs)
                    with self.assertRaises(AssertionError):
                        invariants.canonicalize_labels(jobs, WORKFLOW)

    def test_lookup_rechecks_identity_and_never_falls_back_to_display_name(self) -> None:
        with mock.patch.object(invariants, "contracts", return_value=(entry(),)):
            scoped = invariants.canonicalize_labels(sample_jobs(), WORKFLOW)
            scoped["producer"]["steps"][0].pop("id")
            with self.assertRaisesRegex(AssertionError, "required step ID"):
                invariants.by_contract(scoped, "producer", LABEL)
            with self.assertRaisesRegex(AssertionError, "explicit workflow scope"):
                invariants.by_contract(sample_jobs(), "producer", LABEL)

    def test_identical_job_and_step_ids_in_another_workflow_cannot_alias(self) -> None:
        second = entry(OTHER_WORKFLOW)
        second["display_name"] = "Another contract"
        with mock.patch.object(invariants, "contracts", return_value=(entry(), second)):
            scoped = invariants.canonicalize_labels(sample_jobs(), OTHER_WORKFLOW)
            with self.assertRaisesRegex(AssertionError, "missing or ambiguous"):
                invariants.by_contract(scoped, "producer", LABEL)
            self.assertEqual(invariants.by_contract(scoped, "producer", "Another contract")["id"], "evidence")

    def test_missing_or_malformed_reviewed_inventory_is_rejected(self) -> None:
        malformed = [
            [], {}, {"schema_version": True, "steps": [entry()]},
            {"schema_version": 1, "steps": []},
            {"schema_version": 1, "steps": [entry(), entry()]},
            {"schema_version": 1, "steps": [{**entry(), "step_id": 123}]},
            {"schema_version": 1, "steps": [{**entry(), "workflow": "../first.yml"}]},
            {"schema_version": 1, "steps": [entry(), {**entry(), "step_id": "different"}]},
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "contracts.json"
            with mock.patch.object(invariants, "INDEX", path):
                self.addCleanup(invariants.contracts.cache_clear)
                invariants.contracts.cache_clear()
                with self.assertRaisesRegex(AssertionError, "missing"):
                    invariants.contracts()
                for value in malformed:
                    with self.subTest(value=value):
                        path.write_text(json.dumps(value), encoding="utf-8")
                        invariants.contracts.cache_clear()
                        with self.assertRaises(AssertionError):
                            invariants.contracts()

    def test_cheap_static_preflight_precedes_full_self_tests(self) -> None:
        workflow = policy.parse_workflow(policy.WORKFLOW_PATH.read_text(encoding="utf-8"))
        steps = workflow["jobs"]["preflight"]["steps"]
        preflight = policy.named_step(workflow["jobs"], "preflight", "Validate static apparatus and environment")
        tests = policy.named_step(workflow["jobs"], "preflight", "Run cross-platform apparatus self-tests")
        self.assertLess(steps.index(preflight), steps.index(tests))
        self.assertNotIn("if", preflight)
        invariants.no_error_tolerance(preflight)


class RequiredGraphTests(unittest.TestCase):
    def graph(self) -> dict:
        return {"first": {}, "second": {}, "required": {"needs": ["first", "second"], "if": "always()"}}

    def check(self, jobs: dict, **kwargs) -> None:
        invariants.required_graph(jobs, "required", {"first", "second"}, **kwargs)

    def test_always_and_explicit_reviewed_event_restrictions_pass(self) -> None:
        jobs = self.graph()
        jobs["required"]["if"] = "${{  always()  }}"
        self.check(jobs)
        for expression in ("always() && github.event_name != 'schedule'", "always() && github.event_name == 'schedule'"):
            jobs["required"]["if"] = "${{ " + expression + " }}"
            self.check(jobs, allowed_if=expression)

    def test_always_substring_and_unreviewed_event_narrowing_do_not_pass(self) -> None:
        for expression in (None, False, "false", "success()", "always() && false",
                           "${{ always() && false }}", "always() || success()",
                           "always() && github.event_name != 'schedule'"):
            with self.subTest(expression=expression):
                jobs = self.graph()
                jobs["required"]["if"] = expression
                with self.assertRaises(AssertionError):
                    self.check(jobs)
        with self.assertRaisesRegex(AssertionError, "outside reviewed"):
            self.check(self.graph(), allowed_if="always() && false")

    def test_duplicate_missing_extra_and_nonexistent_dependencies_fail(self) -> None:
        for needs in ([], ["first"], ["first", "second", "second"], ["first", "second", "third"], None):
            with self.subTest(needs=needs):
                jobs = self.graph()
                jobs["required"]["needs"] = needs
                with self.assertRaises(AssertionError):
                    self.check(jobs)
        for missing in ("first", "required"):
            jobs = self.graph()
            del jobs[missing]
            with self.assertRaises(AssertionError):
                self.check(jobs)

    def test_all_dynamic_error_tolerance_fails_on_aggregate_and_producers(self) -> None:
        for job_id in ("required", "first", "second"):
            for value in (True, "true", "${{ true }}", "${{ false }}", "${{ needs.first.result != 'success' }}", None, 0):
                with self.subTest(job=job_id, value=value):
                    jobs = self.graph()
                    jobs[job_id]["continue-on-error"] = value
                    with self.assertRaises(AssertionError):
                        self.check(jobs)
            for value in (False, "false"):
                jobs = self.graph()
                jobs[job_id]["continue-on-error"] = value
                self.check(jobs)

    def test_selected_matrix_condition_requires_the_exact_reviewed_expression(self) -> None:
        jobs = self.graph()
        reviewed = "needs.preparation.outputs.matrix != '[]'"
        jobs["second"]["if"] = "${{ " + reviewed + " }}"
        with self.assertRaisesRegex(AssertionError, "skip condition"):
            self.check(jobs)
        self.check(jobs, dependency_conditions={"second": reviewed})
        jobs["second"]["if"] = reviewed + " && false"
        with self.assertRaisesRegex(AssertionError, "skip condition"):
            self.check(jobs, dependency_conditions={"second": reviewed})
        with self.assertRaisesRegex(AssertionError, "unrequired producer"):
            self.check(self.graph(), dependency_conditions={"third": reviewed})


class RequiredCommandTests(unittest.TestCase):
    def job(self, command: str = "python " + PREFIX) -> dict:
        return {"steps": [{"id": "evidence", "run": command}]}

    def test_actual_single_command_allows_comments_whitespace_and_continuation(self) -> None:
        for command in ("python " + PREFIX, "python3 " + PREFIX,
                        "# explanation\npython " + PREFIX + " # explanation",
                        "python scripts/producer.py \\\n run --output report.json",
                        "python scripts/producer.py `\n run --output report.json"):
            with self.subTest(command=command):
                self.assertEqual(invariants.command_step(self.job(command), PREFIX)["id"], "evidence")

    def test_commented_quoted_or_bypassed_command_is_not_execution(self) -> None:
        for command in ("# python " + PREFIX, "echo python " + PREFIX,
                        'echo "python ' + PREFIX + '"', "exit 0\npython " + PREFIX,
                        "python " + PREFIX + " || true", "python " + PREFIX + "; exit 0",
                        "if false; then\npython " + PREFIX + "\nfi",
                        "python " + PREFIX + " > /dev/null", "python " + PREFIX + " --help",
                        "python " + PREFIX + " --version", "python " + PREFIX + " -h"):
            with self.subTest(command=command):
                with self.assertRaises(AssertionError):
                    invariants.command_step(self.job(command), PREFIX)

    def test_duplicate_and_missing_command_fail(self) -> None:
        jobs = (self.job("echo no producer"), {"steps": self.job()["steps"] * 2})
        for job in jobs:
            with self.assertRaisesRegex(AssertionError, "one executable command"):
                invariants.command_step(job, PREFIX)

    def test_required_command_cannot_tolerate_failure_or_be_skipped(self) -> None:
        for value in ("true", True, "${{ needs.first.result != 'success' }}"):
            job = self.job()
            job["steps"][0]["continue-on-error"] = value
            with self.assertRaises(AssertionError):
                invariants.command_step(job, PREFIX)
        for expression in ("false", "always()", "${{ always() && false }}"):
            job = self.job()
            job["steps"][0]["if"] = expression
            with self.assertRaises(AssertionError):
                invariants.command_step(job, PREFIX)
        job = self.job()
        job["steps"][0]["if"] = "${{ always() }}"
        invariants.command_step(job, PREFIX, allowed_if="always()")
        job["steps"][0]["if"] = "always() && false"
        with self.assertRaises(AssertionError):
            invariants.command_step(job, PREFIX, allowed_if="always()")


if __name__ == "__main__":
    unittest.main()
