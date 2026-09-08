from __future__ import annotations

import pathlib
import textwrap
import unittest

import yaml

from scripts.verification_tools import pins as verification_pins

VERIFICATION_PINS = verification_pins()


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "rust-coverage.yml"
CHECKOUT = f"actions/checkout@{VERIFICATION_PINS['actions']['actions/checkout']['sha']}"
RUST = f"dtolnay/rust-toolchain@{VERIFICATION_PINS['actions']['dtolnay/rust-toolchain']['sha']}"
PYTHON = f"actions/setup-python@{VERIFICATION_PINS['actions']['actions/setup-python']['sha']}"
INSTALL = f"taiki-e/install-action@{VERIFICATION_PINS['actions']['taiki-e/install-action']['sha']}"
UPLOAD = f"actions/upload-artifact@{VERIFICATION_PINS['actions']['actions/upload-artifact']['sha']}"


def normalized(value: object) -> str:
    if not isinstance(value, str):
        raise AssertionError(f"workflow run command must be text: {value!r}")
    return " ".join(textwrap.dedent(value).split())


def named_step(job: dict[str, object], name: str) -> dict[str, object]:
    matches = [
        step
        for step in job["steps"]
        if isinstance(step, dict) and step.get("name") == name
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected exactly one {name!r} step, found {len(matches)}"
        )
    return matches[0]


class WindowsProcessCoverageWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = WORKFLOW_PATH.read_text(encoding="utf-8")
        document = yaml.safe_load(cls.text)
        cls.job = document["jobs"]["windows-process-coverage"]

    def assert_run(self, name: str, expected: str) -> None:
        step = named_step(self.job, name)
        self.assertEqual(normalized(step.get("run")), normalized(expected))
        self.assertNotIn("continue-on-error", step)
        self.assertNotIn("if", step)

    def test_job_is_isolated_bounded_and_noninteractive(self) -> None:
        self.assertEqual(self.job["runs-on"], "windows-2025")
        self.assertEqual(self.job["timeout-minutes"], 25)
        self.assertEqual(
            self.job["env"],
            {"CARGO_TARGET_DIR": "target/llvm-cov-windows-process"},
        )
        self.assertNotIn("needs", self.job)
        job_text = yaml.safe_dump(self.job).lower()
        self.assertNotIn("gui-native-smoke", job_text)
        self.assertNotIn("sorotte-gui-native-smoke", job_text)
        self.assertNotIn("syncplay/syncplay", job_text)

    def test_job_uses_pinned_minimum_toolchain(self) -> None:
        checkout = named_step(self.job, "Checkout")
        self.assertEqual(checkout["uses"], CHECKOUT)
        self.assertEqual(
            checkout.get("with"),
            {"persist-credentials": False},
        )

        rust = named_step(self.job, "Setup Rust")
        self.assertEqual(rust["uses"], RUST)
        self.assertEqual(
            rust.get("with"),
            {
                "toolchain": VERIFICATION_PINS["tools"]["rust"],
                "components": "rustfmt, clippy, llvm-tools-preview",
            },
        )

        python = named_step(self.job, "Setup Python")
        self.assertEqual(python["uses"], PYTHON)
        self.assertEqual(python.get("with"), {"python-version": "3.11"})

        install = named_step(self.job, "Install pinned cargo-llvm-cov")
        self.assertEqual(install["uses"], INSTALL)
        self.assertEqual(
            install.get("with"),
            {"tool": f"cargo-llvm-cov@{VERIFICATION_PINS['tools']['cargo-llvm-cov']}"},
        )

    def test_producer_and_exports_are_exact_and_fail_closed(self) -> None:
        self.assert_run(
            "Generate Windows process coverage profiles",
            """
            python scripts/coverage_windows_process_lanes.py run
            --repo-root .
            --output target/verification/coverage-windows-process-lanes.json
            """,
        )
        self.assert_run(
            "Export Windows LLVM JSON",
            """
            cargo llvm-cov report --json --skip-functions
            --output-path target/coverage-windows-process.json
            """,
        )
        self.assert_run(
            "Export Windows LLVM source view",
            """
            cargo llvm-cov report --text
            --output-path target/coverage-windows-process.txt
            """,
        )
        self.assert_run(
            "Build Windows source-bound physical line map",
            """
            python scripts/llvm_cov_line_map.py
            --repo-root .
            --llvm-json target/coverage-windows-process.json
            --llvm-text target/coverage-windows-process.txt
            --output target/coverage-windows-process-line-map.json
            """,
        )

    def test_artifact_requires_every_attestation_and_derived_view(self) -> None:
        upload = named_step(
            self.job,
            "Upload Windows process coverage artifact",
        )
        self.assertEqual(upload.get("if"), "always()")
        self.assertEqual(upload["uses"], UPLOAD)
        self.assertEqual(
            upload["with"],
            {
                "name": "sorotte-windows-process-llvm-coverage",
                "path": (
                    "target/coverage-windows-process.json\n"
                    "target/coverage-windows-process.txt\n"
                    "target/coverage-windows-process-line-map.json\n"
                    "target/verification/coverage-windows-process-lanes.json\n"
                    "target/verification/coverage-windows-process-logs/\n"
                ),
                "if-no-files-found": "error",
                "retention-days": 14,
                "overwrite": True,
            },
        )


if __name__ == "__main__":
    unittest.main()
