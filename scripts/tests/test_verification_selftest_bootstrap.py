from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify


class SelftestBootstrapTests(unittest.TestCase):
    def test_real_discovery_canonicalizes_child_temp_without_changing_parent(self):
        with tempfile.TemporaryDirectory(prefix="apparatus bootstrap ") as temporary:
            root = Path(temporary).resolve()
            scratch = root / "temporary fixture directory"
            scratch.mkdir()
            (root / "alias parent").mkdir()
            alias = str(root / "alias parent" / ".." / scratch.name)
            if os.name == "nt":
                # Exercise the hosted RUNNER~1 failure with a real 8.3 alias
                # when available; dot segments still cover hosts without 8.3.
                import ctypes
                buffer = ctypes.create_unicode_buffer(32768)
                length = ctypes.windll.kernel32.GetShortPathNameW(str(scratch), buffer, len(buffer))
                if 0 < length < len(buffer) and buffer.value != str(scratch):
                    alias = buffer.value
            self.assertNotEqual(alias, str(scratch))
            self.assertEqual(Path(alias).resolve(), scratch)
            tests = root / "scripts/tests"
            tests.mkdir(parents=True)
            (tests / "test_probe.py").write_text(
                "import json, os, pathlib, tempfile, unittest\n"
                "class ChildProbe(unittest.TestCase):\n"
                " def test_canonical_temporary_paths(self):\n"
                "  values = {key: os.environ[key] for key in ('TMPDIR', 'TEMP', 'TMP')}\n"
                "  for value in values.values():\n"
                "   self.assertEqual(value, str(pathlib.Path(value).resolve()))\n"
                "  with tempfile.TemporaryDirectory() as directory:\n"
                "   self.assertEqual(pathlib.Path(directory), pathlib.Path(directory).resolve())\n"
                "   self.assertEqual(pathlib.Path(directory).parent, pathlib.Path(values['TEMP']))\n"
                "  (pathlib.Path(__file__).resolve().parents[2] / 'probe.json').write_text(json.dumps(values), encoding='utf-8')\n",
                encoding="utf-8",
            )
            output = root / "attempt"
            with mock.patch.dict(os.environ, {name: alias for name in ("TMPDIR", "TEMP", "TMP")}), \
                    mock.patch.object(verify, "ROOT", root), \
                    mock.patch.object(verify, "identity", return_value={"source_sha": "a" * 40}):
                parent_environment = dict(os.environ)
                record = verify.run_lane("static", output, 30)
                self.assertEqual(dict(os.environ), parent_environment)
            self.assertEqual(record["status"], "passed", record.get("primary_failure"))
            self.assertEqual(record["command"][1:],
                             ["-m", "unittest", "discover", "-s", "scripts/tests", "-p", "test_*.py"])
            self.assertEqual(json.loads((root / "probe.json").read_text(encoding="utf-8")),
                             {name: str(scratch) for name in ("TMPDIR", "TEMP", "TMP")})
            self.assertTrue((output / "receipt.json").is_file())
            self.assertTrue((output / "process/process.json").is_file())
            with self.assertRaisesRegex(ValueError, "already exists"):
                verify.run_lane("static", output, 30)


if __name__ == "__main__":
    unittest.main()
