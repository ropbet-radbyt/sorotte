from __future__ import annotations
import pathlib
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import critical_boundaries as boundaries


class CriticalBoundaryTests(unittest.TestCase):
    def test_current_contracts_retain_critical_coverage(self):
        result = boundaries.validate(pathlib.Path(__file__).resolve().parents[2])
        modules = set().union(*map(set, result.values()))
        self.assertIn("crates/sorotte-player-mpv/src/adapter/reconnection.rs", modules)
        self.assertIn("crates/sorotte-server/src/local_clock.rs", modules)
        self.assertIn("crates/sorotte-gui/src/app/runtime_owner/player/telemetry.rs", modules)

    def test_extracted_critical_module_cannot_become_ordinary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve()
            (root / "coverage").mkdir()
            (root / "src/authority").mkdir(parents=True)
            (root / "src/authority.rs").write_text("mod admission;\n", encoding="utf-8")
            (root / "src/authority/admission.rs").write_text("pub fn admit() -> bool { true }\n", encoding="utf-8")
            (root / "coverage/critical-boundaries.toml").write_text('schema_version=1\n[[boundary]]\nid="admission"\nowner="network"\ncontract="Own permits"\nroots=["src/authority.rs"]\n', encoding="utf-8")
            covered = {"src/authority.rs"}
            policy = types.SimpleNamespace(match=lambda path: types.SimpleNamespace(owner="network") if path in covered else None)
            with mock.patch.object(boundaries, "load_critical_path_policy", return_value=policy):
                with self.assertRaisesRegex(boundaries.BoundaryError, "admission.rs lost critical"):
                    boundaries.validate(root)
                covered.add("src/authority/admission.rs")
                self.assertEqual(len(boundaries.validate(root)["admission"]), 2)

    def test_explicit_module_paths_are_followed_but_test_modules_are_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve()
            (root / "root.rs").write_text('#[path = "platform.rs"]\nmod os;\n#[cfg(test)]\nmod tests {\n mod absent;\n let s = "}";\n}\n', encoding="utf-8")
            (root / "platform.rs").write_text("pub fn boundary() {}", encoding="utf-8")
            self.assertEqual(boundaries.owned_modules(root, "root.rs"), {"root.rs", "platform.rs"})


if __name__ == "__main__":
    unittest.main()
