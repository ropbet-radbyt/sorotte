# Coverage-guided framed mpv IPC and transcript evidence — 2026-07-31

## Scope and safety

This slice adds a bounded `libFuzzer`/AddressSanitizer target over Sorotte's
local Rust mpv JSON framing, command-response classification, queued-event
ordering, transcript projection, and attachment/media-generation fencing.
Generated framed input is supplied only to an in-memory test seam. The target
does not open a socket, named pipe, file-backed player endpoint, child process,
network target, credential store, persistent service, or privileged resource.

The fixed production request is always:

```json
{"command":["get_property","pause"],"request_id":1}
```

The input is capped at 65,536 bytes and 64 derived newline-delimited frames.
The runner uses one fuzzer job, a five-second per-input timeout, a 2,048 MiB
RSS limit, a maximum 900-second campaign, exact source and seed manifests, and
an always-written structured report. This is defensive QA of repository-owned
parsing and session code, not network reconnaissance or exploitation.

Implementation commit:

```text
dedb0736c97561780cdd6250b12704bdfc4ca5c7
```

## Generated schedule and oracle contract

The first four control bytes choose independent axes while the remainder is
the bounded framed payload.

Chunking has exactly four modes:

1. one coalesced chunk;
2. one byte per chunk;
3. fixed-width chunks from 1 through 31 bytes; and
4. deterministic xorshift-derived chunks from 1 through 47 bytes.

The scripted transport ends in exactly one of five modes:

1. EOF;
2. read timeout;
3. read disconnect;
4. write timeout; or
5. write disconnect.

An independently implemented line oracle classifies successful matching
responses, server rejection, timeout, disconnect, and protocol corruption. It
retains event order and duplicates before the matching response, requires the
fixed request ID and response error field, and stops at the response barrier.
The production and reference outcomes, successful response payload, and
queued events must match exactly.

The second independent layer derives at most 16 transcript records and proves:

- nonzero attachment epochs and monotonic ingress/receipt order;
- exact command and playlist-entry identity;
- canonical raw-JSON SHA-256 projection;
- replay-batch equivalence and stable record order; and
- closed transcript validation rather than acceptance of a malformed record.

The lifecycle layer uses the production verification harness to require a new
attachment epoch after replacement, removal of the prior attempt, clearing of
the prior physical-media generation, and distinct logical media generations
without resurrection of the first attempt.

The 12 committed seeds are:

```text
bytewise-event-before-response.txt
bytewise-partial-read-timeout.txt
bytewise-response-id-reorder.txt
coalesced-invalid-json.txt
coalesced-success.txt
coalesced-write-disconnect.txt
fixed-width-dropped-response-disconnect.txt
fixed-width-duplicate-events.txt
fixed-width-server-rejection.txt
pseudorandom-malformed-json.txt
pseudorandom-write-timeout.txt
response-barrier-ignores-trailing-malformed.txt
```

## Preserved RED oracle counterexample

The first 30-second campaign is preserved at:

```text
target/fuzz-ci/mpv-framed-transcript-smoke-20260731-v1
```

It failed after 54 executions and 13 new units with one 69-byte artifact:

```text
production outcome Disconnected disagrees with reference ProtocolCorruption
```

Exact RED identities:

```text
source:                  2084a24d7408063baa4b07b4e29af3bbd2cf68b7
source files:            64 before / 64 after, stable
source aggregate:        3b2f35bebd72ca969766e1eb1d34e59d9359104760ebaa0fc894e578c3f0ff04
seed files:              12 before / 12 after, stable
peak RSS:                57 MiB
artifact:                crash-c5663af9617ce737cff61574ff2a0a567a11bc3f
artifact SHA-256:        4d41ace74be6233d002d181954c97a674f64751f2819e92fbd1f52cdcea0336f
artifact aggregate:      f45139c745413ebd00a97acac618bcc2426d8eddc712a2fbee617fb86ac31d83
evidence errors:         0
run-report SHA-256:      96ede4edfe8b2c36df1c79ef7da58107e52beb278c586730af18385e4cd5da03
fuzz.log SHA-256:        5e063eb0b3d5405ddfc959537c05da053e4b0b60447bbc7fed5f8c156d781bc6
```

This was an oracle defect, not a production parser defect. Production's
`read_line_with` decodes a buffered unterminated line only when the transport
returns EOF. A non-EOF read error discards that partial transport frame and is
classified as disconnected. The original reference incorrectly decoded the
same partial bytes on read disconnect.

