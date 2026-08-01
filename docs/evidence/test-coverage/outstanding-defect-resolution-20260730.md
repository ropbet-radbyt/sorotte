# Outstanding test-coverage defect resolution evidence — 2026-07-30

## Result

The two defects that remained after the coverage-guided parser tranche are
resolved on `codex/test-coverage-design`:

| Defect | Production correction | Durable positive proof |
| --- | --- | --- |
| `TC-CLI-003` | partial inbound frame bytes now live in a connected-session-owned `InboundProtocolLineReader`, outside the cancellable `tokio::select!` read future | forced cancellation after an observed application prefix and after an observed framing `\r` both retain the prefix and accept the released valid Hello |
| `TC-PROTOCOL-004` | workspace and fuzz builds enable serde_json 1.0.151's `float_roundtrip` parser feature | raw `70E70` and typed `{"State":{"playstate":{"position":70E70}}}` decode/encode/decode exactly, and both minimized inputs are checked-in corpus seeds |

The four former `#[should_panic]` characterizations are ordinary positive
regressions. `coverage/known-defects.toml` explicitly contains `defect = []`,
and the executable inventory validator reports `0 defects, 0
characterizations`.

## Scope and safety

This work is defensive QA of Sorotte's own local Rust protocol parser and
test-owned loopback session boundary. There is no public network target,
reconnaissance, credential access, persistence, privilege change, or
third-party interaction. Every loopback operation remains bounded by the
existing three-second harness deadline. Fuzz input is local, capped at 65,536
bytes, limited to one job, bounded to five seconds per input and 2,048 MiB RSS,
and run only against public `sorotte-protocol` decode and encode functions.

## TC-CLI-003 correction

The historical defect was caused by ownership, not by `BufReader` itself.
`read_inbound_protocol_line` consumed available bytes into a `Vec` owned by
the read future. When another connected-session `tokio::select!` branch won,
dropping that future also dropped bytes already consumed from the transport.

`InboundProtocolLineReader` now owns the partial frame. The connected session
creates one reader state before entering its loop and each selected read
borrows it. Cancelling the borrow therefore cannot discard the accumulated
prefix. A completed frame moves the buffer out for UTF-8 decoding; terminal
I/O and line-limit errors clear it. The existing one-shot
`read_inbound_protocol_line` wrapper retains the same API for STARTTLS and
unit callers that do not resume a cancelled read.

The two deterministic production-boundary tests are now:

```text
tests::raw_protocol_framing::one_byte_fragmentation_survives_select_cancellation
tests::raw_protocol_framing::split_crlf_survives_select_cancellation
```

Both still prove that bytes were consumed and the read future was cancelled
before the test-owned peer releases the rest of the valid frame. They now
require `ConnectedSessionExit::TransportClosed` after the complete Hello is
accepted.

## TC-PROTOCOL-004 correction

The minimized finite JSON input `70E70` previously decoded to one `f64`,
serialized with ryu, and decoded to its adjacent representation. Enabling
serde_json's `float_roundtrip` feature selects the exact-decimal-to-binary
conversion required for the emitted representation to decode to the same
bits. No accepted numeric syntax is rejected or clamped, and the serialized
wire shape remains standard JSON.

The former characterizations are now:

```text
tests::raw_floating_point_roundtrip_is_exact
tests::typed_state_floating_point_roundtrip_is_exact
```

The deterministic parser corpus also retains:

```text
json-float-roundtrip.json
typed-state-float-roundtrip.json
```

This expands the explicit manifest from 14 to 16 files. The fuzz target's
recursive one-ULP continuation classifier was deleted. Raw `Value` and typed
`ProtocolMessage` roundtrips again use unconditional `assert_eq!`.

## Post-fix coverage-guided campaign

Implementation SHA:

```text
034e10511ae6473f0165f3028a026a0bad4f6db3
```

The first native Windows attempt was preserved at
`target/fuzz-ci/protocol-line-defect-fixes-034e105-v1`. It did not execute an
input: the ASan fuzz executable exited with Windows
`STATUS_DLL_NOT_FOUND (0xc0000135)`. The runner recorded zero artifacts,
stable 29-file source and 16-file seed manifests, and no evidence-collection
error. This is a local sanitizer-runtime launch limitation, not a parser
failure.

The canonical rerun used the previously proven WSL path:

```text
wsl.exe -d Ubuntu \
  --cd /mnt/c/tmp/sorotte-test-coverage-design \
  bash -lc "python3 fuzz/run_protocol_fuzz.py \
    --toolchain nightly-2026-07-29 \
    --source-sha 034e10511ae6473f0165f3028a026a0bad4f6db3 \
    --seconds 180 \
    --seed-corpus crates/sorotte-protocol/tests/corpus/protocol_parser \
    --expected-seed-count 16 \
    --output-root target/fuzz-ci/protocol-line-defect-fixes-034e105-wsl-v1"
```

