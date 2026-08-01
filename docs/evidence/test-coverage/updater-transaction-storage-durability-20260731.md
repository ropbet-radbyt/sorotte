# Updater transaction storage durability — 2026-07-31

## Scope and safety

This slice exercised only Sorotte's local updater transaction in
`crates/sorotte-gui/src/bin/sorotte-gui-updater.rs`. Every new fixture used a
nonce-owned directory below the process temporary directory. The updater paths,
source files, journal, replacement files, backups, and an unchanged sibling
sentinel all remained below that fixture root.

No network, external install, device, volume handle, mount, privilege,
credential, persistence, reconnaissance, or non-test path was used. The
disk-full and access-denied matrix faults are explicit deterministic injected
analogues. They are not physical disk exhaustion or storage-device evidence.

The live Windows denial check was reversible and test-owned: it held one
exclusive handle to one nonce-owned temporary directory, observed the expected
directory-sync sharing failure, released the handle, and then synchronized the
same directory successfully.

## Defect found before the fix

`TC-UPDATER-002` was reported before production behavior changed.

The updater already wrote, flushed, and `sync_all`ed its journal and prepared
replacement files. It did not synchronize the containing directory after:

- creating the recovery journal;
- creating prepared replacement files;
- `ReplaceFileW` or rename directory-entry changes;
- rollback restoration;
- transaction-artifact cleanup; or
- journal removal.

Consequently, the existing process-termination suite proved recovery from
process interruption after observable file-content flushes, but it did not
establish a parent-directory durability boundary. In particular, a storage
stack could acknowledge file contents while still losing a newly created,
renamed, or deleted directory entry after a later OS or power failure.

The first narrow characterization armed the new test seam for the first
parent-directory-sync operation and ran:

```text
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater \
  tc_updater_002_characterizes_missing_parent_directory_sync -- --nocapture
```

Before the fix it failed `0 passed; 1 failed` with:

```text
TC-UPDATER-002: updater transaction completed without reaching a parent-directory sync boundary
```

The finalized regression is named
`tc_updater_002_parent_directory_sync_failure_retains_authenticated_recovery`.
Final review also corrected its nonce-owned temporary fixture label from the
stale `tc-updater-001` spelling to `tc-updater-002`; the selector and assertion
had already used the correct defect identity.

## Narrow production fix

The updater now has one `sync_parent_directory(path)` boundary:

- on Unix, it opens the containing directory for reading and calls
  `sync_all`;
- on Windows, it opens the containing directory for write access with
  `FILE_FLAG_BACKUP_SEMANTICS`, shares read/write/delete, and calls
  `sync_all`, which reaches `FlushFileBuffers`.

Microsoft documents `FILE_FLAG_BACKUP_SEMANTICS` as the required way to obtain
a directory handle and documents that `FlushFileBuffers` requires a
`GENERIC_WRITE` handle:

- <https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory>
- <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers>

A reversible pre-implementation probe against a nonce-owned `C:\tmp`
directory returned:

```text
open=true flush=True error=0
```

The fix synchronizes the containing directory after journal and prepared-file
creation, each updater-owned rename or `ReplaceFileW` mutation, newly created
relative directories, rollback restoration, cleanup deletion, and journal
removal. A `NotFound` cleanup retry synchronizes an existing parent so a prior
successful deletion can be made durable; it does not try to open a parent that
never existed.

An incomplete initial journal write or file flush is removed before any target
mutation. A failure synchronizing an otherwise complete journal retains that
authenticated uncommitted journal so recovery selects rollback. Cleanup
failures after the synced commit record remain deferred: the retained
authenticated committed journal selects forward cleanup.

The updater continues to use `ReplaceFileW` for an existing Windows target so
it retains the existing atomic replacement and metadata behavior. Microsoft
documents `REPLACEFILE_WRITE_THROUGH` as unsupported, which is why the
containing-directory flush is separate:

<https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew>

## Deterministic fault seam and matrix

The seam is compiled only for tests, is thread-local, and arms one one-shot
failure by operation and one-based occurrence. It cannot affect another
parallel test thread. Its operation classes are:

- write;
- file flush;
- replace or rename;
- remove; and
- parent-directory sync.

Its labels are exactly `deterministic injected disk-full analogue` and
`deterministic injected access-denied analogue`.

The 13 schedules were:

| Outcome | Injected boundary |
| --- | --- |
| old | journal write / disk-full analogue |
| old | first prepared-file write / disk-full analogue |
| old | commit-record write / disk-full analogue |
| old | journal flush / disk-full analogue |
| old | first prepared-file flush / disk-full analogue |
| old | commit-record flush / disk-full analogue |
| old | first replacement / access-denied analogue |
| old | second replacement / access-denied analogue |
| old | first prepared-file parent sync / access-denied analogue |
| old | first replacement parent sync / access-denied analogue |
| new | committed backup cleanup removal / access-denied analogue |
| new | committed journal removal / access-denied analogue |
| new | committed cleanup parent sync / access-denied analogue |

For every schedule the test proves:

- every target has the complete old or complete new bytes, including existing,
  added, and removed-file cases;
- a retained journal parses, authenticates every transaction entry, and
  selects rollback or forward cleanup only through its commit record;
- the first recovery succeeds;
- a second recovery is an idempotent no-op;
- successful recovery leaves no journal, prepared file, or rollback backup;
  and
- the sibling sentinel is unchanged and every transaction path remains within
  the nonce-owned fixture.

## Validation

Environment:

```text
rustc 1.97.1 (8bab26f4f 2026-07-14)
implementation commit 716bb4e55c7c9214c726121b3803e0862c306601
C: NTFS, Healthy, OK
```

Focused fault matrix:

```text
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater \
  deterministic_updater_storage_fault_matrix_recovers_complete_old_or_new_installs \
  -- --nocapture
```

Result: `1 passed; 0 failed`; all 13 internal schedules passed.

Fixed defect regression:

```text
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater \
  tc_updater_002_parent_directory_sync_failure_retains_authenticated_recovery \
  -- --nocapture
```

Result: `1 passed; 0 failed`.

Real reversible Windows share denial:

```text
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater \
  windows_parent_directory_sync_reports_reversible_share_denial \
  -- --nocapture
```

Result: `1 passed; 0 failed`.

Complete updater unit and process-interruption suite:

```text
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater -- --nocapture
```

Result: `33 passed; 0 failed`, including all 11 real process-termination
boundaries and both recovery passes at every boundary.

Installed-updater Windows integration:

```text
cargo test --locked -p sorotte-gui \
  --test updater_self_replacement_windows \
  --features updater-integration-test -- --nocapture --test-threads=1
```

Result: exit code `0`. The Windows harness emitted no per-test status text in
this environment. A separate `--list` invocation discovered exactly:

```text
running_installed_updater_can_replace_its_own_installed_path: test
running_installed_updater_recovers_interrupted_replacement_and_restarts: test

2 tests, 0 benchmarks
```

Warning-denied focused lint:

```text
cargo clippy --locked -p sorotte-gui --bin sorotte-gui-updater \
  --all-features --tests -- -D warnings
```

Result: pass.

The exact renamed `TC-UPDATER-002` selector was rerun after final review and
passed `1/1`. The complete updater binary was then rerun and remained
`33/33`, including the 13 injected schedules, the reversible Windows
directory-share denial, and all 11 process-termination boundaries with both
recovery passes.

Updater-only formatting:

```text
rustfmt --edition 2024 --check \
  crates/sorotte-gui/src/bin/sorotte-gui-updater.rs
```

Result: pass.

Final integration also passed repository formatting and diff checks, both
changed workflows under actionlint, all 496 Python policy/infrastructure tests,
the 10-shard mutation policy, the empty known-defect registry, warning-denied
all-target/all-feature workspace Clippy in 15.8 seconds, and the complete
locked all-feature workspace suite on its first attempt in 257.5 seconds.

## Limitations

- The injected disk-full analogue does not fill a real filesystem and does not
  establish physical `ENOSPC`/Windows disk-full behavior.
- Directory and file `sync_all` success proves that the operating system
  accepted the requested flush. It does not prove survival of controller
  write-back caches, torn sectors, filesystem corruption, kernel panic, or
  physical power loss.
- The process-interruption suite terminates an updater process after an
  acknowledged boundary; it is not a machine reset.
- No FAT, ReFS, remote share, removable medium, or alternative Windows storage
  stack was exercised. Unsupported directory-flush behavior fails closed.
- No production directory outside a nonce-owned temporary fixture was opened
  or mutated.