The correction is narrow: only EOF may send trailing partial bytes through the
reference line decoder. Newline-complete malformed JSON remains protocol
corruption, and trailing malformed JSON on EOF remains protocol corruption.
Read timeout, read disconnect, write timeout, write disconnect, response-ID
rules, event ordering, the response barrier, transcript validation, and
lifecycle fencing were not weakened.

The automatic `tmin` attempt returned 1 after its zero-byte candidate triggered
libFuzzer's `SetMaxInputLen` assertion. Its zero-byte output is retained under
`minimized/`; it is not represented as a valid minimized counterexample. The
original 69-byte artifact remains the authoritative RED input. No RED output
was overwritten or deleted.

## Short post-correction GREEN

The distinct rerun is preserved at:

```text
target/fuzz-ci/mpv-framed-transcript-smoke-20260731-v2
```

It passed the 30-second limit with:

```text
executions:              58,462
average executions/sec: 1,885
new units:               1,172
final corpus:            588 files / 31,428 bytes
peak RSS:                442 MiB
artifacts:               0
artifact aggregate:      4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945
source files:            64 before / 64 after, stable
source aggregate:        afccf720e1b276254b0fd6309583b6206d8c1095ee0be6f34f786590e5d12e14
seed files:              12 before / 12 after, stable
evidence errors:         0
run-report SHA-256:      febb548f53bd13e6c3369c3589b86895dcfa95a728f4cfc9063c2bcde98803b5
fuzz.log SHA-256:        0380db6921bff14a464b29a6aefd02462339a0ac5ef6566c85c650fdb1affe7b
```

This short run used the historical base SHA while the reviewed source changes
were still uncommitted. It proves the corrected oracle was immediately green,
but it is not the canonical committed-source campaign.

## Canonical committed-source campaign

After all four slice implementations and the compatibility execution-path
follow-up were committed, a fresh output path was confirmed absent and this
campaign ran:

```text
wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc \
  "python3 fuzz/run_protocol_fuzz.py \
    --target mpv_framed_transcript \
    --toolchain nightly-2026-07-29 \
    --source-sha 3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16 \
    --seconds 180 \
    --seed-corpus crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript \
    --expected-seed-count 12 \
    --output-root target/fuzz-ci/mpv-framed-transcript-deep-3cd64ce-v1"
```

The report at
`target/fuzz-ci/mpv-framed-transcript-deep-3cd64ce-v1/run-report.json`
recorded:

```text
status / exit:           passed / 0
source SHA:              3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16
sanitizer:               address
configured duration:     180 seconds
executions:              322,973
average executions/sec: 1,784
new units:               3,219
slowest unit:            0 seconds
peak RSS:                451 MiB
final corpus:            1,232 files / 71,206 bytes
final corpus aggregate:  ab9ea3b67535b37dd6c7e742bf4c8a1aeb5c4a4096c122c89f5b702443ffa4aa
artifacts:               0 files / 0 bytes
artifact aggregate:      4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945
source files:            64 before / 64 after, stable
source aggregate:        4bda7b371635977346cfef77d433c62f022065803637de83bc8167b262526cd2
seed files:              12 before / 12 after, stable
evidence errors:         0
minimization:            not required
```

Independent filesystem counts matched the report: zero artifact files and
1,232 corpus files.

Canonical bundle identities:

```text
c48a7a6c86972707d513972b0fd91f8f5af3becfc1fa7b10b0e7ebe073f532e5  run-report.json
328db191d7cf551b2836166379013282b9989153b6006ba3bcd7ea9c5f7ff8bb  fuzz.log
```

Tool identities recorded by the report:

```text
rustc 1.99.0-nightly (26ae60a9e 2026-07-28), LLVM 22.1.8
cargo 1.99.0-nightly (3efb1f477 2026-07-17)
cargo-fuzz 0.13.2
Python 3.12.3
Linux 6.6.87.2-microsoft-standard-WSL2 x86_64, glibc 2.39
```

## Four-slice continuation and final implementation-source campaign

All intermediate bundles remain preserved. The first continuation attempt at
`ad410fc` was stopped by an outer 120-second operator bound while Cargo was
still building. Its report therefore remains honestly `running` with no live
process and is diagnostic only:

