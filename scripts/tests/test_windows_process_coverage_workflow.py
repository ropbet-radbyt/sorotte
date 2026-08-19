from __future__ import annotations

import pathlib
import textwrap
import unittest

import yaml


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "rust-coverage.yml"
CHECKOUT = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
RUST = "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4"
PYTHON = "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97"
INSTALL = "taiki-e/install-action@67729d5c413db75907f0ad1e39bb04b9c868ff60"
UPLOAD = "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"


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
        self.assertEqual(self.job["runs-on"], "windows-latest")
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
                "toolchain": "1.97.1",
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
            {"tool": "cargo-llvm-cov@0.8.4"},
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
