# Deep boundary test slice evidence

Date: 2026-07-30

Branch: `codex/test-coverage-design`

Committed base: `b57e43cd65e1b7ef46c56282e8806bf67e080146`

Platform: Windows, Rust 1.97.1 workspace toolchain

## Objective

This slice closes four independent gaps from the application-wide test
strategy:

1. reconnect playlist ownership after restore emission;
2. TLS bundle observation under real filesystem faults and mid-capture
   replacement;
3. updater rollback and recovery-journal state space;
4. mpv IPC behavior at the real Windows named-pipe boundary.

Each stream used the smallest production boundary that could encode the
behavior mechanically. Test-only seams select schedules or initial state; they
do not add success behavior to a production state machine. Product defects
remain executable expected-failure characterizations until deliberately
resolved.

## Results

| Stream | New proof | Stress / breadth | Result |
|---|---|---:|---|
| reconnect acknowledgement lifecycle | independent shrinkable model plus three deterministic histories | 4,096 generated histories; complete all-feature client-core 705/705 | green; no defect |
| TLS captured snapshot | 216 real-filesystem histories, equal-length/mtime collisions, captured-byte loading, and two mid-read replacement boundaries | 648 modeled transitions; complete server package including release verification | one open product defect, `TC-SERVER-004` |
| updater rollback journal | before/after replacement injection and a multi-file reference matrix | 75 schedules across 6 boundary failures, 4 crash prefixes, 54 artifact faults, 4 valid-temp cases, reverse ordering, and 6 link cases | green; no defect |
| Windows mpv kernel transport | seven deterministic tests over the production overlapped named-pipe client | 175/175 stress replays; complete all-feature player package 419 passed / 2 declared ignored | green; no defect |

The first combined workspace run also surfaced and then closed
`TC-HARNESS-015`: the parent CLI libtest could observe a temporary child
fixture role and terminate itself with the leaf's intentional exit code 23.

## Reconnect acknowledgement ownership

The earlier reconnect model reached the initial post-Hello playlist snapshot.
The new model deliberately begins at the narrower ownership boundary after an
empty snapshot arms restoration and after the runtime emits that restore.

The oracle independently tracks:

- active or reconnecting phase and monotonic attempt number;
- current playlist;
- captured reconnect snapshot;
- armed restore intent;
- emitted restore awaiting acknowledgement;
- shared-playlist capability;
- reconnect scheduling and emitted restore actions.

Generated steps include another disconnect, deferred/empty/authoritative
generation snapshots, repeated drains, matching echoes, later empty updates,
divergent authority, and capability disablement. The production session and
reference model are compared after every step. Three named regressions make
the highest-risk histories readable:

1. a second disconnect before acknowledgement re-arms the desired playlist;
2. divergent authority supersedes both armed and already-emitted restore
   ownership;
3. disabling shared playlists permanently discards restore ownership.

The stress run used `PROPTEST_CASES=4096`. The reconnect family passed 73/73,
the complete client-core package passed 703/703, and its all-feature variant
passed 705/705.

## TLS observation and publication

The real-filesystem model enumerates three-step histories from six operations:
stable observation, each of three missing members, a complete invalid
revision, and a complete valid revision. It compares cached context,
fingerprint retention/advance, retry count, TLS acceptability, response, and
transport action across 216 histories and 648 transitions without sleeps.

Additional positive tests prove:

- equal-length edits to each required member remain visible when all mtimes
  collide;
- missing members and later valid/invalid reappearance preserve retry rules;
- terminal retry exhaustion remains terminal;
- rustls loads the already captured bytes after the on-disk files change.

The per-member reader seam then creates complete generations A and B, verifies
both are independently rustls-loadable, and replaces the observed generation
after read 1 and after read 2. Both resulting mixed fingerprints match neither
complete generation, yet rustls accepts both snapshots. The exact
characterization is:

```text
tests::tls_snapshot_fault_tests::known_defect_tls_snapshot_can_mix_members_replaced_during_observation
```

The defect is registered as `TC-SERVER-004`, “Sequential TLS bundle reads can
install a cross-generation snapshot.” Fingerprinting cannot establish a
generation boundary for independently mutable loose files. The complete
solution is immutable versioned bundle directories plus an atomically replaced
manifest/pointer; a bounded identical double-capture is useful only as a
backward-compatible race-reduction fallback.