```text
target/fuzz-ci/mpv-framed-transcript-deep-ad410fc-v1
9c1b45316fcb1a749e5d24def3f75ee7681f184445e8f6802b0cc6683e6fc162  run-report.json
760778c53709e37db710839bb6efbde1ad8f60e5f47298f50edf0b9ece86b5e2  fuzz.log
```

A direct native-Windows attempt at `6ccfd3a` built successfully and then
failed before executing a unit because the target process could not load its
ASan runtime (`0xc0000135`, `STATUS_DLL_NOT_FOUND`). Pointing it at an older
Visual Studio ASan directory produced `DLL_INIT_FAILED`, confirming an
incompatible local runtime rather than a target result. The report retained
12 seed files, stable 65-file source bindings, and zero artifacts:

```text
target/fuzz-ci/mpv-framed-transcript-deep-6ccfd3a-v1
1628b29a2af23c71c76e29a0cf28c0da5645e35a2245ea209905538d275ad1f3  run-report.json
705946f59d18be928328dead3179ef44aef246f3f5af909b0f8eeff588528506  fuzz.log
```

This is `TC-HARNESS-039`, an operator-environment diagnostic. Native Windows
was not the documented canonical execution platform, so no source or workflow
was changed to make that attempt green.

The documented Ubuntu WSL command then passed at `6ccfd3a`: 260,930
executions, 2,754 new units, 1,113 corpus files / 60,648 bytes, 452 MiB peak
RSS, stable source and seeds, and zero artifacts. Its checkpoint identities
are:

```text
84983b5d40d168c6b6c577251b4599d1d4c8a44af821e805ec2bf978c01cde57  run-report.json
7c03fa8ec4d73f5b3cc82cb5eda0d5e66df8351ada645c89184e747590f9c496  fuzz.log
69e40436393f2289206d4b93843dc071739415e43c7729f08f18f6642e549fa1  final corpus aggregate
```

After the last implementation correction was committed, a fresh canonical
path bound the full SHA:

```text
wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc \
  "python3 fuzz/run_protocol_fuzz.py \
    --target mpv_framed_transcript \
    --toolchain nightly-2026-07-29 \
    --source-sha 9f3cb60fbe788575829931b56155f4bc0c19caf0 \
    --seconds 180 \
    --seed-corpus crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript \
    --expected-seed-count 12 \
    --output-root target/fuzz-ci/mpv-framed-transcript-deep-9f3cb60-wsl-v1"
```

The final implementation-source report records:

```text
status / exit:           passed / 0
source SHA:              9f3cb60fbe788575829931b56155f4bc0c19caf0
sanitizer / duration:    address / 180 seconds
executions:              277,044
average executions/sec: 1,530
new units:               2,775
slowest unit:            0 seconds
peak RSS:                451 MiB
final corpus:            1,105 files / 68,351 bytes
final corpus aggregate:  851e0de6fcb558e6ba981a73e55f4b73ab3d1620d2c3da72dec340f37ea5ba57
artifacts:               0 files / 0 bytes
artifact aggregate:      4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945
source files:            65 before / 65 after, stable
source aggregate:        b15e50a01cfe0e50379372026e01f64882396aee21a0c585790aeea4d777f924
seed files:              12 before / 12 after, stable
evidence errors:         0
minimization:            not required
```

Final bundle identities:

```text
4a29518400180e5bb49adee30d40292be9596da800c5132d4bb443a4acafa744  run-report.json
42cbeca1ac73f6040317991e8a6b5dccf7313d2b0fdb09a69cb71759967bcfe0  fuzz.log
```

## Exact-head coverage-policy refresh campaign

The later coverage-map policy correction changed no framed-mpv target source,
but exact source provenance required a new campaign at the resulting commit.
The fresh bounded command was:

```text
wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc \
  "python3 fuzz/run_protocol_fuzz.py \
    --target mpv_framed_transcript \
    --toolchain nightly-2026-07-29 \
    --source-sha 829ab9824d20bc64b03179646c5e182d5c7a4bfb \
    --seconds 180 \
    --seed-corpus crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript \
    --expected-seed-count 12 \
    --output-root target/fuzz-ci/mpv-framed-transcript-deep-829ab98-wsl-v1"
```

The exact-head report records:

