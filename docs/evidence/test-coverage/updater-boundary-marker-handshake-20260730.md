# Updater boundary-marker handshake evidence — 2026-07-30

## Result

`TC-HARNESS-016` is resolved. The updater process-interruption child now
publishes its boundary marker atomically, and the parent acknowledges only the
complete expected payload. The exact process regression passed 100/100 serial
replays, covering 1,100 durable interruption boundaries and 2,200 recovery
subprocesses. No product defect surfaced.

The recorded implementation base was
`8fc81f652d0ca0978150919b91ff6c07d8cb4174`. The complete updater source
`crates/sorotte-gui/src/bin/sorotte-gui-updater.rs` was 3,814 lines and had
SHA-256
`C5B8B6486A305DE28CEA3EF141A127A153DD5CE3F406D38CE98E12BE0F778459`
at the recorded validation point.

## Root cause

The child previously used `fs::write(root.join("boundary-reached"), label)`.
On Windows, marker creation and payload completion are separately observable.
The parent polled only `.exists()`, then immediately read the file and required
the complete label. It could therefore observe a zero-length file before
`fs::write` completed. The failure happened before child termination and
before either recovery process, so it was a harness race rather than updater
recovery evidence.

## Corrected handshake

The test-only child now:

1. creates a same-directory `.boundary-reached.pending` file with
   `create_new`;
2. writes the complete label;
3. flushes and `sync_all`s the file;
4. closes the file; and
5. renames it to `boundary-reached`.

The test parent repeatedly reads the published path and proceeds only when its
bytes exactly equal the expected boundary. It retains the existing premature
child-exit and deadline checks. A deterministic preflight proves that empty,
partial, and incorrect marker contents do not acknowledge readiness, while
atomic publication produces the exact marker and leaves no pending file.

The existing process-test name is unchanged, so the strict Windows process
inventory remains stable.

## Executed proof

Exact regression:

```powershell
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater `
  tests::process_interruption_tests::real_process_termination_recovers_every_durable_transaction_boundary `
  -- --exact --nocapture
```

Result: passed.

A bounded serial stress invoked that exact selector 100 times. All 100 passed
in 64.8 seconds. The complete updater binary suite passed 30/30:

```powershell
cargo test --locked -p sorotte-gui --bin sorotte-gui-updater -- --nocapture
```

Strict GUI lint, formatting, diff whitespace, and the process-lane policy
suite also passed:

```powershell
cargo clippy --locked -p sorotte-gui --all-targets --all-features `
  -- -D warnings
cargo fmt -p sorotte-gui -- --check
python -m unittest `
  scripts.tests.test_coverage_windows_process_lanes `
  scripts.tests.test_windows_process_coverage_workflow -v
git diff --check -- crates/sorotte-gui/src/bin/sorotte-gui-updater.rs
```

The policy suite passed 17/17.

## Scope limits

This correction hardens a test-only same-directory process handshake. It does
not change production updater behavior and does not claim parent-directory
`fsync`, kernel-cache survival, power-loss durability, or atomic rename
semantics across filesystems. Those remain separate production durability
boundaries.
