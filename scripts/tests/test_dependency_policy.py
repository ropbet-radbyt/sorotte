from __future__ import annotations
import copy
import datetime as dt
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import dependency_policy as policy

ROOT = pathlib.Path(__file__).resolve().parents[2]


class DependencyPolicyTests(unittest.TestCase):
    def summary(self, errors=0):
        return {"type": "summary", "fields": {"advisories": {"errors": errors, "helps": 0, "notes": 0, "warnings": 0}, "sources": {"errors": 0, "helps": 0, "notes": 0, "warnings": 0}}}

    def test_known_advisory_fixture_fails_even_with_valid_scanner_summary(self):
        # Identity observed in the actual pre-remediation cargo-deny campaign.
        finding = {"type": "diagnostic", "fields": {"advisory": {"id": "RUSTSEC-2026-0221", "package": "event-listener"}, "code": "unsound", "severity": "error", "message": "unsound listener", "labels": []}}
        result = policy.summarize_deny(json.dumps(finding) + "\n" + json.dumps(self.summary(1)), 1)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["findings"][0]["id"], "RUSTSEC-2026-0221")

    def test_unavailable_database_is_not_zero_findings(self):
        for text in ("", json.dumps({"type": "log", "fields": {"message": "database unavailable"}})):
            with self.assertRaises(policy.DependencyError):
                policy.summarize_deny(text, 1)

    def test_exit_status_summary_duplicates_and_boolean_counters_fail(self):
        with self.assertRaises(policy.DependencyError):
            policy.summarize_deny(json.dumps(self.summary()), 1)
        with self.assertRaises(policy.DependencyError):
            policy.summarize_deny(json.dumps(self.summary()) + "\n" + json.dumps(self.summary()), 0)
        value = self.summary()
        value["fields"]["advisories"]["errors"] = False
        with self.assertRaises(policy.DependencyError):
            policy.summarize_deny(json.dumps(value), 0)

    def test_expired_and_incomplete_exceptions_fail(self):
        base = (ROOT / policy.POLICY).read_text(encoding="utf-8").replace("exception = []", "")
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "coverage").mkdir()
            record = '\n[[exception]]\nid="RUSTSEC-2026-0221"\necosystem="rust"\nrationale="Fixture only"\nowner="security"\nexpires="2026-09-05"\n'
            (root / policy.POLICY).write_text(base + record)
            with self.assertRaisesRegex(policy.DependencyError, "expired"):
                policy.load_policy(root, today=dt.date(2026, 9, 5))
            (root / policy.POLICY).write_text(base + record.replace('owner="security"\n', ""))
            with self.assertRaisesRegex(policy.DependencyError, "requires"):
                policy.load_policy(root)

    def fixture_inventory(self):
        zero = "0" * 64
        package = "sorotte-server"
        target = "x86_64-pc-windows-msvc"
        return {"schema": "sorotte-dependency-inventory-v1", "package": package, "target": target, "features": "default", "dependency_kinds": ["normal", "build"], "source_sha": "a" * 40, "inputs": [{"path": path, "sha256": zero} for path in ["Cargo.toml", "Cargo.lock", f"crates/{package}/Cargo.toml", policy.POLICY, "coverage/native-components.toml"]], "payload": [{"path": "sorotte-server.exe", "sha256": zero}], "resolution_command": ["cargo", "tree", "--locked", "-p", package, "--target", target, "--edges", "normal,build", "--prefix", "none", "--format", "{p}"], "resolution_sha256": zero, "packages": [{"name": package, "version": "0.2.9", "source": None, "license": "Apache-2.0", "repository": None, "notice_files": []}], "native_components": {"schema_version": 1, "component": []}}

    def verify(self, value, hashes=None):
        return policy.validate_inventory(value, payload_hashes=hashes or {"sorotte-server.exe": "0" * 64}, expected_package="sorotte-server", expected_source_sha="a" * 40)

    def test_inventory_matches_actual_artifact_inputs_and_rejects_tamper(self):
        value = self.fixture_inventory()
        self.verify(value)
        with self.assertRaisesRegex(policy.DependencyError, "actual package"):
            self.verify(value, {"sorotte-server.exe": "1" * 64})
        value["inputs"] = [item for item in value["inputs"] if item["path"] != "Cargo.lock"]
        with self.assertRaisesRegex(policy.DependencyError, "graph input"):
            self.verify(value)

    def test_unapproved_source_and_duplicate_package_are_detected(self):
        value = self.fixture_inventory()
        value["packages"][0]["source"] = "git+https://unapproved.invalid/source#" + "a" * 40
        with self.assertRaisesRegex(policy.DependencyError, "unapproved"):
            self.verify(value)
        value = self.fixture_inventory()
        value["packages"].append(copy.deepcopy(value["packages"][0]))
        with self.assertRaisesRegex(policy.DependencyError, "duplicate"):
            self.verify(value)

    def test_repository_policy_has_no_unreviewed_exceptions(self):
        self.assertEqual(policy.load_policy(ROOT)["exception"], [])

    def test_inventory_rejects_wrong_types_without_skipping_domain_validation(self):
        for field, invalid in [("target", []), ("native_components", {"schema_version": True, "component": []})]:
            value = self.fixture_inventory()
            value[field] = invalid
            with self.subTest(field=field), self.assertRaises(policy.DependencyError):
                self.verify(value)
        value = self.fixture_inventory()
        value["packages"][0]["notice_files"] = False
        with self.assertRaises(policy.DependencyError):
            self.verify(value)


if __name__ == "__main__":
    unittest.main()
