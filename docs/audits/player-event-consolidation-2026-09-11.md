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

## Validation

The adapter consolidation passed the full checks below before the additional
hook-status cleanup. Tests used Rust 1.98.1, the pinned Syncplay Python environment, and the required
live interoperability flags described in the preceding cleanup audit.

| Check | Result | Local evidence |
| --- | --- | --- |
| Workspace, all features | 4,387 passed, 0 failed, 30 ignored | `target/player-cleanup-workspace-final.log` |
| Workspace Clippy, all targets and features, warnings denied | Passed | `target/player-cleanup-clippy-final.log` |
| GUI semantic scenarios | 14 passed, including live Python peers | `target/player-cleanup-semantic-final.json` |
| Static verification | All 16 checks passed | `target/player-cleanup-preflight-final.json` |
| Formatting and whitespace | Passed | `cargo fmt --all`; `git diff --check` |

The additional hook-status cleanup passed all 466 mpv tests (4 ignored),
all-target/all-feature mpv Clippy, formatting, and whitespace checks. Logs:
`target/player-cleanup-hook-tests.log` and `target/player-cleanup-hook-clippy.log`.
The merged PR pipeline repairs also passed 41 workflow tests, recorded in
`target/player-cleanup-integrated-workflow-tests.log`.

These are local results for this follow-up. PR #62's hosted and native evidence
belongs to its own source revision and does not qualify this change.

## Remaining consolidation

The player API still exposes typed queues, and the core and GUI still consume
them for focused fixture adapters and disconnected stand-ins. Removing those
paths requires migrating the fixtures to acknowledged batches while preserving
their tests for partial observations, absent timestamps, stale generations,
event ordering, and reacquisition. They have not been hidden behind test-only
consumers or declared removed by this change.
