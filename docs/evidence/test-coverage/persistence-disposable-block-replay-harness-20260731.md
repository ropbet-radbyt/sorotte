# Persistence Disposable Block Replay Harness Evidence — 2026-07-31

## Result

This slice implements a fail-closed, opt-in Linux/WSL capability for extending
Sorotte's SQLite store/worker/platform fault matrix into disposable
block-device replay at explicit flush boundaries.

The implementation and all nonprivileged validation passed. The privileged
`dm-log-writes` capability was **not run**. The read-only WSL preflight found
that `replay-log` is not installed and that the unprivileged process cannot
open `/dev/mapper/control`. No image, loop device, device-mapper mapping,
filesystem, or mount was created or attempted.

Therefore this evidence makes **no power-loss durability claim**. It proves the
safety policy, the Linux build, and the production SQLite worker driver's
plain-temporary-file phase model. A future successful disposable-image report
is required before adding any block-replay durability result.

Files in this slice:

```text
crates/sorotte-server/src/tests.rs
crates/sorotte-server/src/tests/persistence_power_loss_harness_tests.rs
scripts/persistence_power_loss_harness.py
scripts/tests/test_persistence_power_loss_harness.py
docs/evidence/test-coverage/persistence-disposable-block-replay-harness-20260731.md
```

No production Sorotte code changed. The server module addition is test-only.
No workflow or central coverage document changed in this delegated slice.

## Why this is the next persistence boundary

The existing evidence already covers:

- transaction and process-interruption boundaries;
- store- and worker-owned SQLite capacity/write/open faults;
- Windows kernel share denial and Unix namespace denial; and
- normal worker recovery, raw-row equality, SQLite integrity, and reopen.

