from __future__ import annotations

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
SELECTORS = {
    "dependency-policy.yml": ("selection", "dependencies"),
    "gui-native-interactive.yml": ("selection", "native"),
    "package-ci.yml": ("preflight", "release"),
    "rust-fuzz.yml": ("selection", "fuzz"),
}


class WorkflowOptionalArgumentTests(unittest.TestCase):
    def test_actual_selector_commands_handle_unset_empty_and_present_force(self):
        if os.name == "nt":
            git = shutil.which("git")
            bash = Path(git).resolve().parents[1] / "bin/bash.exe" if git else None
            if not bash or not bash.is_file():
                self.skipTest("Git Bash is unavailable")
        else:
            bash = shutil.which("bash")
            if not bash:
                self.skipTest("Bash is unavailable")
        with tempfile.TemporaryDirectory(prefix="selector arguments ") as temporary:
            fixture = Path(temporary).resolve()
            plan = fixture / "plan with spaces.json"
            plan.write_text(json.dumps({"lanes": [{"id": lane, "selected": False}
                                                  for _, lane in SELECTORS.values()]}), encoding="utf-8")
            output = fixture / "output with spaces.txt"
            for filename, (job, lane) in SELECTORS.items():
                workflow = yaml.load((ROOT / ".github/workflows" / filename).read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
                command = next(step["run"] for step in workflow["jobs"][job]["steps"] if step.get("id") == "select")
                self.assertTrue(command.startswith("python scripts/verify.py selected "))
                original_plan = shlex.split(command)[shlex.split(command).index("--plan") + 1]
                command = command.replace(original_plan, shlex.quote(plan.as_posix()), 1)
                command = command.replace("python ", shlex.quote(Path(sys.executable).as_posix()) + " ", 1)
                for force in (None, "", "--force"):
                    with self.subTest(workflow=filename, force=force):
                        output.unlink(missing_ok=True)
                        environment = dict(os.environ, GITHUB_OUTPUT=output.as_posix())
                        environment.pop("FORCE", None)
                        if force is not None:
                            environment["FORCE"] = force
                        result = subprocess.run([str(bash), "-e", "-c", command], cwd=ROOT, env=environment,
                                                capture_output=True, text=True, timeout=20)
                        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertEqual(output.read_text(encoding="utf-8"),
                                         "selected=" + str(force == "--force").lower() + "\n")


if __name__ == "__main__":
    unittest.main()
