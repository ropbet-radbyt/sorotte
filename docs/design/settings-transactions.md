# Settings persistence contract

The shared `sorotte-client-app::app_boundary::persistence` APIs own INI updates
for both Rust clients. This implements audit A03, A05, and A06.

## INI compatibility

The parser retains its existing last-recognized-assignment behavior. The writer
updates every matching key across repeated, case-insensitive sections, including
section names with surrounding whitespace. Removing a field removes every copy.
This prevents an old duplicate from overriding a save or retaining a cleared
credential. Comments, unknown keys, unrelated sections, the BOM, and existing
escaping remain supported; line endings retain the existing writer's LF
normalization contract. Repeating a save is idempotent.

## Transactions and intended changes

A persistent `.<filename>.lock` sidecar owns an OS file lock. The lock covers
reading the latest document, invoking an update callback once, merging, flushing,
and atomic replacement. Writers wait at most five seconds for a busy lock, then
return an `io::ErrorKind::WouldBlock` cause with a retry message. Callbacks are
never automatically retried. Process exit releases the kernel lock; no PID
guessing or stale-file timeout is required. Do not delete lock sidecars while
clients can access the configuration.

Existing paths are canonicalized, including file symlinks. For first creation,
the parent is created and canonicalized before deriving the lock path. Relative
paths, `..`, directory symlinks/junctions, and Windows case aliases therefore use
the same lock. Hard links are separate names: replacing one name intentionally
breaks that link. The contract assumes cooperating Sorotte writers; an external
editor that ignores the lock can still overwrite files.

Choose the API according to the caller's intent:

| Operation | Semantics |
| --- | --- |
| `upsert_*_at_path` | Explicit patch: `Some` assigns the field and `None` leaves it alone. Never pass a previously captured full snapshot. |
| `edit_*_at_path` / `update_*_at_path` | Read latest settings under the lock, invoke the callback once, and persist its changes. Changing a field to `None` removes all its assignments. `edit` returns the committed settings. |
| `merge_*_at_path` | Save only fields changed from the caller's original baseline. Unchanged fields retain their current disk values. Same-field conflicts follow transaction commit order. Return the actual committed settings as the new baseline. |
| `clear_*_at_path` | Record a durable, nonsecret clear tombstone in the sidecar before deleting the INI while holding the lock. |
| `relocate_*_at_path` | Lock canonical source and destination in sorted order, merge edits onto the current source, update the destination, then publish the location once. Roll back destination bytes under the same locks if publication fails. |
| `write_*_atomically_at_path` | Explicit unconditional byte replacement, serialized with other writes. Reserved for already-owned byte documents such as the install locator. |

A genuinely new missing file can initialize a full snapshot, including imported
settings. After Clear, the sidecar tombstone distinguishes that first-run case
from a deleted file: a stale full snapshot writes only its edited fields and
cannot repopulate unchanged credentials. The tombstone contains no credential,
username, path, timestamp, or process ID. A deliberate new credential assignment
through an edit or explicit patch remains supported.

GUI ordinary Save uses the baseline merge and adopts the committed result. GUI
feature patches execute against current disk settings inside the transaction.
Relocation keeps source freshness, target mutation, location publication, and
rollback within the defined locking order. CLI settings are explicit patches
or transactional callbacks, so they use the same locking contract. Readers see
complete old or new documents; relocation publication does not promise a
cross-file snapshot to readers that do not take the transaction lock.

## Credential file permissions

On Windows, `CreateFileW` receives a protected security descriptor in
`SECURITY_ATTRIBUTES`; a temporary file never exists with broader inherited
access, even while it is empty. New files and existing files with inherited or
NULL DACLs receive an explicit current-process-user owner and full-control DACL.
An existing non-NULL protected DACL and owner are treated as explicit user policy
and preserved, including deny rules. Descriptor inspection uses an open handle,
and the new file is checked for DACL protection before writing bytes. Unsupported
security application fails explicitly. Read-only destinations fail before
creating a temporary file. The completed temporary file is synced, closed, and
renamed within the same directory with `MoveFileExW` and write-through flags.
Transient reader/scanner sharing failures retry only that rename for at most
250 milliseconds, without repeating an update callback.
Failures before replacement leave original bytes and ACL unchanged.

On Unix, the temporary file is created with mode `0600`; its open handle is set
to `0600` before writing. Rename preserves this mode. The shared
`create_private_directory` helper creates exactly one new directory with Unix
mode `0700` or a protected Windows current-user DACL whose owner grant inherits
to children. Existing directories, files, and reparse points are rejected and
left unchanged. The parent must already exist.

The tests use isolated fixtures and synthetic secrets. They inspect Windows
descriptors before any temp-file bytes, restrictive deny rules, permissive
parents, inherited ACLs, read-only failures, and error cleanup. Two real child
processes use an external TCP controller and a lock-contention handshake to
prove transaction ordering; a killed lock owner proves kernel recovery. Stale
snapshots, whole-file Clear, migration rollback, generated duplicate round trips,
and GUI end-to-end persistence are covered separately.

API references: [Windows file security](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights)
and [Rust file locks](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock).
