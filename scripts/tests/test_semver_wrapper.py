from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
WRAPPER = REPO_ROOT / "scripts" / "check-semver.ps1"
DEVELOPMENT = REPO_ROOT / "docs" / "DEVELOPMENT.md"
BASELINE = "a" * 40


class SemverWrapperPolicyTests(unittest.TestCase):
    def test_development_uses_the_repository_wrapper(self) -> None:
        documentation = DEVELOPMENT.read_text(encoding="utf-8")
        self.assertIn(
            "./scripts/check-semver.ps1 -BaselineRev <full-base-sha>",
            documentation,
        )
        self.assertNotIn(
            "cargo semver-checks --package $package",
            documentation,
        )

    def test_wrapper_owns_a_short_external_target_and_cleanup(self) -> None:
        wrapper = WRAPPER.read_text(encoding="utf-8")
        for required in (
            "[System.IO.Path]::GetTempPath()",
            "$targetRoot.Length -gt 64",
            "$targetRoot.StartsWith($repoPrefix",
            "$env:CARGO_TARGET_DIR = $targetRoot",
            "Remove-Item -LiteralPath $targetRoot -Recurse -Force",
        ):
            self.assertIn(required, wrapper)

    @unittest.skipUnless(os.name == "nt", "PowerShell execution contract is Windows-only")
    def test_wrapper_executes_from_a_long_checkout_with_a_short_target(self) -> None:
        powershell = shutil.which("powershell.exe") or shutil.which("powershell")
        self.assertIsNotNone(powershell, "Windows policy requires Windows PowerShell")

        with tempfile.TemporaryDirectory() as scratch:
            scratch_path = Path(scratch)
            log_path = scratch_path / "cargo.log"
            cargo_stub = scratch_path / "cargo-stub.cmd"
            cargo_stub.write_text(
                '@echo %CARGO_TARGET_DIR%^|%*>>"%SOROTTE_SEMVER_TEST_LOG%"\r\n'
                "@exit /b 0\r\n",
                encoding="ascii",
            )
            environment = os.environ.copy()
            environment["SOROTTE_SEMVER_TEST_LOG"] = str(log_path)
            result = subprocess.run(
                [
                    str(powershell),
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(WRAPPER),
                    "-BaselineRev",
                    BASELINE,
                    "-Package",
                    "sorotte-protocol",
                    "-CargoExecutable",
                    str(cargo_stub),
                ],
                cwd=REPO_ROOT,
                env=environment,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            target_text, arguments = log_path.read_text(encoding="utf-8").strip().split("|", 1)
            target = Path(target_text)
            self.assertTrue(target.is_absolute())
            self.assertLessEqual(len(str(target)), 64)
            self.assertFalse(target.is_relative_to(REPO_ROOT))
            self.assertFalse(target.exists(), "wrapper must clean its temporary target")
            self.assertEqual(
                arguments,
                f"semver-checks --package sorotte-protocol --baseline-rev {BASELINE}",
            )


if __name__ == "__main__":
    unittest.main()
