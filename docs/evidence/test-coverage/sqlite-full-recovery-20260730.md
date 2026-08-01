# SQLite Full-Write Recovery Evidence — 2026-07-30

## Status

Implemented and green.

The production room-persistence store now has a deterministic regression test for
SQLite's real `SQLITE_FULL` error path:

- `room_persistence_sqlite_full_preserves_old_row_and_recovers_after_limit_lift`
- `crates/sorotte-server/src/tests/persistence_tests.rs`

No product defect was found and no product behavior was changed.

## Boundary being modeled

This slice models an SQLite write that cannot allocate another database page. It
uses `PRAGMA max_page_count` on the exact production connection passed to
`RoomPersistenceStore::save_room`, so the failure is reported by SQLite as
`SQLITE_FULL` (`rusqlite::ErrorCode::DiskFull`).

This is **not** a kernel power-loss, storage-controller, torn-sector, or
`fsync`/write-cache guarantee test. Sorotte's existing subprocess crash tests
cover process interruption at explicit persistence boundaries; true power-loss
durability requires a filesystem or virtual-block-device fault harness outside
an ordinary unit-test process.

## Production path exercised

The test uses only the normal room-persistence operations:

1. `RoomPersistenceStore::open`
2. `RoomPersistenceStore::connection`
3. `RoomPersistenceStore::save_room`
4. `RoomPersistenceStore::load_rooms`
5. a normal store reopen

The fixture first persists a complete version-41 room containing three files,
playlist index 1, position and activity timestamps, an owner bucket, and a
creation timestamp. It checkpoints the baseline WAL, reads the current
`page_count`, and sets `max_page_count` to that exact value.

It then attempts a version-42 replacement containing 512 unique, multi-kilobyte
filenames. Both the legacy multiline playlist and JSON playlist encodings are
therefore materially larger than the baseline and require new pages.

## Mechanical oracles

The test proves all of the following:

- The production save returns `RoomPersistenceError::Sqlite`.
- The classified source is `rusqlite::Error::SqliteFailure` with
  `ErrorCode::DiskFull`, the Rust representation of `SQLITE_FULL`.
- The reported production boundary remains `save persisted room`.
- The reported database path is the actual fixture path.
- A raw eight-column row read before and after the failed upsert is identical:
  multiline playlist, JSON playlist, playlist index, position, last activity,
  persistence version, owner bucket, and creation time.
- `PRAGMA integrity_check` returns `ok` immediately after the failed write.
- A separate normal store reopen and `load_rooms` recovers the complete old
  version-41 state while the constrained connection remains alive.
- Raising `max_page_count` by 4,096 pages on that same connection permits the
  retained version-42 replacement to save normally.
- A final close/reopen returns the complete version-42 state, and a final
  integrity check remains `ok`.

The raw-column equality is intentional: checking only the decoded playlist
would miss partial leaks in `playlistIndex`, `persistenceVersion`, or the scalar
room metadata.

## Rejected service-level fixture

A bounded service-level variant was experimentally attempted by constraining an
external connection and then starting `RoomPersistenceService`. The worker
creates its own production SQLite connection; in this configuration that
connection did not inherit the external connection's `max_page_count`, and the
large write succeeded. Keeping such a test would falsely claim to cover
`SQLITE_FULL`.

The variant was removed rather than adding a test-only connection mutation hook
to production code. Existing room-worker tests already cover unresolved
write-failure retention, degraded/recovered events, and retry of the newest
desired state with deterministic trigger failures. This slice adds the missing
real SQLite error classification and row-integrity proof at the production
storage boundary.

## Stress and validation

- Focused regression:
  `cargo test -p sorotte-server --all-features room_persistence_sqlite_full_preserves_old_row_and_recovers_after_limit_lift -- --nocapture`
  — passed.
- Focused stress: 50/50 independent repetitions passed.
- Owning crate:
  `cargo test -p sorotte-server --all-features`
  — passed: 355 library tests, 14 binary tests, 2 server-binary integration
  tests, and 6 release-verification tests.
- Strict lint:
  `cargo clippy -p sorotte-server --all-targets --all-features -- -D warnings`
  — passed.
- Formatting:
  `cargo fmt --all -- --check`
  — passed.
- Scoped whitespace validation:
  `git diff --check -- crates/sorotte-server/src/tests/persistence_tests.rs docs/evidence/test-coverage/sqlite-full-recovery-20260730.md`
  — passed.
