# Room roster cleanup

This pass audited main at `dac75e44`, after PR #74. The earlier compatibility,
settings, runtime-planning, notification and GUI-session cleanup reports were
reviewed before tracing the remaining compatibility names and internal APIs.

## Unread client roster

The client stored membership and Syncplay readiness twice: in
`RoomState::users`, which supplies its queries and projections, and in
`RoomState::domain`, a `SyncDomain` from `sorotte-core`.

Every production reference to the second copy was a write or reset. Its only
reads were assertions and debug snapshots in tests. Maintaining it still
required room joins/leaves and ready updates during user changes, assigned-name
migration, optimistic readiness rollback, full List replacement and reconnect.

The duplicate field and all of its updates are removed. The existing user map
continues to supply readiness, membership, file metadata and autoplay decisions.
The assigned-username regression checks the visible roster and readiness before
the first List response, including removal from the provisional room. The reset
completeness test retains its populated user-map coverage and drops only the
removed field. Local-pause failure coverage continues to assert player truth and
retained readiness intent.

## Server ownership and terminology

After that removal, the server was the only consumer of `sorotte-core`.
The crate contained a room/user readiness map, two String aliases and two tests;
it did not implement playback synchronization. The server now owns
[`RoomRoster`](../../crates/sorotte-server/src/room_roster.rs), and the workspace
no longer includes `sorotte-core`.

| Previous surface | Current responsibility |
| --- | --- |
| `SyncDomain` / server `domain` field | Server-owned `RoomRoster` / `room_roster` |
| `DomainError` / `ServerRuntimeError::Domain` | `RoomRosterError` / `ServerRuntimeError::RoomRoster` |
| `RoomState` and `UserState` in the removed crate | Room and username map keys with an optional ready flag |
| `users_in_room` allocation followed by a username scan | Direct `user_ready` lookup |
| `users_in_room(...).is_some()` for room presence | `contains_room` lookup |

Missing-room and missing-user error text, unknown readiness, ready-state
replacement, and removal of the last member retain their behavior. Tests cover
those boundaries, independent readiness for the same name in different rooms,
and failed mutations leaving accepted readiness intact. In particular, a member
whose ready flag is `None` must still count as a successful removal.

The glossary, crate lists, both Cargo lockfiles, architecture catalog and package
selection fixture follow the new ownership. The removed client dependency no
longer has a mutation-selection edge; server source changes continue to select
the server's package-owned shards. Historical test-count tables are retained as
recorded evidence. No wire names, supported Syncplay contracts, settings formats,
or persistence schemas change. The cleanup removes duplicated work and a crate;
it does not claim a measured performance improvement.

## Other audit dispositions

- The effect-sink trait has one production implementation and nine test
  implementations. The latter include real failure-injection tests for causal
  state, readiness and participant-status retry. Its defaults deserve a focused
  review, but the trait is not established dead compatibility code. Replacing
  it would require preserving those failure seams and guarded delivery tests.
- `FileDuration` retains integer and floating-point JSON values from inbound
  metadata. Its wire accessor participates in the Python comparison oracle;
  normalizing it away solely because it has few callers would change that
  contract.
- The seven protocol envelope types are used by `ProtocolMessage`'s untagged
  decoding. They are not unused wrappers; a replacement must preserve permissive
  decoding and compound-command ordering.
- Small follow-ups remain: the raw-value `extract_hello` API is exercised only
  by its own tests, while interoperability uses `extract_hello_from_message`;
  `PlexMediaResolver::cache_mut` has no caller, although the identically named
  `PlexSyncEngine` method has a GUI consumer; `local_seek_target_allowed` receives
  a clock value it does not use. These are separate from the roster change.

## Validation

Local results are recorded in `target/audit-evidence`. This is an implementation
check of the working patch, not hosted qualification or release evidence.

- `cargo test --locked --offline --workspace`: 4,298 passed, zero failed,
  23 ignored under the existing test policy; includes default-feature doctests.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings`
  and the corresponding `--all-features` command: both passed.
- `cargo test --locked --offline -p sorotte-compat --lib -- --nocapture`:
  145 passed, zero failed, ignored or filtered tests. The run enabled
  `SYNCPLAY_REQUIRE_LIVE_INTEROP`, `SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY` and
  `SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY`, using Python 3.13 and the clean pinned
  Syncplay checkout at `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`.
- Architecture-index, mutation-selection and package-selection Python suites:
  all 25 tests passed.
- Formatting, diff whitespace, generated architecture and locked workspace/fuzz
  metadata checks passed. Neither dependency graph retains `sorotte-core`.

Static verification preflight also passed. The pull request records subsequent
hosted checks and exact-commit native and nonpublishing package qualification;
the local results above do not substitute for those source-bound checks.
