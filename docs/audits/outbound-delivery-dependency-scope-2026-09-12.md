# Outbound delivery cleanup and v0.2.15 release

The cleanup and queued dependency updates are implemented. Final PR and release
qualification results are recorded in the implementation PR.

The work uses `codex/cleanup-dependency-bundle`, based on main
`d36982f8b3cda8e5181494f56b3037b1bcc2fb4c` after settings cleanup PR #67.
The combined PR contains the cleanup, both Dependabot groups below and the
version bump to 0.2.15. Publication must use the retained artifacts from the
qualified PR head after its unchanged merge, as required by
[release qualification](../RELEASE_QUALIFICATION.md).

## Cleanup finding

The client retains an infallible batch-drain API used to capture messages in
tests, alongside the staged delivery API used by production transports. The GUI
owner explicitly switches to the old API under `cfg(test)` when no transport
driver is installed. A comment calls these fixtures "Legacy owner tests";
another calls the adapter hook "Legacy infallible ownership transfer". Here,
"legacy" means an obsolete test delivery model, not Python Syncplay support.

This keeps duplicate APIs across client-core, client-app and the GUI, plus an
untracked outbound queue that the TCP worker still services. It also means many
owner tests bypass the production delivery and acknowledgement boundary.

The five API call inventories contain 359 calls across 44 distinct Rust files:

| API | Calls | Classification |
| --- | ---: | --- |
| `flush_outbound_protocol_lines` | 119 | All in test code, including the owner branch |
| `flush_queued_protocol_messages` | 111 | All in test code |
| `flush_queued_protocol_lines` | 3 | One test and two production forwarding implementations |
| `push_outbound_protocol_lines` | 10 | All in test code |
| `drain_outbound_protocol_lines` | 116 | 115 test calls and one loopback driver call |

These are call sites, not test counts or a promised change count. Classification
uses the repository's test-path and inline `cfg(test)` module detection in
`scripts/diff_coverage.py`. Direct callers of the underlying
`QueuedRuntimeControl::drain_outbound_messages` and
`drain_outbound_message_lines` are also tests or the forwarding APIs above.

## Implemented changes

- Removed the infallible batch APIs in client-core, client-app and the GUI
  adapter, including the unused `ProtocolOutbox` batch clear/drain operations.
  Core fixtures now distinguish pending queue inspection from successful writes
  with exact lease acknowledgement.
- Removed the owner control-flow fork for tests without a driver. Recording
  fixtures install a deterministic writer and inspect its completed writes;
  delayed, failed and externally driven fixtures retain explicit receipt control.
  GUI adapter tests share a delivery helper through the production adapter trait.
- Removed the untracked reliable queue from the GUI transport handle and its TCP
  consumer. Loopback now consumes liveness explicitly, alongside the reliable
  frame and receipt. A regression covers chat translation, liveness coalescing,
  one matching reliable receipt and an idle second pump.
- Updated TLS transport fixtures to stage a reliable Hello with a receipt instead
  of injecting an untracked line. Existing partial-write, old-worker, reconnect,
  playlist-fence and selected-media tests retain their assertions.
- Documented pending messages, completed writes and delivery receipts in the
  contributor guide. The obsolete compatibility comments were removed with the
  code; the meaningful State heartbeat is described as Syncplay synchronization.

The GUI adapter trait, partial-write representation, TLS transport and reconnect
architecture retain their responsibilities.

## Dependabot scope

The open queue was checked on 2026-09-12. The bundle incorporates both bot PRs:

| Source | Dependency | Current | Target |
| --- | --- | --- | --- |
| [PR #66](https://github.com/ropbet-radbyt/sorotte/pull/66) | `eframe` | 0.36.1 | 0.36.2 |
| PR #66 | `egui` | 0.36.1 | 0.36.2 |
| PR #66 | `reqwest` | 0.13.4 | 0.13.5 |
| [PR #65](https://github.com/ropbet-radbyt/sorotte/pull/65) | `filelock` | 3.32.5 | 3.32.6 |
| PR #65 | `pip-api` | 0.0.34 | 0.0.35 |
| PR #65 | `platformdirs` | 4.11.7 | 4.11.8 |

Reviewed heads: #66 `f5065c472d9e7ee1537cbd6b66c6dff5a602d4c7` and
#65 `5fbceaebead3407a754f50c1edcf2421e4ada9f1`. The reviewed updates were applied to
the scoped base; unrelated lockfile resolution changes were excluded. The
unrelated human PR #25 is outside this bundle.

The bot patches cover only the root `Cargo.lock` and
`requirements/verification-constraints.txt`. The associated maintenance also includes:

- Updated the three workspace dependency declarations in `Cargo.toml` and the
  matching egui family in `Cargo.lock`. Updated `reqwest` in the separate
  `fuzz/Cargo.lock`; unrelated dependency versions remain unchanged.
- Updated the three Python constraints and their normalized LF SHA-256 in
  `coverage/verification-tools.toml` under `[python-resolution]`. Static preflight validates the central manifest and its consumers.
- Validation covers the CI policy, dependency-audit and Python Syncplay requirement sets
  against the revised constraints, including the reviewed Linux/Windows and
  Python 3.11/3.12/3.13 resolution matrix.
- Refreshed and sealed native verification dependency inputs for the changed Cargo
  and Python inputs for native qualification. Evidence from the previous
  dependency bundle does not qualify these updates.

The Rust 1.98.1 toolchain and unrelated action, player and container pins remain
unchanged. Workspace package versions in both lockfiles and the release section
of `coverage/current-architecture.toml` now identify 0.2.15, using the v0.2.14
release source `267291490915452f3b87d5a5a59d257f98c8836a` as its historical base.
The combined implementation PR references both bot PRs; their updates are only
superseded once the combined change lands.

## Preserved contracts and acceptance

Syncplay protocol compatibility and settings import remain supported. This
cleanup does not change wire formats, saved settings, playlist semantics or the
meaning of a delivery acknowledgement.

- A reliable frame remains pending until its exact successful write receipt.
  Partial or failed writes cannot commit selected-media effects or discard the
  unsent tail. Failure releases the appropriate delivery lease for retry.
- Old-worker and stale-receipt fencing, reconnect behavior, backpressure,
  byte accounting and timeout handling retain their coverage and behavior.
- The independent coalesced liveness lane remains available, including through
  loopback. Loopback chat translation and GUI semantic scenarios continue to
  work.
- Tests use production delivery control flow. No untracked reliable injection
  queue or infallible batch facade remains solely to preserve old fixtures.
- Preserve or strengthen the existing receipt, partial-write, playlist ordering,
  same-selection and reconnect regressions. Add coverage only where migration
  exposes a missing behavioral case; do not replace pending-queue assertions
  with unconditional successful acknowledgements.

Qualification for the combined release PR includes formatting, strict Clippy,
the default workspace suite, all-feature nextest and doctests, GUI semantic and
live Python interoperability coverage, HTTP/Plex credential and origin checks,
and the required coverage, mutation, fuzz, dependency, packaging and Windows
native gates. Bind results to the final source and current base, retain the
existing gate thresholds, and clean up ephemeral native runners afterward.
Report net code reduction and final validation in the implementation PR.
