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
- The 120-second fixture combines ffmpeg's built-in FFV1 and PCM codecs with
  fixed-seed broadband noise, avoiding optional encoders and periodic audio
  ambiguity. Its duration makes the production sampled-fast policy decode all
  three non-overlapping 20-second body windows. A drop guard removes the unique
  temporary media and SQLite cache root on success or unwind.
- Assertions cover the public manifest parser and runner, fixed report time,
  algorithm version, exact 120-second ffprobe duration for query and candidate,
  positive verification/index landmarks and decoded ffmpeg PCM bytes for both,
  all three sampled-fast windows, positive retrieval buckets, candidate
  retrieval and expectation outcome, and a populated decision. A typed
  expectation assertion records the failure reason, rank, tier, class, decision
  notes, and top retrieval diagnostics before the JSON serialization checks.
- `media-match-generated-media` is a required non-scheduled Ubuntu CI job. It
  installs the Ubuntu ffmpeg package, explicitly verifies both executables, and
  invokes the ignored integration test by its exact target and test name.
- The ignored-test registry, behavior required-job catalog, aggregate gate,
  and adversarial CI policy tests bind the test to that required job. The test
  cannot be silently conditioned, tolerated, renamed, or dropped.

## Hosted RED and correction

Required job `91093403053` in workflow run `30610965479` proved that the
original 30-second, 440 Hz sine fixture was retrieved but did not satisfy its
`Probable` expectation. Tool installation, version probes, fixture generation,
fingerprinting, and retrieval had all completed before the opaque JSON boolean
assertion failed.

Production decision inspection found two structural weaknesses in that fixture:

- A 30-second input schedules only one 20-second sampled-fast window. STFT
  landmark timestamps occupy less than that full window, while a `Probable`
  same-cut decision requires at least 20 seconds of aligned span.
- A stationary sine repeats the same frequency relationships over time, which
  can reduce the required best-offset margin by populating competing offsets.

The corrected 120-second fixed-seed broadband fixture exercises three
non-overlapping sampled-fast windows and produces time-specific hashes across
them. The minimum tier remains `Probable`; the fixture now represents the
production decision contract instead of weakening it. Typed failure diagnostics
also make any future tier, class, offset-margin, or retrieval regression visible
in the hosted log.

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
