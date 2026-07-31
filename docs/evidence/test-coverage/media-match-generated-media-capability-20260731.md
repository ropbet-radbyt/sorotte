# Media Match generated-media capability lane

Date: 2026-07-31  
Slice: fail-closed real ffmpeg/ffprobe Media Match V3 diagnostic coverage

## Scope

This slice covers Sorotte's own local Media Match diagnostic harness. It
generates a deterministic synthetic Matroska file in an isolated temporary
directory, copies that file as the candidate, and exercises the public
Media Match V3 manifest, fingerprint, retrieval, decision, and JSON-report
boundaries. It does not contact a network service or consume production media.

The previous ignored GUI unit test returned successfully when either ffmpeg or
ffprobe was unavailable. That made a manual invocation indistinguishable from
a real pass and left the production-compatible extraction path outside any
required lane.

## Implementation

- The tool-backed test is now the direct integration target
  `crates/sorotte-media-match/tests/generated_media_v3.rs`.
- Tool probes are fail-closed. A missing executable, failed process launch, or
  nonzero version probe fails the test.
- The fixture uses ffmpeg's built-in FFV1 and PCM codecs, avoiding an optional
  encoder dependency. A drop guard removes the unique temporary media and
  SQLite cache root on success or unwind.
- Assertions cover the public manifest parser and runner, fixed report time,
  algorithm version, candidate retrieval and expectation outcome, extracted
  audio bytes, retrieval statistics, and a populated decision.
- `media-match-generated-media` is a required non-scheduled Ubuntu CI job. It
  installs the Ubuntu ffmpeg package, explicitly verifies both executables, and
  invokes the ignored integration test by its exact target and test name.
- The ignored-test registry, behavior required-job catalog, aggregate gate,
  and adversarial CI policy tests bind the test to that required job. The test
  cannot be silently conditioned, tolerated, renamed, or dropped.

## Validation

Local Windows PATH did not contain ffmpeg or ffprobe, so the tool-backed body
was not represented as a local pass. The required CI lane is the canonical
runtime environment for that proof.

| Command | Result |
| --- | --- |
| `cargo test --locked -p sorotte-media-match --test generated_media_v3 --no-run` | PASS; integration target compiled |
| `cargo test --locked -p sorotte-media-match --all-features` | PASS; 84/84 ordinary tests, one registered capability test ignored by the default command |
| `cargo clippy --locked -p sorotte-media-match --all-targets --all-features -- -D warnings` | PASS |
| `python -m unittest scripts.tests.test_ci_policy -v` | PASS; 13/13 |
| `python scripts/ignored_test_policy.py validate --registry coverage/ignored-tests.toml` | PASS; exact 23-test registry, including five required pull-request tests |
| `python scripts/behavior_evidence.py validate --catalog coverage/behaviors.toml` | PASS; 20 behaviors / 51 proofs / two evidence lanes |

Full repository and committed-source validation is recorded in the central
strategy and final four-slice evidence after all four slices are integrated.
