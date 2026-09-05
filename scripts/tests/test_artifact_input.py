from __future__ import annotations

import hashlib
import io
import pathlib
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import artifact_input as artifacts
from scripts.tests.artifact_malformed import malformed_json_cases


class ArtifactInputTests(unittest.TestCase):
    def test_strict_json_matrix_is_shared_and_redacted(self) -> None:
        valid = b'{"schema_version":1,"result":"passed"}'
        self.assertEqual(artifacts.strict_json_loads(valid)["result"], "passed")
        for name, raw, category in malformed_json_cases(valid):
            if name == "boolean-integer":
                with self.assertRaisesRegex(artifacts.ArtifactInputError, "integer"):
                    artifacts.require_int(artifacts.strict_json_loads(raw)["schema_version"], label="schema_version")
                continue
            with self.subTest(name=name), self.assertRaises(artifacts.ArtifactInputError) as failure:
                artifacts.strict_json_loads(raw)
            self.assertEqual(failure.exception.category, category)
            self.assertNotIn("canary", str(failure.exception))
        with self.assertRaisesRegex(artifacts.ArtifactInputError, "duplicate_key"):
            artifacts.strict_json_loads(b'{"nested":{"secret-canary":1,"secret-canary":2}}')

    def test_actual_bytes_are_bounded_when_metadata_lies_or_stream_grows(self) -> None:
        self.assertEqual(artifacts.read_stream_bounded(io.BytesIO(b"1234"), max_bytes=4), b"1234")
        source = io.BytesIO(b"12345" + b"unread" * 100)
        with self.assertRaisesRegex(artifacts.ArtifactInputError, "byte_limit"):
            artifacts.read_stream_bounded(source, max_bytes=4)
        self.assertEqual(source.tell(), 5)
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "artifact.json"
            path.write_bytes(b"12345")
            with mock.patch.object(pathlib.Path, "stat", return_value=types.SimpleNamespace(st_size=0)):
                with self.assertRaisesRegex(artifacts.ArtifactInputError, "byte_limit"):
                    artifacts.read_bounded(path, max_bytes=4)

    def test_utf8_limits_count_bytes_and_reject_unpaired_input_surrogates(self) -> None:
        self.assertEqual(artifacts.strict_json_loads('"é"', max_bytes=4), "é")
        for text, limit, category in [('"é"', 3, "byte_limit"), ('"\ud800"', 10, "utf8")]:
            with self.subTest(category=category), self.assertRaisesRegex(artifacts.ArtifactInputError, category):
                artifacts.strict_json_loads(text, max_bytes=limit)

    def test_jsonl_bounds_physical_records_individual_lines_and_total_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "ledger.jsonl"
            path.write_bytes(b"{}\n{}\n")
            self.assertEqual(artifacts.strict_jsonl_load(path, max_record_bytes=3, max_bytes=6, max_records=2), [{}, {}])
            for limits, category in [
                ({"max_record_bytes": 2}, "record_bytes"),
                ({"max_bytes": 5}, "byte_limit"),
                ({"max_records": 1}, "record_limit"),
            ]:
                with self.subTest(category=category), self.assertRaisesRegex(artifacts.ArtifactInputError, category):
                    artifacts.strict_jsonl_load(path, **limits)
            path.write_bytes(b"\n\n{}\n")
            with self.assertRaisesRegex(artifacts.ArtifactInputError, "record_limit"):
                artifacts.strict_jsonl_load(path, max_records=2)
            path.write_bytes(b"{}\n{\"x\":NaN}\n")
            with self.assertRaisesRegex(artifacts.ArtifactInputError, "nonfinite"):
                artifacts.strict_jsonl_load(path)
            path.write_bytes(b"{}\n\v\n")
            with self.assertRaisesRegex(artifacts.ArtifactInputError, "json"):
                artifacts.strict_jsonl_load(path)

    def test_integer_primitive_excludes_booleans_and_floats(self) -> None:
        self.assertEqual(artifacts.require_int(1, label="count", minimum=1, maximum=1), 1)
        for value in (True, False, 1.0, "1", None, 0, 2):
            with self.subTest(value=value), self.assertRaises(artifacts.ArtifactInputError):
                artifacts.require_int(value, label="count", minimum=1, maximum=1)

    def test_digest_primitive_is_streamed_and_preserves_existing_format(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "artifact"
            raw = b"independent expected bytes\n"
            path.write_bytes(raw)
            self.assertEqual(artifacts.sha256_file(path), hashlib.sha256(raw).hexdigest())


if __name__ == "__main__":
    unittest.main()
