from __future__ import annotations

import hashlib
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest import mock

from scripts import compat_live_interop as interop
from scripts import verification_tools as tools


ROOT = Path(__file__).resolve().parents[2]


class PinProjectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.checked_files = tools.validate_pin_projections(ROOT)["checked_files"]

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for relative in self.checked_files:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)

    def replace(self, relative: str, before: str, after: str) -> None:
        path = self.root / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(before, text)
        path.write_text(text.replace(before, after), encoding="utf-8")

    def test_current_inputs_need_no_subprocess_or_wrapper_import(self) -> None:
        wrapper = self.root / "scripts/diff_coverage.py"
        with wrapper.open("a", encoding="utf-8") as stream:
            stream.write("\nimport unavailable_heavy_wrapper_dependency\nraise RuntimeError('must not execute')\n")
        with mock.patch.object(tools.subprocess, "run", side_effect=AssertionError("no compilation")), \
                mock.patch.object(tools.subprocess, "check_output", side_effect=AssertionError("no process")):
            result = tools.validate_pin_projections(self.root)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["constraints_packages"], 44)
        self.assertEqual(result["checked_files"], self.checked_files)

    def test_legacy_sha_drift_is_rejected(self) -> None:
        self.replace("scripts/gui_sandbox_bundle.py", tools.pins()["references"]["legacy-sha"], "a" * 40)
        with self.assertRaisesRegex(ValueError, "LEGACY_SHA"):
            tools.validate_pin_projections(self.root)

    def test_tool_version_drift_is_rejected(self) -> None:
        self.replace("scripts/diff_coverage.py", 'CARGO_LLVM_COV_VERSION = "0.8.4"',
                     'CARGO_LLVM_COV_VERSION = "0.8.3"')
        with self.assertRaisesRegex(ValueError, "CARGO_LLVM_COV_VERSION"):
            tools.validate_pin_projections(self.root)

    def test_dynamic_pin_is_rejected_without_evaluation(self) -> None:
        self.replace("scripts/diff_coverage.py", 'CARGO_LLVM_COV_VERSION = "0.8.4"',
                     'CARGO_LLVM_COV_VERSION = dangerous_function()')
        with self.assertRaisesRegex(ValueError, "static literal"):
            tools.validate_pin_projections(self.root)

    def test_duplicate_pin_is_rejected(self) -> None:
        self.replace("scripts/diff_coverage.py", 'CARGO_LLVM_COV_VERSION = "0.8.4"',
                     'CARGO_LLVM_COV_VERSION = "0.8.4"\nCARGO_LLVM_COV_VERSION = "0.8.4"')
        with self.assertRaisesRegex(ValueError, "one literal assignment"):
            tools.validate_pin_projections(self.root)

    def test_mutation_call_argument_drift_is_rejected(self) -> None:
        self.replace("scripts/mutation_tool_canary.py", '"27.1.0"', '"27.0.0"')
        with self.assertRaisesRegex(ValueError, "mutation canary"):
            tools.validate_pin_projections(self.root)

    def test_manifest_change_requires_wrapper_projection(self) -> None:
        self.replace("coverage/verification-tools.toml", 'cargo-nextest = "0.9.137"',
                     'cargo-nextest = "0.9.138"')
        with self.assertRaisesRegex(ValueError, "PINNED_NEXTEST_VERSION"):
            tools.validate_pin_projections(self.root)

    def test_transitive_change_requires_reviewed_digest(self) -> None:
        self.replace("requirements/verification-constraints.txt", "attrs==26.1.0", "attrs==26.0.0")
        with self.assertRaisesRegex(ValueError, "reviewed constraints digest"):
            tools.validate_pin_projections(self.root)

    def test_all_environments_require_exactly_one_local_constraint(self) -> None:
        for replacement in ("", "-c arbitrary.txt", "-c verification-constraints.txt\n-c verification-constraints.txt"):
            path = self.root / "requirements/ci-policy.txt"
            path.write_text((ROOT / "requirements/ci-policy.txt").read_text(encoding="utf-8").replace(
                "-c verification-constraints.txt", replacement), encoding="utf-8")
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                tools.validate_pin_projections(self.root)

    def test_audit_dependency_cannot_be_added_to_interop(self) -> None:
        path = self.root / "requirements/legacy-python-interop.txt"
        with path.open("a", encoding="utf-8") as stream:
            stream.write("\npip-audit==2.10.1\n")
        with self.assertRaisesRegex(ValueError, "legacy-python-interop.txt"):
            tools.validate_pin_projections(self.root)

    def test_certificate_selftest_dependency_cannot_be_removed_from_bootstrap(self) -> None:
        self.replace("requirements/ci-policy.txt", "cryptography==50.0.1\n", "")
        with self.assertRaisesRegex(ValueError, "ci-policy.txt"):
            tools.validate_pin_projections(self.root)

    def test_constraint_pin_duplicates_and_nested_directives_fail(self) -> None:
        path = self.root / "requirements/verification-constraints.txt"
        original = path.read_text(encoding="utf-8")
        for suffix in ("attrs==26.1.0\n", "-c more.txt\n", "idna>=3.19\n"):
            path.write_text(original + suffix, encoding="utf-8")
            with self.subTest(suffix=suffix), self.assertRaisesRegex(ValueError, "unsupported or duplicate"):
                tools.validate_pin_projections(self.root)

    def test_constraint_review_digest_is_stable_across_checkout_line_endings(self) -> None:
        path = self.root / "requirements/verification-constraints.txt"
        text = path.read_text(encoding="utf-8")
        path.write_bytes(text.replace("\n", "\r\n").encode("utf-8"))
        self.assertEqual(tools.validate_pin_projections(self.root)["status"], "passed")
        self.assertNotEqual(hashlib.sha256(path.read_bytes()).hexdigest(),
                            tools.pins()["python-resolution"]["constraints-lf-sha256"])

    def test_missing_projection_input_fails(self) -> None:
        (self.root / "scripts/nextest_ci.py").unlink()
        with self.assertRaisesRegex(ValueError, "missing or indirect"):
            tools.validate_pin_projections(self.root)

    def test_interop_accepts_only_additive_reviewed_constraint(self) -> None:
        data = (self.root / "requirements/legacy-python-interop.txt").read_bytes()
        self.assertEqual(interop.parse_pinned_requirements(data), interop.PINNED_PACKAGES)
        interop.verify_python_constraints(self.root, data)
        for suffix in (b"\n-c verification-constraints.txt\n", b"\n-c other.txt\n"):
            with self.subTest(suffix=suffix), self.assertRaises(interop.InteropContractError):
                interop.parse_pinned_requirements(data + suffix)

    def test_interop_rejects_changed_or_missing_constraints_before_python_probe(self) -> None:
        path = self.root / "requirements/verification-constraints.txt"
        path.write_text(path.read_text(encoding="utf-8") + "\n# unreviewed\n", encoding="utf-8")
        with mock.patch.object(interop.subprocess, "run") as run:
            with self.assertRaisesRegex(interop.InteropContractError, "constraints.*changed"):
                interop.verify_python(self.root, {})
            run.assert_not_called()
            path.unlink()
            with self.assertRaisesRegex(interop.InteropContractError, "constraints.*unavailable"):
                interop.verify_python(self.root, {})
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
