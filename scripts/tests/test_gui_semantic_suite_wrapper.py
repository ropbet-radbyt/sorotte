import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WRAPPER = REPO_ROOT / "scripts" / "gui-semantic-suite.ps1"


@unittest.skipUnless(os.name == "nt", "Windows PowerShell contract")
class GuiSemanticSuiteWrapperTests(unittest.TestCase):
    def test_case_colliding_path_variables_do_not_block_launch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sorotte-semantic-wrapper-") as temp:
            fake_cargo = Path(temp) / "cargo.cmd"
            fake_cargo.write_text(
                "@echo off\r\n"
                "echo %*\r\n"
                "exit /b 0\r\n",
                encoding="utf-8",
            )
            environment = dict(os.environ)
            inherited_path = environment.get("Path") or environment.get("PATH") or ""
            environment["Path"] = inherited_path
            environment["PATH"] = inherited_path

            completed = subprocess.run(
                [
                    "powershell",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(WRAPPER),
                    "-List",
                    "-CargoExecutable",
                    str(fake_cargo),
                ],
                cwd=REPO_ROOT,
                env=environment,
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )

            self.assertEqual(
                completed.returncode,
                0,
                f"stdout={completed.stdout!r}\nstderr={completed.stderr!r}",
            )
            self.assertIn("--bin sorotte-gui-semantic-suite -- --list", completed.stdout)
            self.assertNotIn("Item has already been added", completed.stdout + completed.stderr)


if __name__ == "__main__":
    unittest.main()
