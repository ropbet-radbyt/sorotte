from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest


sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import playback_lifecycle_evidence as evidence  # noqa: E402


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
MODEL_PATH = REPO_ROOT / "coverage" / "playback-lifecycle.toml"


class PlaybackLifecycleEvidenceTests(unittest.TestCase):
    @staticmethod
    def writer(
        path: pathlib.Path,
        *,
        emitter: str,
        binary_role: str,
        roles: tuple[str, ...],
        digest_byte: str,
    ) -> evidence.EvidenceWriter:
        return evidence.EvidenceWriter(
            path,
            run_id="run-001",
            emitter=emitter,
            binary_role=binary_role,
            component_roles=roles,
            product_version="0.2.8",
            product_digest=digest_byte * 64,
        )

    @staticmethod
    def emit(
        writer: evidence.EvidenceWriter,
        *,
        role: str,
        machine: str,
        transition: str,
        causal_predecessors: tuple[str, ...] = (),
    ) -> str:
        return writer.emit(
            process_role=role,
            subject="test-subject",
            machine=machine,
            transition=transition,
            target_kind="server-state" if role == "server" else "player-state",
            trigger="internal",
            authority_before="pending",
            authority_after="observed",
            expected_effect="transition-applied",
            observed_effect="transition-applied",
            disposition="applied",
            identities={"generation": 1},
            causal_predecessors=causal_predecessors,
        )

    def test_complete_product_inventory_merges_with_exact_digests(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            server_path = root / "server.jsonl"
            client_path = root / "client.jsonl"
            merged_path = root / "merged.jsonl"
            summary_path = root / "summary.json"

            server = self.writer(
                server_path,
                emitter="server",
                binary_role="server",
                roles=("server",),
                digest_byte="a",
            )
            server_event = self.emit(
                server,
                role="server",
                machine="application",
                transition="APP-LAUNCH-001",
            )
            server.close()

            client = self.writer(
                client_path,
                emitter="client-controller",
                binary_role="client",
                roles=("client", "player"),
                digest_byte="b",
            )
            self.emit(
                client,
                role="client",
                machine="session",
                transition="SESSION-CONNECT-001",
                causal_predecessors=(server_event,),
            )
            self.emit(
                client,
                role="player",
                machine="local-transport",
                transition="TRANSPORT-LOAD-001",
            )
            client.close()

            summary = evidence.validate_and_merge(
                [server_path, client_path],
                model_path=MODEL_PATH,
                output_path=merged_path,
                summary_path=summary_path,
                required_inventories={
                    "server": frozenset({"server"}),
                    "client-controller": frozenset({"client", "player"}),
                },
                required_roles=frozenset({"server", "client", "player"}),
                expected_digests={"server": "a" * 64, "client-controller": "b" * 64},
                minimum_cross_process_edges=1,
            )

            self.assertEqual(summary["result"], "passed")
            self.assertEqual(summary["process_count"], 2)
            self.assertEqual(summary["transition_count"], 3)
            self.assertEqual(summary["cross_process_edge_count"], 1)
            self.assertEqual(summary["emitted_roles"], {"client": 1, "player": 1, "server": 1})
            self.assertEqual(summary["merged_sha256"], evidence.sha256_file(merged_path))
            self.assertEqual(json.loads(summary_path.read_text(encoding="utf-8")), summary)
            merged = evidence.read_jsonl(merged_path)
            self.assertEqual(len(merged), 5)
            self.assertTrue(all(record["run_id"] == "run-001" for record in merged))

    def test_writer_rejects_raw_paths_urls_and_zero_identities(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            writer = self.writer(
                pathlib.Path(directory) / "unsafe.jsonl",
                emitter="client-a",
                binary_role="client",
                roles=("client",),
                digest_byte="c",
            )
            base = {
                "process_role": "client",
                "machine": "session",
                "transition": "SESSION-CONNECT-001",
                "target_kind": "protocol-message",
                "trigger": "internal",
                "authority_before": "pending",
                "authority_after": "observed",
                "expected_effect": "transition-applied",
                "observed_effect": "transition-applied",
                "disposition": "applied",
            }
            for subject in (
                r"C:\Users\person\video.mkv",
                "https://example.invalid/video",
                "private room",
            ):
                with self.assertRaisesRegex(evidence.EvidenceError, "privacy-safe token"):
                    writer.emit(subject=subject, **base)
            with self.assertRaisesRegex(evidence.EvidenceError, "positive integer"):
                writer.emit(subject="safe-subject", identities={"generation": 0}, **base)
            writer.close()

    def test_validator_rejects_unknown_transition_and_wrong_machine(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid-model.jsonl"
            writer = self.writer(
                path,
                emitter="server",
                binary_role="server",
                roles=("server",),
                digest_byte="d",
            )
            self.emit(
                writer,
                role="server",
                machine="session",
                transition="APP-LAUNCH-001",
            )
            writer.close()
            with self.assertRaisesRegex(evidence.EvidenceError, "wrong machine"):
                evidence.validate_and_merge([path], model_path=MODEL_PATH)

    def test_validator_rejects_broken_local_and_cross_process_causality(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "broken-cause.jsonl"
            writer = self.writer(
                path,
                emitter="server",
                binary_role="server",
                roles=("server",),
                digest_byte="e",
            )
            self.emit(
                writer,
                role="server",
                machine="application",
                transition="APP-LAUNCH-001",
            )
            writer.close()
            records = evidence.read_jsonl(path)
            records[1]["causal_predecessors"] = ["missing.00000001"]
            path.write_text(
                "".join(json.dumps(record) + "\n" for record in records),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(evidence.EvidenceError, "immediate local cause"):
                evidence.validate_and_merge([path], model_path=MODEL_PATH)

    def test_validator_requires_a_real_cross_process_edge(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "isolated.jsonl"
            writer = self.writer(
                path,
                emitter="client-a",
                binary_role="client",
                roles=("client",),
                digest_byte="1",
            )
            self.emit(
                writer,
                role="client",
                machine="session",
                transition="SESSION-CONNECT-001",
            )
            writer.close()
            with self.assertRaisesRegex(evidence.EvidenceError, "too few cross-process"):
                evidence.validate_and_merge(
                    [path],
                    model_path=MODEL_PATH,
                    minimum_cross_process_edges=1,
                )

    def test_validator_requires_declared_roles_and_exact_artifact_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "client.jsonl"
            writer = self.writer(
                path,
                emitter="client-a",
                binary_role="client",
                roles=("client", "player"),
                digest_byte="f",
            )
            self.emit(
                writer,
                role="client",
                machine="session",
                transition="SESSION-CONNECT-001",
            )
            writer.close()
            with self.assertRaisesRegex(evidence.EvidenceError, "emitted no transitions"):
                evidence.validate_and_merge(
                    [path],
                    model_path=MODEL_PATH,
                    required_roles=frozenset({"client", "player"}),
                )
            with self.assertRaisesRegex(evidence.EvidenceError, "exact artifact"):
                evidence.validate_and_merge(
                    [path],
                    model_path=MODEL_PATH,
                    expected_digests={"client-a": "0" * 64},
                )


if __name__ == "__main__":
    unittest.main()
