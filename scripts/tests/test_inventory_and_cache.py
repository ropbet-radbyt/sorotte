from __future__ import annotations

import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from scripts import cargo_input_cache as cache
from scripts import test_inventory as inventory

ROOT = Path(__file__).resolve().parents[2]


def nextest_listing(scope: str = "media-lib", tests: dict[str, bool] | None = None) -> dict:
    tests = tests if tests is not None else {"tests::kept": False, "tests::old": True}
    arguments = inventory.SCOPES[scope]
    return {"test-count": len(tests), "rust-suites": {"binary": {
        "package-name": arguments[1], "binary-name": arguments[-1] if "--bin" in arguments else arguments[1],
        "kind": "bin" if "--bin" in arguments else "lib", "status": "listed",
        "testcases": {name: {"kind": "test", "ignored": ignored, "filter-match": {"status": "matches"}}
                      for name, ignored in tests.items()},
    }}}


def reviewed_fixture() -> dict:
    return {"schema_version": 2, "status": "passed", "identity": {"source_sha": "a" * 40},
            "scopes": {scope: {"cargo_scope": arguments, "tests": ["tests::kept", "tests::old"],
                               "ignored": ["tests::old"]} for scope, arguments in inventory.SCOPES.items()}}


class TestInventoryTests(unittest.TestCase):
    def test_checked_in_inventory_preserves_legacy_complete_names_and_totals(self):
        data = inventory.validate(inventory.load(inventory.REVIEWED))
        for scope in inventory.SCOPES:
            self.assertEqual(inventory.reviewed(scope), data["scopes"][scope]["tests"])
            self.assertTrue(set(data["scopes"][scope]["ignored"]) <= set(inventory.reviewed(scope)))

    def test_fresh_listing_preserves_ignored_identities_in_legacy_totals(self):
        data = nextest_listing()
        self.assertEqual(inventory.flatten(data), ["tests::kept", "tests::old"])
        self.assertEqual(inventory.listing(data, "media-lib"),
                         {"tests": ["tests::kept", "tests::old"], "ignored": ["tests::old"]})

    def test_updater_scope_exposes_missing_extra_and_ignored_inventory_drift(self):
        entry = inventory.validate(inventory.load(inventory.REVIEWED))["scopes"]["updater-bin"]
        self.assertEqual(entry["cargo_scope"],
                         ["-p", "sorotte-gui", "--all-features", "--bin", "sorotte-gui-updater"])
        self.assertEqual(entry["ignored"], [])
        self.assertIn("tests::windows_junction_fixture_reports_an_occupied_path_without_replacing_it", entry["tests"])
        self.assertIn("tests::windows_link_fixture_replaces_an_input_while_its_original_handle_is_open", entry["tests"])
        expected = {"tests": entry["tests"], "ignored": entry["ignored"]}
        original = {name: False for name in entry["tests"]}
        self.assertEqual(inventory.listing(nextest_listing("updater-bin", original), "updater-bin"), expected)
        first = entry["tests"][0]
        for label, tests in (
            ("removed", {name: ignored for name, ignored in original.items() if name != first}),
            ("added", {**original, "tests::unreviewed_updater_case": False}),
            ("newly_ignored", {**original, first: True}),
        ):
            with self.subTest(change=label):
                actual = inventory.listing(nextest_listing("updater-bin", tests), "updater-bin")
                difference = inventory.scope_difference(expected, actual)
                self.assertEqual({key for key, names in difference.items() if names}, {label})
        wrong_binary = nextest_listing("updater-bin", original)
        wrong_binary["rust-suites"]["binary"]["binary-name"] = "sorotte-gui"
        with self.assertRaisesRegex(ValueError, "wrong binary"):
            inventory.listing(wrong_binary, "updater-bin")

    def test_renamed_deleted_added_and_ignored_status_changes_are_explicit(self):
        before = {"tests": ["tests::kept", "tests::old"], "ignored": ["tests::old"]}
        cases = [
            ({"tests": ["tests::kept", "tests::renamed"], "ignored": ["tests::renamed"]},
             {"added": ["tests::renamed"], "removed": ["tests::old"], "newly_ignored": [], "no_longer_ignored": []}),
            ({"tests": ["tests::kept"], "ignored": []},
             {"added": [], "removed": ["tests::old"], "newly_ignored": [], "no_longer_ignored": []}),
            ({"tests": ["tests::added", "tests::kept", "tests::old"], "ignored": ["tests::old"]},
             {"added": ["tests::added"], "removed": [], "newly_ignored": [], "no_longer_ignored": []}),
            ({"tests": before["tests"], "ignored": ["tests::kept"]},
             {"added": [], "removed": [], "newly_ignored": ["tests::kept"], "no_longer_ignored": ["tests::old"]}),
        ]
        for after, expected in cases:
            with self.subTest(after=after):
                self.assertEqual(inventory.scope_difference(before, after), expected)

    def test_incomplete_ambiguous_filtered_or_wrong_scope_listings_fail(self):
        mutations = {
            "empty": lambda d: d.update(**{"test-count": 0, "rust-suites": {}}),
            "wrong-count": lambda d: d.update(**{"test-count": 1}),
            "missing-ignored": lambda d: d["rust-suites"]["binary"]["testcases"]["tests::old"].pop("ignored"),
            "nonboolean-ignored": lambda d: d["rust-suites"]["binary"]["testcases"]["tests::old"].update(ignored="false"),
            "not-listed": lambda d: d["rust-suites"]["binary"].update(status="skipped"),
            "wrong-package": lambda d: d["rust-suites"]["binary"].update(**{"package-name": "sorotte-gui"}),
            "wrong-target": lambda d: d["rust-suites"]["binary"].update(kind="bin"),
            "filtered": lambda d: d["rust-suites"]["binary"]["testcases"]["tests::old"].update(**{"filter-match": {"status": "mismatch"}}),
            "duplicate-binary-name": lambda d: d["rust-suites"].update(other=copy.deepcopy(d["rust-suites"]["binary"])),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                candidate = nextest_listing()
                mutate(candidate)
                with self.assertRaises(ValueError):
                    inventory.listing(candidate, "media-lib")

    def test_reviewed_schema_requires_every_scope_and_explicit_ignored_subset(self):
        mutations = [lambda d: d.update(schema_version=1), lambda d: d.update(status="incomplete"),
                     lambda d: d["scopes"].pop("compat"),
                     lambda d: d["scopes"]["compat"].pop("ignored"),
                     lambda d: d["scopes"]["compat"].update(ignored=["unknown"]),
                     lambda d: d["scopes"]["compat"].update(tests=["same", "same"]),
                     lambda d: d["scopes"]["compat"].update(cargo_scope=["--workspace"])]
        for mutate in mutations:
            value = reviewed_fixture()
            mutate(value)
            with self.assertRaises(ValueError): inventory.validate(value)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "duplicate.json"
            path.write_text('{"scopes":{},"scopes":{}}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                inventory.load(path)

    def test_both_collecting_commands_reject_authority_and_hardlink_overwrite(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            authority = root / "reviewed.json"
            authority.write_text(json.dumps(reviewed_fixture()), encoding="utf-8")
            alias = root / "authority-alias.json"
            os.link(authority, alias)
            before = authority.read_bytes()
            with mock.patch.object(inventory, "REVIEWED", authority), mock.patch.object(inventory, "identity") as identity:
                for command in ("propose", "check"):
                    for output in (authority, alias):
                        with self.subTest(command=command, output=output), contextlib.redirect_stderr(io.StringIO()):
                            self.assertEqual(inventory.main([command, "--output", str(output)]), 1)
                identity.assert_not_called()
            self.assertEqual(authority.read_bytes(), before)

    def test_diff_is_read_only_and_nonzero_for_same_count_rename_or_status_change(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            authority, proposal = root / "reviewed.json", root / "proposed.json"
            baseline = reviewed_fixture()
            authority.write_text(json.dumps(baseline), encoding="utf-8")
            before = authority.read_bytes()
            for change in ("rename", "ignore", "unchanged"):
                candidate = copy.deepcopy(baseline)
                if change == "rename": candidate["scopes"]["compat"].update(tests=["tests::kept", "tests::renamed"], ignored=[])
                if change == "ignore": candidate["scopes"]["compat"]["ignored"] = ["tests::kept", "tests::old"]
                proposal.write_text(json.dumps(candidate), encoding="utf-8")
                output = io.StringIO()
                with mock.patch.object(inventory, "REVIEWED", authority), contextlib.redirect_stdout(output):
                    self.assertEqual(inventory.main(["diff", "--proposed", str(proposal)]), int(change != "unchanged"))
                rows = [json.loads(row) for row in output.getvalue().splitlines()]
                self.assertEqual(len(rows), len(inventory.SCOPES))
                self.assertEqual(authority.read_bytes(), before)

    def simulate_collection(self, output: Path, *, defect: str | None = None) -> dict:
        calls = []
        def run(command, **kwargs):
            scope = next(scope for scope, arguments in inventory.SCOPES.items() if command[7:-2] == arguments)
            self.assertEqual(command[:4], ["cargo", "nextest", "list", "--locked"])
            self.assertEqual(command[4:7], ["--run-ignored", "all", "--ignore-default-filter"])
            self.assertEqual(command[-2:], ["--message-format", "json"])
            self.assertTrue(kwargs["check"])
            self.assertEqual(kwargs["timeout"], 1800)
            calls.append(scope)
            if defect == "failure" and len(calls) == 2: raise subprocess.CalledProcessError(9, command)
            if defect == "timeout": raise subprocess.TimeoutExpired(command, 1800)
            if defect == "cancelled": raise KeyboardInterrupt("inventory cancelled")
            return subprocess.CompletedProcess(command, 0, stdout=json.dumps(nextest_listing(scope)).encode())
        identities = [{"source_sha": "a" * 40}, {"source_sha": ("b" if defect == "source-drift" else "a") * 40}]
        version = "cargo-nextest 0.9.1" if defect == "wrong-pin" else "cargo-nextest 0.9.137 (reviewed build)"
        with mock.patch.object(inventory, "identity", side_effect=identities), \
                mock.patch.object(inventory.subprocess, "check_output", return_value=version), \
                mock.patch.object(inventory.subprocess, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
            return inventory.collect(output)

    def test_collect_uses_fresh_exact_locked_scopes_and_records_listing_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "proposal.json"
            value = self.simulate_collection(output)
            self.assertEqual(value["status"], "passed")
            self.assertEqual(set(value["scopes"]), set(inventory.SCOPES))
            self.assertEqual(len(value["attempts"]), len(inventory.SCOPES))
            for scope, entry in value["scopes"].items():
                self.assertEqual(entry["listing_sha256"], hashlib.sha256(json.dumps(nextest_listing(scope)).encode()).hexdigest())
                self.assertTrue((output.with_suffix(".listings") / f"{scope}.json").is_file())
            original = output.read_bytes()
            with self.assertRaisesRegex(ValueError, "fresh"): self.simulate_collection(output)
            self.assertEqual(output.read_bytes(), original)

    def test_collection_failure_timeout_cancellation_and_source_drift_preserve_receipts(self):
        for defect in ("failure", "timeout", "cancelled", "source-drift", "wrong-pin"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary) / "proposal.json"
                with self.assertRaises((ValueError, subprocess.SubprocessError, KeyboardInterrupt)):
                    self.simulate_collection(output, defect=defect)
                value = inventory.load(output)
                expected = "timed_out" if defect == "timeout" else "cancelled" if defect == "cancelled" else "failed"
                self.assertEqual(value["status"], expected)
                with self.assertRaises(ValueError): inventory.validate(value)
                if defect == "failure":
                    self.assertEqual(value["attempts"][0]["status"], "passed")
                    self.assertEqual(value["attempts"][1]["status"], "failed")


class CargoInputCacheTests(unittest.TestCase):
    def write_lock(self, path: Path, packages: dict[str, bytes]) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        text = "version = 4\n"
        for name, payload in packages.items():
            text += (f'[[package]]\nname = "{name}"\nversion = "1.0.0"\n'
                     'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
                     f'checksum = "{hashlib.sha256(payload).hexdigest()}"\n')
        path.write_text(text, encoding="utf-8")
        return path

    def fixture(self, root: Path):
        locks = [self.write_lock(root / "Cargo.lock", {"main": b"main"}),
                 self.write_lock(root / "fuzz/Cargo.lock", {"fuzz": b"fuzz"})]
        registry = root / "registry/cache/index.example-123"
        registry.mkdir(parents=True)
        (registry / "main-1.0.0.crate").write_bytes(b"main")
        (registry / "fuzz-1.0.0.crate").write_bytes(b"fuzz")
        return registry.parent, locks

    def symlink(self, link: Path, target: Path, *, directory: bool = False):
        try: link.symlink_to(target, target_is_directory=directory)
        except OSError as error: self.skipTest(f"this host cannot create fixture symlinks: {error}")

    def test_both_lock_archives_are_verified_and_cold_cache_is_valid(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            report = cache.verify(registry, locks)
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["locked_archive_count"], 2)
            self.assertEqual({row["outcome"] for row in report["files"]}, {"verified"})
            self.assertEqual([row["sha256"] for row in report["locks"]], [cache.digest(path) for path in locks])
            cold = cache.verify(root / "cold/registry/cache", locks)
            self.assertEqual(cold["files"], [])
            self.assertEqual(cold["locked_archive_count"], 2)

    def test_corrupt_repair_removes_only_reviewed_archive_and_leaves_other_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            archive = registry / "index.example-123/fuzz-1.0.0.crate"
            archive.write_bytes(b"corrupt")
            untouched = [registry / "index.example-123/unlocked-1.0.0.crate", root / "registry/src/source.rs",
                         root / "target/debug/binary", root / "advisory.json"]
            for path in untouched:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"preserve")
            result = cache.verify(registry, locks)
            self.assertFalse(archive.exists())
            self.assertTrue(all(path.read_bytes() == b"preserve" for path in untouched))
            self.assertEqual(result["unreferenced_archive_count"], 1)
            self.assertIn("removed-for-locked-redownload", [row["outcome"] for row in result["files"]])
            archive.write_bytes(b"fuzz")
            self.assertTrue(all(row["outcome"] == "verified" for row in cache.verify(registry, locks)["files"]))

    def test_conflicting_or_malformed_authority_fails_before_any_deletion(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            archive = registry / "index.example-123/main-1.0.0.crate"
            archive.write_bytes(b"corrupt")
            self.write_lock(locks[1], {"main": b"different authority"})
            with self.assertRaisesRegex(ValueError, "conflicting"):
                cache.verify(registry, locks)
            self.assertEqual(archive.read_bytes(), b"corrupt")
            for invalid in ('version = 4\npackage = []\n',
                            'version = 4\n[[package]]\nname = "main"\nversion = "1.0.0"\nsource = "registry+https://example"\n',
                            'version = 4\n[[package]]\nname = "main"\nversion = "../escape"\nsource = "registry+https://example"\nchecksum = "bad"\n'):
                locks[1].write_text(invalid, encoding="utf-8")
                with self.assertRaises(ValueError): cache.verify(registry, locks)
                self.assertEqual(archive.read_bytes(), b"corrupt")

    def test_non_registry_roots_and_parent_traversal_cannot_trigger_repair(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            for candidate in (root, root / "target", root / "registry/src", registry / "../cache"):
                with self.subTest(candidate=candidate), self.assertRaises(ValueError): cache.verify(candidate, locks)

    def test_linked_archive_is_rejected_without_touching_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            target = root / "outside.crate"
            target.write_bytes(b"corrupt external input")
            linked = registry / "index.example-123/main-1.0.0.crate"
            linked.unlink()
            self.symlink(linked, target)
            with self.assertRaisesRegex(ValueError, "direct paths"): cache.verify(registry, locks)
            self.assertEqual(target.read_bytes(), b"corrupt external input")

    def test_linked_registry_directory_and_root_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            target = root / "outside"
            target.mkdir()
            (target / "main-1.0.0.crate").write_bytes(b"external")
            self.symlink(registry / "linked", target, directory=True)
            with self.assertRaisesRegex(ValueError, "direct paths"): cache.verify(registry, locks)
            alias = root / "alias/registry/cache"
            alias.parent.mkdir(parents=True)
            self.symlink(alias, registry, directory=True)
            with self.assertRaisesRegex(ValueError, "direct paths"): cache.verify(alias, locks)
            self.assertEqual((target / "main-1.0.0.crate").read_bytes(), b"external")

    def test_changed_archive_or_lock_authority_is_not_deleted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            archive = registry / "index.example-123/main-1.0.0.crate"
            archive.write_bytes(b"corrupt")
            digest = cache.digest
            def replace_during_hash(path):
                actual = digest(path)
                if path == archive: archive.write_bytes(b"changed after hash")
                return actual
            with mock.patch.object(cache, "digest", side_effect=replace_during_hash):
                with self.assertRaisesRegex(ValueError, "changed"): cache.verify(registry, locks)
            self.assertEqual(archive.read_bytes(), b"changed after hash")
            def change_authority(path):
                if path == locks[0]: return "f" * 64
                return digest(path)
            with mock.patch.object(cache, "digest", side_effect=change_authority):
                with self.assertRaisesRegex(ValueError, "authority changed"): cache.verify(registry, locks)
            self.assertTrue(archive.exists())

    def test_command_receipts_are_fresh_and_cannot_overwrite_cached_inputs_or_locks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry, locks = self.fixture(root)
            args = ["--cache-root", str(registry), "--lock", str(locks[0]), "--lock", str(locks[1])]
            output = root / "evidence/cache.json"
            self.assertEqual(cache.main([*args, "--output", str(output)]), 0)
            self.assertEqual(json.loads(output.read_text())["status"], "passed")
            before = output.read_bytes()
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(cache.main([*args, "--output", str(output)]), 1)
                self.assertEqual(cache.main([*args, "--output", str(locks[0])]), 1)
                self.assertEqual(cache.main([*args, "--output", str(registry / "receipt.json")]), 1)
            self.assertEqual(output.read_bytes(), before)
            self.write_lock(locks[1], {"main": b"conflicting"})
            failure = root / "failure.json"
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(cache.main([*args, "--output", str(failure)]), 1)
            self.assertEqual(json.loads(failure.read_text())["status"], "failed")


if __name__ == "__main__":
    unittest.main()