Those probes do not model which completed block writes survive at an explicit
flush boundary. Linux's `dm-log-writes` target records write data, FUA, and
flush ordering and is specifically intended to replay worst-case power-failure
states. The upstream description and target interface are documented in
[the Linux kernel `dm-log-writes` guide](https://docs.kernel.org/admin-guide/device-mapper/log-writes.html);
the matching userspace replay implementation is
[`josefbacik/log-writes`](https://github.com/josefbacik/log-writes).

This harness uses that mechanism only with new sparse regular files owned by
the current run. It never accepts an existing device, image, mount, mapper, or
loop path.

## Fixed safety contract

The nonprivileged plan schema is:

```text
sorotte-disposable-powerloss-v1
```

Its important invariants are:

1. The privileged mode is Linux-only and requires all of:
   - effective uid `0`;
   - a numeric, non-root `SUDO_UID` and matching `SUDO_GID`;
   - exact confirmation token `sorotte-owned-images-only`;
   - the `log-writes` device-mapper target; and
   - every pinned local tool, including `replay-log`.
2. There is no CLI option for a device, loop, mapper, image, mount, work
   directory, database path, or filesystem target.
3. The workspace is constructed with `mkdtemp` as one new direct child of
   canonical `/var/tmp`, with a `sorotte-powerloss-` prefix and a 32-hex-digit
   nonce recorded in `.sorotte-powerloss-owned-v1`.
4. Every image is created with exclusive, no-follow semantics as one fixed
   regular file directly under that owned root:

   | Image | Exact size |
   |---|---:|
   | `live-data.img` | 268,435,456 bytes |
   | `write-log.img` | 536,870,912 bytes |
   | `replay-baseline.img` | 268,435,456 bytes |
   | `replay-app-ack.img` | 268,435,456 bytes |
   | `replay-syncfs.img` | 268,435,456 bytes |

5. Immediately before loop actions, the harness revalidates the nonce-bound
   root, canonical image path, real regular-file type, owner, exact byte size,
   `/dev/loopN` block-device type, exact loop backing file, and exact block
   size.
6. Immediately before mapper actions, it revalidates both loops and requires
   the exact process/nonce mapper name, mapped size, one exact five-field
   `log-writes` table, sector count, ordered data/log operands whose rendered
   major:minor identities match the recorded loop roles, and the same two
   recorded loop dependencies as an unordered cross-check.
7. Immediately before mount, sync, unmount, replay, `e2fsck`, mapper removal,
   or loop detach, the corresponding recorded binding and all lower ownership
   invariants are checked again.
8. The mapper is formatted only after those checks. No `/dev/sd*`,
   `/dev/nvme*`, `/dev/vd*`, raw physical disk, existing mount, or broad
   directory can be supplied.
9. Every subprocess uses an argument array; the harness does not use a shell,
   `os.system`, or a string-built destructive command.
10. Automatic workspace deletion is disabled. Failure evidence is preserved.
    Best-effort teardown targets only in-memory bindings created and
    revalidated by this nonce; a mismatched binding is left untouched and
    reported.

The Rust driver independently fails closed. It is inert without the exact
enable token, validates the 32-digit nonce and marker, rejects symlinked roots
and directories, and permits only:

```text
<canonical-owned-root>/mount/sorotte/rooms.sqlite3
```

The originating non-root user runs Cargo and the test process. Root is used
only for the disposable loop/mapper/filesystem plumbing.

## Implemented replay phases

The privileged capability, when its preflight eventually passes, performs:

1. create the five fixed sparse images;
2. attach the live and log images with `losetup --nooverlap --direct-io=on`;
3. construct one nonce-named `dm-log-writes` mapping;
4. create ext4 through that mapper and mount only the owned mount directory;
5. write the complete baseline through the production
   `RoomPersistenceService`, require `flush()` acknowledgement, call `sync -f`,
   and insert mark `baseline-flushed`;
6. write the complete higher-version replacement through the same production
   worker, require acknowledgement, and insert mark `replacement-app-ack`;
7. call `sync -f` and insert mark `replacement-syncfs`;
8. unmount and remove the live mapper;
9. replay each mark from the write log into a separate zeroed owned image;
10. mount each replay as a restart/recovery event, then use the production
    store plus an independent raw eight-column query and
    `PRAGMA integrity_check`; and
11. cleanly unmount and require a read-only `e2fsck` result.

The exact recovery contract is:

| Replay cut | Required result |
|---|---|
| `baseline-flushed` | complete baseline |
| `replacement-app-ack` | complete baseline or complete replacement |
| `replacement-syncfs` | complete replacement |

The old-or-new check compares every modeled room field and the raw playlist,
JSON playlist, index, position, activity time, persistence version, owner
bucket, and creation time. A mixed generation, missing row, additional row,
SQLite integrity failure, or unexpected state fails the run.

A completed capability writes a nonce-bound `run-report.json` containing source
hashes, commands, tool output, observed states, failures, and teardown errors.
The report remains in the owned `/var/tmp/sorotte-powerloss-*` directory.

## Read-only WSL preflight

Command:

```bash
python3 scripts/persistence_power_loss_harness.py --preflight
```

Exact deterministic report:

```json
{
  "capability_prerequisites_present": false,
  "destructive_actions_attempted": false,
  "dm_targets": [],
  "dm_targets_error": "/dev/mapper/control: open failed: Permission denied\nFailure to communicate with kernel device-mapper driver.\nIncompatible libdevmapper 1.02.185 (2022-05-18) and kernel driver (unknown version).\nCommand failed.",
  "effective_uid": 1000,
  "log_writes_target_present": false,
  "missing_tools": [
    "replay-log"
  ],
  "mode": "read-only-preflight",
  "platform": "Linux",
  "platform_release": "6.6.87.2-microsoft-standard-WSL2",
  "ready_for_privileged_run": false,
  "repo_paths_present": {
    "Cargo.toml": true,
    "crates/sorotte-server/Cargo.toml": true,
    "crates/sorotte-server/src/tests/persistence_power_loss_harness_tests.rs": true
  },
  "running_as_root": false,
  "schema": "sorotte-disposable-powerloss-v1",
  "sudo_uid_present": false,
  "tools": {
    "blockdev": "/usr/sbin/blockdev",
    "cargo": "/home/shaun/.cargo/bin/cargo",
    "dmsetup": "/usr/sbin/dmsetup",
    "e2fsck": "/usr/sbin/e2fsck",
    "findmnt": "/usr/bin/findmnt",
    "losetup": "/usr/sbin/losetup",
    "mkfs.ext4": "/usr/sbin/mkfs.ext4",
    "mount": "/usr/bin/mount",
    "replay-log": null,
    "runuser": "/usr/sbin/runuser",
    "sync": "/usr/bin/sync",
    "umount": "/usr/bin/umount"
  },
  "wsl": true
}
```

SHA-256 of that exact UTF-8 JSON plus its final newline:

```text
624c3f931ee84ede48fd224efd86953eeb9f5d034bd59e84dd95a11d5db17128
```

The fixed WSL plan JSON SHA-256 was:

```text
c0392082de74e77b62d4c920f217258c8a2342991e684ef078ed3b578b2afe02
```

Stable implementation hashes after validation:

```text
6cab2b07ad899af52372cb22ec492ea240284e278b5a94cfd1588cf524c5ea4a  crates/sorotte-server/src/tests/persistence_power_loss_harness_tests.rs
d51fc819a3fe02f20cdbbb70035ca59c6f84d214f6f6ae9f940b9dff819fcd0a  scripts/persistence_power_loss_harness.py
f94b9d90e5a622f61958dde84c38a54c5b84fa2ecb4140ae13b06723d13a4712  scripts/tests/test_persistence_power_loss_harness.py
```

The denied mapper-control query is a read-only prerequisite check, not a
device action. Because `replay-log` was absent and mapper target availability
could not be established, the harness was not run with `sudo`. No request for
Linux elevation was made.

## Validation completed

### Nonprivileged policy and syntax

```text
python -m unittest scripts.tests.test_persistence_power_loss_harness -v
10/10 passed

python -m py_compile \
  scripts/persistence_power_loss_harness.py \
  scripts/tests/test_persistence_power_loss_harness.py
passed

python scripts/persistence_power_loss_harness.py --plan-json
passed; read-only; no state change
```

The policy tests execute the owned-root and image guards against newly created
ordinary temporary files. They prove rejection of an outside image, wrong
size, symlink, malformed ownership marker, missing confirmation, and
string/shell command execution. They also prove that swapped data/log loop
roles, a wrong log operand, and extra mapper-table fields fail closed; bind the
Rust token/path/integrity contract; and reject physical-device literals.

### Rust and platform compilation

Windows:

```text
cargo test --locked -p sorotte-server --lib \
  disposable_block_driver_path_contract_is_fail_closed -- --nocapture
1/1 passed

cargo test --locked -p sorotte-server --lib \
  room_persistence_disposable_block_driver -- --nocapture
1/1 passed; driver remained inert without the enable token

cargo test --locked -p sorotte-server --lib
368/368 passed

cargo clippy --locked -p sorotte-server --all-targets -- -D warnings
passed
```

Nonprivileged Ubuntu WSL, using the ignored isolated target directory
`target/wsl-powerloss-harness`:

```text
cargo +1.97.1 test --locked -p sorotte-server --lib \
  disposable_block_driver_path_contract_is_fail_closed -- --nocapture
1/1 passed

cargo +1.97.1 test --locked -p sorotte-server --lib \
  room_persistence_disposable_block_driver -- --nocapture
1/1 passed; driver remained inert without the enable token

cargo +1.97.1 test --locked -p sorotte-server --lib \
  disposable_block_driver_phase_model_round_trips_on_plain_temp_store \
  -- --nocapture
1/1 passed

cargo +1.97.1 clippy --locked -p sorotte-server --all-targets -- -D warnings
passed
```

The successful plain-temp phase model exercised the production worker:

```text
baseline write -> worker acknowledgement -> close/reopen -> exact baseline
replacement write -> worker acknowledgement -> close/reopen -> exact replacement
```

That result validates the driver and its complete-state oracle. It is not a
filesystem crash, block replay, cache flush, or power-loss result.

Targeted Rust formatting passed:

```text
rustfmt --edition 2024 --check \
  crates/sorotte-server/src/tests/persistence_power_loss_harness_tests.rs

git diff --check -- <the five slice paths>
passed
```

## How the capability may be run later

Only after installing `replay-log` from a reviewed source and confirming that
the WSL/Linux kernel exposes `dm-log-writes`, first re-run:

```bash
python3 scripts/persistence_power_loss_harness.py --preflight
```

If and only if every prerequisite is present, the explicit run form is:

```bash
sudo --preserve-env=PATH,CARGO_HOME,RUSTUP_HOME \
  python3 scripts/persistence_power_loss_harness.py \
  --run \
  --confirm sorotte-owned-images-only
```

Do not substitute a physical device or existing image. The script has no
option that permits one.

## Exact limitations

- No privileged capability run completed, so there is no observed
  `dm-log-writes` replay, disposable filesystem restart, or power-loss
  durability evidence in this slice.
- The successful plain-temp production-worker sequence proves logical
  write/ack/reopen behavior only.
- Even a future successful run proves only the recorded ext4,
  `dm-log-writes`, tool, kernel, and three named replay cuts. It does not prove
  physical media, controller or drive write caches, torn sectors, real host
  power removal, NTFS, another filesystem, or another kernel.
- The `replacement-app-ack` cut intentionally allows old or new complete
  state. It does not assert that worker acknowledgement alone is a physical
  storage guarantee.
- The capability checks three semantically chosen marks, not every individual
  write, FUA, or flush entry in the log.
- WSL device-mapper support under elevation was not probed because elevation
  was neither required for the safe implementation work nor authorized for
  this run.
- No product defect surfaced. The known-defect registry remains unchanged.
