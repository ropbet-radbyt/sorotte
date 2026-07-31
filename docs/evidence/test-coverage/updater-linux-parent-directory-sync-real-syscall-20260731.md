# Linux updater parent-directory sync real-syscall evidence

Date: 2026-07-31

Source base before this uncommitted slice:
`c594c290308bf3c0b381a255d75179c7cb177c10`

## Scope and safety

This bounded defensive test exercises only Sorotte's local updater transaction
and recovery code in
`crates/sorotte-gui/src/bin/sorotte-gui-updater.rs`. It creates a nonce-owned
fixture below `std::env::temp_dir()`, runs as the ordinary Ubuntu WSL user, and
changes permissions only on the fixture's updater target directory. It uses no
mounts, devices, privileges, production paths, real disk exhaustion, network
access, persistence, or broad deletion.

`UnixDirectoryPermissionsGuard` restores the directory's exact original
permissions explicitly before recovery and again on unwind if the test panics.
`DurabilityFixtureRoot` removes only its nonce-owned fixture.

## Coverage gap and design

The existing updater suite already covered:

- deterministic injected parent-directory sync failures across the transaction
  matrix;
- authenticated old-or-new recovery and idempotent re-entry;
- a real Windows directory share-denial syscall case.

It did not exercise the Unix `OpenOptions::read(true)` plus directory
`sync_all()` path under a real reversible host denial.

The Linux-only test
`linux_parent_directory_read_denial_recovers_old_install_idempotently` closes
that gap:

1. It prepares a three-file transaction containing an existing-file update, a
   new file, and a removed file.
2. At `ApplyProgress::BeforeReplace(1)`, after all prepared files and the
   journal have been synchronized, it changes only the owned target directory
   to mode `0300` (owner write and search, without owner read).
3. It proves the real directory-sync syscall fails with
   `Permission denied (os error 13)` and asserts that no synthetic storage fault
   is armed.
4. Production `fs::rename(target, backup)` can still execute with write/search
   permission. The immediately following production
   `sync_parent_directory`/`sync_directory` read-open fails with the same host
   denial.
5. The updater retains an authenticated uncommitted journal and exposes a
   complete old install, rather than a mixed target set.
6. After restoring the original permissions, the first recovery removes every
   prepared file, backup, and journal. A second recovery is an idempotent no-op.

The assertions also preserve an unmanaged sentinel inside the target tree and
a sibling sentinel outside the transaction. Every source, target, temporary,
backup, and journal path remains below the nonce-owned fixture.

## Validation

Ubuntu WSL identity and focused real-syscall regression:

```powershell
wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc `
  "id -u && cargo test --locked -p sorotte-gui --bin sorotte-gui-updater linux_parent_directory_read_denial_recovers_old_install_idempotently -- --nocapture"
```

Result: UID `1000`; `1 passed`, `0 failed`, `27 filtered out`.

Complete updater binary suite under Ubuntu WSL:

```powershell
wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc `
  "cargo test --locked -p sorotte-gui --bin sorotte-gui-updater -- --nocapture --test-threads=1"
```

Result: `28 passed`, `0 failed`.

Linux warning-denied lint:

```powershell
wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc `
  "cargo clippy --locked -p sorotte-gui --bin sorotte-gui-updater --all-features -- -D warnings"
```

Result: passed.

Windows cross-platform regression:

```powershell
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater `
  tc_updater_002_parent_directory_sync_failure_retains_authenticated_recovery `
  -- --nocapture
cargo clippy --locked -p sorotte-gui --bin sorotte-gui-updater `
  --all-features -- -D warnings
```

Result: focused test `1 passed`, `0 failed`; Clippy passed.

Static checks:

```powershell
cargo fmt --all -- --check
git diff --check -- crates/sorotte-gui/src/bin/sorotte-gui-updater.rs
```

Result: both passed.

## Finding and limitations

No product defect was found. This slice closes a platform-specific coverage gap
by exercising the existing production Unix durability boundary with a real
kernel permission denial.

The test does not model physical power loss, volatile device caches, real
storage exhaustion, filesystems that misreport `fsync`, mount-specific
durability, or privileged execution. It is intentionally Linux-only and
depends on ordinary-user permission enforcement; this evidence was produced as
UID `1000`. The existing Windows share-denial case remains the platform oracle
for Windows directory handles.
