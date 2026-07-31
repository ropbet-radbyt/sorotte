from __future__ import annotations

import unittest

from scripts import mpv_version_matrix


MINIMUM_SHA = "41f6a645068483470267271e1d09966ca3b9f413"
NEWEST_SHA = "d12f2ce19c918875981e00ed276f153bdf40a2ac"


class MpvVersionMatrixTests(unittest.TestCase):
    def validate(self, *, identity: str, source_sha: str, first_line: str):
        return mpv_version_matrix.validate_observation(
            identity=identity,
            source_sha=source_sha,
            minimum_source_sha=MINIMUM_SHA,
            newest_source_sha=NEWEST_SHA,
            minimum_version="0.41.0",
            first_line=first_line,
        )

    def test_release_and_snapshot_version_lines_are_accepted(self) -> None:
        minimum = self.validate(
            identity="minimum",
            source_sha=MINIMUM_SHA,
            first_line="mpv v0.41.0 Copyright mpv project",
        )
        newest = self.validate(
            identity="newest",
            source_sha=NEWEST_SHA,
            first_line="mpv v0.41.0-dev-gd12f2ce19 Copyright mpv project",
        )
        unprefixed = mpv_version_matrix.parse_mpv_version("mpv 0.42.1")

        self.assertEqual(minimum["version"], "0.41.0")
        self.assertEqual(newest["version"], "0.41.0")
        self.assertEqual(unprefixed, (0, 42, 1))

    def test_version_parser_rejects_partial_embedded_and_malformed_values(self) -> None:
        for value in (
            "mpv v0.41",
            "mpv xv0.41.0",
            "mpv v0.41.0x",
            "mpv version unknown",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                mpv_version_matrix.parse_mpv_version(value)

    def test_minimum_rejects_newer_version_and_all_endpoints_reject_older(self) -> None:
        with self.assertRaisesRegex(ValueError, "no longer reports"):
            self.validate(
                identity="minimum",
                source_sha=MINIMUM_SHA,
                first_line="mpv v0.42.0",
            )
        with self.assertRaisesRegex(ValueError, "or newer"):
            self.validate(
                identity="newest",
                source_sha=NEWEST_SHA,
                first_line="mpv v0.40.0",
            )

    def test_source_contract_rejects_unknown_floating_collapsed_and_drifted(self) -> None:
        cases = (
            {
                "identity": "other",
                "source_sha": NEWEST_SHA,
                "minimum_source_sha": MINIMUM_SHA,
                "newest_source_sha": NEWEST_SHA,
            },
            {
                "identity": "newest",
                "source_sha": "master",
                "minimum_source_sha": MINIMUM_SHA,
                "newest_source_sha": NEWEST_SHA,
            },
            {
                "identity": "newest",
                "source_sha": MINIMUM_SHA,
                "minimum_source_sha": MINIMUM_SHA,
                "newest_source_sha": MINIMUM_SHA,
            },
            {
                "identity": "newest",
                "source_sha": MINIMUM_SHA,
                "minimum_source_sha": MINIMUM_SHA,
                "newest_source_sha": NEWEST_SHA,
            },
        )
        for case in cases:
            with self.subTest(case=case), self.assertRaises(ValueError):
                mpv_version_matrix.validate_source_identity(**case)

    def test_minimum_version_requires_exact_three_component_tuple(self) -> None:
        for value in ("0.41", "v0.41.0", "0.41.0-dev", ""):
            with self.subTest(value=value), self.assertRaises(ValueError):
                mpv_version_matrix.parse_version_tuple(value, label="minimum")


if __name__ == "__main__":
    unittest.main()
