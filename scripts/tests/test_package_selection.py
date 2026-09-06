from __future__ import annotations
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify

ROOT = Path(__file__).resolve().parents[2]


class PackageSelectionTests(unittest.TestCase):
    def test_each_package_producer_and_consumer_use_the_same_exact_head(self):
        workflow = yaml.load((ROOT / ".github/workflows/package-ci.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(workflow["env"]["VERIFICATION_SHA"], "${{ github.event.pull_request.head.sha || github.sha }}")
        for job in workflow["jobs"].values():
            for step in job["steps"]:
                if step.get("uses", "").startswith("actions/checkout@"):
                    self.assertEqual(step["with"]["ref"], "${{ env.VERIFICATION_SHA }}")
                if "verify_gui_release_artifact.py" in step.get("run", ""):
                    self.assertIn('--expected-source-sha "$env:VERIFICATION_SHA"', step["run"])
                if "verify_server_release_artifact.py" in step.get("run", ""):
                    self.assertIn('--expected-source-sha "${{ env.VERIFICATION_SHA }}"', step["run"])
        self.assertEqual(workflow["jobs"]["archive"]["if"], "needs.preflight.outputs.selected == 'true'")

    def test_docs_do_not_select_archives_but_package_harness_and_product_changes_do(self):
        policy = json.loads(verify.POLICY.read_text(encoding="utf-8"))
        for path, expected in (("docs/tutorial.md", False), ("scripts/package-gui-release.ps1", True),
                               ("crates/sorotte-core/src/lib.rs", True), ("Dockerfile", True)):
            lane = next(item for item in verify.select([path], policy) if item["id"] == "release")
            self.assertEqual(lane["selected"], expected, path)
        with self.assertRaises(ValueError): verify.gate("release", False, ["archive=skipped"], ["archive"], None)
        for outcome in ("skipped", "cancelled", "failure"):
            with self.assertRaises(ValueError): verify.gate("release", True, [f"archive={outcome}"], ["archive"], None)

    def test_package_metadata_uses_checkout_when_github_event_identifies_another_commit(self):
        shell = shutil.which("powershell") or shutil.which("pwsh")
        if not shell:
            self.skipTest("PowerShell is unavailable")
        with tempfile.TemporaryDirectory(prefix="package-source-") as folder:
            subprocess.run(["git", "init", "--quiet", folder], check=True, capture_output=True)
            subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "--quiet", "-m", "fixture"], cwd=folder, check=True, capture_output=True)
            expected = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=folder, text=True).strip()
            source = str(ROOT / "scripts/package-gui-release.ps1").replace("'", "''")
            command = f"""$tokens=$null; $errors=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile('{source}',[ref]$tokens,[ref]$errors)
if ($errors.Count) {{ throw 'Package source does not parse' }}
$function=$ast.Find({{param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-GitSha'}},$true)
Invoke-Expression $function.Extent.Text
$env:GITHUB_SHA='{'b' * 40}'
Get-GitSha
"""
            result = subprocess.run([shell, "-NoProfile", "-Command", command], cwd=folder, check=True, capture_output=True, text=True, timeout=20)
            self.assertEqual(result.stdout.strip(), expected)


if __name__ == "__main__": unittest.main()
