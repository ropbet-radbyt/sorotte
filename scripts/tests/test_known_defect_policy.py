from __future__ import annotations

import datetime as dt
import pathlib
import sys
import tempfile
import textwrap
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import known_defect_policy as policy


class KnownDefectPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        (self.root / "crates" / "demo" / "src").mkdir(parents=True)
        (self.root / "coverage").mkdir()
        (self.root / "docs").mkdir()
        (self.root / "crates" / "demo" / "Cargo.toml").write_text(
            '[package]\nname = "demo-package"\nversion = "0.0.0"\n',
            encoding="utf-8",
        )
        self.source = self.root / "crates" / "demo" / "src" / "lib.rs"
        self.source.write_text(
            textwrap.dedent(
                """\
                #[test]
                #[should_panic(expected = "desired invariant")]
                fn known_defect_reproduction() {
                    assert!(false, "desired invariant");
                }

                #[test]
                fn known_defect_classifier_is_narrow() {
                    assert!(true);
                }
                """
            ),
            encoding="utf-8",
        )
        (self.root / "docs" / "findings.md").write_text(
            "## TC-DEMO-001: demonstration defect\n",
            encoding="utf-8",
        )
        self.catalog = self.root / "coverage" / "behaviors.toml"
        self.catalog.write_text(
            textwrap.dedent(
                """\
                schema_version = 1
                [[behavior]]
                id = "DEMO-OK-001"
                [[behavior.proof]]
                test = "tests::positive_contract"
                """
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def registry(
        self,
        *,
        expected: str = "desired invariant",
        selector: str = "tests::known_defect_reproduction",
        expiry: str = "2030-01-01",
        extra_root: str = "",
        extra_defect: str = "",
    ) -> dict:
        registry_path = self.root / "coverage" / "known-defects.toml"
        registry_path.write_text(
            textwrap.dedent(
                f"""\
                schema_version = 1
                {extra_root}

                [[defect]]
                id = "TC-DEMO-001"
                title = "demonstration defect"
                severity = "high"
                owners = ["demo"]
                finding = "docs/findings.md"
                expires = "{expiry}"
                {extra_defect}

                [[defect.characterization]]
                package = "demo-package"
                source = "crates/demo/src/lib.rs"
                test = "{selector}"
                expected_panic = "{expected}"
                """
            ),
            encoding="utf-8",
        )
        return policy.load_toml(registry_path, "test registry")

    def validate(self, registry: dict) -> tuple[int, int]:
        return policy.validate_registry(
            registry,
            repo_root=self.root,
            catalog_path=self.catalog,
            today=dt.date(2026, 7, 28),
        )

    def test_valid_registry_matches_exact_executable_inventory(self) -> None:
        self.assertEqual(self.validate(self.registry()), (1, 1))

    def test_explicit_empty_registry_matches_an_empty_executable_inventory(self) -> None:
        self.source.write_text(
            "#[test]\nfn positive_contract() { assert!(true); }\n",
            encoding="utf-8",
        )
        self.assertEqual(
            self.validate({"schema_version": 1, "defect": []}),
            (0, 0),
        )

    def test_empty_registry_rejects_an_unregistered_characterization(self) -> None:
        with self.assertRaisesRegex(policy.PolicyError, "unregistered known-defect"):
            self.validate({"schema_version": 1, "defect": []})

    def test_positive_known_defect_named_test_without_should_panic_is_not_inventory(self) -> None:
        found = policy.scan_characterizations(self.root)
        self.assertEqual(
            list(found),
            [("crates/demo/src/lib.rs", "known_defect_reproduction")],
        )

    def test_unregistered_characterization_fails_closed(self) -> None:
        registry = self.registry()
        registry["defect"][0]["characterization"] = []
        with self.assertRaisesRegex(policy.PolicyError, "non-empty list"):
            self.validate(registry)

    def test_additional_unregistered_characterization_fails_closed(self) -> None:
        with self.source.open("a", encoding="utf-8") as handle:
            handle.write(
                textwrap.dedent(
                    """\

                    #[test]
                    #[should_panic(expected = "second invariant")]
                    fn known_defect_second_reproduction() {
                        panic!("second invariant");
                    }
                    """
                )
            )
        with self.assertRaisesRegex(policy.PolicyError, "unregistered known-defect"):
            self.validate(self.registry())

    def test_stale_registry_entry_fails_closed(self) -> None:
        registry = self.registry(selector="tests::known_defect_removed")
        with self.assertRaisesRegex(policy.PolicyError, "unregistered known-defect"):
            self.validate(registry)

    def test_expected_panic_drift_fails_closed(self) -> None:
        with self.assertRaisesRegex(policy.PolicyError, "expected panic drifted"):
            self.validate(self.registry(expected="weakened oracle"))

    def test_expired_defect_fails_closed(self) -> None:
        with self.assertRaisesRegex(policy.PolicyError, "expired on"):
            self.validate(self.registry(expiry="2026-07-27"))

    def test_missing_finding_heading_fails_closed(self) -> None:
        (self.root / "docs" / "findings.md").write_text(
            "## TC-OTHER-001: another finding\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(policy.PolicyError, "no exact markdown heading"):
            self.validate(self.registry())

    def test_wrong_package_fails_closed(self) -> None:
        registry = self.registry()
        registry["defect"][0]["characterization"][0]["package"] = "other"
        with self.assertRaisesRegex(policy.PolicyError, "expected 'demo-package'"):
            self.validate(registry)

    def test_known_defect_cannot_be_a_positive_behavior_proof(self) -> None:
        self.catalog.write_text(
            textwrap.dedent(
                """\
                schema_version = 1
                [[behavior]]
                id = "DEMO-BAD-001"
                [[behavior.proof]]
                test = "tests::known_defect_reproduction"
                """
            ),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(policy.PolicyError, "positive behavior proof"):
            self.validate(self.registry())

    def test_unknown_registry_fields_fail_closed(self) -> None:
        with self.assertRaisesRegex(policy.PolicyError, "unknown keys"):
            self.validate(self.registry(extra_root='unexpected = "value"'))

    def test_unknown_defect_fields_fail_closed(self) -> None:
        with self.assertRaisesRegex(policy.PolicyError, "unknown keys"):
            self.validate(self.registry(extra_defect='unexpected = "value"'))

    def test_source_escape_fails_closed(self) -> None:
        registry = self.registry()
        registry["defect"][0]["characterization"][0]["source"] = "../outside.rs"
        with self.assertRaisesRegex(policy.PolicyError, "normalized repository-relative"):
            self.validate(registry)

    def test_should_panic_known_defect_must_be_an_ordinary_test(self) -> None:
        self.source.write_text(
            textwrap.dedent(
                """\
                #[should_panic(expected = "desired invariant")]
                fn known_defect_reproduction() {
                    panic!("desired invariant");
                }
                """
            ),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(policy.PolicyError, "not an ordinary #\\[test\\]"):
            policy.scan_characterizations(self.root)

    def test_bare_should_panic_oracle_fails_closed(self) -> None:
        self.source.write_text(
            textwrap.dedent(
                """\
                #[test]
                #[should_panic]
                fn known_defect_reproduction() {
                    panic!("untracked reason");
                }
                """
            ),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(policy.PolicyError, "exact should_panic"):
            policy.scan_characterizations(self.root)


if __name__ == "__main__":
    unittest.main()
