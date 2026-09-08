from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import cargo_input_cache as cache


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
