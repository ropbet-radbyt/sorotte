# Persistence Worker Fault Evidence — 2026-07-30

## Status

Implemented and green.

This slice closes the worker-boundary gap left by the production-store
`SQLITE_FULL` proof. It exercises the real `RoomPersistenceService`, its
actor-owned SQLite connection, the production transaction and upsert, desired
state retention, worker health transitions, and normal close/reopen recovery.

No product defect was found. Production behavior was not changed.

## Regressions

The owning tests are ordinary deterministic Rust tests in
`crates/sorotte-server/src/persistence_actor.rs`:

- `room_worker_start_classifies_filesystem_path_open_failure`
- `room_worker_sqlite_full_preserves_old_row_and_recovers_on_same_connection`
- `room_worker_read_only_connection_preserves_old_row_and_recovers`

The slice adds one test-only hook immediately before the production worker
opens its transaction. The hook receives the exact actor-owned
`rusqlite::Connection` and the real `ServerPersistenceEffect`. Its field,
method, and call site are all compiled only under `cfg(test)`; no connection
wrapper, alternate store implementation, or failure branch exists in a
production build.

## Worker-owned `SQLITE_FULL`

The previous store-level experiment correctly rejected an external-connection
fixture because SQLite's `max_page_count` is connection-local in this setup.
The new regression applies the limit to the connection that
`RoomPersistenceService` actually moved into the room worker.

The test:

1. Persists and checkpoints a complete version-41 baseline.
2. Starts the production room worker.
3. At its pre-transaction boundary, reads `page_count` and sets
   `max_page_count` to exactly that value on the worker-owned connection.
4. Enqueues a version-42 replacement whose two playlist encodings require
   materially more pages.
5. Observes the normal failed-effect and degraded-worker surfaces.
6. Raises the page limit on that same connection for a newer version-43
   desired state.
7. Observes normal application and exactly one recovery transition.
8. Closes and reopens the store through the normal production API.

Mechanical oracles prove:

- the capacity failure is reported for the exact version-42 effect with
  SQLite's stable `database or disk is full` message;
- every observed version-42 transaction has no allocation headroom on the
  actor-owned connection;
- the degraded worker count is one while the newest desired state is
  unresolved;
- repeated retries do not duplicate the degraded transition;
- no applied event is emitted for version 42;
- a raw eight-column row is byte/value identical before and after failure:
  multiline playlist, JSON playlist, playlist index, position, last activity,
  persistence version, owner bucket, and creation time;
- `PRAGMA integrity_check` remains `ok`;
- capacity restoration happens on the same actor-owned connection;
- version 43 is applied, degradation clears, and exactly one recovery event is
  emitted;
- a normal reopen returns the complete version-43 state.

The adjacent production-store regression continues to provide the typed
`rusqlite::ErrorCode::DiskFull` assertion. The worker's public event contract
intentionally carries an error string rather than `rusqlite::Error`, so this
test asserts the worker-visible classification without inventing a test-only
typed event API.

## Filesystem-open and write-denial boundaries

The startup regression creates a valid store, replaces its closed database file
with a directory at the same path, and starts `RoomPersistenceService`. This
forces the worker's real `connect persistence worker` call through the SQLite
VFS and proves:

- the error retains the exact production action and path;
- the source is `rusqlite::Error::SqliteFailure`;
- the primary result code is `ErrorCode::CannotOpen` (`SQLITE_CANTOPEN`);
- the VFS context remains `unable to open database file`.

The write-denial regression switches the actor-owned connection into SQLite
`query_only` mode at the same pre-transaction seam. It proves that a denied
production upsert:

- reports the exact version-2 effect and `readonly database` failure;
- leaves all eight baseline columns unchanged;
- preserves SQLite integrity;
- retains unresolved desired state and degraded worker health;
- succeeds with a newer version-3 state after write access is restored on the
  same connection;
- emits one recovery transition and survives a normal reopen.

`query_only` is deliberately described as a deterministic SQLite
connection-authorization test. It is not evidence about NTFS/POSIX ACL
propagation. The filesystem-collision test supplies a real VFS/open failure,
while ACL-specific coverage would require platform-specific identity and
permission orchestration that is brittle inside ordinary unit-test processes.

## Durability boundary

These tests do not claim kernel power-loss, torn-sector, storage-controller,
filesystem-journal, `fsync`, or write-cache durability. Existing subprocess
crash tests cover process interruption at explicit schema, migration,
transaction, and metadata boundaries. Physical durability needs a disposable
filesystem or virtual block-device harness outside this unit-test layer.

## Validation

- Focused room-worker family:
  `cargo test --locked -p sorotte-server --lib room_worker_ -- --nocapture --test-threads=1`
  — passed: 9/9.
- Focused fault stress:
  each of the three new regressions was repeated independently 50 times
  serially — passed: 150/150.
- Owning crate:
  `cargo test --locked -p sorotte-server --all-features`
  — passed: 358 library tests, 14 server-binary unit tests, 2 binary
  integration tests, and 6 release-verification tests.
- Strict lint:
  `cargo clippy --locked -p sorotte-server --all-targets --all-features -- -D warnings`
  — passed.
- Formatting:
  `cargo fmt --all -- --check`
  — passed.
- Scoped whitespace validation:
  `git diff --check -- crates/sorotte-server/src/persistence_actor.rs docs/evidence/test-coverage/persistence-worker-faults-20260730.md`
  — passed.
