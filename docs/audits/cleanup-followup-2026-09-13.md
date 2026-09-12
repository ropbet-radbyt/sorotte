# Cleanup follow-up

This pass audited `453d8396c18efed97a02df35a8dd4c608ee0a290`, main after
[PR #70](https://github.com/ropbet-radbyt/sorotte/pull/70), and implements the four
remaining candidates identified there. The earlier compatibility, player,
settings and dead-code audits were read first.

## Unconsumed protocol diagnostics

The client and server each collected eight kinds of compatibility diagnostic,
retained up to 128 records, and bounded dynamic text fields to 512 bytes.
Every call to either `drain_compatibility_fallbacks` API was in a unit test.
CLI, GUI, networking, simulation and Python interoperability had no consumer.
This was bounded unused work; no memory leak or performance improvement is claimed.

The diagnostic enums, truncation helpers, normalizer wrappers, collection/context
plumbing, retained queues and drain APIs are removed together. The
[client normalizer](../../crates/sorotte-client-core/src/inbound.rs) and
[server normalizer](../../crates/sorotte-server/src/inbound.rs) now return their
commands directly. Syncplay capability defaults, sanitization, command ordering
and rejection of malformed extensions retain their existing behavior.

Tests retain assertions about accepted filenames, dropped invalid metadata,
capability defaults and barrier/readiness state. The
[client participant-status tests](../../crates/sorotte-client-core/src/session/tests/participant_status_tests.rs)
and [server counterpart](../../crates/sorotte-server/src/tests/participant_status_tests.rs)
now verify that a malformed update leaves accepted state intact and that a later
valid update is accepted. A malformed client extension also accompanies valid
playstate to verify that unrelated state still applies. Untrusted diagnostic
text must remain absent from retained debug state. Malformed scope handling and
snapshot invalidation are preserved.

## CLI reconnect ownership

The removed shared network-loop planner contained 14 functions and 16 mapping
tests, plus separately exported types. Only the CLI executed it. Connection
outcomes crossed event, attempt, source, plan, execution, disposition and
error-action representations; a separate optional error allowed impossible
failure-without-error states.

The [CLI network loop](../../crates/sorotte-cli/src/session_runner/network_loop.rs)
now owns one `ConnectionAttemptOutcome` enum. Failed outcomes carry their original
error. Completion directly performs the appropriate disconnect, Plex shutdown,
retry reset and backoff, returning `ControlFlow` to its caller. Startup values
are read into the CLI's owned state without shared pass-through planning DTOs.
The connected-session exit enum also lives in the CLI. Shared connected-session
behavior still belongs to `sorotte-client-app`.

| Outcome | Preserved behavior |
| --- | --- |
| TCP failure or timeout | Retain failed-attempt count and original error; reconnect while allowed. |
| Session error | Shut down Plex and disconnect, then apply the same retry budget and retain the error. |
| Transport closure | Reset retries, disconnect, and schedule reconnect. |
| Runtime-window expiration | Reset retries and return success without disconnect or backoff. |

Paused-clock tests cover original error type/text, retry reset versus exhaustion,
runtime-window completion, exact exponential delays and no terminal sleep.
The existing [socket tests](../../crates/sorotte-cli/src/tests/connected_session_reconnect_restore/room_switch_and_network_loop.rs)
and [production lifecycle seam](../../crates/sorotte-cli/src/tests/playback_lifecycle_product_seam.rs)
remain. Redundant app-boundary export-availability tests are removed; behavior
tests for those internal APIs and state/persistence round trips remain.

## GUI bookkeeping and unused actions

The startup-support and ignored-option counters are removed from the settings
projection and draft. Their only display was a test-only text summary.
The CLI support table remains because it supplies real help output.

The unconstructed `ToggleMainWindowPlaybackButtons`,
`ToggleMainWindowAutoplayControls` and `CancelSessionDisconnect` shell actions
and their routes are removed. The first two operations still run through
`MenuActionId`; pending disconnect cancellation still uses the general pending
operation path. The methods, live menus, visibility settings and persistence
remain.

The action enum's dead-code allowance now applies only outside test builds.
Tests construct the remaining seam-only actions, so compiling tests exposes
variants that have no caller anywhere. Removing the previous blanket allowance
is what exposed the unused disconnect action.

## Terminology

| Previous name or wording | Current concept/name |
| --- | --- |
| `FirstRunConfigurationDialogState` / `FirstRunConfigurationDialogDraft` | `GuiConfigurationState` / `GuiConfigurationDraft`, used throughout normal operation |
| `SyncplayConfigurationGetter*` Rust support tables and CLI headings | `SyncplayStartupOptionSupport`, `SyncplayIniFieldSupport` and `SyncplayInputSupportStatus` |
| `ManifestRead::Legacy` | `Version1Or2`, preserving readers and migration into checksummed version 3 slots |
| `legacy_readiness` | `syncplay_readiness` |
| `legacy` readiness test participant | `unsupported_participant`, lacking coordinated-start support |
| “Legacy extension peers” | Peers without a stable playback request ID |
| “legacy model/adapter” position fallback | Session/list position before accepted transport telemetry |

The first-run decision remains separate from the settings state. Actual Python
`ConfigurationGetter` probe references retain the upstream class name.

The [glossary](../../CONTEXT.md) distinguishes stable playback request identity
from its connection nonce. Current requests supply an ID, but the server
deliberately omits that ID from snapshots sent to other participants. Absence
therefore does not establish an obsolete request path; the optional wire field
and privacy behavior remain.

Existing serde names such as `excludedLegacy`, established INI aliases,
`quota:legacy-unattributed`, Python reference fixture/environment names and
Windows `LegacyIAccessible` remain external or stored representations.
The direct media-index root also initializes new stores. Those concepts cannot
be removed merely because of their spelling.

## Evidence boundaries

The audit used workspace-wide Rust/fuzz reference searches and targeted reads
of callers, producers, tests, persistence and protocol boundaries. Symbol
frequency was a search aid, not proof of dead code.

The tests described above are retained or strengthened behavior obligations.
Execution results are recorded separately in the candidate's PR checks and local
`target/audit-evidence` / `target/verification` receipts. This report neither
substitutes for exact-source qualification nor claims a release.