Focused snapshot tests passed 5/5. The TLS selector passed 31/31 plus release
verification. The complete server package passed 342 library, 14 binary-unit,
2 integration, and 6 release-verification tests. Warning-denied server Clippy
passed.

## Updater rollback and journal recovery

A test-only post-replacement hook completes the existing failure boundary
without changing release behavior. The reference setup uses one replaced
existing file, one newly added file, and one removed file. It distinguishes
committed from uncommitted journals and temporary, target, and backup
artifacts.

The matrix covers:

- failure before and after each of three replacements;
- every installed prefix from zero through all three files;
- missing, corrupt, and cross-file-substituted temporary/target/backup state
  in committed and uncommitted recovery;
- authenticated leftover temporary cleanup;
- observable reverse rollback order;
- link/reparse-point substitution for every artifact in both journal modes;
- idempotent successful recovery and stable fail-closed diagnostics on
  ambiguous re-entry.

The model executes 54 artifact-fault schedules and 75 schedules in total.
Target and backup bytes that decide installed state remain authenticated.
Uncommitted temporary bytes are treated only as disposable scratch. The
updater suite passed 28/28, its focused fault matrix passed 10/10 stress
replays, and warning-denied all-feature GUI Clippy passed.

## Windows mpv named-pipe transport

The fixture creates unique real `\\.\pipe\...` instances with blocking
server-side barriers while the production client uses its normal overlapped
transport. It records exact request frames and kernel write sizes.

The seven tests prove:

- one-byte writes split JSON and multibyte UTF-8 at every boundary;
- one coalesced kernel write carries two events followed by one response
  without reordering;
- requests remain exactly one newline-delimited JSON frame;
- stale, future, and duplicate response IDs fail correlation and terminally
  fence reuse;
- a partial JSON prefix followed by pipe close fails boundedly;
- server disappearance after a request and before a request cover the two
  named-pipe disconnect directions;
- a withheld response cancels at the bounded production command deadline;
- a new client can recover on the same pipe name with a new logical
  generation and independent request IDs;
- request correlation remains correct across `u64::MAX -> 0 -> 1`.

Windows named pipes do not expose socket-style independent read/write
half-close. This suite therefore claims both observable broken-pipe
directions, not a synthetic FIN-only half-close.

The new seven tests passed 25 consecutive repetitions (175/175). The focused
named-pipe inventory passed 8/8 including the pre-existing cancellation test.
The complete all-feature player package passed 419 tests with its two declared
manual/fixture ignores unchanged. Warning-denied all-target/all-feature Clippy
passed.

## TC-HARNESS-015

The first locked all-feature workspace attempt stopped in the CLI test binary:

```text
process didn't exit successfully ... sorotte_cli-....exe (exit code: 23)
```

Code 23 belongs to the intentional `early-exit-leaf` fixture. The parent exact
fixture entrypoint was also an ordinary parallel test and read its
process-global role without the mutex held by role-mutating tests.

The entrypoint now observes and dispatches the role while holding the same
`TestEnvGuard`. A barrier-driven regression proves observation cannot complete
while a transient role is installed and sees the restored value after release.
The owning module passed 15/15, CLI Clippy passed, and ten consecutive complete
all-feature CLI library runs passed 10/10 in 111.7 seconds.

## Integrated validation

The final combined tree passed:

```text
cargo fmt --all -- --check
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python -m unittest discover -s scripts/tests -p "test_*.py"
C:\Users\shaun\go\bin\actionlint.exe
python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml \
  --catalog coverage/behaviors.toml \
  --repo-root .
git diff --check
```

Measured final results:

- locked all-feature workspace: exit 0 in 205.1 seconds;
- warning-denied workspace Clippy: exit 0 in 7.07 seconds;
- infrastructure/policy suite: 354/354 in 13.910 seconds;
- actionlint: no findings;
- known-defect policy: exactly 1 defect and 1 characterization;
- formatting and whitespace checks: clean.

The open TLS characterization passes only because its exact expected invariant
panic occurs. The registry prevents that expected failure from being presented
as positive behavior evidence. All other tests in this slice are ordinary
positive regressions.
