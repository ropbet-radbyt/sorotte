# Outbound delivery cleanup and queued dependency updates

Status: scoped; implementation and PR qualification have not started.

Use one branch, `codex/cleanup-dependency-bundle`, based on main
`d36982f8b3cda8e5181494f56b3037b1bcc2fb4c` after settings cleanup PR #67.
The intended implementation is one PR containing the cleanup and both Dependabot
groups below, with focused commits for review.

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

## Implementation scope

1. Replace fixture batch capture with explicit queue inspection or staged
   delivery. Tests observing pending or coalescible work must inspect it without
   acknowledging it. Tests modelling a completed write must stage the frame and
   acknowledge the exact receipt. Provide shared fixture support where it
   removes repetition, with clear names for inspection, successful delivery and
   failed delivery.
2. Remove the alternate owner path from
   `crates/sorotte-gui/src/app/runtime_owner/session_transport.rs`. Migrate owner
   fixtures to deterministic transport drivers using the same delivery path as
   production, including delayed and failed writes.
3. Remove `flush_outbound_protocol_lines` from the GUI adapter trait, concrete
   client-core adapter and forwarding implementation in
   `crates/sorotte-gui/src/app/runtime_stack/`. Retain the staged delivery,
   acknowledgement and failure methods.
4. Remove the untracked reliable queue and its push/drain operations from
   `runtime_stack/transport/handle.rs`, and the TCP worker branch that consumes
   it in `runtime_stack/transport/tcp.rs`. Update `transport/loopback.rs` to
   handle liveness explicitly: its current batch drain also carries legitimate
   liveness messages, so deleting that call alone would lose behavior.
5. Remove the obsolete batch-flush forwarding methods in
   `crates/sorotte-client-app/src/application.rs` and
   `crates/sorotte-client-core/src/runtime/queued_control.rs`, then the unused
   batch drains in client-core `control.rs`. Remove lower-level helpers only
   after checking their remaining callers. Keep real notification drains and
   reconnect operations, which have different responsibilities.
6. Update contributor documentation to describe queue inspection, pending frame,
   completed write and delivery receipt precisely. Remove the obsolete
   compatibility comments together with their code.

The GUI adapter trait itself, partial-write representation, TLS transport and
reconnect architecture do not need a redesign for this cleanup.

## Dependabot scope

The open queue was checked on 2026-09-12. Include both bot PRs:

| Source | Dependency | Current | Target |
| --- | --- | --- | --- |
| [PR #66](https://github.com/ropbet-radbyt/sorotte/pull/66) | `eframe` | 0.36.1 | 0.36.2 |
| PR #66 | `egui` | 0.36.1 | 0.36.2 |
| PR #66 | `reqwest` | 0.13.4 | 0.13.5 |
| [PR #65](https://github.com/ropbet-radbyt/sorotte/pull/65) | `filelock` | 3.32.5 | 3.32.6 |
| PR #65 | `pip-api` | 0.0.34 | 0.0.35 |
| PR #65 | `platformdirs` | 4.11.7 | 4.11.8 |

Reviewed heads: #66 `f5065c472d9e7ee1537cbd6b66c6dff5a602d4c7` and
#65 `5fbceaebead3407a754f50c1edcf2421e4ada9f1`. Both patches apply cleanly to
the scoped base. Recheck the queue before implementation because bot branches
can rebase. The unrelated human PR #25 is outside this bundle.

The bot patches cover only the root `Cargo.lock` and
`requirements/verification-constraints.txt`. Complete the associated maintenance:

- Update the three workspace dependency declarations in `Cargo.toml` and the
  matching egui family in `Cargo.lock`. Update `reqwest` in the separate
  `fuzz/Cargo.lock`; avoid unrelated dependency resolution churn.
- Update the three Python constraints and their normalized LF SHA-256 in
  `coverage/verification-tools.toml` under `[python-resolution]`. Run
  `python scripts/verify.py pins --check` and the static preflight so the central
  manifest and its consumers agree.
- Validate the CI policy, dependency-audit and Python Syncplay requirement sets
  against the revised constraints, including the reviewed Linux/Windows and
  Python 3.11/3.12/3.13 resolution matrix.
- Refresh and seal native verification dependency inputs for the changed Cargo
  and Python inputs before native qualification. Evidence from the previous
  dependency bundle does not qualify these updates.

Keep the existing Rust 1.98.1 toolchain and unrelated action, player and container
pins. Any necessary dependency-driven source fixes belong in this same branch.
The combined implementation PR should reference both bot PRs; their updates are
only superseded once the combined change lands.

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

Qualification for the eventual combined PR includes formatting, strict Clippy,
the default workspace suite, all-feature nextest and doctests, GUI semantic and
live Python interoperability coverage, HTTP/Plex credential and origin checks,
and the required coverage, mutation, fuzz, dependency, packaging and Windows
native gates. Bind results to the final source and current base, retain the
existing gate thresholds, and clean up ephemeral native runners afterward.
Report net code reduction and final validation in the implementation PR.
