# Persistence Platform Syscall Fault Evidence — 2026-07-30

## Status

Implemented and green on the current Windows host.

This slice extends the existing production-store `SQLITE_FULL`, worker-owned
`SQLITE_FULL`, SQLite `query_only`, VFS path-collision, and explicit
process-interruption evidence with a real host-filesystem denial. It changes no
production behavior.

The owning test module is:

- `crates/sorotte-server/src/tests/persistence_platform_syscall_fault_tests.rs`

The module is registered by the server's existing unit-test root and contains
compile-time platform contracts rather than a runtime skip:

- Windows:
  `room_persistence_windows_share_denial_preserves_and_recovers_durable_state`
- Unix:
  `room_persistence_unix_namespace_denial_preserves_and_recovers_durable_state`

The Windows regression was executed on this host. The Unix counterpart was
also compiled and executed under local Ubuntu WSL with an isolated Cargo
target directory.

## Windows kernel sharing denial

The Windows regression first uses the production `RoomPersistenceStore` to
persist a complete version-41 room:

- two playlist entries in both multiline and JSON encodings;
- playlist index;
- position and last-activity timestamp;
- persistence version;
- owner bucket;
- creation timestamp.

It requests `PRAGMA wal_checkpoint(TRUNCATE)`, requires a non-busy checkpoint,
closes the seeding connection, captures the complete raw eight-column row,
checks `PRAGMA integrity_check`, and reads the resulting main database bytes.

The test then opens the real database file with
`std::os::windows::fs::OpenOptionsExt::share_mode(0)`. This is a Windows kernel
file-sharing contract, not an SQLite pragma, trigger, alternate store, or
test-only production hook. While that handle is live, the test mechanically
proves:

- `std::fs::rename` is rejected with Win32 error 32,
  `ERROR_SHARING_VIOLATION`;
- `std::fs::remove_file` is rejected with the same kernel error;
- the production `RoomPersistenceService` fresh connection fails at
  `connect persistence worker`;
- SQLite retains `ErrorCode::CannotOpen` (`SQLITE_CANTOPEN`) and
  `unable to open database file` VFS context;
- bytes read through the already-authorized exclusive handle are exactly
  unchanged across the failed worker start.

Dropping the handle removes the host condition. The test then proves:

- the complete version-41 raw row is unchanged;
- SQLite integrity remains `ok`;
- the normal room worker starts using the same store path;
- a complete version-42 replacement is acknowledged by `flush`;
- a normal close and `RoomPersistenceStore::open` reload the full replacement;
- every raw replacement column matches its expected value;
- final SQLite integrity remains `ok`.

The exclusive `File` closes during unwinding, and the fixture's `Drop`
implementation removes the main database and its WAL, SHM, and rollback-journal
sidecars. The test therefore does not leave a denied handle or temporary
database behind after either success or panic.

## Unix namespace-denial counterpart

Unix does not apply Windows share-mode rules to an open file. A chmod-based
test would also turn green incorrectly under a privileged test identity.
Instead, the Unix-only contract uses real filesystem namespace syscalls:

1. rename the checkpointed main database to a unique displaced path;
2. create a directory at the production database pathname;
3. require the production worker open to fail as `SQLITE_CANTOPEN`;
4. prove the displaced database bytes, full raw row, and integrity are
   unchanged;
5. remove the directory and rename the database back;
6. prove the same worker write, raw-column, integrity, and reopen recovery as
   Windows.

An unwind guard restores the displaced database before fixture cleanup. This
expresses the same externally imposed production-open-denial and recovery
contract without depending on process identity or returning green at runtime
when permissions cannot be enforced.

## What this proves

Together with the earlier persistence slices, the new probe proves that:

- a real Windows kernel sharing denial reaches the production worker boundary;
- host-level rename and delete denial is independently observed rather than
  inferred from SQLite;
- a failed worker connection cannot modify a checkpointed durable baseline;
- removing the host condition permits ordinary worker write and reopen
  recovery;
- complete raw persistence state and structural SQLite integrity survive the
  denial.

No independent product defect was found.

## Durability limit

`wal_checkpoint(TRUNCATE)` provides a concrete SQLite checkpoint boundary for
the test fixture. This test does **not** prove that a filesystem journal,
kernel page cache, drive write cache, or storage controller persisted bytes
after `fsync`, power loss, torn sectors, or device removal. It also does not
inject short writes or claim virtual-block-device durability. Those guarantees
need a disposable filesystem or block-device harness with explicit crash and
flush control outside an ordinary unit-test process.

## Validation

- Focused Windows regression:
  `cargo test --locked -p sorotte-server --lib room_persistence_windows_share_denial_preserves_and_recovers_durable_state -- --nocapture --test-threads=1`
  — passed: 1/1.
- Focused Unix regression under Ubuntu WSL:
  `CARGO_TARGET_DIR=target/wsl-server-syscall cargo +1.97.1 test --locked -p sorotte-server --lib room_persistence_unix_namespace_denial_preserves_and_recovers_durable_state -- --nocapture --test-threads=1`
  — passed: 1/1 in 0.11 seconds, with 365 tests filtered out.
- Serial Windows stress:
  the same regression was executed 50 times with one test thread
  — passed: 50/50.
- Complete server package:
  `cargo test --locked -p sorotte-server --all-features`
  — passed: 366 library tests, 14 server-binary unit tests, 2 binary
  integration tests, 6 release-verification tests, and doc tests.
- Strict package lint:
  `cargo clippy --locked -p sorotte-server --all-targets --all-features -- -D warnings`
  — passed.
- Package formatting:
  `cargo fmt -p sorotte-server -- --check`
  — passed.
- Scoped whitespace validation:
  `git diff --check -- crates/sorotte-server/src/tests.rs crates/sorotte-server/src/tests/persistence_platform_syscall_fault_tests.rs docs/evidence/test-coverage/persistence-platform-syscall-faults-20260730.md`
  — passed.
