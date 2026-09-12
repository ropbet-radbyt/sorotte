# Dead code and Windows server-fixture cleanup

Base: `b0de9d663c3b44b00ddb44f71561e9db2818956c` (main after v0.2.15).

This pass prioritizes demonstrable mistakes and code with no consumers. The
connection/session planning architecture is outside this change. Syncplay wire
compatibility, settings import, stored-data readers, playback authority and
delivery receipts remain supported behavior.

## Windows nightly failure

[Run 34684982691, job 103530301540](https://github.com/ropbet-radbyt/sorotte/actions/runs/34684982691/job/103530301540)
compiled successfully, then failed
`release_verify_direct_protocol_room_state_chat_playlist_and_fanout`. Alice timed
out waiting for Bob's joined event. Her entire recorded inbound history was a
bare Hello with null `realversion` and `features`, identical to the fixture's
outgoing Hello. The real server sends playlist/readiness snapshots and a Hello
with server metadata.

The startup fixture released an ephemeral listener reservation, spawned the
server and accepted any successful TCP connection as readiness. Windows permits
a TCP connection whose local and remote endpoints are identical. If the client
receives the just-released port before the child listens, that self-connection
echoes the client's writes. The fixture can then accept its own Hello as the
server's reply and wait for a joined event that can never arrive.

A local Windows socket probe reproduced the self-connection and exact byte echo.
A Rust regression then bound a socket, connected it to its own address without a
listener, observed the echoed Hello, and demonstrated that the old fixture
incorrectly accepted it. The regression failed before the fix and passed after it.
The historical CI log records only the peer endpoint, so this is a reproduced
mechanism consistent with that failure, not a recovered trace proving both
endpoints in the original execution.

Both server executable test fixtures now reject identical TCP endpoints. Startup
polling retries that rejected connection within its existing deadline. The
regression also accepts a real listener connection after the rejection. Failure
diagnostics now include both local and remote endpoints. Timeouts and the joined,
chat, readiness, file, playlist, playstate and list assertions are unchanged.

## Removed Rust code

Workspace identifier searches covered ordinary targets, feature-gated code,
tests and fuzz targets. Compilation checks the active target graph; textual
search also covers the orphan source file that compilation cannot see.

- The old 265-line `widget_views/main_window/browser.rs` implementation has no
  module declaration or include. The live room projection remains in
  `widget_views/main_window.rs` and its renderer.
- Removed unused Plex cache accessors, separate match-resolution entry points,
  library forwarding and timeline-interval setter. The interval was never
  changed; the existing constant now supplies the comparison directly.
- Removed unused media-index write/delete forwarding and cache pruning APIs.
  Active inventory invalidation, fingerprint persistence and stored formats remain.
- Removed unused snapshot predicates, scripted batch injection and verification
  harness conversion helpers. Ordered event consumption and adversarial lifecycle
  tests still use their existing active entry points.
- Removed unused application conversion/queue-count accessors, config error
  conversion, capability query and wall-clock reconnect-validation wrapper.
- Removed two unused protocol builders without changing their wire fields.
- Removed the unused server playlist-snapshot builder and its dead-code allowance.
- Removed unused persistence subscription/health APIs from the runtime and actor
  handle, plus the handle's unconsumed clones. Worker failure reporting, recovery,
  durability barriers and their tests remain.
- Removed an unused Python-peer convenience constructor and unused public
  semantic catalog wrappers. Descriptor tests call the existing catalog directly;
  its production catalog rendering remains active.

Sorotte has no external Rust crate consumers, so these APIs do not need source
compatibility aliases or replacement wrappers.

## Removed Python code

Twelve duplicate-key/nonfinite-number callbacks in eight verification scripts
were left behind after their readers adopted `artifact_input`:

- `behavior_evidence.py`
- `coverage_ci_guard.py`
- `coverage_profile_lanes.py`
- `diff_coverage.py`
- `llvm_cov_line_map.py`
- `mutation_ci.py`
- `playback_lifecycle_oracle.py`
- `verify_server_release_artifact.py`

None was still installed as a JSON parser hook or called by another module.
The shared strict parser and each caller's error translation remain. Existing
malformed-input and duplicate-key tests exercise those actual entry points.
`compat_live_interop.py` still uses its local parser hooks, which are retained.

## Nextest evidence path

The all-feature validation exposed another reproducible tooling bug. All 4,386
tests passed, but `nextest_ci.py` failed because it searched for JUnit under the
shared `CARGO_TARGET_DIR`. The real report was in this workspace's
`target/nextest/ci/junit.xml`.

Nextest's [default store](https://nexte.st/docs/configuration/reference/#storedir)
and [JUnit location](https://nexte.st/docs/machine-readable/junit/) are relative
to the workspace, independently of Cargo's build cache. The wrapper now uses
that location for report cleanup, validation and policy evidence. Its config
validator already rejects overrides to the default store. The incorrect Cargo
path helper and its import are removed.

The regression covers absolute and relative shared build paths, a fresh report,
and a producer that exits successfully without producing a report. It requires
stale workspace evidence to be removed before execution and unrelated evidence
in the shared build directory to remain untouched. All four cases failed
against the old wrapper and passed after the fix. Existing missing-report,
malformed-report, flaky-result and leaked-process policies remain enforced.

## Validation

Initial local evidence is retained under `target/audit-evidence/`:

- `34684982691-103530301540.log`: original failed job log.
- `self-connect-baseline.log`: deterministic regression fails against the old
  fixture's acceptance behavior.
- `self-connect-fixed.log`: the same socket mechanism is rejected after the fix.
- `unused-api-candidates.json`: initial conservative identifier inventory.
- `nextest-all-features/`: original passing producer output and failed wrapper
  receipt, including a preserved copy of the JUnit from its actual location.
- `nextest-location-baseline.log` and `nextest-location-fixed.log`: failing
  path regression followed by all 19 nextest policy tests passing.

Completed local checks:

- Formatting and `git diff --check` passed.
- Default and all-feature workspace Clippy passed with warnings denied.
- `cargo test --locked --workspace`: 4,301 passed, 23 ignored.
- Pinned nextest 0.9.143 with the corrected wrapper: 4,386 passed, 30 skipped,
  zero failed/flaky/leaked tests; policy receipt passed. The run retained the
  required retry, fail-on-flaky and leak-detection settings.
- All-feature workspace doctests passed.
- `scripts/server-release-verify.ps1 -NoWorkspace`: passed, including server
  package tests, live Python interoperability and all 11 strict matrix tests.
- Static preflight passed with normal process permissions. The restricted
  account correctly failed the owned process-control prerequisite.
- The full Python suite passed (1,214 tests, four skips). After adding the
  nextest path regression, all 47 nextest and CI policy tests passed.

The first full Python run also emitted background subprocess decoding errors
while reporting success. Neither the isolated shell execution tests nor a
verbose full-suite diagnostic repeat reproduced them; no speculative decoding
change is included. Both full-run logs are retained.

These local checks ran against the working patch. The server report's
`sourceSha` field records the base HEAD above. Hosted and native check results
for the committed source are recorded on the pull request.