```text
status / exit:           passed / 0
source SHA:              829ab9824d20bc64b03179646c5e182d5c7a4bfb
sanitizer / duration:    address / 180 seconds
executions:              326,303
average executions/sec: 1,802
new units:               3,220
slowest unit:            0 seconds
peak RSS:                451 MiB
final corpus:            1,190 files / 66,395 bytes
final corpus aggregate:  e9f946720bf6576a8133eddc92d54df7c6eff660daa31ef338e90834e1c0d987
artifacts:               0 files / 0 bytes
artifact aggregate:      4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945
source files:            65 before / 65 after, stable
source aggregate:        b15e50a01cfe0e50379372026e01f64882396aee21a0c585790aeea4d777f924
seed files:              12 before / 12 after, stable
evidence errors:         0
minimization:            not required
```

Independent filesystem counts matched the report: zero artifact files and
1,190 corpus files. The tool identities remain the pinned nightly LLVM 22.1.8,
cargo-fuzz 0.13.2, Python 3.12.3, and Ubuntu WSL2 environment recorded by the
structured report.

Exact-head bundle identities:

```text
cf32c5060accd566f51d5154a2bf30cd7d564009b17ed9152468d09cf1b2b65f  run-report.json
48bdc7bbd2a355458ef1799b6013efc04bf92724748e2c8554f45d7cbee3d55b  fuzz.log
```

## Deterministic and policy validation

The owning deterministic tests and target build passed:

```text
cargo test --locked -p sorotte-player-mpv --all-features \
  transcript::tests:: -- --nocapture
# 9/9 passed

cargo test --locked -p sorotte-player-mpv --all-features \
  transcript -- --nocapture
# 15/15 passed

cargo test --locked -p sorotte-player-mpv --all-features --no-run
# passed

cargo clippy --locked -p sorotte-player-mpv --all-targets \
  --all-features -- -D warnings
# passed

cargo +nightly-2026-07-29 fuzz build --fuzz-dir fuzz \
  --sanitizer address mpv_framed_transcript
# passed under Ubuntu WSL
```

The policy suite passed `20/20`. It binds the exact target name, checked-in
12-file corpus, source paths, nightly date, cargo-fuzz 0.13.2, ASan, durations,
limits, source SHA, upload-on-failure behavior, action pins, and fail-closed
report semantics. Both changed workflows passed actionlint. Formatting,
Python syntax, and `git diff --check` also passed.

The workflow runs 45 seconds for pull requests and pushes and 900 seconds for
scheduled/manual execution, within a 25-minute job timeout. It uploads the
complete evidence directory even on failure and rejects missing artifacts.

Source file identities after review:

```text
6aa284a306fbc9ef57890229f870772d31e67acd69a8a8bbf34808da48a6fe34  fuzz/fuzz_targets/mpv_framed_transcript.rs
7f4180a7a85dee6cadd213d48623e9003383e1b683953fa7006a861fb2d5aba8  fuzz/run_protocol_fuzz.py
3f15fc7488f8bcc3e9b3e28164d4b4a280b1ec4b5211f7a61f24d638e73918ba  scripts/tests/test_protocol_fuzz_policy.py
0449123c5a3443b10c1875a8399757d00aad53b716344cb80c1f9909bac96567  crates/sorotte-player-mpv/src/ipc.rs
```

Final integration passed all 496 Python infrastructure/policy tests, the
10-shard mutation policy, the empty known-defect registry, repository
formatting and diff checks, both changed workflows under actionlint,
warning-denied all-target/all-feature workspace Clippy in 15.8 seconds, and
the complete locked all-feature workspace test suite on its first attempt in
257.5 seconds.

## Limitations

- The target crosses the production Rust line-reader and transcript/lifecycle
  seams but uses an in-memory scripted transport; it does not claim kernel
  named-pipe or Unix-domain-socket timing.
- The fixed command exercises one serializable `get_property pause` request.
  It does not generate the complete mpv command grammar.
- ASan was the only sanitizer used in the canonical campaign. This is not
  Miri, ThreadSanitizer, MemorySanitizer, or whole-workspace fuzz coverage.
- A finite 180-second campaign and 12 seeds cannot prove absence of every
  parser, ordering, or lifecycle defect. The scheduled 900-second lane extends
  search depth but retains the same bounded oracle.
- The RED counterexample was an independently diagnosed oracle mismatch. It
  did not justify or produce a production behavior change.
