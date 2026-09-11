# Player adapter consolidation, 2026-09-11

Follow-up to the compatibility and terminology cleanup, based on PR #62 at
`dee5ce963923eb60e9d4832f107231bc2890eda6`.

## Completed

- Removed `ConnectedMpvPlayer` and `SimulatedPlayer`, their forwarding macro,
  wrapper-only tests, and the cache-injection helper used only by those tests.
  Callers now use `MpvAdapter::with_json_ipc` or `MpvAdapter::simulated` directly.
  Connection-health assertions remain on the adapter's timeout regression.
- Replaced the GUI test player's three independent queues with the mpv
  simulator. GUI scenarios now consume tracked load completion, media identity,
  transport observations, and acknowledged batches through the production
  lifecycle reducer. Exact open-file observation recording remains available to
  native tests.
- Centralized the GUI owner's repeated adapter selection for commands and
  observations. Updated fixtures to observe ordered file identity, actual file
  size, and telemetry capability while retaining unavailable-telemetry coverage.
- Fixed simulated unload: it previously cleared the adapter's path without
  notifying ordered consumers that the physical load had ended. It now delivers
  the same stop lifecycle event as mpv. The new regression failed before the fix
  and passes after it.
- Removed coverage-policy entries for the deleted wrapper module. The actual
  adapter remains within the existing critical player-runtime boundary.
- Removed the bare `partially-applied` mpv hook status accepted only for
  provisional development builds. The bundled hook's `failed` status plus
  `applicationState=partially-applied` remains supported and covered by both the
  adapter and Lua tests. Player test names now distinguish the Syncplay bridge,
  typed command-progress queue, playback snapshot, and scripted fixtures.

- Removed `MpvNetworkMediaOptionsTransitionOutcome` and its merged getter, which
  had no application consumers. Removed the sequencing counter and event wrappers
  that existed only to merge the two queues. Hook health and media-policy events
  retain their separate bounded queues and authoritative runtime snapshot.
  Migrated the IPC, CLI, and real-mpv tests to those actual application APIs;
  empty-queue checks still assert that both channels have been consumed.
- Initial hosted qualification exposed a live Syncplay comparator race: a room
  change could be attributed no output after 60 milliseconds of silence, before
  Python announced the destination. The collector now waits for that room
  announcement on the requesting client's connection. The delayed-response
  regression failed before the fix and also checks that an older room's output
  cannot satisfy the wait. The existing two-second maximum and exact outbound
  sequence comparisons remain in force.

## Validation

Final checks used Rust 1.98.1, the pinned Syncplay Python environment, and the
required live interoperability flags described in the preceding cleanup audit.

| Check | Result | Local evidence |
| --- | --- | --- |
| Workspace, all features | 4,388 passed, 0 failed, 30 ignored | `target/player-cleanup-workspace-complete.log` |
| Workspace Clippy, all targets and features, warnings denied | Passed | `target/player-cleanup-clippy-complete.log` |
| GUI semantic scenarios | 14 passed, including live Python peers | `target/player-cleanup-semantic-complete.json` |
| Static verification | All 16 checks passed | `target/player-cleanup-preflight-complete.json` |
| Real mpv bridge lifecycle, default features | Passed with audio/video output disabled | `target/player-cleanup-real-mpv-split-events.log` |
| Integrated workflow regression tests | 41 passed | `target/player-cleanup-integrated-workflow-tests.log` |
| Compatibility suite after the collector repair | 145 passed, 0 failed, 7 ignored, with live Syncplay prerequisites | `target/pr63-room-reply-compat-after.log` |
| Workspace Clippy after the collector repair | Passed | `target/pr63-room-reply-clippy.log` |
| Formatting and whitespace | Passed | `cargo fmt --all`; `git diff --check` |

The separate real-mpv run exercised the opt-in bridge test against
`mpv v0.41.0-1012-ge8673660a`; its executable SHA-256 was
`547aaba0dec693894a271e26e83e413f00bc4063b4a00dc8a11d1ee88c6eaefe`.
It verified bridge discovery, settings acknowledgement, competing ownership,
network/local transitions, lease expiry, and clean ownership release through
actual JSON IPC. It did not exercise visible Windows GUI interaction.

These are local results for this follow-up. PR #62's hosted and native evidence
belongs to its own source revision and does not qualify this change.

## Remaining consolidation

The player API still exposes typed queues, and the core and GUI still consume
them for focused fixture adapters and disconnected stand-ins. Removing those
paths requires migrating the fixtures to acknowledged batches while preserving
their tests for partial observations, absent timestamps, stale generations,
event ordering, and reacquisition. They have not been hidden behind test-only
consumers or declared removed by this change.
