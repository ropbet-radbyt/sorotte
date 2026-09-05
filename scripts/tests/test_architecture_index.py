from __future__ import annotations

import copy
import contextlib
import io
import pathlib
import sys
import tempfile
import tomllib
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import architecture_index as index

ROOT = pathlib.Path(__file__).resolve().parents[2]


class ArchitectureIndexTests(unittest.TestCase):
    def setUp(self):
        self.data = tomllib.loads((ROOT / index.CATALOG).read_text(encoding="utf-8"))

    def test_representative_boundaries_have_live_source_and_executable_proof(self):
        data = index.validate(ROOT, self.data)
        boundaries = {item["id"]: item for item in data["boundary"]}
        for name in ("Wire framing and room capacity", "Owned player shutdown", "Private settings transactions", "Player retry and physical recovery"):
            self.assertIn(name, boundaries)
            self.assertTrue(boundaries[name]["owners"])
            self.assertTrue(boundaries[name]["proof"])
            self.assertTrue(boundaries[name]["environment"])
        self.assertEqual(data["release"]["fixing_sha"], "pending")
        self.assertEqual(data["release"]["hosted"], "pending")

    def test_source_move_cannot_leave_a_silently_broken_owner(self):
        self.data["boundary"][0]["owners"][0] = "crates/removed-owner.rs"
        with self.assertRaisesRegex(index.IndexError, "missing repository reference"):
            index.validate(ROOT, self.data)

    def test_renamed_or_non_test_symbol_cannot_claim_executable_proof(self):
        for symbol in ("a_removed_regression", "next_non_whitespace"):
            data = copy.deepcopy(self.data)
            data["boundary"][0]["proof"][0]["symbol"] = symbol
            with self.subTest(symbol=symbol), self.assertRaises(index.IndexError):
                index.validate(ROOT, data)

    def test_missing_catalog_identity_and_audit_task_are_rejected(self):
        data = copy.deepcopy(self.data)
        next(item for item in data["boundary"] if item["catalog"])["catalog"][0]["id"] = "removed-invariant"
        with self.assertRaisesRegex(index.IndexError, "catalog identity"):
            index.validate(ROOT, data)
        self.data["boundary"] = [item for item in self.data["boundary"] if "A20" not in item["tasks"]]
        with self.assertRaisesRegex(index.IndexError, "task map"):
            index.validate(ROOT, self.data)

    def test_ignored_real_player_command_must_actually_run_its_test(self):
        data = copy.deepcopy(self.data)
        boundary = next(item for item in data["boundary"] if item["id"] == "Player retry and physical recovery")
        proof = boundary["proof"][1]
        proof["command"] = proof["command"].replace("--ignored", "")
        with self.assertRaisesRegex(index.IndexError, "would not execute"):
            index.validate(ROOT, data)

    def test_generated_index_matches_the_validated_current_catalog(self):
        self.assertEqual((ROOT / index.DOCUMENT).read_text(encoding="utf-8"), index.render(index.validate(ROOT, self.data)))

    def test_cli_write_and_check_agree_across_checkout_line_endings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "docs").mkdir()
            argv = ["architecture_index.py", "--repo-root", str(root)]
            with mock.patch.object(index, "validate", return_value=self.data), contextlib.redirect_stdout(io.StringIO()):
                with mock.patch.object(sys, "argv", argv + ["--write"]):
                    self.assertEqual(index.main(), 0)
                output = root / index.DOCUMENT
                contents = output.read_bytes()
                self.assertNotIn(b"\r", contents)
                with mock.patch.object(sys, "argv", argv):
                    self.assertEqual(index.main(), 0)
                    output.write_bytes(contents.replace(b"\n", b"\r\n"))
                    self.assertEqual(index.main(), 0)
                    output.write_bytes(contents + b"stale content\n")
                    with contextlib.redirect_stderr(io.StringIO()):
                        self.assertEqual(index.main(), 1)


if __name__ == "__main__":
    unittest.main()
