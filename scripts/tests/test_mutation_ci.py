from __future__ import annotations

import contextlib
import copy
import dataclasses
import hashlib
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import mutation_ci  # noqa: E402


class MutationFixture:
    source = "crates/demo/src/lib.rs"

    def __init__(self, root: pathlib.Path) -> None:
        self.root = root
        self.results = root / "mutants.out"
        self.results.mkdir(parents=True)
        self.source_bytes = b"pub fn demo() -> bool {\n    true\n}\n"
        self.mutant = {
            "diff": (
                "--- crates/demo/src/lib.rs\n"
                "+++ replace demo -> bool with false\n"
                "@@ -1,3 +1,3 @@\n"
                " pub fn demo() -> bool {\n"
                "-    true\n"
                "+    false\n"
                " }\n"
            ),
            "file": self.source,
            "function": {
                "function_name": "demo",
                "return_type": "-> bool",
                "span": {
                    "start": {"line": 1, "column": 1},
                    "end": {"line": 3, "column": 2},
                },
            },
            "genre": "FnValue",
            "name": (
                "crates/demo/src/lib.rs:2:5: "
                "replace demo -> bool with false"
            ),
            "package": "demo",
            "replacement": "false",
            "span": {
                "start": {"line": 2, "column": 5},
                "end": {"line": 2, "column": 9},
            },
        }
        self.inventory = [self.mutant]
        self.outcomes = {
            "outcomes": [
                {
                    "scenario": "Baseline",
                    "summary": "Success",
                    "log_path": "log/baseline.log",
                    "diff_path": None,
                    "phase_results": [
                        self.phase("Build", "Success", no_run=True),
                        self.phase("Test", "Success"),
                    ],
                },
                {
                    "scenario": {
                        "Mutant": {
                            key: value
                            for key, value in self.mutant.items()
                            if key != "diff"
                        }
                    },
                    "summary": "CaughtMutant",
                    "log_path": "log/mutant.log",
                    "diff_path": "diff/mutant.diff",
                    "phase_results": [
                        self.phase("Build", "Success", no_run=True),
                        self.phase("Test", {"Failure": 101}),
                    ],
                },
            ],
            "total_mutants": 1,
            "missed": 0,
            "caught": 1,
            "timeout": 0,
            "unviable": 0,
            "success": 0,
            "start_time": "2026-07-29T00:00:00Z",
            "end_time": "2026-07-29T00:00:01Z",
            "cargo_mutants_version": "27.1.0",
        }
        self.write()

    @staticmethod
    def phase(
        name: str,
        status: str | dict[str, int],
        *,
        no_run: bool = False,
    ) -> dict[str, object]:
        argv = [
            "cargo",
            "test",
            "--locked",
            "--all-features",
            "--verbose",
            "--package=demo@0.1.0",
        ]
        if no_run:
            argv.insert(3, "--no-run")
        return {
            "phase": name,
            "duration": 0.01,
            "process_status": status,
            "argv": argv,
        }

    def write(self, results: pathlib.Path | None = None) -> None:
        destination = results or self.results
        (destination / "log").mkdir(parents=True, exist_ok=True)
        (destination / "diff").mkdir(parents=True, exist_ok=True)
        self.write_json(destination / "mutants.json", self.inventory)
        self.write_json(destination / "outcomes.json", self.outcomes)
        self.write_json(
            destination / "lock.json",
            {
                "cargo_mutants_version": "27.1.0",
                "start_time": "2026-07-29T00:00:00Z",
                "hostname": "fixture-host",
                "username": "fixture-user",
            },
        )
        (destination / "log" / "baseline.log").write_text(
            "baseline\n",
            encoding="utf-8",
        )
        (destination / "log" / "mutant.log").write_text(
            "caught\n",
            encoding="utf-8",
        )
        (destination / "diff" / "mutant.diff").write_text(
            self.mutant["diff"],
            encoding="utf-8",
        )
        (destination / "caught.txt").write_text(
            self.mutant["name"] + "\n",
            encoding="utf-8",
        )
        (destination / "missed.txt").write_text("", encoding="utf-8")
        (destination / "timeout.txt").write_text("", encoding="utf-8")
        (destination / "unviable.txt").write_text("", encoding="utf-8")

    @staticmethod
    def write_json(path: pathlib.Path, value: object) -> None:
        path.write_text(
            json.dumps(value, indent=2) + "\n",
            encoding="utf-8",
        )

    def binding(self) -> list[dict[str, object]]:
        return [
            {
                "path": self.source,
                "bytes": len(self.source_bytes),
                "sha256": hashlib.sha256(self.source_bytes).hexdigest(),
            }
        ]


class MutationEvaluationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.fixture = MutationFixture(self.root)
        self.shard = mutation_ci.ShardPolicy(
            identifier="demo",
            owner="demo-owner",
            package="demo",
            files=(self.fixture.source,),
            mutant_filter="",
            test_target="package",
            test_filter="",
            jobs=2,
            timeout_seconds=60,
            build_timeout_seconds=120,
            minimum_viable_kill_percent=mutation_ci.decimal.Decimal("100.00"),
            max_missed=0,
            max_timeouts=0,
            require_baseline=True,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def evaluate(
        self,
        *,
        exit_code: int = 0,
        accepted: tuple[mutation_ci.AcceptedUnviable, ...] = (),
        source_after: list[dict[str, object]] | None = None,
    ) -> dict[str, object]:
        binding = self.fixture.binding()
        return mutation_ci.evaluate_results(
            results_dir=self.fixture.results,
            shard=self.shard,
            accepted=accepted,
            expected_version="27.1.0",
            producer_exit_code=exit_code,
            pre_inventory=self.fixture.inventory,
            source_before=binding,
            source_after=source_after if source_after is not None else binding,
        )

    def rewrite_outcomes(self) -> None:
        self.fixture.write_json(
            self.fixture.results / "outcomes.json",
            self.fixture.outcomes,
        )

    def test_complete_caught_fixture_passes_with_source_bound_inventory(self) -> None:
        report = self.evaluate()

        self.assertEqual(report["status"], "passed")
        self.assertEqual(
            report["summary"],
            {
                "caught": 1,
                "missed": 0,
                "timeout": 0,
                "unviable": 0,
                "total_mutants": 1,
                "viable_mutants": 1,
                "viable_kill_percent": "100.00",
                "baseline": "passed",
            },
        )
        self.assertEqual(report["survivors"], [])
        self.assertEqual(
            report["inventory"]["canonical_sha256"],
            report["inventory"]["pre_run_canonical_sha256"],
        )

    def test_top_level_mutant_with_null_function_metadata_is_supported(self) -> None:
        self.fixture.mutant["function"] = None
        self.fixture.outcomes["outcomes"][1]["scenario"]["Mutant"]["function"] = None
        self.fixture.write()

        normalized = mutation_ci.parse_inventory(
            self.fixture.inventory,
            shard=self.shard,
            label="top-level fixture",
        )
        self.assertIsNone(normalized[0]["function"])
        self.assertEqual(
            mutation_ci.mutant_identity(normalized[0]),
            (
                self.fixture.source,
                mutation_ci.TOP_LEVEL_FUNCTION,
                mutation_ci.TOP_LEVEL_RETURN_TYPE,
                "FnValue",
                "false",
            ),
        )
        self.assertEqual(self.evaluate()["status"], "passed")

    def test_non_null_function_metadata_remains_strictly_structured(self) -> None:
        for invalid in [False, "demo", [], {}]:
            mutant = copy.deepcopy(self.fixture.mutant)
            mutant["function"] = invalid
            with self.subTest(function=invalid), self.assertRaises(
                mutation_ci.MutationCiError
            ):
                mutation_ci.parse_inventory(
                    [mutant],
                    shard=self.shard,
                    label="invalid function fixture",
                )

    def test_summary_count_cannot_contradict_detailed_outcomes(self) -> None:
        self.fixture.outcomes["caught"] = 0
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "summary counts contradict detail",
        ):
            self.evaluate()

    def test_outcome_timestamps_must_be_ordered_utc_instants(self) -> None:
        self.fixture.outcomes["end_time"] = "2026-07-28T23:59:59Z"
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "precedes",
        ):
            self.evaluate()

    def test_mutation_cargo_phases_must_retain_all_features(self) -> None:
        self.fixture.outcomes["outcomes"][1]["phase_results"][1]["argv"].remove(
            "--all-features"
        )
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "all package features",
        ):
            self.evaluate()

    def test_mutation_cargo_phases_must_retain_configured_test_scope(self) -> None:
        self.shard = dataclasses.replace(
            self.shard,
            test_target="lib",
            test_filter="auth::tests::",
        )
        for outcome in self.fixture.outcomes["outcomes"]:
            for phase in outcome["phase_results"]:
                if phase["phase"] == "Test":
                    phase["argv"].extend(["--lib", "auth::tests::"])
        self.fixture.outcomes["outcomes"][1]["phase_results"][1]["argv"].remove(
            "auth::tests::"
        )
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "does not exactly match the configured 'lib' test scope",
        ):
            self.evaluate()

    def test_every_inventory_mutant_requires_exactly_one_outcome(self) -> None:
        self.fixture.outcomes["outcomes"].pop()
        self.fixture.outcomes["caught"] = 0
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "outcomes are incomplete",
        ):
            self.evaluate()

    def test_status_text_must_match_structured_outcomes(self) -> None:
        (self.fixture.results / "caught.txt").write_text("", encoding="utf-8")

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "caught.txt does not exactly match",
        ):
            self.evaluate()

    def test_outcome_mutant_must_exactly_match_inventory(self) -> None:
        scenario = self.fixture.outcomes["outcomes"][1]["scenario"]["Mutant"]
        scenario["replacement"] = "true"
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "does not exactly match the inventory",
        ):
            self.evaluate()

    def test_source_change_during_run_is_rejected_before_policy_scoring(self) -> None:
        changed = copy.deepcopy(self.fixture.binding())
        changed[0]["sha256"] = "0" * 64

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "sources changed during the run",
        ):
            self.evaluate(source_after=changed)

    def test_duplicate_json_keys_are_rejected(self) -> None:
        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "duplicate key",
        ):
            mutation_ci.parse_json_bytes(
                b'{"caught": 1, "caught": 0}',
                label="adversarial",
            )

    def test_artifact_path_traversal_is_rejected(self) -> None:
        self.fixture.outcomes["outcomes"][1]["log_path"] = "../outside.log"
        self.rewrite_outcomes()

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "safe relative artifact path",
        ):
            self.evaluate()

    def test_nonzero_producer_with_all_mutants_caught_is_rejected(self) -> None:
        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "exited nonzero despite complete",
        ):
            self.evaluate(exit_code=1)

    def test_survivor_is_a_policy_failure_and_requires_nonzero_producer(self) -> None:
        outcome = self.fixture.outcomes["outcomes"][1]
        outcome["summary"] = "MissedMutant"
        outcome["phase_results"][1]["process_status"] = "Success"
        self.fixture.outcomes["caught"] = 0
        self.fixture.outcomes["missed"] = 1
        self.fixture.write()
        (self.fixture.results / "caught.txt").write_text("", encoding="utf-8")
        (self.fixture.results / "missed.txt").write_text(
            self.fixture.mutant["name"] + "\n",
            encoding="utf-8",
        )

        report = self.evaluate(exit_code=1)

        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["survivors"], [self.fixture.mutant["name"]])
        self.assertIn("missed mutants 1 exceed maximum 0", report["errors"])

    def test_zero_producer_with_survivor_is_contradictory(self) -> None:
        outcome = self.fixture.outcomes["outcomes"][1]
        outcome["summary"] = "MissedMutant"
        outcome["phase_results"][1]["process_status"] = "Success"
        self.fixture.outcomes["caught"] = 0
        self.fixture.outcomes["missed"] = 1
        self.fixture.write()
        (self.fixture.results / "caught.txt").write_text("", encoding="utf-8")
        (self.fixture.results / "missed.txt").write_text(
            self.fixture.mutant["name"] + "\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "exited zero despite missed",
        ):
            self.evaluate(exit_code=0)

    def test_stale_accepted_unviable_entry_fails_policy(self) -> None:
        accepted = mutation_ci.AcceptedUnviable(
            identifier="stale",
            shard="demo",
            file=self.fixture.source,
            function="demo",
            return_type="-> bool",
            genre="FnValue",
            replacement="false",
            reason="A deliberately long fixture explanation for the exception.",
            review_by=mutation_ci.dt.date(2099, 1, 1),
        )

        report = self.evaluate(accepted=(accepted,))

        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["stale_accepted_unviable"], ["stale"])


class MutationPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name) / "repo"
        (self.repo / "crates" / "demo" / "src").mkdir(parents=True)
        (self.repo / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/demo"]\nresolver = "3"\n',
            encoding="utf-8",
        )
        (self.repo / "crates" / "demo" / "Cargo.toml").write_text(
            '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2024"\n',
            encoding="utf-8",
        )
        (self.repo / "crates" / "demo" / "src" / "lib.rs").write_text(
            "pub fn demo() -> bool { true }\n",
            encoding="utf-8",
        )
        self.policy_path = self.repo / "coverage" / "mutation-policy.toml"
        self.policy_path.parent.mkdir()
        self.write_policy()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def policy_text(self, *, accepted: bool = True) -> str:
        prefix = (
            'schema_version = 3\n'
            'cargo_mutants_version = "27.1.0"\n'
        )
        if not accepted:
            prefix += "accepted_unviable = []\n"
        body = """

[[shard]]
id = "demo"
owner = "demo-owner"
package = "demo"
files = ["crates/demo/src/lib.rs"]
mutant_filter = ""
test_target = "package"
test_filter = ""
jobs = 2
timeout_seconds = 60
build_timeout_seconds = 120
minimum_viable_kill_percent = "100.00"
max_missed = 0
max_timeouts = 0
require_baseline = true
"""
        exception = """

[[accepted_unviable]]
id = "const-exception"
shard = "demo"
file = "crates/demo/src/lib.rs"
function = "demo"
return_type = "-> bool"
genre = "FnValue"
replacement = "false"
reason = "The fixture compiler rejects this deliberately synthetic mutation."
review_by = "2099-01-01"
"""
        return prefix + body + (exception if accepted else "")

    def write_policy(self, value: str | None = None) -> None:
        self.policy_path.write_text(
            value if value is not None else self.policy_text(),
            encoding="utf-8",
        )

    def load(self) -> mutation_ci.MutationPolicy:
        return mutation_ci.load_policy(
            self.repo.resolve(),
            self.policy_path,
            today=mutation_ci.dt.date(2026, 7, 29),
        )

    def test_valid_policy_binds_source_to_its_nearest_package(self) -> None:
        policy = self.load()

        self.assertEqual(policy.cargo_mutants_version, "27.1.0")
        self.assertEqual(policy.shard("demo").files, ("crates/demo/src/lib.rs",))
        self.assertEqual(policy.shard("demo").mutant_filter, "")
        self.assertEqual(policy.shard("demo").test_target, "package")
        self.assertEqual(policy.shard("demo").test_filter, "")
        self.assertEqual(len(policy.accepted_for("demo")), 1)

    def test_accepted_unviable_allows_implicit_unit_return_identity(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                'return_type = "-> bool"',
                'return_type = ""',
            )
        )

        accepted = self.load().accepted_for("demo")

        self.assertEqual(len(accepted), 1)
        self.assertEqual(accepted[0].return_type, "")

    def test_unknown_policy_field_is_rejected(self) -> None:
        self.write_policy(self.policy_text() + "\nunknown = true\n")

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "fields do not match schema",
        ):
            self.load()

    def test_source_path_traversal_is_rejected(self) -> None:
        value = self.policy_text().replace(
            'files = ["crates/demo/src/lib.rs"]',
            'files = ["../outside.rs"]',
        )
        self.write_policy(value)

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "normalized literal relative path",
        ):
            self.load()

    def test_package_mismatch_is_rejected(self) -> None:
        self.write_policy(
            self.policy_text().replace('package = "demo"', 'package = "other"')
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "belongs to package",
        ):
            self.load()

    def test_boolean_is_not_accepted_as_integer_budget(self) -> None:
        self.write_policy(self.policy_text().replace("jobs = 2", "jobs = true"))

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "jobs must be an integer",
        ):
            self.load()

    def test_percentage_must_be_a_decimal_string(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                'minimum_viable_kill_percent = "100.00"',
                "minimum_viable_kill_percent = 100.0",
            )
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "must be a non-empty string",
        ):
            self.load()

    def test_expired_accepted_unviable_is_rejected(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                'review_by = "2099-01-01"',
                'review_by = "2026-07-28"',
            )
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "review expired",
        ):
            self.load()

    def test_baseline_cannot_be_disabled_by_policy(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                "require_baseline = true",
                "require_baseline = false",
            )
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "cannot disable the baseline",
        ):
            self.load()

    def test_test_filter_requires_library_target(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                'test_filter = ""',
                'test_filter = "auth::tests::"',
            )
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "test_filter requires test_target = 'lib'",
        ):
            self.load()

    def test_test_filter_rejects_cargo_arguments(self) -> None:
        self.write_policy(
            self.policy_text()
            .replace('test_target = "package"', 'test_target = "lib"')
            .replace('test_filter = ""', 'test_filter = "-- --nocapture"')
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "Rust test selector prefix",
        ):
            self.load()

    def test_mutant_filter_rejects_invalid_regular_expression(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                'mutant_filter = ""',
                'mutant_filter = "("',
            )
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "not a valid regular expression",
        ):
            self.load()

    def test_mutant_filter_is_required_single_line_and_bounded(self) -> None:
        missing = self.policy_text().replace('mutant_filter = ""\n', "")
        self.write_policy(missing)
        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "fields do not match schema",
        ):
            self.load()

        for value, message in (
            ("x" * (mutation_ci.MAX_MUTANT_FILTER_BYTES + 1), "exceeds"),
            ("demo\nother", "must remain on one line"),
        ):
            with self.subTest(message=message):
                self.write_policy(
                    self.policy_text().replace(
                        'mutant_filter = ""',
                        f"mutant_filter = {json.dumps(value)}",
                    )
                )
                with self.assertRaisesRegex(
                    mutation_ci.MutationCiError,
                    message,
                ):
                    self.load()

    def test_mutant_filter_is_owned_and_inventory_is_fail_closed(self) -> None:
        self.write_policy(
            self.policy_text().replace(
                'mutant_filter = ""',
                'mutant_filter = "replace demo"',
            )
        )
        shard = self.load().shard("demo")
        command = mutation_ci.cargo_mutants_base_command(shard)
        self.assertEqual(command[command.index("--re") + 1], "replace demo")

        mutant = copy.deepcopy(MutationFixture(self.repo).mutant)
        mutant["name"] = mutant["name"].replace("replace demo", "replace other")
        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "outside shard mutant_filter",
        ):
            mutation_ci.parse_inventory(
                [mutant],
                shard=shard,
                label="filtered inventory",
            )

    def test_accepted_unviable_must_belong_to_declared_shard_file(self) -> None:
        (self.repo / "crates" / "demo" / "src" / "other.rs").write_text(
            "pub fn other() {}\n",
            encoding="utf-8",
        )
        self.write_policy(
            self.policy_text().replace(
                'file = "crates/demo/src/lib.rs"',
                'file = "crates/demo/src/other.rs"',
            )
        )

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "is not owned by shard",
        ):
            self.load()

    def test_command_is_owned_and_locked_by_the_wrapper(self) -> None:
        shard = self.load().shard("demo")

        self.assertEqual(
            mutation_ci.cargo_mutants_base_command(shard),
            [
                "cargo",
                "mutants",
                "--package",
                "demo",
                "--file",
                "crates/demo/src/lib.rs",
                "--no-config",
                "--colors",
                "never",
                "--no-times",
                "--no-shuffle",
                "--all-features",
                "--cargo-arg=--locked",
                "--jobs",
                "2",
                "--timeout",
                "60",
                "--build-timeout",
                "120",
            ],
        )

    def test_focused_library_test_scope_is_owned_by_the_wrapper(self) -> None:
        self.write_policy(
            self.policy_text()
            .replace('test_target = "package"', 'test_target = "lib"')
            .replace('test_filter = ""', 'test_filter = "auth::tests::"')
        )
        shard = self.load().shard("demo")

        command = mutation_ci.cargo_mutants_base_command(shard)
        self.assertIn("--cargo-test-arg=--lib", command)
        self.assertIn("--cargo-test-arg=auth::tests::", command)
        self.assertLess(
            command.index("--cargo-test-arg=--lib"),
            command.index("--cargo-test-arg=auth::tests::"),
        )
        self.assertEqual(
            mutation_ci.cargo_test_inventory_command(shard),
            [
                "cargo",
                "test",
                "--package",
                "demo",
                "--locked",
                "--all-features",
                "--lib",
                "auth::tests::",
                "--",
                "--list",
                "--format",
                "terse",
            ],
        )

    def test_function_prefix_test_scope_is_owned_and_fenced(self) -> None:
        self.write_policy(
            self.policy_text()
            .replace('test_target = "package"', 'test_target = "lib"')
            .replace(
                'test_filter = ""',
                'test_filter = "auth::tests::participant_status_"',
            )
        )
        shard = self.load().shard("demo")

        self.assertIn(
            "--cargo-test-arg=auth::tests::participant_status_",
            mutation_ci.cargo_mutants_base_command(shard),
        )
        self.assertEqual(
            mutation_ci.parse_test_inventory(
                "auth::tests::participant_status_epoch: test\n",
                shard=shard,
            ),
            ["auth::tests::participant_status_epoch"],
        )
        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "escaped the configured test selector prefix",
        ):
            mutation_ci.parse_test_inventory(
                "auth::tests::other_status: test\n",
                shard=shard,
            )

    def test_test_inventory_rejects_empty_selection(self) -> None:
        shard = self.load().shard("demo")

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "must contain at least one test",
        ):
            mutation_ci.parse_test_inventory("", shard=shard)

    def test_focused_test_inventory_rejects_selector_escape(self) -> None:
        self.write_policy(
            self.policy_text()
            .replace('test_target = "package"', 'test_target = "lib"')
            .replace('test_filter = ""', 'test_filter = "auth::tests::"')
        )
        shard = self.load().shard("demo")

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "escaped the configured test selector prefix",
        ):
            mutation_ci.parse_test_inventory(
                "other::auth::tests::collision: test\n",
                shard=shard,
            )

    def test_mutation_inventory_rejects_empty_selection(self) -> None:
        shard = self.load().shard("demo")

        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "must contain at least one mutant",
        ):
            mutation_ci.parse_inventory(
                [],
                shard=shard,
                label="pre-run cargo-mutants inventory",
            )


class MutationRunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name) / "repo"
        (self.repo / "crates" / "demo" / "src").mkdir(parents=True)
        (self.repo / "coverage").mkdir()
        (self.repo / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/demo"]\nresolver = "3"\n',
            encoding="utf-8",
        )
        (self.repo / "crates" / "demo" / "Cargo.toml").write_text(
            '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2024"\n',
            encoding="utf-8",
        )
        source = self.repo / MutationFixture.source
        source.write_bytes(b"pub fn demo() -> bool {\n    true\n}\n")
        policy = MutationPolicyTests.policy_text(self, accepted=False)
        (self.repo / "coverage" / "mutation-policy.toml").write_text(
            policy,
            encoding="utf-8",
        )
        self.git("init", "-b", "main")
        self.git("config", "user.email", "mutation-tests@example.invalid")
        self.git("config", "user.name", "Mutation Tests")
        self.git("add", ".")
        self.git("commit", "-m", "fixture")
        fixture_root = pathlib.Path(self.temporary.name) / "artifact-template"
        fixture_root.mkdir()
        self.fixture = MutationFixture(fixture_root)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def git(self, *argv: str) -> str:
        process = subprocess.run(
            ["git", "-C", str(self.repo), *argv],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        return process.stdout.strip()

    def test_generated_output_paths_must_remain_below_target(self) -> None:
        accepted = mutation_ci.resolve_output_path(
            self.repo,
            "target/verification/mutation.json",
            label="report output",
        )
        self.assertEqual(
            accepted,
            (self.repo / "target" / "verification" / "mutation.json").resolve(),
        )

        outside_target = [
            "mutants.out",
            "review-output",
            "target/../mutation-report.json",
            str(self.repo / "mutation-report.json"),
        ]
        for candidate in outside_target:
            with self.subTest(candidate=candidate):
                with self.assertRaisesRegex(
                    mutation_ci.MutationCiError,
                    "must remain below the repository target directory",
                ):
                    mutation_ci.resolve_output_path(
                        self.repo,
                        candidate,
                        label="mutation output",
                    )

        outside = self.repo / "outside-target"
        outside.mkdir()
        target = self.repo / "target"
        target.mkdir()
        link = target / "escape-link"
        try:
            link.symlink_to(outside, target_is_directory=True)
        except OSError:
            # Windows installations without Developer Mode cannot create
            # unprivileged symlinks; resolve-based confinement is still
            # exercised by the lexical and absolute escape cases above.
            return
        with self.assertRaisesRegex(
            mutation_ci.MutationCiError,
            "must remain below the repository target directory",
        ):
            mutation_ci.resolve_output_path(
                self.repo,
                "target/escape-link/mutants.out",
                label="mutation results root",
            )

    def test_report_set_manifest_selects_each_shard_exactly_once(self) -> None:
        manifest = self.repo / "coverage" / "mutation-report-set.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "kind": "sorotte-mutation-report-set",
                    "policy": "coverage/mutation-policy.toml",
                    "reports": {
                        "demo": "target/verification/mutation-demo.json",
                    },
                }
            ),
            encoding="utf-8",
        )
        stdout = io.StringIO()
        with mock.patch.object(mutation_ci, "verify_report", return_value=0) as verify:
            with contextlib.redirect_stdout(stdout):
                result = mutation_ci.main(
                    [
                        "verify-report-set",
                        "--repo-root",
                        str(self.repo),
                        "--policy",
                        "coverage/mutation-policy.toml",
                        "--manifest",
                        "coverage/mutation-report-set.json",
                    ]
                )

        self.assertEqual(result, 0)
        verify.assert_called_once()
        call = verify.call_args.args[0]
        self.assertEqual(call.shard, "demo")
        self.assertEqual(call.report, "target/verification/mutation-demo.json")
        self.assertIn("1 uniquely selected shard report", stdout.getvalue())

    def test_run_writes_passed_report_after_pre_inventory_reconciliation(self) -> None:
        calls: list[list[str]] = []

        def fake_run(
            argv: list[str],
            *,
            cwd: pathlib.Path,
        ) -> subprocess.CompletedProcess[str]:
            self.assertEqual(cwd, self.repo)
            calls.append(list(argv))
            if argv == ["cargo", "mutants", "--version"]:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout="cargo-mutants 27.1.0\n",
                    stderr="",
                )
            if argv[:2] == ["cargo", "test"]:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout="demo_test: test\n",
                    stderr="cargo test progress\n",
                )
            if "--list" in argv:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout=json.dumps(self.fixture.inventory),
                    stderr="",
                )
            output = pathlib.Path(argv[argv.index("--output") + 1])
            self.fixture.write(output / "mutants.out")
            return subprocess.CompletedProcess(
                argv,
                0,
                stdout="1 mutants tested: 1 caught\n",
                stderr="",
            )

        report_path = self.repo / "target" / "verification" / "mutation.json"
        with mock.patch.object(mutation_ci, "run_process", side_effect=fake_run):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                result = mutation_ci.main(
                    [
                        "run",
                        "--repo-root",
                        str(self.repo),
                        "--policy",
                        "coverage/mutation-policy.toml",
                        "--shard",
                        "demo",
                        "--results-root",
                        "target/mutation-ci/demo",
                        "--output",
                        "target/verification/mutation.json",
                    ]
                )

        self.assertEqual(result, 0)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["git"]["head_sha"], self.git("rev-parse", "HEAD"))
        self.assertFalse(report["git"]["configured_sources_dirty"])
        self.assertEqual(
            report["test_scope"],
            {"target": "package", "filter": ""},
        )
        self.assertEqual(
            report["test_inventory"],
            {
                "count": 1,
                "canonical_sha256": mutation_ci.canonical_digest(["demo_test"]),
                "tests": ["demo_test"],
            },
        )
        self.assertEqual(
            report["source_bindings"]["before"],
            report["source_bindings"]["after"],
        )
        self.assertEqual(
            report["verification_input_bindings"]["before"],
            report["verification_input_bindings"]["after"],
        )
        self.assertGreaterEqual(len(report["verification_input_bindings"]["before"]), 4)
        self.assertIn("--cargo-arg=--locked", report["command"])
        self.assertIn("--no-config", report["command"])
        self.assertIn("--no-shuffle", report["command"])
        self.assertEqual(
            calls[1],
            [
                "cargo",
                "test",
                "--package",
                "demo",
                "--locked",
                "--all-features",
                "--",
                "--list",
                "--format",
                "terse",
            ],
        )
        self.assertEqual(len(calls), 4)

        verification_stdout = io.StringIO()
        with mock.patch.object(mutation_ci, "run_process", side_effect=fake_run):
            with contextlib.redirect_stdout(verification_stdout):
                verified = mutation_ci.main(
                    [
                        "verify-report",
                        "--repo-root",
                        str(self.repo),
                        "--policy",
                        "coverage/mutation-policy.toml",
                        "--shard",
                        "demo",
                        "--report",
                        "target/verification/mutation.json",
                    ]
                )
        self.assertEqual(verified, 0)
        self.assertIn("mutation evidence current: demo", verification_stdout.getvalue())

        def changed_inventory_run(
            argv: list[str],
            *,
            cwd: pathlib.Path,
        ) -> subprocess.CompletedProcess[str]:
            self.assertEqual(cwd, self.repo)
            self.assertEqual(argv[:2], ["cargo", "test"])
            return subprocess.CompletedProcess(
                argv,
                0,
                stdout="demo_test: test\nnew_regression: test\n",
                stderr="",
            )

        inventory_stderr = io.StringIO()
        with mock.patch.object(
            mutation_ci,
            "run_process",
            side_effect=changed_inventory_run,
        ):
            with contextlib.redirect_stderr(inventory_stderr):
                stale_inventory = mutation_ci.main(
                    [
                        "verify-report",
                        "--repo-root",
                        str(self.repo),
                        "--policy",
                        "coverage/mutation-policy.toml",
                        "--shard",
                        "demo",
                        "--report",
                        "target/verification/mutation.json",
                    ]
                )
        self.assertEqual(stale_inventory, 2)
        self.assertIn("test inventory is stale", inventory_stderr.getvalue())

        added_test = self.repo / "crates" / "demo" / "tests" / "status.rs"
        added_test.parent.mkdir()
        added_test.write_text("#[test]\nfn added_status_test() {}\n", encoding="utf-8")
        verification_input_stderr = io.StringIO()
        with contextlib.redirect_stderr(verification_input_stderr):
            stale_verification_input = mutation_ci.main(
                [
                    "verify-report",
                    "--repo-root",
                    str(self.repo),
                    "--policy",
                    "coverage/mutation-policy.toml",
                    "--shard",
                    "demo",
                    "--report",
                    "target/verification/mutation.json",
                ]
            )
        self.assertEqual(stale_verification_input, 2)
        self.assertIn(
            "verification inputs are stale",
            verification_input_stderr.getvalue(),
        )
        added_test.unlink()
        added_test.parent.rmdir()

        (self.repo / MutationFixture.source).write_bytes(
            b"pub fn demo() -> bool {\n    false\n}\n"
        )
        verification_stderr = io.StringIO()
        with contextlib.redirect_stderr(verification_stderr):
            stale = mutation_ci.main(
                [
                    "verify-report",
                    "--repo-root",
                    str(self.repo),
                    "--policy",
                    "coverage/mutation-policy.toml",
                    "--shard",
                    "demo",
                    "--report",
                    "target/verification/mutation.json",
                ]
            )
        self.assertEqual(stale, 2)
        self.assertIn("source bindings are stale", verification_stderr.getvalue())

    def test_run_rejects_zero_selected_tests_before_mutant_inventory(self) -> None:
        calls: list[list[str]] = []

        def fake_run(
            argv: list[str],
            *,
            cwd: pathlib.Path,
        ) -> subprocess.CompletedProcess[str]:
            self.assertEqual(cwd, self.repo)
            calls.append(list(argv))
            if argv == ["cargo", "mutants", "--version"]:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout="cargo-mutants 27.1.0\n",
                    stderr="",
                )
            if argv[:2] == ["cargo", "test"]:
                return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
            self.fail(f"unexpected command after empty test inventory: {argv}")

        report_path = self.repo / "target" / "verification" / "zero-tests.json"
        with mock.patch.object(mutation_ci, "run_process", side_effect=fake_run):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                result = mutation_ci.main(
                    [
                        "run",
                        "--repo-root",
                        str(self.repo),
                        "--policy",
                        "coverage/mutation-policy.toml",
                        "--shard",
                        "demo",
                        "--results-root",
                        "target/mutation-ci/zero-tests",
                        "--output",
                        "target/verification/zero-tests.json",
                    ]
                )

        self.assertEqual(result, 2)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "error")
        self.assertEqual(
            report["errors"],
            ["cargo test inventory must contain at least one test"],
        )
        self.assertEqual(len(calls), 2)

    def test_run_rejects_zero_mutants_before_mutation_execution(self) -> None:
        calls: list[list[str]] = []

        def fake_run(
            argv: list[str],
            *,
            cwd: pathlib.Path,
        ) -> subprocess.CompletedProcess[str]:
            self.assertEqual(cwd, self.repo)
            calls.append(list(argv))
            if argv == ["cargo", "mutants", "--version"]:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout="cargo-mutants 27.1.0\n",
                    stderr="",
                )
            if argv[:2] == ["cargo", "test"]:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout="demo_test: test\n",
                    stderr="",
                )
            if "--list" in argv:
                return subprocess.CompletedProcess(
                    argv,
                    0,
                    stdout="[]",
                    stderr="",
                )
            self.fail(f"unexpected mutation execution after empty inventory: {argv}")

        report_path = self.repo / "target" / "verification" / "zero-mutants.json"
        with mock.patch.object(mutation_ci, "run_process", side_effect=fake_run):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                result = mutation_ci.main(
                    [
                        "run",
                        "--repo-root",
                        str(self.repo),
                        "--policy",
                        "coverage/mutation-policy.toml",
                        "--shard",
                        "demo",
                        "--results-root",
                        "target/mutation-ci/zero-mutants",
                        "--output",
                        "target/verification/zero-mutants.json",
                    ]
                )

        self.assertEqual(result, 2)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "error")
        self.assertEqual(
            report["errors"],
            ["pre-run cargo-mutants inventory must contain at least one mutant"],
        )
        self.assertEqual(len(calls), 3)

    def test_version_drift_is_rejected_exactly(self) -> None:
        completed = subprocess.CompletedProcess(
            ["cargo", "mutants", "--version"],
            0,
            stdout="cargo-mutants 27.1.1\n",
            stderr="",
        )
        with mock.patch.object(mutation_ci, "run_process", return_value=completed):
            with self.assertRaisesRegex(
                mutation_ci.MutationCiError,
                "must report exactly",
            ):
                mutation_ci.verify_tool(self.repo, "27.1.0")


if __name__ == "__main__":
    unittest.main()
