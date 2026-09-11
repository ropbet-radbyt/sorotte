# Compatibility and terminology cleanup

Base: `7aee810` (0.2.14, including the playlist and synchronization fixes).

Syncplay protocol interoperability and settings import remain first-class features.
Sorotte's Rust crates have no consumers outside this workspace. Their APIs can
change together without aliases or alternate implementations for hypothetical
downstream consumers. This distinction governs the cleanup.

## What `legacy` meant

| Actual concept | Canonical terminology | Disposition |
| --- | --- | --- |
| Current application functionality originally ported from Python | Feature names: `local_commands`, `notifications`, `language`, `session_loop`, `stored_settings`, `stored_config` | Renamed modules and their callers; removed `_legacy_compatible` decoration from ordinary operations. |
| Syncplay protocol versions, version-derived capabilities, password tokens, CLI syntax, settings value serialization, and mpv interface | `Syncplay` / `syncplay` | Retained and named explicitly. Python reference probes remain required verification. |
| Readiness provided by Syncplay's ready flag | `SyncplayReady` / `from_syncplay_ready` | Retained. It does not claim generation-scoped technical readiness. |
| A participant missing the capabilities needed for coordinated start | `ExcludedUnsupported`, `ExcludeUnsupported`, `UnsupportedParticipant` | Retained. GUI explanations describe the missing capability rather than the client's age or product. |
| A participant missing detailed status reporting | `StatusUnsupported` | Retained, distinct from unavailable, waiting, or stale reports. |
| Room control received through ordinary Syncplay playstate | `SyncplayRemoteUser`, `SyncplayLocalEcho` | Retained separately from server barrier and buffering-policy authority. |
| Independent player observation queues versus acknowledged event batches | `TypedQueues`; unsequenced observations | Retained for current fixture adapters; see the remaining consolidation candidate below. |
| Coarse position obtained from a user-list snapshot | List-position fallback | Retained separately from scoped participant telemetry; it cannot create a precise room offset. |
| Existing staged-directory updater input | `StagedDirectory` | Retained with its validation and transactional upgrade tests. This is an installation input format, not Syncplay interoperability. |
| A media index stored directly at its root rather than in a selected generation directory | Direct index root | Retained. This path also initializes new stores, so treating it as obsolete would be incorrect. |
| Persisted rooms without recorded quota ownership | Unattributed room owner | Retained so existing persisted rooms remain accounted for. |
| The general mpv IPC environment variable, distinct from the client-specific variable | `MpvEnv` versus `ClientEnv` | Retained because both are supported configuration inputs. |
| Microsoft UI Automation's `LegacyIAccessible` interface | Its actual Windows API name | Retained unchanged. This is an external platform name. |

The glossary in [CONTEXT.md](../../CONTEXT.md) records the product distinctions.
Cache snapshots and sparse transport telemetry remain separate because absence
means different things in each channel; combining them solely to remove an old
compatibility comment would change observation semantics.

## Removed code and obligations

- The media-open readiness no-op, its runtime/application/GUI forwarding chain,
  and both GUI conditions whose only effect was calling that no-op. These calls
  could neither change readiness nor emit protocol messages. Removed the two
  tests that only asserted this inert API returned nothing; retained actual
  player/readiness lifecycle tests.
- The unused production storage resolver that converted read failures into
  `None`. CLI and GUI already use the checked resolver that reports failures.
- The `StoredClientSettingsMvp` alias and artificial `StoredClientSettingsV1`
  naming. There is now one `StoredClientSettings` DTO and its existing INI format.
- Redundant controlled-room and managed-mpv argument forwarding functions.
- The retired Syncplay update-service client that was compiled only for tests:
  HTTP request construction, WordPress response cleanup, response/status parsing,
  fallback/localized messages, and the `Unknown` update status kept solely for
  those tests. Sorotte update checks use GitHub. The active Syncplay public-server
  directory and its parsing/request tests remain.
- Five downstream Rust source-compatibility fixtures and external API-evolution
  annotations on plain protocol records, participant-status enums, resource
  snapshots, and performance measurements. Removed the corresponding unreachable
  wildcard branches and enum compile-fail examples. The remaining participant
  integration tests are named for their sanitizer and round-trip behavior.
  The two sanitizing presentation constructors retain their construction guards
  and behavioral coverage.
