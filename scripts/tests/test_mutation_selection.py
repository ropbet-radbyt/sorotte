from __future__ import annotations
import copy
import pathlib
import sys
import tempfile
import tomllib
import unittest
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import mutation_selection as selection

ROOT = pathlib.Path(__file__).resolve().parents[2]


class MutationSelectionTests(unittest.TestCase):
    def setUp(self):
        self.policy = tomllib.loads((ROOT / selection.POLICY).read_text(encoding="utf-8"))
        self.catalog = tomllib.loads((ROOT / selection.CATALOG).read_text(encoding="utf-8"))

    def select(self, *changed):
        return selection.selected_shards(self.policy, self.catalog, list(changed), full=False)

    def test_secret_config_and_timing_changes_select_relevant_existing_shards(self):
        cases = [("crates/sorotte-secret/src/lib.rs", "privacy-secret"), ("crates/sorotte-client-app/src/sorotte_ini/merge.rs", "client-runtime-config"), ("crates/sorotte-client-core/src/ping.rs", "client-ping"), ("crates/sorotte-server/src/local_clock.rs", "server-local-clock")]
        mandatory = set(self.policy["required_report_set"][0]["shards"])
        for path, required in cases:
            with self.subTest(path=path):
                selected = self.select(path)
                self.assertIn(required, selected)
                self.assertLessEqual(mandatory, selected)

    def test_docs_do_not_trigger_mutation_work(self):
        self.assertEqual(self.select("docs/DEVELOPMENT.md", "README.md", "crates/sorotte-secret/README.md", "fixtures/README.md"), set())

    def test_windows_checkout_line_endings_preserve_immutable_policy_identity(self):
        self.assertTrue(selection.checkout_matches_blob(b"schema_version=1\n", b"schema_version=1\r\n"))
        self.assertFalse(selection.checkout_matches_blob(b"schema_version=1\n", b"schema_version=2\r\n"))
        self.assertFalse(selection.checkout_matches_blob(None, b"schema_version=1\r\n"))
        self.assertFalse(selection.checkout_matches_blob(b"field = true\n", b"field=true\r\n"))

    def test_shared_verification_apparatus_selects_full_campaign(self):
        expected = {shard["id"] for shard in self.policy["shard"]}
        for path in (".gitattributes", "scripts/verify.py", "scripts/verification_tools.py", "scripts/test_inventory.py", "coverage/verification-tools.toml", "coverage/verification-lanes.json", "coverage/test-inventories.json"):
            self.assertEqual(self.select(path), expected)

    def test_selectors_and_lockfiles_recompute_all_shards(self):
        all_shards = {shard["id"] for shard in self.policy["shard"]}
        for path in ("Cargo.lock", "Cargo.toml", "coverage/mutation-policy.toml", "coverage/mutation-selection.toml", "scripts/mutation_ci.py", "scripts/artifact_input.py"):
            self.assertEqual(self.select(path), all_shards)

    def test_feature_manifest_and_test_extraction_select_package_checks(self):
        for path in ("crates/sorotte-client-app/Cargo.toml", "crates/sorotte-client-app/src/tests/extracted.rs"):
            self.assertIn("client-runtime-config", self.select(path))
            self.assertIn("settings-duplicate-keys", self.select(path))

    def test_declared_dependency_selects_timing_for_network_change(self):
        self.assertIn("server-local-clock", self.select("crates/sorotte-server/src/network/resource_tests.rs"))

    def test_unknown_dependency_shard_fails_closed(self):
        self.catalog["dependency"][0]["shards"].append("removed-shard")
        with self.assertRaises(selection.SelectionError):
            self.select("README.md")

    def test_omitted_or_duplicate_selected_report_is_never_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "one").mkdir()
            (root / "one/mutation-privacy-secret.json").write_text("{}")
            with mock.patch.object(selection.mutation_ci, "verify_report", return_value=0) as verify:
                with self.assertRaisesRegex(selection.SelectionError, "unique and complete"):
                    selection.verify_selected(root, {"shards": ["privacy-secret", "server-auth"]}, root)
                verify.assert_not_called()
                (root / "two").mkdir()
                (root / "two/mutation-privacy-secret.json").write_text("{}")
                with self.assertRaisesRegex(selection.SelectionError, "unique and complete"):
                    selection.verify_selected(root, {"shards": ["privacy-secret"]}, root)

    def test_complete_reports_still_require_source_bound_verifier(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "mutation-privacy-secret.json").write_text("{}")
            with mock.patch.object(selection.mutation_ci, "verify_report", return_value=2):
                with self.assertRaisesRegex(selection.SelectionError, "evidence failed"):
                    selection.verify_selected(root, {"shards": ["privacy-secret"]}, root)

    def test_base_catalog_cannot_be_weakened_by_head(self):
        head = copy.deepcopy(self.catalog)
        head["dependency"] = []
        changed = ["fixtures/protocol/ping.json"]
        union = selection.selected_shards(self.policy, self.catalog, changed, full=False) | selection.selected_shards(self.policy, head, changed, full=False)
        self.assertIn("client-ping", union)
        self.assertIn("protocol-codec", union)

    def test_new_boundary_shards_require_actual_selected_tests_and_zero_survivors(self):
        expected = {
            "plex-http-origin": ("sorotte-plex", "http_boundary_tests::canonical_origin_"),
            "settings-duplicate-keys": ("sorotte-client-app", "sorotte_ini::duplicate_tests::"),
            "server-local-clock": ("sorotte-server", "local_clock::tests::"),
            "server-resource-permits": ("sorotte-server", "resources::tests::"),
        }
        by_id = {shard["id"]: shard for shard in self.policy["shard"]}
        for identifier, (package, test_filter) in expected.items():
            with self.subTest(shard=identifier):
                shard = by_id[identifier]
                self.assertEqual((shard["package"], shard["test_filter"]), (package, test_filter))
                self.assertEqual(shard["test_target"], "lib")
                self.assertTrue(shard["require_baseline"])
                self.assertEqual(shard["minimum_viable_kill_percent"], "100.00")
                self.assertEqual((shard["max_missed"], shard["max_timeouts"]), (0, 0))
                self.assertIn(identifier, self.select(shard["files"][0]))

    def test_plan_recomputes_base_catalog_union_from_immutable_git_inputs(self):
        raw_policy = (ROOT / selection.POLICY).read_bytes()
        base_catalog = (ROOT / selection.CATALOG).read_bytes()
        head_catalog = b'schema_version=1\nglobal_inputs=["Cargo.lock"]\ndependency=[]\n'
        base, head = "a" * 40, "b" * 40
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "coverage").mkdir()
            (root / selection.POLICY).write_bytes(raw_policy)
            (root / selection.CATALOG).write_bytes(head_catalog)
            def revision(_root, sha, path):
                return raw_policy if path == selection.POLICY else base_catalog if sha == base else head_catalog
            def git(_root, *args):
                return (head + "\n").encode() if args[0] == "rev-parse" else b"fixtures/protocol/ping.json\0"
            known = SimpleNamespace(shards=[SimpleNamespace(identifier=item["id"]) for item in self.policy["shard"]])
            with mock.patch.object(selection, "git", side_effect=git), mock.patch.object(selection, "read_revision", side_effect=revision), mock.patch.object(selection.mutation_ci, "load_policy", return_value=known):
                plan = selection.plan(root, base, head)
                self.assertIn("client-ping", plan["shards"])
                self.assertEqual([item["revision"] for item in plan["inputs"]], [base, head])
                (root / selection.CATALOG).write_bytes(base_catalog)
                with self.assertRaisesRegex(selection.SelectionError, "immutable"):
                    selection.plan(root, base, head)


if __name__ == "__main__":
    unittest.main()
