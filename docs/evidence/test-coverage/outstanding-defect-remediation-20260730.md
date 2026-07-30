# Outstanding defect remediation evidence

Date: 2026-07-30

Branch: `codex/test-coverage-design`

Platform: Windows, Rust 1.97.1 workspace toolchain

## Scope

This slice resolves every defect present in `coverage/known-defects.toml` at
the start of implementation:

| Defect | Implemented behavior |
|---|---|
| `TC-CLIENT-002` | Reconnect reset cancels connection-scoped local-pause and room-pause transactions and restores healthy reducer state. |
| `TC-PROTOCOL-002` | Nested `Set` order is scanned from the last surviving duplicate payload while top-level first-position/last-value compatibility is preserved. |
| `TC-PROTOCOL-003` | `DecodedMessageLineItem::Debug` renders only exact known command names and uses a fixed marker for unknown names. |
| `TC-GUI-001` | Version probes require a strict, tool-specific UTF-8 ffmpeg/ffprobe banner. |
| `TC-GUI-002` | Version probes drain stdout and stderr concurrently, retain at most 64 KiB per pipe, and still kill, join, and reap on timeout. |
| `TC-PLEX-001` | Plex parts are narrowed by exact basename, folded basename, exact size, then closest known duration. |
| `TC-GUI-003` | Genuine Plex ambiguity is terminal for its exact automatic-resolution context, warns once, and has no retry deadline. |

No public `PlexError` variant was added: a repository source-compatibility
test proves downstream exhaustive matches are supported. Plex instead exposes
the canonical ambiguity classification through a method, and the GUI converts
that result to a typed `PermanentForContext` failure before formatting its
message.

## Positive regression conversion

Every former expected failure is now an ordinary passing test:

```text
reconnect_reset_matches_a_fresh_reference
reconnect_reset_rejects_stale_reducer_completions
duplicate_top_level_set_uses_surviving_payload_order
decoded_item_debug_redacts_credential_bearing_unknown_command
version_probe_rejects_unusable_success_output
version_probe_drains_finite_output_larger_than_pipe_capacity
resolver_uses_filename_and_size_evidence
permanent_plex_ambiguity_warns_once_without_automatic_retry
```

`decoded_item_debug_preserves_supported_command_names` separately locks the
exact diagnostic allowlist so redaction cannot silently hide supported
commands.

The registry policy reports:

```text
valid known-defect registry: 0 defects, 0 characterizations
```

Its dedicated 21-test policy suite passes, including the explicit-empty
registry positive control and unregistered-characterization fail-closed
controls.

## Adversarial and stress proof

- The complete media-tool process-fault module passed 25/25 repetitions. Each
  repetition uses real child processes for exit-zero malformed output,
  nonzero exit, unterminated output, timeout/kill/reap, and 512 KiB writes to
  both stdout and stderr. The large-output oracle requires a successful exit,
  complete concurrent drain, exact bounded captures, and both truncation
  markers; any process or pipe error fails the test.
- Reconnect regressions reject both stale success and stale failure
  completions, then prove a new post-reset transaction can start.
- The Plex evidence selector passed 50/50 repetitions. Its 20
  forward/reverse cases per repetition provide 1,000 resolver observations
  agreeing with the independent test oracle.
- The terminal Plex ambiguity schedule passed 50/50 repetitions. Every run
  retained one terminal attempt, one warning, one system-chat announcement,
  no deadline, and a projected `Failed` source state.
- Genuine duplicate and unidentified multipart Plex parts still fail closed.
  Duration still breaks a remaining filename/size tie.
- The existing transient Plex miss regression still proves bounded retry and
  later successful activation.

## Broad validation

The integrated tree passed:

```text
cargo fmt --all -- --check
cargo test --locked -p sorotte-client-core --all-features
cargo test --locked -p sorotte-protocol --all-features
cargo test --locked -p sorotte-plex --all-features
cargo test --locked -p sorotte-gui --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python -m unittest discover -s scripts/tests -p "test_*.py" -v
powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
```

The final full workspace test command completed successfully in 214.1 seconds.
Warning-denied workspace Clippy completed successfully in 6.11 seconds. The
Python policy suite passed 364/364 tests, and the semantic GUI suite passed
14/14 scenarios.

The first strict native GUI run preserved a UI Automation navigation timeout
at:

```text
target/verification/gui-native-smoke/20260730T045048424Z-64584
```

No test GUI process remained. One isolated retry against the same rebuilt
binary then passed the complete required ten-scenario native contract:
AccessKit menu and accessibility inventories, physical menu input, detached
and attached Open Media behavior, live-Python and controlled-room paths,
missing-media continuation, observable File -> Exit shutdown, zero stderr,
and bounded closure. The accepted report recorded 100 accessible names and
completed in 112,744 ms. Both the initial diagnostic and passing retry are
retained; no product code was changed in response to the timing-only failure.

## Result

All seven registered defects are resolved, their regression oracles are
positive, the defect registry is explicitly empty, and no additional product
defect surfaced during remediation. The historical failing measurements
remain in `plex-part-selection-retry-20260730.md`; the implemented design and
trade-offs remain in `docs/OUTSTANDING_DEFECT_REMEDIATION_DESIGN.md`.
