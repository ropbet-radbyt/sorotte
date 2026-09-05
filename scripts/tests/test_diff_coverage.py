from __future__ import annotations

import contextlib
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import diff_coverage as coverage  # noqa: E402


class DiffCoverageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.repo = pathlib.Path(self.temporary.name)
        (self.repo / "src").mkdir()
        self.source = self.repo / "src" / "lib.rs"
        self.source.write_text(
            "pub fn answer(flag: bool) -> u32 {\n"
            "    if flag {\n"
            "        42\n"
            "    } else {\n"
            "        0\n"
            "    }\n"
            "}\n",
            encoding="utf-8",
        )
        self.critical_dir = self.repo / "crates" / "critical" / "src"
        self.critical_dir.mkdir(parents=True)
        (self.critical_dir / "placeholder.rs").write_text(
            "pub fn placeholder() {}\n",
            encoding="utf-8",
        )
        self.policy_path = (
            self.repo / coverage.DEFAULT_CRITICAL_POLICY_PATH
        )
        self.write_policy()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write_policy(
        self,
        *,
        minimum: str = "90.00",
        rules: list[dict[str, str]] | None = None,
    ) -> None:
        configured = rules or [
            {
                "id": "critical-core",
                "category": "lifecycle",
                "owner": "runtime",
                "match": "directory",
                "path": "crates/critical/src/",
            }
        ]
        lines = [
            "schema_version = 1",
            "",
            "[policy]",
            f'critical_minimum_percent = "{minimum}"',
        ]
        for rule in configured:
            lines.extend(
                [
                    "",
                    "[[critical_path]]",
                    f'id = "{rule["id"]}"',
                    f'category = "{rule["category"]}"',
                    f'owner = "{rule["owner"]}"',
                    f'match = "{rule["match"]}"',
                    f'path = "{rule["path"]}"',
                ]
            )
        self.policy_path.parent.mkdir(parents=True, exist_ok=True)
        self.policy_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def lcov(
        self,
        lines: dict[int, int] | None = None,
        *,
        source: str = "src/lib.rs",
    ) -> str:
        line_hits = lines if lines is not None else {1: 1, 2: 1, 3: 1, 5: 0}
        directives = [f"SF:{source}"]
        directives.extend(f"DA:{line},{hits}" for line, hits in line_hits.items())
        directives.extend(
            [
                f"LF:{len(line_hits)}",
                f"LH:{sum(hits > 0 for hits in line_hits.values())}",
                "end_of_record",
            ]
        )
        return "\n".join(directives) + "\n"

    def patch(
        self,
        *,
        old_path: str = "src/lib.rs",
        new_path: str = "src/lib.rs",
        old_range: str = "1",
        new_range: str = "1",
        body: list[str] | None = None,
        metadata: list[str] | None = None,
    ) -> str:
        body = body if body is not None else ["-pub fn old() {}", "+pub fn answer(flag: bool) -> u32 {"]
        lines = [
            f"diff --git a/{old_path} b/{new_path}",
            *(metadata or []),
            f"--- a/{old_path}",
            f"+++ b/{new_path}",
            f"@@ -{old_range} +{new_range} @@",
            *body,
        ]
        return "\n".join(lines) + "\n"

    def new_file_patch(self, path: str, lines: list[str]) -> str:
        count = len(lines)
        return (
            f"diff --git a/{path} b/{path}\n"
            "new file mode 100644\n"
            "--- /dev/null\n"
            f"+++ b/{path}\n"
            f"@@ -0,0 +1,{count} @@\n"
            + "".join(f"+{line}\n" for line in lines)
        )

    def write_inputs(self, lcov: str, diff: str) -> tuple[pathlib.Path, pathlib.Path]:
        lcov_path = self.repo / "coverage.info"
        diff_path = self.repo / "changes.diff"
        lcov_path.write_text(lcov, encoding="utf-8")
        diff_path.write_text(diff, encoding="utf-8")
        return lcov_path, diff_path

    def build(
        self,
        lcov: str,
        diff: str,
        *,
        minimum: str = "100",
    ) -> dict:
        lcov_path, diff_path = self.write_inputs(lcov, diff)
        return coverage.build_report(
            repo_root=self.repo,
            lcov_path=lcov_path,
            diff_path=diff_path,
            base=None,
            head=None,
            minimum_text=minimum,
        )

    def assert_input_error(self, lcov: str, diff: str, message: str) -> None:
        with self.assertRaisesRegex(coverage.DiffCoverageError, message):
            self.build(lcov, diff)

    def test_covered_line_passes_at_default_policy(self) -> None:
        report = self.build(self.lcov(), self.patch())
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["covered_lines"], 1)
        self.assertEqual(report["summary"]["percent"], "100.00")
        self.assertEqual(
            report["inputs"]["coverage_line_model"],
            coverage.LCOV_LINE_MODEL,
        )
        self.assertEqual(
            report["inputs"]["lcov_summary_audit"]["status"],
            "consistent",
        )
        self.assertRegex(report["inputs"]["lcov_sha256"], r"^sha256:[0-9a-f]{64}$")
        self.assertRegex(report["inputs"]["diff_sha256"], r"^sha256:[0-9a-f]{64}$")
        self.assertEqual(
            report["inputs"]["critical_path_policy"],
            coverage.DEFAULT_CRITICAL_POLICY_PATH,
        )
        self.assertRegex(
            report["inputs"]["critical_path_policy_sha256"],
            r"^sha256:[0-9a-f]{64}$",
        )
        self.assertEqual(report["coverage_classes"]["ordinary"]["status"], "passed")
        self.assertEqual(
            report["coverage_classes"]["critical"]["status"],
            "not-applicable",
        )

    def test_uncovered_line_fails_threshold_without_becoming_input_error(self) -> None:
        report = self.build(
            self.lcov(),
            self.patch(
                old_range="5",
                new_range="5",
                body=["-        other", "+        0"],
            ),
        )
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["uncovered_lines"], 1)
        self.assertEqual(report["summary"]["percent"], "0.00")
        self.assertIn("below required", report["errors"][0])

    def test_exact_decimal_threshold_is_not_float_rounded_up(self) -> None:
        self.source.write_text("a();\nb();\nc();\n", encoding="utf-8")
        diff = self.patch(
            old_range="1,0",
            new_range="1,3",
            body=["+a();", "+b();", "+c();"],
        )
        passed = self.build(self.lcov({1: 1, 2: 1, 3: 0}), diff, minimum="66.66")
        failed = self.build(self.lcov({1: 1, 2: 1, 3: 0}), diff, minimum="66.67")
        self.assertEqual(passed["status"], "passed")
        self.assertEqual(failed["status"], "failed")
        self.assertEqual(passed["summary"]["percent"], "66.66")

    def test_line_absent_from_present_lcov_source_is_non_coverable(self) -> None:
        report = self.build(
            self.lcov(),
            self.patch(
                old_range="7",
                new_range="7",
                body=["-}", "+}"],
            ),
        )
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["coverable_lines"], 0)
        self.assertEqual(report["summary"]["non_coverable_lines"], 1)
        self.assertIsNone(report["summary"]["percent"])
        self.assertEqual(
            report["files"][0]["lines"][0]["reason"],
            "lexical-structure-absent-from-lcov-unique-da-map",
        )

    def test_platform_gated_statement_missing_from_mapped_file_fails_closed(self) -> None:
        self.source.write_text(
            "pub fn answer(flag: bool) -> u32 {\n"
            "    if flag {\n"
            "        42\n"
            "    } else {\n"
            "        0\n"
            "    }\n"
            "}\n"
            "\n"
            "#[cfg(windows)]\n"
            "pub fn windows_only() {\n"
            "    launch_windows_player();\n"
            "}\n",
            encoding="utf-8",
        )
        diff = (
            "diff --git a/src/lib.rs b/src/lib.rs\n"
            "--- a/src/lib.rs\n"
            "+++ b/src/lib.rs\n"
            "@@ -8,0 +9,4 @@\n"
            "+#[cfg(windows)]\n"
            "+pub fn windows_only() {\n"
            "+    launch_windows_player();\n"
            "+}\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["unmapped_lines"], 1)
        self.assertEqual(report["summary"]["non_coverable_lines"], 3)
        lines = report["files"][0]["lines"]
        self.assertEqual(lines[0]["status"], "non-coverable")
        self.assertEqual(lines[1]["status"], "non-coverable")
        self.assertEqual(lines[2]["status"], "unmapped")
        self.assertEqual(
            lines[2]["reason"],
            "executable-looking-line-absent-from-lcov-unique-da-map",
        )
        self.assertEqual(lines[3]["status"], "non-coverable")

    def test_multiline_attributes_imports_and_signatures_are_structural(self) -> None:
        self.source.write_text(
            "#[cfg(any(\n"
            "    windows,\n"
            "    target_os = \"linux\",\n"
            "))]\n"
            "use crate::{\n"
            "    Alpha,\n"
            "    Beta,\n"
            "};\n"
            "pub fn declared_only(\n"
            "    value: bool,\n"
            ") -> bool;\n",
            encoding="utf-8",
        )
        source_lines = self.source.read_text(encoding="utf-8").splitlines()
        structural = coverage.lexical_non_coverable_lines(source_lines)
        self.assertEqual(structural, set(range(1, 12)))

    def test_attribute_sharing_a_line_with_behavior_is_not_exempt(self) -> None:
        source_lines = [
            "#[cfg(windows)] launch_windows_player();",
            "#[cfg(any(",
            "    windows,",
            "))] launch_other_player();",
        ]
        structural = coverage.lexical_non_coverable_lines(source_lines)
        self.assertEqual(structural, {2, 3})

    def test_inline_cfg_test_lines_cannot_dilute_production_coverage(self) -> None:
        self.source.write_text(
            "pub fn product() {\n"
            "    production_side_effect();\n"
            "}\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    #[test]\n"
            "    fn covers_new_behavior() {\n"
            "        test_setup();\n"
            "        test_action_one();\n"
            "        test_action_two();\n"
            "        test_assertion();\n"
            "    }\n"
            "}\n",
            encoding="utf-8",
        )
        diff = (
            "diff --git a/src/lib.rs b/src/lib.rs\n"
            "--- a/src/lib.rs\n"
            "+++ b/src/lib.rs\n"
            "@@ -1,0 +2 @@\n"
            "+    production_side_effect();\n"
            "@@ -7,0 +9,4 @@\n"
            "+        test_setup();\n"
            "+        test_action_one();\n"
            "+        test_action_two();\n"
            "+        test_assertion();\n"
        )
        report = self.build(
            self.lcov({2: 0, 9: 1, 10: 1, 11: 1, 12: 1}),
            diff,
            minimum="80",
        )

        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["changed_rust_lines"], 5)
        self.assertEqual(report["summary"]["production_changed_lines"], 1)
        self.assertEqual(report["summary"]["inline_test_lines"], 4)
        self.assertEqual(report["summary"]["inline_test_files"], 1)
        self.assertEqual(report["summary"]["coverable_lines"], 1)
        self.assertEqual(report["summary"]["covered_lines"], 0)
        self.assertEqual(report["summary"]["uncovered_lines"], 1)
        self.assertEqual(report["summary"]["percent"], "0.00")
        self.assertEqual(report["coverage_classes"]["ordinary"]["status"], "failed")
        self.assertEqual(
            report["coverage_classes"]["ordinary"]["summary"]["inline_test_lines"],
            4,
        )
        self.assertEqual(report["files"][0]["summary"]["production_changed"], 1)
        self.assertEqual(report["files"][0]["summary"]["inline_test"], 4)
        self.assertEqual(
            [line["status"] for line in report["files"][0]["lines"]],
            ["uncovered"] + ["excluded-inline-test"] * 4,
        )
        self.assertTrue(
            all(
                line.get("reason") == "cfg-test-support-inline-item"
                for line in report["files"][0]["lines"][1:]
            )
        )

    def test_inline_cfg_test_scanner_ignores_braces_in_non_code_tokens(self) -> None:
        source_lines = [
            "pub fn production() {}",
            "#[cfg(test)]",
            "#[allow(dead_code)]",
            "pub(crate) mod tests {",
            '    const ORDINARY: &str = "{ not a brace }";',
            '    const RAW: &str = r###"} /* { nested */"###;',
            '    const BYTE_RAW: &[u8] = br##"{ }"##;',
            '    const C_STRING: &CStr = c"} {";',
            "    // } line-comment brace",
            "    /* { outer /* } nested */ } */",
            "    fn outer() {",
            "        let value = || { if true { 1 } else { 2 } };",
            "        let closing = '}';",
            "    }",
            "    mod nested {",
            "        fn inner() {",
            '            assert_eq!("{", "{");',
            "        }",
            "    }",
            "}",
            "pub fn after() {}",
        ]

        inline_test = coverage.inline_cfg_test_module_lines(
            source_lines,
            source="src/lib.rs",
        )

        after_line = source_lines.index("pub fn after() {}") + 1
        self.assertEqual(inline_test, set(range(2, after_line)))

    def test_inline_cfg_test_scanner_accepts_multiline_attribute_and_body(self) -> None:
        source_lines = [
            "#[cfg(",
            "    test",
            ")]",
            "#[allow(dead_code)]",
            "mod tests",
            "{",
            "    mod nested {",
            "        fn test_behavior() {}",
            "    }",
            "}",
        ]

        inline_test = coverage.inline_cfg_test_module_lines(
            source_lines,
            source="src/lib.rs",
        )

        self.assertEqual(inline_test, set(range(1, len(source_lines) + 1)))

    def test_inline_cfg_test_scanner_ignores_cfg_text_in_non_code_tokens(self) -> None:
        source_lines = [
            'const ORDINARY: &str = "#[cfg(test)] mod tests {";',
            'const RAW: &str = r##"#[cfg(test)] mod tests { }"##;',
            "// #[cfg(test)] mod tests {",
            "/* #[cfg(test)] mod tests { } */",
        ]

        inline_test = coverage.inline_cfg_test_module_lines(
            source_lines,
            source="src/lib.rs",
        )

        self.assertEqual(inline_test, set())

    def test_inline_cfg_test_scanner_fails_closed_on_ambiguous_module(self) -> None:
        with self.assertRaisesRegex(
            coverage.DiffCoverageError,
            r"src/lib\.rs:1 has an ambiguous inline #\[cfg\(test\)\] module",
        ):
            coverage.inline_cfg_test_module_lines(
                ["#[cfg(test)]", "mod tests = generated!();"],
                source="src/lib.rs",
            )

    def test_inline_cfg_test_scanner_fails_closed_on_unclosed_module(self) -> None:
        with self.assertRaisesRegex(
            coverage.DiffCoverageError,
            r"src/lib\.rs:1 has an unclosed inline test-support item",
        ):
            coverage.inline_cfg_test_module_lines(
                ["#[cfg(test)]", "mod tests {", "    fn behavior() {}"],
                source="src/lib.rs",
            )

    def test_changed_production_file_fails_closed_on_unsafe_inline_test_syntax(
        self,
    ) -> None:
        cases = {
            "ambiguous": (
                ["#[cfg(test)]", "mod tests = generated!();"],
                r"ambiguous inline #\[cfg\(test\)\] module",
            ),
            "unclosed": (
                ["#[cfg(test)]", "mod tests {", "    fn behavior() {}"],
                r"unclosed inline test-support item",
            ),
        }
        for name, (source_lines, message) in cases.items():
            with self.subTest(name=name):
                self.source.write_text(
                    "\n".join(source_lines) + "\n",
                    encoding="utf-8",
                )
                diff = self.patch(
                    old_range="1,0",
                    new_range=f"1,{len(source_lines)}",
                    body=[f"+{line}" for line in source_lines],
                )
                with self.assertRaisesRegex(coverage.DiffCoverageError, message):
                    self.build(self.lcov({len(source_lines): 1}), diff)

    def test_external_cfg_test_module_is_not_excluded(self) -> None:
        inline_test = coverage.inline_cfg_test_module_lines(
            ["#[cfg(test)]", "mod tests;"],
            source="src/lib.rs",
        )

        self.assertEqual(inline_test, set())

    def test_complete_inline_test_support_items_are_excluded_without_platform_code(self) -> None:
        source_lines = [
            "struct ProductState {",
            "    #[cfg(test)]",
            "    observer: Option<bool>,",
            "    live: bool,",
            "}",
            "#[cfg(feature = \"test-support\")]",
            "fn test_helper() {",
            "    observe_test();",
            "}",
            "#[cfg(feature = \"fuzz-support\")]",
            "const FUZZ_LIMIT: usize = 4;",
            "#[cfg(all(test, windows))]",
            "mod windows_tests {",
            "    fn process_fixture() {}",
            "}",
            "#[cfg(any(test, feature = \"gui-semantic-smoke\"))]",
            "pub use crate::semantic::Scenario;",
            "#[cfg(windows)]",
            "fn production_windows() {",
            "    launch_windows_player();",
            "}",
        ]

        inline_test = coverage.inline_cfg_test_module_lines(
            source_lines,
            source="src/lib.rs",
        )

        self.assertEqual(
            inline_test,
            set(range(2, 4))
            | set(range(6, 10))
            | set(range(10, 12))
            | set(range(12, 16))
            | set(range(16, 18)),
        )
        self.assertTrue(set(range(18, 22)).isdisjoint(inline_test))

    def test_compile_time_items_literals_and_pattern_headers_are_structural(self) -> None:
        source_lines = [
            "pub mod participant_status {",
            "mod external_module;",
            "enum Mode {",
            "    First,",
            "    Second(u8),",
            "}",
            "struct Snapshot {",
            "    value: usize,",
            "}",
            "const LIMIT: usize =",
            "    64 * 1024;",
            "type Callback =",
            "    Arc<dyn Fn() + Send>;",
            '    "format-only argument",',
            "    callback,",
            "    0,",
            "    loop {",
            "        Some(_) => {",
            "        if let ConnectionScopedState {",
            "        } else {",
            "        ..",
        ]

        structural = coverage.lexical_non_coverable_lines(source_lines)

        self.assertEqual(structural, set(range(1, len(source_lines) + 1)))

    def test_compiler_uninstrumented_struct_glue_is_structural(self) -> None:
        source_lines = [
            "    event: ConnectedSessionEventPlan {",
            "    crate::app::PlaybackControlObservation {",
            "        enabled: false,",
            "        epoch: 0,",
            "    } else if let (",
            "        PlaybackDiagnostic::Empty | PlaybackDiagnostic::Ended",
            "        enabled: compute_enabled(),",
            "        launch_windows_player();",
            "    true",
        ]

        structural = coverage.lexical_non_coverable_lines(source_lines)

        self.assertEqual(structural, {1, 2, 3, 4, 5, 6})

    def test_multiline_expression_glue_is_structural_without_hiding_complete_calls(self) -> None:
        source_lines = [
            "    invoke(",
            "        argument",
            "    );",
            "    previous =",
            "        current;",
            "    Ok(",
            "        Resolution::Pending",
            "            | Resolution::Missing,",
            "    ) => {",
            "    return Err(InteropError::Invalid(",
            "        detail,",
            "    ));",
            "    final_value",
            "    do_work();",
        ]

        structural = coverage.lexical_non_coverable_lines(source_lines)

        self.assertTrue({3, 4, 6, 7, 8, 9, 10, 12}.issubset(structural))
        self.assertNotIn(2, structural)
        self.assertNotIn(13, structural)
        self.assertNotIn(14, structural)

    def test_compiler_uninstrumented_path_chain_pattern_and_literal_glue_is_structural(
        self,
    ) -> None:
        source_lines = [
            "    format!(",
            '        "{}",',
            "        sorotte_lifecycle_evidence::EVIDENCE_PATH_ENV",
            "    );",
            "    self.inner",
            "        .lock()",
            "        .writer",
            "        .flush()",
            "    match phase {",
            "        Phase::Waiting { generation }",
            "        | Phase::Ready {",
            "            generation, ..",
            "        }",
            "        | Phase::Degraded { timed_out: false, .. }",
            "        => generation,",
            "    }",
            "    Step {",
            '        transition: "GATE-DEGRADE-001",',
            '        authority_after: "degraded",',
            "        computed: compute_authority(),",
            "    }",
        ]

        structural = coverage.lexical_non_coverable_lines(source_lines)

        self.assertTrue({3, 7, 11, 14, 18, 19}.issubset(structural))
        self.assertNotIn(6, structural)
        self.assertNotIn(8, structural)
        self.assertNotIn(20, structural)

    def test_structural_continuations_do_not_exempt_executable_lookalikes(self) -> None:
        source_lines = [
            "    dangerous::perform();",
            "    dangerous::STATE",
            "    .flush()",
            "    transition: compute_transition(),",
            "    | Phase::Ready { value } if allowed(value) => use_value(value),",
            "    invoke(",
            "        Phase::Ready { value: compute_value() }",
            "    );",
        ]

        structural = coverage.lexical_non_coverable_lines(source_lines)

        self.assertTrue(set(range(1, 6)).isdisjoint(structural))
        self.assertNotIn(7, structural)

    def test_wholly_unmapped_executable_new_file_fails_closed(self) -> None:
        new_source = self.repo / "src" / "new.rs"
        new_source.write_text("pub fn new_behavior() -> bool {\n    true\n}\n", encoding="utf-8")
        diff = (
            "diff --git a/src/new.rs b/src/new.rs\n"
            "new file mode 100644\n"
            "--- /dev/null\n"
            "+++ b/src/new.rs\n"
            "@@ -0,0 +1,3 @@\n"
            "+pub fn new_behavior() -> bool {\n"
            "+    true\n"
            "+}\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["unmapped_lines"], 1)
        self.assertEqual(report["summary"]["non_coverable_lines"], 2)

    def test_comment_only_new_file_without_lcov_is_non_coverable(self) -> None:
        new_source = self.repo / "src" / "notes.rs"
        new_source.write_text("// documentation only\n\n", encoding="utf-8")
        diff = (
            "diff --git a/src/notes.rs b/src/notes.rs\n"
            "new file mode 100644\n"
            "--- /dev/null\n"
            "+++ b/src/notes.rs\n"
            "@@ -0,0 +1,2 @@\n"
            "+// documentation only\n"
            "+\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["non_coverable_lines"], 2)
        self.assertEqual(report["summary"]["unmapped_lines"], 0)

    def test_renamed_file_uses_new_path_coverage_and_source(self) -> None:
        renamed = self.repo / "src" / "renamed.rs"
        renamed.write_text(self.source.read_text(encoding="utf-8"), encoding="utf-8")
        diff = self.patch(
            old_path="src/old.rs",
            new_path="src/renamed.rs",
            metadata=[
                "similarity index 90%",
                "rename from src/old.rs",
                "rename to src/renamed.rs",
            ],
        )
        report = self.build(self.lcov(source="src/renamed.rs"), diff)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["files"][0]["change_kind"], "renamed")
        self.assertEqual(report["files"][0]["old_path"], "src/old.rs")

    def test_pure_rename_without_content_headers_is_supported(self) -> None:
        renamed = self.repo / "src" / "renamed.rs"
        renamed.write_text(self.source.read_text(encoding="utf-8"), encoding="utf-8")
        diff = (
            "diff --git a/src/lib.rs b/src/renamed.rs\n"
            "similarity index 100%\n"
            "rename from src/lib.rs\n"
            "rename to src/renamed.rs\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["changed_rust_lines"], 0)
        self.assertEqual(report["files"][0]["change_kind"], "renamed")

    def test_non_rust_diff_is_ignored_in_denominator(self) -> None:
        readme = self.repo / "README.md"
        readme.write_text("new\n", encoding="utf-8")
        diff = (
            "diff --git a/README.md b/README.md\n"
            "--- a/README.md\n"
            "+++ b/README.md\n"
            "@@ -1 +1 @@\n"
            "-old\n"
            "+new\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["changed_files"], 0)

    def test_test_only_paths_are_reported_outside_production_denominator(self) -> None:
        cases = (
            "crates/example/tests/integration.rs",
            "crates/example/src/tests/helper.rs",
            "crates/example/src/tests.rs",
            "crates/example/src/ipc_tests.rs",
            "crates/example/src/test_support.rs",
            "crates/example/src/lifecycle/property_tests.rs",
            "crates/example/benches/throughput.rs",
            "crates/example/examples/demo.rs",
            "crates/sorotte-gui/src/bin/sorotte-gui-native-smoke.rs",
            "crates/sorotte-gui/src/bin/sorotte-gui-native-smoke/platform_driver/windows_impl.rs",
            "crates/sorotte-gui/src/bin/sorotte-gui-semantic-smoke.rs",
            "crates/sorotte-gui/src/bin/sorotte-gui-semantic-suite.rs",
            "crates/sorotte-gui/src/bin/sorotte-gui-startup-bench.rs",
            "fuzz/fuzz_targets/framed_session.rs",
        )
        for path in cases:
            with self.subTest(path=path):
                self.assertTrue(coverage.is_test_only_rust_path(path))
        self.assertFalse(
            coverage.is_test_only_rust_path("crates/example/src/testament.rs")
        )
        self.assertFalse(
            coverage.is_test_only_rust_path("crates/example/src/lib.rs")
        )
        self.assertFalse(
            coverage.is_test_only_rust_path(
                "crates/sorotte-gui/src/bin/sorotte-gui-native-smoker.rs"
            )
        )
        self.assertFalse(
            coverage.is_test_only_rust_path("fuzz/src/production.rs")
        )

    def test_covered_test_addition_cannot_rescue_uncovered_production_line(self) -> None:
        test_source = self.repo / "tests" / "coverage.rs"
        test_source.parent.mkdir()
        test_source.write_text("assert!(true);\n", encoding="utf-8")
        production_diff = self.patch(
            old_range="5",
            new_range="5",
            body=["-        other", "+        0"],
        )
        test_diff = (
            "diff --git a/tests/coverage.rs b/tests/coverage.rs\n"
            "new file mode 100644\n"
            "--- /dev/null\n"
            "+++ b/tests/coverage.rs\n"
            "@@ -0,0 +1 @@\n"
            "+assert!(true);\n"
        )
        lcov = self.lcov() + self.lcov({1: 1}, source="tests/coverage.rs")
        report = self.build(lcov, production_diff + test_diff)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["production_changed_lines"], 1)
        self.assertEqual(report["summary"]["excluded_test_lines"], 1)
        self.assertEqual(report["summary"]["covered_lines"], 0)
        self.assertEqual(report["summary"]["uncovered_lines"], 1)
        self.assertEqual(report["summary"]["percent"], "0.00")
        self.assertEqual(report["summary"]["excluded_test_files"], 1)
        test_report = next(
            item for item in report["files"] if item["path"] == "tests/coverage.rs"
        )
        self.assertEqual(test_report["scope"], "test-only")
        self.assertEqual(test_report["lines"][0]["status"], "excluded-test")

    def test_test_to_production_pure_rename_materializes_full_target(self) -> None:
        production = self.repo / "src" / "promoted.rs"
        production.write_text(
            "pub fn promoted() -> bool {\n    true\n}\n",
            encoding="utf-8",
        )
        diff = (
            "diff --git a/tests/promoted.rs b/src/promoted.rs\n"
            "similarity index 100%\n"
            "rename from tests/promoted.rs\n"
            "rename to src/promoted.rs\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["production_changed_lines"], 3)
        self.assertEqual(report["summary"]["excluded_test_lines"], 0)
        self.assertEqual(report["summary"]["unmapped_lines"], 1)

    def test_non_rust_to_rust_pure_rename_cannot_bypass_changed_lines(self) -> None:
        introduced = self.repo / "src" / "introduced.rs"
        introduced.write_text("pub fn introduced() -> bool {\n    true\n}\n", encoding="utf-8")
        diff = (
            "diff --git a/src/introduced.txt b/src/introduced.rs\n"
            "similarity index 100%\n"
            "rename from src/introduced.txt\n"
            "rename to src/introduced.rs\n"
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["changed_rust_lines"], 3)
        self.assertEqual(report["summary"]["unmapped_lines"], 1)
        self.assertEqual(report["summary"]["non_coverable_lines"], 2)

    def test_ordinary_and_critical_ratchets_are_separate_results(self) -> None:
        ordinary_path = "src/ordinary.rs"
        critical_path = "crates/critical/src/core.rs"
        ordinary_lines = ["ordinary_behavior();"]
        critical_lines = [f"critical_behavior_{index}();" for index in range(1, 11)]
        (self.repo / ordinary_path).write_text(
            "\n".join(ordinary_lines) + "\n",
            encoding="utf-8",
        )
        (self.repo / critical_path).write_text(
            "\n".join(critical_lines) + "\n",
            encoding="utf-8",
        )
        diff = self.new_file_patch(
            ordinary_path,
            ordinary_lines,
        ) + self.new_file_patch(critical_path, critical_lines)
        ordinary_lcov = self.lcov({1: 1}, source=ordinary_path)
        critical_at_ninety = self.lcov(
            {line: 1 if line <= 9 else 0 for line in range(1, 11)},
            source=critical_path,
        )

        exact = self.build(
            ordinary_lcov + critical_at_ninety,
            diff,
            minimum="80",
        )
        self.assertEqual(exact["status"], "passed")
        self.assertEqual(exact["coverage_classes"]["ordinary"]["status"], "passed")
        self.assertEqual(exact["coverage_classes"]["ordinary"]["summary"]["percent"], "100.00")
        self.assertEqual(exact["coverage_classes"]["critical"]["status"], "passed")
        self.assertEqual(exact["coverage_classes"]["critical"]["summary"]["percent"], "90.00")
        self.assertEqual(exact["policy"]["ordinary_minimum_percent"], "80.00")
        self.assertEqual(exact["policy"]["critical_minimum_percent"], "90.00")

        critical_at_eighty = self.lcov(
            {line: 1 if line <= 8 else 0 for line in range(1, 11)},
            source=critical_path,
        )
        failed = self.build(
            ordinary_lcov + critical_at_eighty,
            diff,
            minimum="80",
        )
        self.assertEqual(failed["status"], "failed")
        self.assertEqual(failed["summary"]["percent"], "81.81")
        self.assertEqual(failed["coverage_classes"]["ordinary"]["status"], "passed")
        self.assertEqual(failed["coverage_classes"]["critical"]["status"], "failed")
        self.assertIn(
            "critical changed-line coverage 80.00% is below required 90.00%",
            failed["coverage_classes"]["critical"]["errors"],
        )
        critical_report = next(
            item for item in failed["files"] if item["path"] == critical_path
        )
        self.assertEqual(critical_report["coverage_class"], "critical")
        self.assertEqual(critical_report["critical_path"]["id"], "critical-core")

    def test_ordinary_failure_does_not_change_passing_critical_result(self) -> None:
        ordinary_path = "src/ordinary_four.rs"
        critical_path = "crates/critical/src/perfect.rs"
        ordinary_lines = [f"ordinary_{index}();" for index in range(1, 5)]
        critical_lines = [f"critical_{index}();" for index in range(1, 11)]
        (self.repo / ordinary_path).write_text(
            "\n".join(ordinary_lines) + "\n",
            encoding="utf-8",
        )
        (self.repo / critical_path).write_text(
            "\n".join(critical_lines) + "\n",
            encoding="utf-8",
        )
        report = self.build(
            self.lcov(
                {line: 1 if line <= 3 else 0 for line in range(1, 5)},
                source=ordinary_path,
            )
            + self.lcov(
                {line: 1 for line in range(1, 11)},
                source=critical_path,
            ),
            self.new_file_patch(ordinary_path, ordinary_lines)
            + self.new_file_patch(critical_path, critical_lines),
            minimum="80",
        )
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["coverage_classes"]["ordinary"]["status"], "failed")
        self.assertEqual(
            report["coverage_classes"]["ordinary"]["summary"]["percent"],
            "75.00",
        )
        self.assertEqual(report["coverage_classes"]["critical"]["status"], "passed")
        self.assertEqual(
            report["coverage_classes"]["critical"]["summary"]["percent"],
            "100.00",
        )

    def test_unmapped_critical_line_fails_only_the_critical_result(self) -> None:
        critical_path = "crates/critical/src/unmapped.rs"
        critical_lines = ["sensitive_behavior();"]
        (self.repo / critical_path).write_text(
            "\n".join(critical_lines) + "\n",
            encoding="utf-8",
        )
        report = self.build(
            self.lcov(),
            self.new_file_patch(critical_path, critical_lines),
            minimum="80",
        )
        self.assertEqual(report["status"], "failed")
        self.assertEqual(
            report["coverage_classes"]["ordinary"]["status"],
            "not-applicable",
        )
        self.assertEqual(report["coverage_classes"]["critical"]["status"], "failed")
        self.assertEqual(
            report["coverage_classes"]["critical"]["summary"]["unmapped_lines"],
            1,
        )
        self.assertIn(
            "critical coverage has 1 changed Rust line(s) with no LCOV source mapping",
            report["coverage_classes"]["critical"]["errors"],
        )

    def test_directory_match_is_component_exact_not_a_text_prefix(self) -> None:
        path = "crates/criticality/src/not_critical.rs"
        lines = ["ordinary_behavior();"]
        target = self.repo / path
        target.parent.mkdir(parents=True)
        target.write_text("\n".join(lines) + "\n", encoding="utf-8")
        report = self.build(
            self.lcov({1: 1}, source=path),
            self.new_file_patch(path, lines),
            minimum="100",
        )
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["files"][0]["coverage_class"], "ordinary")
        self.assertIsNone(report["files"][0]["critical_path"])
        self.assertEqual(
            report["coverage_classes"]["critical"]["status"],
            "not-applicable",
        )

    def test_ordinary_to_critical_pure_rename_materializes_target(self) -> None:
        path = "crates/critical/src/promoted.rs"
        lines = [f"promoted_{index}();" for index in range(1, 11)]
        (self.repo / path).write_text(
            "\n".join(lines) + "\n",
            encoding="utf-8",
        )
        diff = (
            f"diff --git a/src/promoted.rs b/{path}\n"
            "similarity index 100%\n"
            "rename from src/promoted.rs\n"
            f"rename to {path}\n"
        )
        report = self.build(
            self.lcov(
                {line: 1 if line <= 8 else 0 for line in range(1, 11)},
                source=path,
            ),
            diff,
            minimum="80",
        )
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["production_changed_lines"], 10)
        self.assertEqual(report["files"][0]["coverage_class"], "critical")
        self.assertEqual(report["files"][0]["critical_path"]["matched_on"], "new-path")
        self.assertEqual(
            report["coverage_classes"]["critical"]["summary"]["percent"],
            "80.00",
        )

    def test_critical_to_ordinary_pure_rename_keeps_critical_threshold(self) -> None:
        path = "src/demoted.rs"
        lines = [f"demoted_{index}();" for index in range(1, 11)]
        (self.repo / path).write_text(
            "\n".join(lines) + "\n",
            encoding="utf-8",
        )
        diff = (
            f"diff --git a/crates/critical/src/demoted.rs b/{path}\n"
            "similarity index 100%\n"
            "rename from crates/critical/src/demoted.rs\n"
            f"rename to {path}\n"
        )
        report = self.build(
            self.lcov(
                {line: 1 if line <= 8 else 0 for line in range(1, 11)},
                source=path,
            ),
            diff,
            minimum="80",
        )
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["production_changed_lines"], 10)
        self.assertEqual(report["files"][0]["coverage_class"], "critical")
        self.assertEqual(report["files"][0]["critical_path"]["matched_on"], "old-path")
        self.assertEqual(
            report["coverage_classes"]["critical"]["summary"]["percent"],
            "80.00",
        )

    def test_overlapping_critical_path_rules_fail_closed(self) -> None:
        nested = self.critical_dir / "nested"
        nested.mkdir()
        (nested / "production.rs").write_text("nested_behavior();\n", encoding="utf-8")
        directory = {
            "id": "parent",
            "category": "lifecycle",
            "owner": "runtime",
            "match": "directory",
            "path": "crates/critical/src/",
        }
        cases = {
            "directory-file": [
                directory,
                {
                    "id": "contained-file",
                    "category": "privacy",
                    "owner": "security",
                    "match": "file",
                    "path": "crates/critical/src/placeholder.rs",
                },
            ],
            "nested-directories": [
                directory,
                {
                    "id": "nested",
                    "category": "privacy",
                    "owner": "security",
                    "match": "directory",
                    "path": "crates/critical/src/nested/",
                },
            ],
            "duplicate-file": [
                {
                    "id": "first-file",
                    "category": "lifecycle",
                    "owner": "runtime",
                    "match": "file",
                    "path": "crates/critical/src/placeholder.rs",
                },
                {
                    "id": "second-file",
                    "category": "privacy",
                    "owner": "security",
                    "match": "file",
                    "path": "crates/critical/src/placeholder.rs",
                },
            ],
        }
        for name, rules in cases.items():
            with self.subTest(name=name):
                self.write_policy(rules=rules)
                with self.assertRaisesRegex(
                    coverage.DiffCoverageError,
                    "rules overlap",
                ):
                    self.build(self.lcov(), "")

    def test_missing_and_globbed_critical_targets_fail_closed(self) -> None:
        cases = {
            "missing": (
                "crates/critical/src/missing.rs",
                "does not exist",
            ),
            "glob": (
                "crates/*/src/",
                "glob syntax",
            ),
        }
        for name, (path, message) in cases.items():
            with self.subTest(name=name):
                self.write_policy(
                    rules=[
                        {
                            "id": "unsafe-target",
                            "category": "privacy",
                            "owner": "security",
                            "match": "directory" if path.endswith("/") else "file",
                            "path": path,
                        }
                    ]
                )
                with self.assertRaisesRegex(coverage.DiffCoverageError, message):
                    self.build(self.lcov(), "")

    def test_critical_threshold_cannot_be_lowered_or_reformatted(self) -> None:
        for value in ("89.99", "90", "090.00", "100.00"):
            with self.subTest(value=value):
                self.write_policy(minimum=value)
                with self.assertRaisesRegex(
                    coverage.DiffCoverageError,
                    "must be exactly 90.00",
                ):
                    self.build(self.lcov(), "")

    def test_policy_has_no_blanket_exclusion_escape_hatch(self) -> None:
        policy = self.policy_path.read_text(encoding="utf-8")
        policy = policy.replace(
            "[[critical_path]]",
            'excluded_paths = ["crates/"]\n\n[[critical_path]]',
            1,
        )
        self.policy_path.write_text(policy, encoding="utf-8")
        with self.assertRaisesRegex(coverage.DiffCoverageError, "unexpected"):
            self.build(self.lcov(), "")

    def test_explicit_diff_cannot_change_the_critical_policy(self) -> None:
        policy_patch = self.patch(
            old_path=coverage.DEFAULT_CRITICAL_POLICY_PATH,
            new_path=coverage.DEFAULT_CRITICAL_POLICY_PATH,
            old_range="4",
            new_range="4",
            body=[
                '-critical_minimum_percent = "90.00"',
                '+critical_minimum_percent = "80.00"',
            ],
        )
        with self.assertRaisesRegex(
            coverage.DiffCoverageError,
            "changes the critical path policy",
        ):
            self.build(self.lcov(), policy_patch, minimum="80")

    def test_missing_default_policy_produces_fail_closed_cli_artifact(self) -> None:
        lcov_path, diff_path = self.write_inputs(self.lcov(), self.patch())
        report_path = self.repo / "missing-policy.json"
        self.policy_path.unlink()
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = coverage.main(
                [
                    "--repo-root",
                    str(self.repo),
                    "--lcov",
                    str(lcov_path),
                    "--diff",
                    str(diff_path),
                    "--json-out",
                    str(report_path),
                ]
            )
        self.assertEqual(status, 2)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "error")
        self.assertIn("io: cannot read critical path policy", report["errors"][0])

    def test_empty_diff_is_a_valid_no_rust_change_result(self) -> None:
        report = self.build(self.lcov(), "")
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["summary"]["changed_rust_lines"], 0)

    def test_diff_is_bound_to_current_source_content(self) -> None:
        self.assert_input_error(
            self.lcov(),
            self.patch(body=["-pub fn old() {}", "+pub fn stale() {}"]),
            "does not match current source",
        )

    def test_context_lines_are_also_bound_to_current_source(self) -> None:
        diff = self.patch(
            old_range="1,2",
            new_range="1,2",
            body=[
                " pub fn answer(flag: bool) -> u32 {",
                "-    stale",
                "+    if flag {",
            ],
        )
        report = self.build(self.lcov(), diff)
        self.assertEqual(report["status"], "passed")
        stale = diff.replace(" pub fn answer", " pub fn different")
        self.assert_input_error(self.lcov(), stale, "does not match current source")

    def test_no_newline_marker_is_accepted_only_after_content(self) -> None:
        valid = self.patch() + "\\ No newline at end of file\n"
        # A marker appended after the complete segment is still after content.
        self.assertEqual(self.build(self.lcov(), valid)["status"], "passed")
        invalid = self.patch(body=[r"\ No newline at end of file", "-old", "+pub fn answer(flag: bool) -> u32 {"])
        self.assert_input_error(self.lcov(), invalid, "misplaced no-newline")

    def test_malformed_hunk_counts_fail_closed(self) -> None:
        malformed = self.patch(
            old_range="1,2",
            new_range="1,2",
            body=["-old", "+pub fn answer(flag: bool) -> u32 {"],
        )
        self.assert_input_error(self.lcov(), malformed, "hunk count mismatch")

    def test_overlapping_hunks_fail_closed(self) -> None:
        diff = (
            self.patch()
            + "@@ -1 +1 @@\n"
            + "-old again\n"
            + "+pub fn answer(flag: bool) -> u32 {\n"
        )
        self.assert_input_error(self.lcov(), diff, "overlapping or out-of-order")

    def test_duplicate_diff_target_fails_closed(self) -> None:
        self.assert_input_error(
            self.lcov(),
            self.patch() + self.patch(),
            "duplicate target path",
        )

    def test_unsafe_and_ambiguous_diff_paths_fail_closed(self) -> None:
        cases = {
            "parent": self.patch(old_path="../lib.rs", new_path="../lib.rs"),
            "backslash": self.patch(old_path=r"src\lib.rs", new_path=r"src\lib.rs"),
            "non_git": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
        }
        for name, diff in cases.items():
            with self.subTest(name=name):
                self.assert_input_error(self.lcov(), diff, "path|Git unified")

    def test_diff_and_content_headers_must_agree_and_be_ordered(self) -> None:
        disagree = self.patch().replace("+++ b/src/lib.rs", "+++ b/src/other.rs")
        self.assert_input_error(self.lcov(), disagree, "disagrees")
        reversed_headers = self.patch().replace(
            "--- a/src/lib.rs\n+++ b/src/lib.rs",
            "+++ b/src/lib.rs\n--- a/src/lib.rs",
        )
        self.assert_input_error(self.lcov(), reversed_headers, "immediately after")

    def test_binary_rust_diff_fails_but_non_rust_binary_is_ignored(self) -> None:
        rust = (
            "diff --git a/src/lib.rs b/src/lib.rs\n"
            "index 123..456 100644\n"
            "GIT binary patch\n"
        )
        self.assert_input_error(self.lcov(), rust, "binary content")
        non_rust = (
            "diff --git a/image.png b/image.png\n"
            "index 123..456 100644\n"
            "GIT binary patch\n"
        )
        report = self.build(self.lcov(), non_rust)
        self.assertEqual(report["status"], "passed")

    def test_incomplete_rename_metadata_fails_closed(self) -> None:
        diff = (
            "diff --git a/src/lib.rs b/src/renamed.rs\n"
            "similarity index 100%\n"
            "rename from src/lib.rs\n"
        )
        self.assert_input_error(self.lcov(), diff, "incomplete")

    def test_copy_metadata_is_rejected_to_prevent_zero_line_new_file_bypass(self) -> None:
        diff = (
            "diff --git a/src/lib.rs b/src/copied.rs\n"
            "similarity index 100%\n"
            "copy from src/lib.rs\n"
            "copy to src/copied.rs\n"
        )
        self.assert_input_error(self.lcov(), diff, "copy metadata")

    def test_added_content_resembling_a_file_header_is_not_misparsed(self) -> None:
        self.source.write_text("++ token\n", encoding="utf-8")
        diff = self.patch(body=["-old", "+++ token"])
        report = self.build(self.lcov({1: 1}), diff)
        self.assertEqual(report["status"], "passed")

    def test_quoted_git_paths_with_octal_utf8_are_decoded(self) -> None:
        unicode_source = self.repo / "src" / "café.rs"
        unicode_source.write_text("pub fn café() {}\n", encoding="utf-8")
        diff = (
            'diff --git "a/src/caf\\303\\251.rs" "b/src/caf\\303\\251.rs"\n'
            '--- "a/src/caf\\303\\251.rs"\n'
            '+++ "b/src/caf\\303\\251.rs"\n'
            "@@ -1 +1 @@\n"
            "-pub fn old() {}\n"
            "+pub fn café() {}\n"
        )
        report = self.build(self.lcov({1: 1}, source="src/café.rs"), diff)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["files"][0]["path"], "src/café.rs")

    def test_lcov_absolute_path_inside_repository_is_normalized(self) -> None:
        report = self.build(self.lcov(source=str(self.source)), self.patch())
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["files"][0]["path"], "src/lib.rs")

    def test_external_lcov_sources_are_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as external:
            external_source = pathlib.Path(external) / "dependency.rs"
            external_source.write_text("fn dependency() {}\n", encoding="utf-8")
            external_record = self.lcov({1: 1}, source=str(external_source))
            report = self.build(external_record + self.lcov(), self.patch())
        self.assertEqual(report["status"], "passed")

    def test_duplicate_lcov_source_records_fail_closed(self) -> None:
        self.assert_input_error(
            self.lcov() + self.lcov(),
            self.patch(),
            "duplicate source records",
        )

    def test_duplicate_da_line_fails_closed(self) -> None:
        lcov = (
            "SF:src/lib.rs\n"
            "DA:1,1\n"
            "DA:1,0\n"
            "LF:2\n"
            "LH:1\n"
            "end_of_record\n"
        )
        self.assert_input_error(lcov, self.patch(), "duplicates source line")

    def test_lcov_summary_mismatch_preserves_both_models_without_changing_da_policy(self) -> None:
        contradictory = self.lcov().replace("LF:4", "LF:6").replace("LH:3", "LH:2")

        report = self.build(contradictory, self.patch())

        self.assertEqual(report["status"], "passed")
        audit = report["inputs"]["lcov_summary_audit"]
        self.assertEqual(audit["status"], "producer-summary-mismatch")
        self.assertEqual(
            audit["records"],
            {
                "total": 1,
                "repository": 1,
                "ignored_external": 0,
                "summary_mismatched": 1,
                "lf_mismatched": 1,
                "lh_mismatched": 1,
            },
        )
        self.assertEqual(
            audit["declared_summary"],
            {"lines_found": 6, "lines_hit": 2},
        )
        self.assertEqual(
            audit["unique_da_summary"],
            {"lines_found": 4, "lines_hit": 3},
        )
        self.assertEqual(
            audit["mismatches"],
            [
                {
                    "record": 1,
                    "source": "src/lib.rs",
                    "fields": ["LF", "LH"],
                    "declared": {"lines_found": 6, "lines_hit": 2},
                    "unique_da": {"lines_found": 4, "lines_hit": 3},
                }
            ],
        )

    def test_lcov_summary_mismatch_cannot_invent_an_absent_executable_da_line(self) -> None:
        contradictory = self.lcov({1: 1, 2: 1, 3: 1}).replace("LF:3", "LF:4")
        diff = self.patch(
            old_range="5",
            new_range="5",
            body=["-        other", "+        0"],
        )

        report = self.build(contradictory, diff)

        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["summary"]["coverable_lines"], 0)
        self.assertEqual(report["summary"]["unmapped_lines"], 1)
        self.assertEqual(
            report["files"][0]["lines"][0]["reason"],
            "executable-looking-line-absent-from-lcov-unique-da-map",
        )

    def test_lcov_rejects_an_internally_impossible_declared_summary(self) -> None:
        malformed = self.lcov().replace("LH:3", "LH:5")
        self.assert_input_error(malformed, self.patch(), "LH cannot exceed LF")

    def test_lcov_cli_prints_the_preserved_summary_contradiction(self) -> None:
        contradictory = self.lcov().replace("LF:4", "LF:6")
        lcov_path, diff_path = self.write_inputs(contradictory, self.patch())
        stdout = io.StringIO()
        stderr = io.StringIO()

        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = coverage.main(
                [
                    "--repo-root",
                    str(self.repo),
                    "--lcov",
                    str(lcov_path),
                    "--diff",
                    str(diff_path),
                ]
            )

        self.assertEqual(status, 0)
        self.assertIn("diff coverage: passed", stdout.getvalue())
        self.assertIn(
            "LF/LH summaries contradict unique DA records in 1/1 source record(s)",
            stderr.getvalue(),
        )
        self.assertIn(coverage.LCOV_LINE_MODEL, stderr.getvalue())

    def test_lcov_requires_final_record_terminator(self) -> None:
        malformed = self.lcov().replace("end_of_record\n", "")
        self.assert_input_error(malformed, self.patch(), "missing end_of_record")

    def test_lcov_rejects_unknown_directive_and_negative_hit_count(self) -> None:
        unknown = self.lcov().replace("LF:4", "SURPRISE:value\nLF:4")
        self.assert_input_error(unknown, self.patch(), "unsupported")
        negative = self.lcov().replace("DA:1,1", "DA:1,-1")
        self.assert_input_error(negative, self.patch(), "non-negative")

    def test_lcov_rejects_missing_repository_source_as_stale(self) -> None:
        self.assert_input_error(
            self.lcov(source="src/missing.rs"),
            self.patch(),
            "does not exist",
        )

    def test_lcov_line_map_cannot_extend_beyond_current_source(self) -> None:
        self.assert_input_error(
            self.lcov({999: 1}),
            self.patch(),
            "beyond its current",
        )

    def test_minimum_rejects_nan_negative_and_over_100(self) -> None:
        for value in ("NaN", "-1", "100.01", " 90", "1e2", "66.666"):
            with self.subTest(value=value):
                with self.assertRaisesRegex(coverage.DiffCoverageError, "minimum"):
                    self.build(self.lcov(), self.patch(), minimum=value)

    def test_cli_returns_distinct_policy_and_input_exit_codes_and_writes_json(self) -> None:
        lcov_path, diff_path = self.write_inputs(
            self.lcov(),
            self.patch(
                old_range="5",
                new_range="5",
                body=["-other", "+        0"],
            ),
        )
        report_path = self.repo / "report.json"
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = coverage.main(
                [
                    "--repo-root",
                    str(self.repo),
                    "--lcov",
                    str(lcov_path),
                    "--diff",
                    str(diff_path),
                    "--json-out",
                    str(report_path),
                ]
            )
        self.assertEqual(status, 1)
        self.assertEqual(json.loads(report_path.read_text())["status"], "failed")

        diff_path.write_text("not a diff\n", encoding="utf-8")
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = coverage.main(
                [
                    "--repo-root",
                    str(self.repo),
                    "--lcov",
                    str(lcov_path),
                    "--diff",
                    str(diff_path),
                    "--json-out",
                    str(report_path),
                ]
            )
        self.assertEqual(status, 2)
        self.assertEqual(json.loads(report_path.read_text())["status"], "error")

    def test_explicit_diff_and_base_head_are_mutually_exclusive(self) -> None:
        lcov_path, diff_path = self.write_inputs(self.lcov(), self.patch())
        with self.assertRaisesRegex(coverage.DiffCoverageError, "cannot be combined"):
            coverage.build_report(
                repo_root=self.repo,
                lcov_path=lcov_path,
                diff_path=diff_path,
                base="main",
                head="HEAD",
                minimum_text="100",
            )

    def test_unexpected_cli_failure_still_fails_closed_with_json_artifact(self) -> None:
        lcov_path, diff_path = self.write_inputs(self.lcov(), self.patch())
        report_path = self.repo / "unexpected.json"
        stdout = io.StringIO()
        stderr = io.StringIO()
        with mock.patch.object(
            coverage,
            "build_report",
            side_effect=RuntimeError("synthetic internal fault"),
        ), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = coverage.main(
                [
                    "--repo-root",
                    str(self.repo),
                    "--lcov",
                    str(lcov_path),
                    "--diff",
                    str(diff_path),
                    "--json-out",
                    str(report_path),
                ]
            )
        self.assertEqual(status, 2)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "error")
        self.assertIn("unexpected RuntimeError", report["errors"][0])

    def test_base_head_mode_resolves_commits_generates_diff_and_binds_head(self) -> None:
        self.run_git("init", "-q")
        self.run_git("config", "user.name", "Coverage Test")
        self.run_git("config", "user.email", "coverage@example.invalid")
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "base")
        base = self.run_git("rev-parse", "HEAD").strip()
        self.source.write_text(
            "pub fn answer(flag: bool) -> u32 {\n"
            "    if flag {\n"
            "        43\n"
            "    } else {\n"
            "        0\n"
            "    }\n"
            "}\n",
            encoding="utf-8",
        )
        self.run_git("add", "src/lib.rs")
        self.run_git("commit", "-qm", "head")
        head = self.run_git("rev-parse", "HEAD").strip()
        lcov_path = self.repo / "coverage.info"
        lcov_path.write_text(self.lcov({3: 1}), encoding="utf-8")

        report = coverage.build_report(
            repo_root=self.repo,
            lcov_path=lcov_path,
            diff_path=None,
            base=base,
            head=head,
            minimum_text="100",
        )
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["inputs"]["base_sha"], base)
        self.assertEqual(report["inputs"]["head_sha"], head)
        self.assertEqual(report["summary"]["covered_lines"], 1)
        versions = report["inputs"]["critical_path_policy_versions"]
        self.assertEqual(
            [(item["origin"], item["revision"], item["present"]) for item in versions],
            [("base", base, True), ("head", head, True)],
        )
        self.assertEqual(versions[0]["sha256"], versions[1]["sha256"])
        effective_rule = next(
            rule
            for rule in report["policy"]["critical_path_policy"]["rules"]
            if rule["id"] == "critical-core"
        )
        self.assertEqual(effective_rule["policy_origins"], ["base", "head"])

    def test_base_policy_rule_cannot_be_deleted_to_weaken_the_same_change(self) -> None:
        self.run_git("init", "-q")
        self.run_git("config", "user.name", "Coverage Test")
        self.run_git("config", "user.email", "coverage@example.invalid")
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "base")
        base = self.run_git("rev-parse", "HEAD").strip()

        target_path = "crates/critical/src/placeholder.rs"
        target = self.repo / target_path
        changed_lines = [f"critical_behavior_{index}();" for index in range(1, 11)]
        target.write_text("\n".join(changed_lines) + "\n", encoding="utf-8")
        replacement_dir = self.repo / "crates" / "other" / "src"
        replacement_dir.mkdir(parents=True)
        (replacement_dir / "keep.rs").write_text(
            "pub fn keep() {}\n",
            encoding="utf-8",
        )
        self.write_policy(
            rules=[
                {
                    "id": "other-critical",
                    "category": "lifecycle",
                    "owner": "runtime",
                    "match": "directory",
                    "path": "crates/other/src/",
                }
            ]
        )
        self.run_git("add", "crates", coverage.DEFAULT_CRITICAL_POLICY_PATH)
        self.run_git("commit", "-qm", "head weakens policy")
        head = self.run_git("rev-parse", "HEAD").strip()
        lcov_path = self.repo / "coverage.info"
        lcov_path.write_text(
            self.lcov(
                {
                    line: 1 if line <= 8 else 0
                    for line in range(1, len(changed_lines) + 1)
                },
                source=target_path,
            ),
            encoding="utf-8",
        )

        report = coverage.build_report(
            repo_root=self.repo,
            lcov_path=lcov_path,
            diff_path=None,
            base=base,
            head=head,
            minimum_text="80",
        )

        self.assertEqual(report["status"], "failed")
        self.assertEqual(
            report["coverage_classes"]["ordinary"]["status"],
            "not-applicable",
        )
        self.assertEqual(report["coverage_classes"]["critical"]["status"], "failed")
        self.assertEqual(
            report["coverage_classes"]["critical"]["summary"]["percent"],
            "80.00",
        )
        file_report = report["files"][0]
        self.assertEqual(file_report["coverage_class"], "critical")
        self.assertEqual(file_report["critical_path"]["id"], "critical-core")
        self.assertEqual(file_report["critical_path"]["policy_origins"], ["base"])
        versions = report["inputs"]["critical_path_policy_versions"]
        self.assertEqual(
            [(item["origin"], item["present"]) for item in versions],
            [("base", True), ("head", True)],
        )
        self.assertNotEqual(versions[0]["sha256"], versions[1]["sha256"])

    def test_base_policy_can_be_absent_when_head_introduces_the_gate(self) -> None:
        self.policy_path.unlink()
        self.run_git("init", "-q")
        self.run_git("config", "user.name", "Coverage Test")
        self.run_git("config", "user.email", "coverage@example.invalid")
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "base without policy")
        base = self.run_git("rev-parse", "HEAD").strip()

        self.write_policy()
        self.source.write_text(
            self.source.read_text(encoding="utf-8").replace("42", "43"),
            encoding="utf-8",
        )
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "introduce policy")
        head = self.run_git("rev-parse", "HEAD").strip()
        lcov_path = self.repo / "coverage.info"
        lcov_path.write_text(self.lcov({3: 1}), encoding="utf-8")

        report = coverage.build_report(
            repo_root=self.repo,
            lcov_path=lcov_path,
            diff_path=None,
            base=base,
            head=head,
            minimum_text="100",
        )

        self.assertEqual(report["status"], "passed")
        versions = report["inputs"]["critical_path_policy_versions"]
        self.assertEqual(versions[0]["origin"], "base")
        self.assertFalse(versions[0]["present"])
        self.assertIsNone(versions[0]["sha256"])
        self.assertEqual(versions[1]["origin"], "head")
        self.assertTrue(versions[1]["present"])
        self.assertRegex(versions[1]["sha256"], r"^sha256:[0-9a-f]{64}$")

    def test_base_head_policy_union_rejects_cross_revision_overlap(self) -> None:
        self.run_git("init", "-q")
        self.run_git("config", "user.name", "Coverage Test")
        self.run_git("config", "user.email", "coverage@example.invalid")
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "base directory rule")
        base = self.run_git("rev-parse", "HEAD").strip()

        self.write_policy(
            rules=[
                {
                    "id": "nested-file",
                    "category": "lifecycle",
                    "owner": "runtime",
                    "match": "file",
                    "path": "crates/critical/src/placeholder.rs",
                }
            ]
        )
        self.run_git("add", coverage.DEFAULT_CRITICAL_POLICY_PATH)
        self.run_git("commit", "-qm", "head overlapping file rule")
        head = self.run_git("rev-parse", "HEAD").strip()
        lcov_path = self.repo / "coverage.info"
        lcov_path.write_text(self.lcov(), encoding="utf-8")

        with self.assertRaisesRegex(
            coverage.DiffCoverageError,
            "base/head critical path policy union is ambiguous",
        ):
            coverage.build_report(
                repo_root=self.repo,
                lcov_path=lcov_path,
                diff_path=None,
                base=base,
                head=head,
                minimum_text="80",
            )

    def test_base_head_mode_rejects_wrong_checkout_and_dirty_rust(self) -> None:
        lcov_path, _ = self.write_inputs(self.lcov(), self.patch())
        sha = "a" * 40
        with mock.patch.object(
            coverage,
            "resolve_commit",
            side_effect=[sha, "b" * 40, "c" * 40],
        ):
            with self.assertRaisesRegex(coverage.DiffCoverageError, "does not match"):
                coverage.load_diff_input(
                    repo_root=self.repo,
                    diff_path=None,
                    base="base",
                    head="head",
                )

        with mock.patch.object(
            coverage,
            "resolve_commit",
            side_effect=[sha, sha, sha],
        ), mock.patch.object(
            coverage,
            "run_git",
            return_value=" M src/lib.rs\n",
        ):
            with self.assertRaisesRegex(coverage.DiffCoverageError, "clean Rust"):
                coverage.load_diff_input(
                    repo_root=self.repo,
                    diff_path=None,
                    base="base",
                    head="head",
                )
        self.assertTrue(lcov_path.exists())

    def run_git(self, *args: str) -> str:
        process = subprocess.run(
            ["git", "-C", str(self.repo), *args],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        return process.stdout


if __name__ == "__main__":
    unittest.main()
