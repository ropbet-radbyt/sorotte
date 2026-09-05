# Lifecycle recorder health and verification inputs

The enabled Rust lifecycle recorder remembers its first validation,
serialization, write, or flush failure. It refuses later observations and
returns that failure at finalization. The CLI, server, and GUI already
propagate their shutdown emissions and final flush; a discarded intermediate
emission error therefore cannot turn into successful process finalization.
The disabled recorder retains its existing inexpensive no-op behavior.

`LifecycleEvidenceRecorder::with_writer` uses the same protocol as the
create-new file constructor. Each complete record, including its newline, is
serialized into a buffer capped at 16 KiB before the destination sees bytes.
A partially completed I/O operation can leave diagnostic output, but it can
never restore recorder health. The shared mutex makes sequence assignment,
causal chaining, writes, and first-failure publication one operation.
Malformed token values are never written or copied into the health error.
Writer-provided error text is omitted from retained health diagnostics.
Schema version 1, digest representation, and healthy causal behavior remain
unchanged. Event IDs remain within the schema's 128-byte/eight-digit limits.

The public `EvidenceError` enum retains its 0.2.x variants. New recorder
failures use a separate, non-exhaustive `RecordingFailure` type carried by the
existing I/O variant; `EvidenceError::recording_failure` exposes the typed
cause. Exhaustive downstream matches remain source-compatible, and sticky
health still retains only the redacted first failure.

`scripts/artifact_input.py` shares only serialization primitives. It rejects
duplicate keys at any depth, NaN/Infinity, floats that overflow to infinity,
invalid UTF-8, and trailing data after a JSON value. Actual stream reads have
byte limits even when a file grows after its metadata was checked. JSONL has
separate physical-line, record-count, and total-byte limits. Blank physical
lines count toward the record limit; only JSON whitespace is ignored.
Integer validation excludes booleans. Error categories identify byte limits,
record limits, encoding, JSON, duplicate keys, non-finite numbers, and integer
types without quoting the malformed input.

| Input | Limit |
|---|---|
| Release gate report | 16 MiB |
| Behavior evidence shard | 1 MiB |
| Lifecycle JSONL file | 128 MiB, 200,000 physical records, 64 KiB per record |
| Mutation JSON | 32 MiB |
| Source-bound coverage map | 128 MiB |
| LLVM JSON export | 256 MiB |
| Coverage profile report / lane log | 8 MiB / 128 MiB |
| Coverage finalizer JSON | 32 MiB, with explicit LLVM/map limits |
| Package or dependency manifest | 1 MiB |
| Lifecycle TOML model or schedule | 4 MiB |

Closed schemas, supported versions, exact source/platform/digest bindings,
required proofs, and behavioral oracles stay with each consumer. Lifecycle,
release, behavior, and coverage schema version 1 and mutation policy/report
schema version 3 remain explicit. GUI update/install manifests retain their
existing v1/v2 identities. Pre-0.2.9 packages retain the historical closed
payload inventory. Version 0.2.9 and newer additionally require
`DEPENDENCIES.json` and `THIRD-PARTY-NOTICES.txt`; package consumers independently
hash the payload, validate its dependency bindings, and check the exact target.

Validation includes the recorder regression that previously accepted a final
flush after a rejected observation; transient writer failures before writing,
mid-record, at newline, and at flush; concurrent failures; and a Rust producer
feeding healthy and faulted output to the actual Python lifecycle CLI. A
reusable malformed-artifact matrix exercises twelve CLI entrypoints, including
the release gate's failed-then-passed duplicate status and authenticated
package manifests. Existing wrong-SHA, wrong-platform, proof, source-binding,
and tampered-digest tests continue to apply. Recorder health does not replace
the lifecycle transition requirements or independent release oracles.

Focused commands:

```text
cargo test --locked -p sorotte-lifecycle-evidence
cargo clippy --locked -p sorotte-lifecycle-evidence --all-targets -- -D warnings
python -m unittest scripts.tests.test_artifact_input scripts.tests.test_artifact_entrypoints scripts.tests.test_release_dependency_inventory
python -m unittest discover -s scripts/tests
```
