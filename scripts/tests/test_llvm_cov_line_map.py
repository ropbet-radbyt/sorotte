from __future__ import annotations

import contextlib
import copy
import io
import json
import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import llvm_cov_line_map as line_map  # noqa: E402


class LlvmCovLineMapTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name)
        self.source = self.repo / "crates" / "demo" / "src" / "lib.rs"
        self.source.parent.mkdir(parents=True)
        (self.repo / "Cargo.toml").write_text(
            "[workspace]\nmembers = [\"crates/demo\"]\n",
            encoding="utf-8",
        )
        self.source_text = (
            "pub fn value() -> i32 {\n"
            "    let answer = 42;\n"
            "\n"
            "    answer\n"
            "}\n"
        )
        self.source.write_text(self.source_text, encoding="utf-8", newline="\n")
        self.json_path = self.repo / "coverage.json"
        self.text_path = self.repo / "coverage.txt"
        self.output_path = self.repo / "line-map.json"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def summary(line_count: int = 7, line_covered: int = 5) -> dict:
        def metric(
            count: int,
            covered: int,
            *,
            not_covered: bool,
        ) -> dict:
            value = {
                "count": count,
                "covered": covered,
                "percent": 0.0 if count == 0 else covered * 100.0 / count,
            }
            if not_covered:
                value["notcovered"] = count - covered
            return value

        return {
            "branches": metric(0, 0, not_covered=True),
            "functions": metric(1, 1, not_covered=False),
            "instantiations": metric(2, 1, not_covered=False),
            "lines": metric(line_count, line_covered, not_covered=False),
            "mcdc": metric(0, 0, not_covered=True),
            "regions": metric(8, 6, not_covered=True),
        }

    def json_document(self) -> dict:
        return {
            "data": [
                {
                    "files": [
                        {
                            "branches": [],
                            "mcdc_records": [],
                            "expansions": [],
                            "filename": str(self.source.resolve()),
                            "segments": [
                                [1, 1, 1, True, True, False],
                                [4, 5, 0, False, False, False],
                            ],
                            "summary": self.summary(),
                        }
                    ],
                    "totals": self.summary(),
                }
            ],
            "type": line_map.SUPPORTED_LLVM_EXPORT_TYPE,
            "version": line_map.SUPPORTED_LLVM_EXPORT_VERSION,
            "cargo_llvm_cov": {
                "version": line_map.SUPPORTED_CARGO_LLVM_COV_VERSION,
                "manifest_path": str((self.repo / "Cargo.toml").resolve()),
            },
        }

    def native_text(
        self,
        *,
        counts: list[str] | None = None,
        source_lines: list[str] | None = None,
        annotation: bool = True,
        header: pathlib.Path | None = None,
        separator: str = "",
    ) -> str:
        lines = (
            self.source_text.removesuffix("\n").split("\n")
            if source_lines is None
            else source_lines
        )
        tokens = counts or ["1", "0", "", "1.00k", "1"]
        output = [f"{header or self.source}:"]
        for number, (token, source) in enumerate(zip(tokens, lines), start=1):
            output.append(f"{number:5d}|{token:>7}|{source}")
            if annotation and number == 1:
                output.append("                              ^1")
        output.append(separator)
        return "\n".join(output)

    def write_artifacts(
        self,
        *,
        document: dict | None = None,
        native_text: str | None = None,
    ) -> None:
        self.json_path.write_text(
            json.dumps(document or self.json_document()),
            encoding="utf-8",
        )
        self.text_path.write_text(
            self.native_text() if native_text is None else native_text,
            encoding="utf-8",
            newline="\n",
        )

    def build(
        self,
        *,
        document: dict | None = None,
        native_text: str | None = None,
    ) -> dict:
        self.write_artifacts(document=document, native_text=native_text)
        return line_map.build_report(
            repo_root=self.repo,
            llvm_json_path=self.json_path,
            llvm_text_path=self.text_path,
        )

    def assert_invalid(
        self,
        message: str,
        *,
        document: dict | None = None,
        native_text: str | None = None,
    ) -> None:
        with self.assertRaisesRegex(line_map.LlvmCovLineMapError, message):
            self.build(document=document, native_text=native_text)

    def test_valid_artifacts_preserve_both_non_equivalent_line_models(self) -> None:
        report = self.build()

        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["line_model"], "unique-physical-source-lines")
        self.assertEqual(report["summary"]["instrumented_line_count"], 4)
        self.assertEqual(report["summary"]["covered_line_count"], 3)
        self.assertEqual(report["summary"]["physical_line_percent"], "75.000000")
        self.assertEqual(report["summary"]["llvm_summary_line_count"], 7)
        self.assertEqual(report["summary"]["llvm_summary_covered_line_count"], 5)
        self.assertEqual(report["summary"]["llvm_minus_physical_line_count"], 3)
        self.assertEqual(
            report["summary"]["llvm_minus_physical_covered_line_count"],
            2,
        )
        self.assertEqual(report["files"][0]["lines"], [[1, 1], [2, 0], [4, 1], [5, 1]])
        self.assertRegex(
            report["files"][0]["source_sha256"],
            r"^sha256:[0-9a-f]{64}$",
        )
        self.assertRegex(
            report["inputs"]["llvm_json"]["sha256"],
            r"^sha256:[0-9a-f]{64}$",
        )

    def test_cli_writes_deterministic_success_and_machine_readable_error(self) -> None:
        self.write_artifacts()
        first = line_map.main(
            [
                "--repo-root",
                str(self.repo),
                "--llvm-json",
                str(self.json_path),
                "--llvm-text",
                str(self.text_path),
                "--output",
                str(self.output_path),
            ]
        )
        successful = self.output_path.read_bytes()
        second = line_map.main(
            [
                "--repo-root",
                str(self.repo),
                "--llvm-json",
                str(self.json_path),
                "--llvm-text",
                str(self.text_path),
                "--output",
                str(self.output_path),
            ]
        )
        self.assertEqual((first, second), (0, 0))
        self.assertEqual(successful, self.output_path.read_bytes())

        self.source.write_text("pub fn drifted() {}\n", encoding="utf-8")
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            exit_code = line_map.main(
                [
                    "--repo-root",
                    str(self.repo),
                    "--llvm-json",
                    str(self.json_path),
                    "--llvm-text",
                    str(self.text_path),
                    "--output",
                    str(self.output_path),
                ]
            )
        self.assertEqual(exit_code, 2)
        error = json.loads(self.output_path.read_text(encoding="utf-8"))
        self.assertEqual(error["status"], "error")
        self.assertIn("source content disagrees", error["errors"][0])
        self.assertIn("LLVM line-map input error", stderr.getvalue())

    def test_crlf_source_is_normalized_for_rows_but_hashed_as_stored(self) -> None:
        self.source.write_bytes(self.source_text.replace("\n", "\r\n").encode("utf-8"))
        report = self.build()
        self.assertEqual(report["status"], "passed")
        self.assertEqual(
            report["files"][0]["source_sha256"],
            line_map.sha256_bytes(self.source.read_bytes()),
        )

    def test_supported_native_positive_count_abbreviations_are_binary(self) -> None:
        report = self.build(
            native_text=self.native_text(
                counts=["2", "152k", "1.04M", "3G", "9T"],
            )
        )
        self.assertEqual(report["files"][0]["covered_line_count"], 5)
        self.assertEqual(
            report["files"][0]["lines"],
            [[1, 1], [2, 1], [3, 1], [4, 1], [5, 1]],
        )

    def test_unknown_native_count_tokens_fail_closed(self) -> None:
        for token in ("-1", "#####", "NaN", "0.5k", "1.234k", "1e3"):
            with self.subTest(token=token):
                self.assert_invalid(
                    "unsupported execution-count token",
                    native_text=self.native_text(
                        counts=[token, "0", "", "1", "1"],
                    ),
                )

    def test_source_content_line_number_and_truncation_are_bound(self) -> None:
        stale = self.native_text().replace("let answer = 42;", "let answer = 41;")
        self.assert_invalid("source content disagrees", native_text=stale)

        out_of_order = self.native_text().replace("    2|", "    3|", 1)
        self.assert_invalid("expected 2", native_text=out_of_order)

        truncated = "\n".join(self.native_text().splitlines()[:-1])
        self.assert_invalid("truncated", native_text=truncated)

    def test_headers_order_separators_and_unknown_rows_fail_closed(self) -> None:
        external = self.repo.parent / "external.rs"
        external.write_text(self.source_text, encoding="utf-8")
        self.assert_invalid(
            "outside the repository",
            native_text=self.native_text(header=external),
        )
        self.assert_invalid(
            "not source line",
            native_text=self.native_text().replace(
                "    1|      1|",
                "unexpected\n    1|      1|",
                1,
            ),
        )
        self.assert_invalid(
            "unexpected content",
            native_text=self.native_text(separator="\n"),
        )
        self.assert_invalid(
            "not source line",
            native_text=self.native_text().replace(
                f"{self.source}:\n",
                f"{self.source}:\n                              ^1\n",
                1,
            ),
        )

    def test_json_schema_tool_manifest_and_skip_functions_are_pinned(self) -> None:
        mutations = [
            ("export version", ("version",), "99.0.0", "export version"),
            ("export type", ("type",), "other", "type must"),
            (
                "tool version",
                ("cargo_llvm_cov", "version"),
                "0.8.5",
                "cargo-llvm-cov version",
            ),
        ]
        for name, path, replacement, message in mutations:
            with self.subTest(name=name):
                document = self.json_document()
                target = document
                for component in path[:-1]:
                    target = target[component]
                target[path[-1]] = replacement
                self.assert_invalid(message, document=document)

        document = self.json_document()
        document["data"][0]["functions"] = []
        self.assert_invalid("unknown.*functions", document=document)

        external_manifest = self.repo.parent / "Cargo.toml"
        external_manifest.write_text("[workspace]\n", encoding="utf-8")
        document = self.json_document()
        document["cargo_llvm_cov"]["manifest_path"] = str(external_manifest)
        self.assert_invalid("outside the repository", document=document)

    def test_unknown_missing_and_duplicate_json_fields_fail_closed(self) -> None:
        document = self.json_document()
        document["surprise"] = True
        self.assert_invalid("unknown.*surprise", document=document)

        document = self.json_document()
        del document["data"][0]["files"][0]["segments"]
        self.assert_invalid("missing.*segments", document=document)

        self.write_artifacts()
        raw = self.json_path.read_text(encoding="utf-8")
        duplicated = raw.replace(
            '"type": "llvm.coverage.json.export"',
            '"type": "llvm.coverage.json.export", '
            '"type": "llvm.coverage.json.export"',
        )
        self.json_path.write_text(duplicated, encoding="utf-8")
        with self.assertRaisesRegex(
            line_map.LlvmCovLineMapError,
            "duplicates object key",
        ):
            line_map.build_report(
                repo_root=self.repo,
                llvm_json_path=self.json_path,
                llvm_text_path=self.text_path,
            )

    def test_file_paths_and_identities_cannot_escape_or_duplicate(self) -> None:
        external = self.repo / "outside.rs"
        external.write_text(self.source_text, encoding="utf-8")
        document = self.json_document()
        document["data"][0]["files"][0]["filename"] = str(external)
        self.assert_invalid("below crates", document=document)

        document = self.json_document()
        duplicate = copy.deepcopy(document["data"][0]["files"][0])
        document["data"][0]["files"].append(duplicate)
        document["data"][0]["totals"]["lines"]["count"] *= 2
        document["data"][0]["totals"]["lines"]["covered"] *= 2
        document["data"][0]["totals"]["lines"]["percent"] = 5 / 7 * 100
        self.assert_invalid("duplicate source file", document=document)

    def test_segments_are_typed_nonempty_and_ordered(self) -> None:
        document = self.json_document()
        document["data"][0]["files"][0]["segments"] = []
        self.assert_invalid("must not be empty", document=document)

        document = self.json_document()
        document["data"][0]["files"][0]["segments"][0][3] = 1
        self.assert_invalid("must be a boolean", document=document)

        document = self.json_document()
        document["data"][0]["files"][0]["segments"].reverse()
        self.assert_invalid("not ordered", document=document)

        document = self.json_document()
        document["data"][0]["files"][0]["branches"] = [[1, 2, 3]]
        self.assert_invalid("branches must be empty", document=document)

    def test_native_layout_rejects_tabs_outside_source_content(self) -> None:
        tabbed_line_number = self.native_text().replace("    1|", "\t1|", 1)
        self.assert_invalid("not source line", native_text=tabbed_line_number)

        tabbed_annotation = self.native_text().replace(
            "                              ^1",
            "\t^1",
            1,
        )
        self.assert_invalid("not source line", native_text=tabbed_annotation)

    def test_summary_arithmetic_and_file_aggregation_fail_closed(self) -> None:
        document = self.json_document()
        document["data"][0]["totals"]["lines"]["percent"] = 99.0
        self.assert_invalid("percent.*disagrees", document=document)

        document = self.json_document()
        document["data"][0]["files"][0]["summary"]["regions"]["notcovered"] = 99
        self.assert_invalid("do not sum", document=document)

        document = self.json_document()
        document["data"][0]["totals"]["lines"]["covered"] = 4
        document["data"][0]["totals"]["lines"]["percent"] = 4 / 7 * 100
        self.assert_invalid("per-file line summaries", document=document)

    def test_malformed_utf8_nul_and_bare_carriage_return_fail_closed(self) -> None:
        self.write_artifacts()
        self.json_path.write_bytes(b"\xff")
        with self.assertRaisesRegex(line_map.LlvmCovLineMapError, "valid UTF-8"):
            line_map.build_report(
                repo_root=self.repo,
                llvm_json_path=self.json_path,
                llvm_text_path=self.text_path,
            )

        self.write_artifacts(native_text=self.native_text() + "\x00")
        with self.assertRaisesRegex(line_map.LlvmCovLineMapError, "NUL"):
            line_map.build_report(
                repo_root=self.repo,
                llvm_json_path=self.json_path,
                llvm_text_path=self.text_path,
            )

        self.write_artifacts(native_text=self.native_text().replace("\n", "\r", 1))
        with self.assertRaisesRegex(
            line_map.LlvmCovLineMapError,
            "bare carriage return",
        ):
            line_map.build_report(
                repo_root=self.repo,
                llvm_json_path=self.json_path,
                llvm_text_path=self.text_path,
            )


if __name__ == "__main__":
    unittest.main()
