from __future__ import annotations

import copy
from pathlib import Path
import re
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import coverage_tool_canary as canary


class CoveragePatternCanaryTests(unittest.TestCase):
    def setUp(self):
        self.sources = {"lib.rs": canary.LIB, "worker.rs": canary.WORKER}
        self.locations = {}
        self.files = []
        self.view = []
        for filename, content in self.sources.items():
            segments = []
            for number, line in enumerate(content.splitlines(), 1):
                match = re.search(r"// ((?:PATTERN|STRUCTURAL)_[A-Z_]+)$", line)
                if not match:
                    continue
                marker = match[1]
                self.locations[marker] = (filename, number)
                count = ""
                if marker in canary.PATTERN_EXECUTABLE_MARKERS:
                    count = "2"
                    segments.append([number, 5, 2, True, True, False])
                self.view.append(f"{number}|{count}|{line}")
            self.files.append({"filename": filename, "segments": segments})
        self.report = {"data": [{"files": self.files}]}

    def observe(self, report=None, view=None, sources=None):
        return canary.pattern_observations(
            report if report is not None else self.report,
            "\n".join(view if view is not None else self.view),
            sources if sources is not None else self.sources,
        )

    def test_lf_and_crlf_preserve_independent_pattern_and_structural_evidence(self):
        expected = self.observe()
        self.assertEqual(set(expected["executable"]), canary.PATTERN_EXECUTABLE_MARKERS)
        self.assertEqual(set(expected["structural"]), canary.PATTERN_STRUCTURAL_MARKERS)
        self.assertEqual(expected, self.observe(sources={k: v.replace("\n", "\r\n") for k, v in self.sources.items()}))

    def test_each_executable_mapping_is_required_even_when_classifier_exempts_it(self):
        for marker in canary.PATTERN_EXECUTABLE_MARKERS:
            with self.subTest(marker=marker):
                filename, number = self.locations[marker]
                report = copy.deepcopy(self.report)
                entry = next(f for f in report["data"][0]["files"] if f["filename"] == filename)
                entry["segments"] = [s for s in entry["segments"] if s[0] != number]
                with mock.patch.object(canary, "lexical_non_coverable_lines", return_value=set(range(1, 1000))):
                    with self.assertRaisesRegex(ValueError, "executable mapping lost"):
                        self.observe(report=report)

    def test_each_text_mapping_must_independently_agree_with_json(self):
        for marker in canary.PATTERN_EXECUTABLE_MARKERS:
            with self.subTest(marker=marker):
                view = [line.replace("|2|", "||") if line.endswith(marker) else line for line in self.view]
                with self.assertRaisesRegex(ValueError, "executable mapping lost"):
                    self.observe(view=view)

    def test_zero_hits_cannot_replace_executed_accept_or_reject_paths(self):
        for marker in canary.PATTERN_EXECUTABLE_MARKERS:
            with self.subTest(marker=marker):
                filename, number = self.locations[marker]
                report = copy.deepcopy(self.report)
                entry = next(f for f in report["data"][0]["files"] if f["filename"] == filename)
                next(s for s in entry["segments"] if s[0] == number)[2] = 0
                view = [line.replace("|2|", "|0|") if line.endswith(marker) else line for line in self.view]
                with self.assertRaisesRegex(ValueError, "executable mapping lost"):
                    self.observe(report=report, view=view)

    def test_a_mapped_wrapper_cannot_be_silently_exempted(self):
        for marker in canary.PATTERN_STRUCTURAL_MARKERS:
            for count in (0, 1):
                with self.subTest(marker=marker, count=count):
                    filename, number = self.locations[marker]
                    report = copy.deepcopy(self.report)
                    entry = next(f for f in report["data"][0]["files"] if f["filename"] == filename)
                    entry["segments"].append([number, 5, count, True, True, False])
                    with self.assertRaisesRegex(ValueError, "unexpectedly instrumented"):
                        self.observe(report=report)

    def test_a_text_wrapper_counter_is_not_equivalent_to_blank(self):
        for marker in canary.PATTERN_STRUCTURAL_MARKERS:
            with self.subTest(marker=marker):
                view = [line.replace("||", "|0|") if line.endswith(marker) else line for line in self.view]
                with self.assertRaisesRegex(ValueError, "unexpectedly instrumented"):
                    self.observe(view=view)

    def test_missing_classifier_responsibility_fails_despite_agreeing_llvm_views(self):
        with mock.patch.object(canary, "lexical_non_coverable_lines", return_value=set()):
            with self.assertRaisesRegex(ValueError, "misclassified as executable"):
                self.observe()

    def test_source_object_and_text_marker_duplicates_are_rejected(self):
        report = copy.deepcopy(self.report)
        report["data"][0]["files"].append(copy.deepcopy(self.files[0]))
        with self.assertRaisesRegex(ValueError, "duplicate coverage pattern source object"):
            self.observe(report=report)
        with self.assertRaisesRegex(ValueError, "duplicate coverage pattern source-view marker"):
            self.observe(view=self.view + self.view[:1])

    def test_missing_marker_and_wrong_source_view_line_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "source-view inventory incomplete"):
            self.observe(view=self.view[1:])
        view = self.view.copy()
        view[0] = "999|" + view[0].split("|", 1)[1]
        with self.assertRaisesRegex(ValueError, "line identity mismatch"):
            self.observe(view=view)
        sources = dict(self.sources)
        sources["lib.rs"] = sources["lib.rs"].replace("// PATTERN_TUPLE_ACCEPT", "// missing marker")
        with self.assertRaisesRegex(ValueError, "source marker inventory incomplete"):
            self.observe(sources=sources)


if __name__ == "__main__":
    unittest.main()
