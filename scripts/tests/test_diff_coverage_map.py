from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import diff_coverage as coverage  # noqa: E402


class DiffCoverageMapTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name)
        self.source = self.repo / "crates" / "demo" / "src" / "lib.rs"
        self.source.parent.mkdir(parents=True)
        self.source_text = (
            "pub fn answer() -> u32 {\n"
            "    let value = 42;\n"
            "\n"
            "    value\n"
            "}\n"
        )
        self.source.write_text(self.source_text, encoding="utf-8", newline="\n")
        critical = self.repo / "crates" / "critical" / "src"
        critical.mkdir(parents=True)
        (critical / "lib.rs").write_text(
            "pub fn critical_placeholder() {}\n",
            encoding="utf-8",
        )
        policy = self.repo / coverage.DEFAULT_CRITICAL_POLICY_PATH
        policy.parent.mkdir(parents=True)
        policy.write_text(
            "schema_version = 1\n"
            "\n"
            "[policy]\n"
            'critical_minimum_percent = "90.00"\n'
            "\n"
            "[[critical_path]]\n"
            'id = "critical-placeholder"\n'
            'category = "lifecycle"\n'
            'owner = "runtime"\n'
            'match = "file"\n'
            'path = "crates/critical/src/lib.rs"\n',
            encoding="utf-8",
        )
        self.map_path = self.repo / "coverage-map.json"
        self.diff_path = self.repo / "change.diff"
        self.diff_path.write_text(self.patch(), encoding="utf-8")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def patch(self) -> str:
        additions = "".join(
            f"+{line}\n" for line in self.source_text.removesuffix("\n").split("\n")
        )
        return (
            "diff --git a/crates/demo/src/lib.rs b/crates/demo/src/lib.rs\n"
            "new file mode 100644\n"
            "--- /dev/null\n"
            "+++ b/crates/demo/src/lib.rs\n"
            "@@ -0,0 +1,5 @@\n"
            f"{additions}"
        )

    @staticmethod
    def percent(covered: int, count: int) -> str | None:
        return coverage.coverage_percent_text(covered, count)

    def map_document(
        self,
        *,
        lines: list[list[int]] | None = None,
        llvm_count: int = 8,
        llvm_covered: int = 6,
    ) -> dict:
        mapped = lines or [[1, 1], [2, 1], [4, 0], [5, 1]]
        instrumented = len(mapped)
        covered = sum(row[1] for row in mapped)
        source_raw = self.source.read_bytes()
        return {
            "schema_version": coverage.LLVM_LINE_MAP_SCHEMA_VERSION,
            "kind": coverage.LLVM_LINE_MAP_KIND,
            "status": "passed",
            "line_model": coverage.LLVM_LINE_MODEL,
            "inputs": {
                "llvm_json": {
                    "path": "target/diff-coverage.json",
                    "size_bytes": 100,
                    "sha256": "sha256:" + "1" * 64,
                },
                "llvm_text": {
                    "path": "target/diff-coverage.txt",
                    "size_bytes": 200,
                    "sha256": "sha256:" + "2" * 64,
                },
            },
            "producer": {
                "llvm_export_type": coverage.LLVM_EXPORT_TYPE,
                "llvm_export_version": coverage.LLVM_EXPORT_VERSION,
                "cargo_llvm_cov_version": coverage.CARGO_LLVM_COV_VERSION,
                "manifest_path": "Cargo.toml",
            },
            "summary": {
                "file_count": 1,
                "source_line_count": 5,
                "instrumented_line_count": instrumented,
                "covered_line_count": covered,
                "uncovered_line_count": instrumented - covered,
                "physical_line_percent": self.percent(covered, instrumented),
                "llvm_summary_line_count": llvm_count,
                "llvm_summary_covered_line_count": llvm_covered,
                "llvm_summary_line_percent": self.percent(
                    llvm_covered,
                    llvm_count,
                ),
                "llvm_minus_physical_line_count": llvm_count - instrumented,
                "llvm_minus_physical_covered_line_count": llvm_covered - covered,
            },
            "files": [
                {
                    "path": "crates/demo/src/lib.rs",
                    "source_sha256": coverage.sha256_bytes(source_raw),
                    "source_line_count": 5,
                    "instrumented_line_count": instrumented,
                    "covered_line_count": covered,
                    "lines": mapped,
                }
            ],
            "errors": [],
        }

    def write_map(self, document: dict | None = None) -> pathlib.Path:
        self.map_path.write_text(
            json.dumps(document or self.map_document()),
            encoding="utf-8",
        )
        return self.map_path

    def build(
        self,
        *,
        document: dict | None = None,
        minimum: str = "50",
    ) -> dict:
        self.write_map(document)
        return coverage.build_report(
            repo_root=self.repo,
            lcov_path=None,
            coverage_map_path=self.map_path,
            diff_path=self.diff_path,
            base=None,
            head=None,
            minimum_text=minimum,
        )

    def assert_invalid(self, document: dict, message: str) -> None:
        with self.assertRaisesRegex(coverage.DiffCoverageError, message):
            self.build(document=document)

    def test_canonical_map_drives_changed_physical_line_policy(self) -> None:
        report = self.build()

        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["coverable_lines"], 4)
        self.assertEqual(report["summary"]["covered_lines"], 3)
        self.assertEqual(report["summary"]["non_coverable_lines"], 1)
        self.assertEqual(report["summary"]["percent"], "75.00")
        self.assertEqual(
            report["inputs"]["coverage_kind"],
            "llvm-physical-line-map",
        )
        self.assertEqual(
            report["inputs"]["coverage_line_model"],
            "unique-physical-source-lines",
        )
        self.assertRegex(
            report["inputs"]["coverage_map_sha256"],
            r"^sha256:[0-9a-f]{64}$",
        )
        self.assertEqual(
            report["inputs"]["coverage_producer"]["cargo_llvm_cov_version"],
            "0.8.4",
        )

    def test_platform_maps_union_physical_lines_without_double_counting(self) -> None:
        linux_path = self.repo / "coverage-linux.json"
        windows_path = self.repo / "coverage-windows.json"
        linux_path.write_text(
            json.dumps(
                self.map_document(lines=[[1, 1], [2, 0], [4, 0], [5, 1]])
            ),
            encoding="utf-8",
        )
        windows_path.write_text(
            json.dumps(
                self.map_document(lines=[[1, 0], [2, 1], [4, 1], [5, 0]])
            ),
            encoding="utf-8",
        )

        report = coverage.build_report(
            repo_root=self.repo,
            lcov_path=None,
            coverage_map_paths=[linux_path, windows_path],
            diff_path=self.diff_path,
            base=None,
            head=None,
            minimum_text="100",
        )

        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["coverable_lines"], 4)
        self.assertEqual(report["summary"]["covered_lines"], 4)
        self.assertEqual(
            report["inputs"]["coverage_kind"],
            "llvm-physical-line-map-union",
        )
        self.assertEqual(len(report["inputs"]["coverage_maps"]), 2)
        self.assertEqual(
            [item["path"] for item in report["inputs"]["coverage_maps"]],
            [str(linux_path), str(windows_path)],
        )

    def test_platform_map_union_rejects_duplicate_content(self) -> None:
        first = self.repo / "coverage-first.json"
        second = self.repo / "coverage-second.json"
        payload = json.dumps(self.map_document())
        first.write_text(payload, encoding="utf-8")
        second.write_text(payload, encoding="utf-8")

        with self.assertRaisesRegex(
            coverage.DiffCoverageError,
            "duplicate content",
        ):
            coverage.build_report(
                repo_root=self.repo,
                lcov_path=None,
                coverage_map_paths=[first, second],
                diff_path=self.diff_path,
                base=None,
                head=None,
                minimum_text="50",
            )

    def test_canonical_uncovered_and_unmapped_lines_fail_policy_not_input(self) -> None:
        uncovered = self.build(minimum="80")
        self.assertEqual(uncovered["status"], "failed")
        self.assertEqual(uncovered["summary"]["uncovered_lines"], 1)

        document = self.map_document(lines=[[1, 1], [4, 1], [5, 1]])
        unmapped = self.build(document=document, minimum="50")
        self.assertEqual(unmapped["status"], "failed")
        line = unmapped["files"][0]["lines"][1]
        self.assertEqual(line["status"], "unmapped")
        self.assertEqual(
            line["reason"],
            "executable-looking-line-absent-from-canonical-coverage-map",
        )

    def test_map_is_rebound_to_every_current_source_digest_and_line_count(self) -> None:
        document = self.map_document()
        self.source.write_text(
            self.source_text.replace("42", "43"),
            encoding="utf-8",
        )
        self.assert_invalid(document, "source digest is stale")

        self.source.write_text(self.source_text, encoding="utf-8", newline="\n")
        document = self.map_document()
        document["files"][0]["source_line_count"] = 6
        document["summary"]["source_line_count"] = 6
        self.assert_invalid(document, "physical source line count is stale")

    def test_map_schema_kind_status_line_model_and_producer_are_strict(self) -> None:
        mutations = [
            ("schema_version", 2, "unsupported schema"),
            ("kind", "other", "unsupported kind"),
            ("line_model", "instances", "unsupported line model"),
        ]
        for field, value, message in mutations:
            with self.subTest(field=field):
                document = self.map_document()
                document[field] = value
                self.assert_invalid(document, message)

        document = self.map_document()
        document["status"] = "error"
        document["errors"] = ["producer failed"]
        self.assert_invalid(document, "producer failed")

        document = self.map_document()
        document["producer"]["cargo_llvm_cov_version"] = "latest"
        self.assert_invalid(document, "pinned tool contract")

    def test_map_unknown_fields_artifact_metadata_and_duplicate_keys_fail(self) -> None:
        document = self.map_document()
        document["unknown"] = True
        self.assert_invalid(document, "unknown.*unknown")

        document = self.map_document()
        document["inputs"]["llvm_json"]["sha256"] = "1" * 64
        self.assert_invalid(document, "lowercase SHA-256")

        self.write_map()
        raw = self.map_path.read_text(encoding="utf-8")
        raw = raw.replace(
            '"kind": "sorotte-llvm-line-map"',
            '"kind": "sorotte-llvm-line-map", '
            '"kind": "sorotte-llvm-line-map"',
        )
        self.map_path.write_text(raw, encoding="utf-8")
        with self.assertRaisesRegex(coverage.DiffCoverageError, "duplicate_key"):
            coverage.build_report(
                repo_root=self.repo,
                lcov_path=None,
                coverage_map_path=self.map_path,
                diff_path=self.diff_path,
                base=None,
                head=None,
                minimum_text="50",
            )

    def test_map_file_rows_are_binary_sorted_unique_and_in_range(self) -> None:
        mutations = [
            ([[2, 1], [1, 1], [4, 0], [5, 1]], "out of order"),
            ([[1, 1], [1, 1], [4, 0], [5, 1]], "duplicate"),
            ([[1, 2], [2, 1], [4, 0], [5, 1]], "binary execution"),
            ([[1, 1], [2, 1], [4, 0], [6, 1]], "out of range"),
        ]
        for rows, message in mutations:
            with self.subTest(rows=rows):
                document = self.map_document()
                document["files"][0]["lines"] = rows
                self.assert_invalid(document, message)

    def test_map_file_and_global_arithmetic_fail_closed(self) -> None:
        document = self.map_document()
        document["files"][0]["covered_line_count"] = 4
        self.assert_invalid(document, "covered line count is inconsistent")

        document = self.map_document()
        document["summary"]["uncovered_line_count"] = 99
        self.assert_invalid(document, "uncovered line count is inconsistent")

        document = self.map_document()
        document["summary"]["physical_line_percent"] = "75"
        self.assert_invalid(document, "canonical percentage")

        document = self.map_document()
        document["summary"]["llvm_minus_physical_line_count"] = 0
        self.assert_invalid(document, "delta is inconsistent")

    def test_exactly_one_coverage_input_is_required(self) -> None:
        self.write_map()
        with self.assertRaisesRegex(coverage.DiffCoverageError, "exactly one"):
            coverage.build_report(
                repo_root=self.repo,
                lcov_path=None,
                coverage_map_path=None,
                diff_path=self.diff_path,
                base=None,
                head=None,
                minimum_text="50",
            )

        lcov = self.repo / "coverage.info"
        lcov.write_text("TN:\nend_of_record\n", encoding="utf-8")
        with self.assertRaisesRegex(coverage.DiffCoverageError, "exactly one"):
            coverage.build_report(
                repo_root=self.repo,
                lcov_path=lcov,
                coverage_map_path=self.map_path,
                diff_path=self.diff_path,
                base=None,
                head=None,
                minimum_text="50",
            )

    def test_cli_accepts_canonical_map_and_writes_source_bound_report(self) -> None:
        self.write_map()
        output = self.repo / "report.json"
        exit_code = coverage.main(
            [
                "--repo-root",
                str(self.repo),
                "--coverage-map",
                str(self.map_path),
                "--diff",
                str(self.diff_path),
                "--minimum",
                "50",
                "--json-out",
                str(output),
            ]
        )
        self.assertEqual(exit_code, 0)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["inputs"]["coverage_kind"], "llvm-physical-line-map")


if __name__ == "__main__":
    unittest.main()