- `RecordingFailure`, its boxing into an I/O error, and its downcast accessor.
  Recorder health, size-limit, and sequence-exhaustion failures are direct
  `EvidenceError` variants. Writer-error redaction and sticky first-failure
  behavior remain covered by the existing failure and concurrency tests.
- The public Rust API compatibility CI job, merge dependency, local verification
  lane, pinned checker, PowerShell wrapper, wrapper tests, and associated step
  registry entries. Removed the preflight restriction that existed only for
  immutable semver exports. Behavioral, interop, coverage, mutation, packaging,
  and native qualification gates remain.

## Boundaries deliberately preserved

The serialized readiness spellings `excludeLegacy`, `excludedLegacy`,
`incompatibleLegacyParticipant`, and `excludedLegacyClients` remain unchanged
through explicit serde names. A regression checks their encoding and the barrier
status round trip. Renaming a Rust concept does not require a wire migration.

Likewise, existing INI keys, QSettings-compatible GUI storage, the mpv
`syncplayintf` interface, the `SYNCPLAY_LEGACY_ROOT` test-harness environment
variable, recorded `.legacy_trace.json` Python reference fixtures, and persisted
`quota:legacy-unattributed` buckets retain their established representations.
Historical audit/evidence documents describe their original snapshots and have
not been rewritten as proof of this cleanup.

Existing settings value aliases and older media-index manifest readers also
remain because they read stored data. Diagnostics now distinguish manifest
versions 1 and 2, a direct index root, and missing retention timestamps.

## Remaining consolidation candidate

The real mpv adapter, including its simulated wrapper, uses acknowledged ordered
batches. Independent typed queues still feed focused fixture adapters and the
disconnected stand-in, and both the core and GUI contain consumers for them.
They are not needed for third-party Rust adapters. Removing the duplicate
consumer path requires migrating those lifecycle tests to acknowledged batches
while preserving their assertions about absent timestamps, partial observations,
and event ordering. This pass names that distinction accurately and leaves this
larger player-event consolidation explicit rather than claiming it was removed.

## Validation

Final checks used the pinned Rust 1.98.1 toolchain and an isolated Python
environment installed from `requirements/legacy-python-interop.txt`. Python
reference tests used Syncplay commit
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`. The interpreter and reference checkout
were selected explicitly through `SYNCPLAY_PYTHON_BIN` and
`SYNCPLAY_LEGACY_ROOT`, with `SYNCPLAY_REQUIRE_LIVE_INTEROP=1`,
`SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1`, and
`SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY=1`.

| Check | Result | Local evidence |
| --- | --- | --- |
| `cargo test --locked --workspace --all-features` | 4,388 passed, 0 failed, 30 ignored; no warnings | `target/workspace-final.log` |
| `scripts/gui-semantic-suite.ps1 -Json -OutputPath target/gui-semantic-final.json` | All 14 scenarios passed, including both live Python peer flows | `target/gui-semantic-final.json` |
| CI policy, verification tools/frontdoor/pins, architecture, coverage lanes, live interop tooling, and Sandbox bundle unit tests | 201 passed | `target/tool-tests-final.log` |
| `python scripts/verify.py preflight --phase static --output target/static-preflight-final.json` | All 16 checks passed | `target/static-preflight-final.json` |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Passed | `target/source-api-clippy.log` |
| `cargo fmt --all -- --check`, `git diff --check`, architecture index verification | Passed | Checked in this worktree |

Logs are retained locally under the ignored `target/` directory. Earlier failed
attempts are also retained: the restricted process sandbox could not complete
process-control/helper checks, and an initial live Python flow selected an
interpreter without Twisted. Host execution and explicit selection of the pinned
Python environment resolved those execution problems without weakening tests.

The 30 ignored Rust tests include standalone-mpv/media and opt-in integration
cases. Physical Windows UI smoke, hosted CI, and release qualification were not
run. These are local results for the uncommitted cleanup worktree, not evidence
for a published release.
