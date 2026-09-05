from __future__ import annotations

import json
import pathlib
import tempfile
import unittest
from unittest import mock

from scripts.tests import test_gui_release_artifact as gui
from scripts.tests import test_server_release_artifact as server
from scripts.tests.package_inventory_fixture import inventory_bytes


class ReleaseDependencyInventoryTests(unittest.TestCase):
    def test_new_release_consumers_bind_the_dependency_inventory_to_each_payload(self) -> None:
        for platform in ("gui", "windows", "linux"):
            module = gui if platform == "gui" else server
            for fault in (None, "missing", "tampered", "wrong-target", "duplicate"):
                with self.subTest(platform=platform, fault=fault), tempfile.TemporaryDirectory() as temporary, mock.patch.object(module, "VERSION", "0.2.9"):
                    builder = gui.GuiArtifactBuilder(pathlib.Path(temporary)) if platform == "gui" else server.ArtifactBuilder(pathlib.Path(temporary), platform)
                    package = "sorotte-gui" if platform == "gui" else "sorotte-server"
                    target = "x86_64-unknown-linux-gnu" if platform == "linux" else "x86_64-pc-windows-msvc"
                    builder.payloads["THIRD-PARTY-NOTICES.txt"] = b"Synthetic notice fixture\n"
                    body = inventory_bytes(builder.payloads, package=package, target=target, source_sha=module.SOURCE_SHA)
                    if fault == "tampered":
                        value = json.loads(body)
                        value["payload"][0]["sha256"] = "f" * 64
                        body = json.dumps(value).encode()
                    if fault == "wrong-target":
                        value = json.loads(body)
                        value["target"] = "x86_64-pc-windows-msvc" if platform == "linux" else "x86_64-unknown-linux-gnu"
                        value["resolution_command"][6] = value["target"]
                        body = json.dumps(value).encode()
                    if fault == "duplicate":
                        body = body.replace(b'{', b'{"schema":"forged",', 1)
                    if fault != "missing":
                        builder.payloads["DEPENDENCIES.json"] = body
                    builder.write()
                    if fault is None:
                        report = builder.verify()
                        self.assertEqual(report["status"], "verified")
                    else:
                        expected = {"missing": "inventory", "tampered": "actual package payload", "wrong-target": "target", "duplicate": "duplicate_key"}[fault]
                        with self.assertRaisesRegex(module.artifact.VerificationError, expected):
                            builder.verify()

    def test_versioned_legacy_package_inventory_remains_explicit(self) -> None:
        for version in ("0.2.3", "0.2.4", "0.2.8", "0.2.8-dev.1"):
            self.assertEqual(server.artifact.dependency_files_for_version(version), set())
        for version in ("0.2.9", "0.2.9-dev.1", "1.0.0", "unrecognized-version"):
            self.assertEqual(server.artifact.dependency_files_for_version(version), {"DEPENDENCIES.json", "THIRD-PARTY-NOTICES.txt"})


if __name__ == "__main__":
    unittest.main()