Result:

| Statistic | Value |
| --- | ---: |
| status | passed |
| fuzzer exit | 0 |
| executed units | 1,994,358 |
| average executions/second | 10,958 |
| new units | 7,163 |
| slowest unit | 0 seconds |
| peak RSS | 533 MiB |
| final corpus | 1,987 files / 429,068 bytes |
| artifacts | 0 |
| evidence errors | 0 |

The run started at `2026-07-30T09:07:38.474838+00:00` and finished at
`2026-07-30T09:11:42.356894+00:00`, including setup and evidence collection.

Attestations:

| Evidence | Identity |
| --- | --- |
| bound source before/after | 29 files / 363,262 bytes |
| bound source aggregate | `bb80ae1203cfdd754ab8bde7172e24c960a73fd7a566bcae31e1534c716eeda8` |
| seed source | 16 files / 866 bytes |
| seed aggregate | `438c044fd552e7b2b6d7dd7633e99bcadca5fb0e6ceff1b4eee8522ff8a81909` |
| final corpus aggregate | `70d4e19723402ecceab30b692513dab5c3b75e43ce9b69ccc0ed5d488e6118ed` |
| empty-artifact aggregate | `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945` |
| report | 383,122 bytes / `cfd3909ad7b0f378ffa4dc7dd9ca09ad0d5c5c5abff7976db125e274fd69a26b` |
| log | 894,558 bytes / `21e589c44e108cfe07f3f51e46ad6920f6afe97bac927031ffbc9e6c194d162a` |

Source and seed manifests were byte-identical before and after execution.

The preserved native launch attempt is bound by:

| Evidence | Identity |
| --- | --- |
| report | 22,275 bytes / `52aea6284ca65738bc76ec1d2f4a0446fcbbdb4afbf9468348a241f9b170dbeb` |
| log | 1,361 bytes / `44ab33c32056663678721cab3c99ab008ececfcc13f9b96f0c6a586e3929dd11` |

## Tool identities

```text
cargo-fuzz 0.13.2
rustc 1.99.0-nightly (26ae60a9e 2026-07-28)
rustc commit 26ae60a9eeb20b4935be49d7a931a650fa1d2923
cargo 1.99.0-nightly (3efb1f477 2026-07-17)
LLVM 22.1.8
Python 3.12.3
Linux 6.6.87.2-microsoft-standard-WSL2 x86_64
```

## Validation

Focused validation completed before the final workspace gate:

| Check | Result |
| --- | --- |
| CLI raw loopback framing selector | 5/5 positive tests |
| CLI raw framing stress | 50/50 matrices; 250/250 test executions |
| complete protocol package | 88 library + 6 parser integration tests |
| deterministic corpus stress | 50/50 runs; 800/800 file replays |
| protocol fuzz plus known-defect policy suites | 34/34 |
| known-defect registry validation | 0 defects / 0 characterizations |
| pinned nightly ASan fuzz build | passed |
| post-fix 180-second exact-oracle campaign | 1,994,358 executions / 0 artifacts |
| formatting and diff whitespace | passed |
| strict all-target/all-feature workspace Clippy | passed in 22.173 seconds |
| complete Python infrastructure/policy suite | 399/399 in 20.427 seconds |
| direct mutation policy | 8 shards / 16 accepted-unviable identities |
| behavior catalog | 20 behaviors / 51 proofs / 2 lanes |
| ignored-test policy | 23 exact tests |
| fuzz and mutation workflow actionlint | passed |
| complete locked all-feature workspace retry | passed in 208.298 seconds |

The first complete workspace attempt stopped after 125.962 seconds at the
independent updater-harness race `TC-HARNESS-016`: the parent observed the
`boundary-reached` marker while its contents were still empty and expected
`replaced-6`. The exact test then passed alone, failed at iteration 5 of a
serial stress, and passed a subsequent 20/20 diagnostic capture. No updater
source was changed. The complete workspace retry passed unchanged, including
that test, every integration and release-verification target, and all
doctests. The finding and its existence-versus-content race are recorded in
the central findings ledger.

## Limits

The fix and evidence do not make the parser campaign exhaustive. One
protocol-line target is not framed-transport, reconnect, TLS, server-dispatch,
or mpv-IPC fuzzing. The WSL AddressSanitizer run does not establish
Windows-specific sanitizer execution, and the native launch failure above is
retained explicitly. Deterministic loopback tests remain the proof of
connected-session cancellation behavior. These changes do not claim
operating-system durability, real-player integration, or GUI rendering and
accessibility coverage. `TC-HARNESS-016` remains an independently
characterized test-handshake defect outside the deterministic expected-failure
registry.
