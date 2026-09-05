from __future__ import annotations

import copy
import datetime as dt
import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import ignored_test_policy as policy  # noqa: E402


class IgnoredTestPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name)
        self.source = self.repo / "crates" / "example" / "src" / "lib.rs"
        self.source.parent.mkdir(parents=True)
        self.source.write_text(
            """
// #[ignore = "commented attributes are not tests"]
const ATTRIBUTE_EXAMPLE: &str = r#"#[ignore = "string attributes are not tests"]"#;

#[test]
#[ignore = "requires a provisioned external service"]
fn external_contract() {
    assert!(true);
}
""".lstrip(),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def entry(self) -> dict:
        return {
            "id": "IGN-EXAMPLE-001",
            "source": "crates/example/src/lib.rs",
            "test": "external_contract",
            "source_reason": "requires a provisioned external service",
            "tier": "manual",
            "owner": "cli",
            "rationale": "The external service is not provisioned on ordinary test hosts.",
            "prerequisites": ["provisioned external service"],
            "operating_systems": ["linux"],
        }

    def registry(self) -> dict:
        return {
            "schema_version": 1,
            "policy": {
                "allowed_tiers": list(policy.EXPECTED_TIERS),
                "allowed_owners": list(policy.EXPECTED_OWNERS),
                "allowed_operating_systems": list(
                    policy.EXPECTED_OPERATING_SYSTEMS
                ),
            },
            "ignored_test": [self.entry()],
        }

    def discovered(self) -> list[policy.IgnoredTest]:
        return policy.discover_ignored_tests(self.repo)

    def test_discovers_only_real_attached_test_attributes(self) -> None:
        discovered = self.discovered()
        self.assertEqual(len(discovered), 1)
        self.assertEqual(discovered[0].source, "crates/example/src/lib.rs")
        self.assertEqual(discovered[0].test, "external_contract")
        self.assertEqual(
            discovered[0].source_reason,
            "requires a provisioned external service",
        )

    def test_valid_registry_matches_source_exactly(self) -> None:
        tiers = policy.validate_registry(
            self.registry(),
            self.discovered(),
            as_of=dt.date(2026, 7, 28),
        )
        self.assertEqual(tiers, {"manual": 1})

    def test_missing_extra_and_changed_reasons_fail_closed(self) -> None:
        missing = self.registry()
        missing["ignored_test"] = []
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "missing="):
            policy.validate_registry(missing, self.discovered())

        extra = self.registry()
        second = copy.deepcopy(extra["ignored_test"][0])
        second["id"] = "IGN-EXAMPLE-002"
        second["test"] = "not_in_source"
        extra["ignored_test"].append(second)
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "extra="):
            policy.validate_registry(extra, self.discovered())

        changed = self.registry()
        changed["ignored_test"][0]["source_reason"] = "a different source reason"
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "source reason changed"):
            policy.validate_registry(changed, self.discovered())

    def test_bare_conditional_and_non_test_ignores_are_rejected(self) -> None:
        variants = {
            "bare": """
#[test]
#[ignore]
fn external_contract() {}
""",
            "conditional": """
#[test]
#[cfg_attr(windows, ignore = "not on Windows")]
fn external_contract() {}
""",
            "multiline-conditional": """
#[test]
#[cfg_attr(
    windows,
    ignore = "not on Windows"
)]
fn external_contract() {}
""",
            "non-test": """
#[ignore = "not actually a test"]
fn external_contract() {}
""",
        }
        for name, source in variants.items():
            with self.subTest(name=name):
                self.source.write_text(source.lstrip(), encoding="utf-8")
                with self.assertRaises(policy.IgnoredTestPolicyError):
                    self.discovered()

    def test_registry_schema_and_vocabulary_cannot_self_expand(self) -> None:
        boolean_schema = self.registry()
        boolean_schema["schema_version"] = True
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "unsupported"):
            policy.validate_registry(boolean_schema, self.discovered())

        extra_tier = self.registry()
        extra_tier["policy"]["allowed_tiers"].append("never")
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "must equal"):
            policy.validate_registry(extra_tier, self.discovered())

        unknown_field = self.registry()
        unknown_field["ignored_test"][0]["command"] = "echo forged"
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "unknown="):
            policy.validate_registry(unknown_field, self.discovered())

    def test_duplicate_ids_and_identities_are_rejected(self) -> None:
        duplicate_identity = self.registry()
        duplicate_entry = copy.deepcopy(duplicate_identity["ignored_test"][0])
        duplicate_entry["id"] = "IGN-EXAMPLE-002"
        duplicate_identity["ignored_test"].append(duplicate_entry)
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "duplicate.*identity"):
            policy.validate_registry(duplicate_identity, self.discovered())

        duplicate_id = self.registry()
        second = copy.deepcopy(duplicate_id["ignored_test"][0])
        second["test"] = "another_contract"
        duplicate_id["ignored_test"].append(second)
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "duplicate.*ID"):
            policy.validate_registry(duplicate_id, self.discovered())

    def test_tier_specific_fields_are_required_and_strict(self) -> None:
        pull_request = self.registry()
        entry = pull_request["ignored_test"][0]
        entry["tier"] = "pull-request"
        entry["required_job"] = "real-boundary"
        policy.validate_registry(pull_request, self.discovered())

        invalid_job = copy.deepcopy(pull_request)
        invalid_job["ignored_test"][0]["required_job"] = "Real Boundary"
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "job ID"):
            policy.validate_registry(invalid_job, self.discovered())

        maintenance = self.registry()
        entry = maintenance["ignored_test"][0]
        entry["tier"] = "maintenance"
        entry["mutates_fixtures"] = False
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "must be true"):
            policy.validate_registry(maintenance, self.discovered())

    def test_expired_quarantine_fails(self) -> None:
        registry = self.registry()
        entry = registry["ignored_test"][0]
        entry["tier"] = "quarantined"
        entry["tracking"] = "issue:1234"
        entry["review_by"] = "2026-07-27"
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "expired"):
            policy.validate_registry(
                registry,
                self.discovered(),
                as_of=dt.date(2026, 7, 28),
            )

        entry["review_by"] = "2026-07-28"
        policy.validate_registry(
            registry,
            self.discovered(),
            as_of=dt.date(2026, 7, 28),
        )

    def test_subprocess_fixture_requires_a_source_bound_ordinary_parent(self) -> None:
        registry = self.registry()
        entry = registry["ignored_test"][0]
        entry.update(tier="subprocess-fixture", required_job="checks", parent_test="tests::parent_contract")
        source = self.source.read_text(encoding="utf-8")
        self.source.write_text(source + "\n#[test]\nfn parent_contract() {}\n", encoding="utf-8")
        self.assertEqual(
            policy.validate_registry(registry, self.discovered(), repo_root=self.repo),
            {"subprocess-fixture": 1},
        )
        with self.assertRaisesRegex(policy.IgnoredTestPolicyError, "source root"):
            policy.validate_registry(registry, self.discovered())
        for parent in ["parent_contract", "tests::external_contract", "tests::missing"]:
            invalid = copy.deepcopy(registry)
            invalid["ignored_test"][0]["parent_test"] = parent
            with self.subTest(parent=parent), self.assertRaises(policy.IgnoredTestPolicyError):
                policy.validate_registry(invalid, self.discovered(), repo_root=self.repo)

    def test_subprocess_parent_cannot_be_a_comment_string_or_plain_function(self) -> None:
        registry = self.registry()
        registry["ignored_test"][0].update(
            tier="subprocess-fixture", required_job="checks", parent_test="tests::parent_contract"
        )
        source = self.source.read_text(encoding="utf-8")
        for parent in [
            "// #[test]\n// fn parent_contract() {}\n",
            'const EXAMPLE: &str = r#"#[test]\nfn parent_contract() {}"#;\n',
            "fn parent_contract() {}\n",
            "#[test]\nfn parent_contract() {}\n#[test]\nfn parent_contract() {}\n",
        ]:
            self.source.write_text(source + "\n" + parent, encoding="utf-8")
            with self.subTest(parent=parent), self.assertRaises(policy.IgnoredTestPolicyError):
                policy.validate_registry(registry, self.discovered(), repo_root=self.repo)


if __name__ == "__main__":
    unittest.main()
