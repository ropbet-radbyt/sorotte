from __future__ import annotations

import ast
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from scripts import verification_pin_sync as sync
from scripts import verification_tools

ROOT = Path(__file__).resolve().parents[2]


class PinUpdateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.checked_files = sorted(set(verification_tools.validate_pin_projections(ROOT)["checked_files"]
                                       + sync.plan(ROOT)[1]))

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for relative in self.checked_files:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)

    def replace(self, path, old, new):
        target = self.root / path
        text = target.read_text(encoding="utf-8")
        self.assertIn(old, text)
        target.write_text(text.replace(old, new), encoding="utf-8")

    def bump(self):
        manifest = verification_tools.pins()
        for tool, new in (("rust", "9.8.7"), ("cargo-nextest", "9.8.6"),
                          ("cargo-llvm-cov", "9.8.5"), ("syft", "9.8.4"), ("cosign", "9.8.3")):
            self.replace(sync.MANIFEST, f'{tool} = "{manifest["tools"][tool]}"', f'{tool} = "{new}"')
        self.replace(sync.MANIFEST, manifest["rust-windows"]["commit"], "a" * 40)
        self.replace(sync.MANIFEST, manifest["actions"]["dtolnay/rust-toolchain"]["sha"], "b" * 40)

    def test_current_manifest_needs_no_rewrite_or_process(self):
        with mock.patch.object(verification_tools.subprocess, "check_output", side_effect=AssertionError("no tools")):
            changes, _ = sync.plan(self.root)
        self.assertEqual(changes, [])

    def test_selection_preflight_needs_only_the_standard_library(self):
        result = subprocess.run(
            [sys.executable, "-S", "-c",
             "from scripts.verification_tools import validate_pin_projections; validate_pin_projections()"],
            cwd=ROOT, capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_reviewed_bump_updates_consumers_without_editing_tests_or_historical_inputs(self):
        historic = self.root / "docs/historical.md"
        historic.write_text("rustc 1.98.1; evidence for an earlier source\n", encoding="utf-8")
        self.bump()
        changes, _ = sync.plan(self.root)
        self.assertTrue(changes)
        self.assertFalse(any(change.path.startswith("scripts/tests/") for change in changes))
        sync.write_changes(self.root, changes)
        self.assertEqual(sync.plan(self.root)[0], [])
        self.assertEqual(verification_tools.validate_pin_projections(self.root)["status"], "passed")
        self.assertIn("rustc 1.98.1", historic.read_text())
        compiler = ast.parse((self.root / "scripts/coverage_windows_process_lanes.py").read_text())
        constants = {node.targets[0].id: node.value.value for node in compiler.body
                     if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
                     and isinstance(node.value, ast.Constant)}
        self.assertEqual(constants["PINNED_RUST_RELEASE"], "9.8.7")
        self.assertEqual(constants["PINNED_RUST_COMMIT"], "a" * 40)
        profile = json.loads((self.root / "verification/windows-native-guest.json").read_text())
        self.assertEqual(profile["rust_toolchain"], "9.8.7")
        container = (self.root / "Dockerfile.server").read_text()
        self.assertIn("install 9.8.7 --profile minimal && rustup default 9.8.7", container)
        self.assertIn("FROM rust:1.98.0-trixie@sha256:", container)
        workflow = (self.root / ".github/workflows/rust-ci.yml").read_text()
        self.assertIn("toolchain: 9.8.7", workflow)
        self.assertIn("dtolnay/rust-toolchain@" + "b" * 40, workflow)
        self.assertIn("cargo-nextest@9.8.6", workflow)
        self.assertIn("cargo-llvm-cov@9.8.5", workflow)

    def test_unreviewed_action_and_dynamic_or_duplicate_assignment_fail_before_writing(self):
        before = (self.root / "rust-toolchain.toml").read_bytes()
        self.bump()
        path = self.root / "scripts/diff_coverage.py"
        original = path.read_text()
        version = verification_tools.pins()["tools"]["cargo-llvm-cov"]
        literal = f'CARGO_LLVM_COV_VERSION = "{version}"'
        for changed in (original.replace(literal, 'CARGO_LLVM_COV_VERSION = dangerous_function()'),
                        original + '\nCARGO_LLVM_COV_VERSION = "9.8.5"\n'):
            path.write_text(changed, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "literal string assignment"):
                sync.plan(self.root)
            self.assertEqual((self.root / "rust-toolchain.toml").read_bytes(), before)
        path.write_text(original, encoding="utf-8")
        self.replace(".github/workflows/rust-ci.yml", "actions/checkout@", "unreviewed/action@")
        with self.assertRaisesRegex(ValueError, "undeclared action"):
            sync.plan(self.root)

    def test_changed_projection_aborts_apply_before_any_write(self):
        self.bump()
        changes, _ = sync.plan(self.root)
        last = self.root / changes[-1].path
        last.write_bytes(last.read_bytes() + b"\n# concurrent edit\n")
        with self.assertRaisesRegex(ValueError, "changed during planning"):
            sync.write_changes(self.root, changes)
        self.assertEqual((self.root / changes[0].path).read_bytes(), changes[0].before)

    def test_preview_and_check_do_not_write_and_apply_is_idempotent(self):
        self.bump()
        paths = [change.path for change in sync.plan(self.root)[0]]
        before = {path: (self.root / path).read_bytes() for path in paths}
        for flag in ([], ["--check"]):
            with redirect_stdout(io.StringIO()) as output, redirect_stderr(io.StringIO()):
                self.assertEqual(sync.main(["--repo-root", str(self.root), *flag]), 1)
            self.assertIn("rust-toolchain.toml", output.getvalue())
            self.assertEqual(before, {path: (self.root / path).read_bytes() for path in paths})
        with redirect_stdout(io.StringIO()):
            self.assertEqual(sync.main(["--repo-root", str(self.root), "--write"]), 0)
            self.assertEqual(sync.main(["--repo-root", str(self.root), "--check"]), 0)

    def test_crlf_and_unrelated_utf8_text_survive_an_update(self):
        for relative in self.checked_files:
            path = self.root / relative
            path.write_bytes(path.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))
        wrapper = self.root / "scripts/diff_coverage.py"
        marker = '# unrelated label: caf\u00e9 \U0001f980\r\n'.encode("utf-8")
        wrapper.write_bytes(marker + wrapper.read_bytes())
        self.assertEqual(sync.plan(self.root)[0], [])
        self.bump()
        changes, _ = sync.plan(self.root)
        sync.write_changes(self.root, changes)
        self.assertTrue(wrapper.read_bytes().startswith(marker))
        for change in changes:
            self.assertNotIn(b"\n", change.after.replace(b"\r\n", b""), change.path)
        self.assertEqual(sync.plan(self.root)[0], [])

    def test_quoted_workflow_pins_update_without_rewriting_shell_text(self):
        manifest = verification_tools.pins()
        path = self.root / ".github/workflows/quoted-pins.yaml"
        path.write_text(
            'jobs:\n  check:\n    steps:\n'
            f'      - uses: "dtolnay/rust-toolchain@{manifest["actions"]["dtolnay/rust-toolchain"]["sha"]}"\n'
            "        with: {toolchain: 'stable'}\n"
            '      - run: |\n          toolchain: 1.2.3\n          uses: unreviewed/action@main\n',
            encoding="utf-8")
        self.bump()
        sync.write_changes(self.root, sync.plan(self.root)[0])
        workflow = path.read_text(encoding="utf-8")
        self.assertIn('uses: "dtolnay/rust-toolchain@' + "b" * 40 + '"', workflow)
        self.assertIn("with: {toolchain: '9.8.7'}", workflow)
        self.assertIn('run: |\n          toolchain: 1.2.3\n          uses: unreviewed/action@main\n', workflow)

    def test_missing_ambiguous_and_indirect_workflow_pins_fail(self):
        sha = verification_tools.pins()["actions"]["dtolnay/rust-toolchain"]["sha"]
        path = self.root / ".github/workflows/broken-pins.yml"
        for inputs in ("{}", "{toolchain: 1.2.3, toolchain: 1.2.4}",
                       "{toolchain: &pin 1.2.3}", "{toolchain: '${{ matrix.rust }}'}"):
            path.write_text(f"jobs:\n  check:\n    steps:\n      - uses: dtolnay/rust-toolchain@{sha}\n"
                            f"        with: {inputs}\n", encoding="utf-8")
            with self.subTest(inputs=inputs), self.assertRaises(ValueError):
                sync.plan(self.root)

    def test_shared_composite_action_pins_remain_reviewed_and_updatable(self):
        relative = ".github/actions/windows-playback-qualification/action.yml"
        manifest = verification_tools.pins()
        original = manifest["actions"]["actions/upload-artifact"]["sha"]
        self.replace(sync.MANIFEST, original, "c" * 40)
        changes, _ = sync.plan(self.root)
        self.assertIn(relative, [change.path for change in changes])
        sync.write_changes(self.root, changes)
        action = (self.root / relative).read_text(encoding="utf-8")
        self.assertIn("actions/upload-artifact@" + "c" * 40, action)
        self.assertNotIn(original, action)
        self.assertEqual(sync.plan(self.root)[0], [])
        self.replace(relative, "actions/upload-artifact@", "unreviewed/action@")
        with self.assertRaisesRegex(ValueError, "undeclared action"):
            sync.plan(self.root)

    def test_malformed_authority_fails(self):
        self.replace(sync.MANIFEST, 'rust = "' + verification_tools.pins()["tools"]["rust"] + '"',
                     'rust = "stable"')
        with self.assertRaisesRegex(ValueError, "exact release version"):
            sync.plan(self.root)


if __name__ == "__main__":
    unittest.main()
