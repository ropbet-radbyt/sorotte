# Test-coverage continuation handoff: next four slices

> **Historical closure notice — 2026-08-01:** This handoff is preserved as the
> pre-implementation checkpoint recorded at `2084a24`. Every “Remaining work”
> block and the final execution order below has since been completed. The four
> primary implementation commits are `b9e8c2b`, `af494bd`, `716bb4e`, and
> `dedb073`; the integrated evidence was finalized through `289845a`. Exact
> documentation-inclusive hosted acceptance subsequently passed at
> `612917ac8461040549217453bdebfc5001f2378c` in workflow `30679354953`.
> Current capability-scoped follow-up is listed in
> [`hosted-ci-closure-20260801.md`](hosted-ci-closure-20260801.md); do not use
> the historical remaining-work lists below as current branch status. The
> original handoff body is otherwise retained unchanged for provenance.

Date: 2026-07-31<br>
Repository: `C:\tmp\sorotte-test-coverage-design`<br>
Branch: `codex/test-coverage-design`<br>
Recorded baseline HEAD: `2084a24d7408063baa4b07b4e29af3bbd2cf68b7`<br>
Recorded remote head before this dirty tranche: `2084a24d7408063baa4b07b4e29af3bbd2cf68b7`

## Read this first

Continue only in `C:\tmp\sorotte-test-coverage-design` on
`codex/test-coverage-design`. Preserve every dirty and generated file. Do not
reset, restore, clean, switch branches, or overwrite evidence roots.

This tranche is bounded defensive QA of Sorotte's own local Rust JSON
framing/session, updater, GUI/player, and Python-compatibility code. Generated
framed input is consumed only in memory by local functions. There is no network
target, reconnaissance, credential work, persistence, privilege work, or
exploitation. The automated cyber warning that interrupted the framed-input
subagent was a false positive.

The source handoff for the broader campaign was read in full before this tranche:

- `docs/evidence/test-coverage/test-slices-handoff-20260730-more-slices.md`

No file from this tranche has been staged, committed, or pushed.

## Frozen branch and worktree state

The prior completed batch ended in these pushed commits:

1. `583716a` `Add interactive native CI contract`
2. `1a280b4` `Add native GUI real-mpv vertical`
3. `aa697f0` `Add disposable persistence replay harness`
4. `c34f132` `Verify tested server container publication`
5. `2084a24` `Update test coverage strategy`

The current dirty tranche starts from `2084a24`. The current tracked modifications
are:

```text
.github/workflows/rust-ci.yml
.github/workflows/rust-fuzz.yml
crates/sorotte-compat/src/legacy_process.rs
crates/sorotte-compat/src/lib.rs
crates/sorotte-compat/src/python_probe/process.rs
crates/sorotte-compat/src/tests.rs
crates/sorotte-compat/src/tests/assertions/legacy_client_assertions.rs
crates/sorotte-compat/src/tests/assertions/legacy_process_assertions.rs
crates/sorotte-compat/src/tests/assertions/legacy_tls_assertions.rs
crates/sorotte-gui/src/bin/sorotte-gui-native-smoke/native_smoke_runner/real_mpv_vertical.rs
crates/sorotte-gui/src/bin/sorotte-gui-updater.rs
crates/sorotte-player-mpv/Cargo.toml
crates/sorotte-player-mpv/src/ipc.rs
crates/sorotte-player-mpv/src/lib.rs
fuzz/Cargo.lock
fuzz/Cargo.toml
fuzz/run_protocol_fuzz.py
scripts/coverage_profile_lanes.py
scripts/gui-real-mpv-vertical.ps1
scripts/gui_real_mpv_vertical_contract.py
scripts/tests/test_ci_policy.py
scripts/tests/test_gui_real_mpv_vertical_contract.py
scripts/tests/test_protocol_fuzz_policy.py
```

The current untracked files are:

```text
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/bytewise-event-before-response.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/bytewise-partial-read-timeout.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/bytewise-response-id-reorder.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/coalesced-invalid-json.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/coalesced-success.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/coalesced-write-disconnect.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/fixed-width-dropped-response-disconnect.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/fixed-width-duplicate-events.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/fixed-width-server-rejection.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/pseudorandom-malformed-json.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/pseudorandom-write-timeout.txt
crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript/response-barrier-ignores-trailing-malformed.txt
docs/evidence/test-coverage/compat-required-live-interop-20260731.md
docs/evidence/test-coverage/native-gui-real-mpv-owned-process-recovery-20260731.md
docs/evidence/test-coverage/updater-transaction-storage-durability-20260731.md
fuzz/fuzz_targets/mpv_framed_transcript.rs
scripts/compat_live_interop.py
scripts/tests/test_compat_live_interop.py
```

This handoff file is an additional untracked file.

Generated evidence under `target/` and build output under `fuzz/target/` are
intentionally ignored by Git. Preserve the named bundles below.

## Subagent outcomes

Four subagents were assigned one slice each:

| Slice | Outcome |
|---|---|
| Native GUI real-mpv owned-process recovery | Complete; reviewed by the root agent |
| Required live Python compatibility | Complete; reviewed by the root agent |
| Updater transaction storage durability | Complete; reviewed by the root agent; identifier corrected |
| In-memory framed mpv transcript randomized target | Implementation and campaigns substantially complete; subagent response interrupted twice by the false-positive classifier |

The interrupted subagent did not stage, commit, push, start a network service, or
leave a related campaign process running. At the final process check, only the
ordinary WSL service remained.

## Slice 1: native GUI real-mpv owned-process recovery

Status: implementation, focused validation, evidence, and canonical local
campaign complete.

Files:

- `crates/sorotte-gui/src/bin/sorotte-gui-native-smoke/native_smoke_runner/real_mpv_vertical.rs`
- `scripts/gui-real-mpv-vertical.ps1`
- `scripts/gui_real_mpv_vertical_contract.py`
- `scripts/tests/test_gui_real_mpv_vertical_contract.py`
- `docs/evidence/test-coverage/native-gui-real-mpv-owned-process-recovery-20260731.md`

Behavior added:

- Opt-in `-ExerciseOwnedMpvRecovery`.
- Establish the ordinary Play/Pause baseline.
- Terminate only the attested GUI-owned mpv PID.
- Observe automatic owned-player relaunch in the active GUI session.
- Reject old or foreign post-boundary process observations.
- Re-open media and prove Play/Pause through the replacement mpv.
- Exit through the native GUI and prove all owned PIDs are gone.

The first preserved RED bundle assumed that a manual Retry modal was required.
The product instead automatically relaunched mpv. This was a harness/oracle
error, not a product defect:

```text
target/verification/gui-real-mpv-owned-process-recovery/20260730T222451895Z-64648
```

The corrected GREEN bundle is:

```text
target/verification/gui-real-mpv-owned-process-recovery/20260730T222937617Z-55004
```

Recorded result:

- 20 assertions and 13 artifacts.
- GUI PID `65976`.
- Initial mpv PID `50296`.
- Replacement mpv PID `50600`.
- Distinct IPC endpoints.
- Exact binary, digest, and version attestation.
- Replacement media open, Play, and Pause succeeded.
- Old/foreign post-boundary observations were rejected.
- All PIDs were absent after native Exit.

Healthy-path regression bundle:

```text
target/verification/gui-real-mpv-vertical/20260730T223430082Z-64900
```

That retained the existing 13 assertions and 10 artifacts.

Focused validation reported by the slice owner:

- Python contract tests: 12/12.
- Focused Rust tests: 6/6.
- Focused `cargo check`: passed.
- Focused warning-denied Clippy: passed.
- Rust formatting: passed.
- Recovery and healthy-path native campaigns: passed.

Remaining work:

1. Review the final combined diff again after the other slices settle.
2. Commit the implementation.
3. After all full build-producing gates, rerun both the recovery campaign and
   healthy regression so the recorded GUI digest matches the final built
   binary.
4. Update the evidence file to the final canonical bundles and digest.

## Slice 2: required live Python compatibility

Status: implementation, focused validation, evidence, and complete local matrix
complete.

Files:

- `.github/workflows/rust-ci.yml`
- `crates/sorotte-compat/src/legacy_process.rs`
- `crates/sorotte-compat/src/lib.rs`
- `crates/sorotte-compat/src/python_probe/process.rs`
- `crates/sorotte-compat/src/tests.rs`
- `crates/sorotte-compat/src/tests/assertions/legacy_client_assertions.rs`
- `crates/sorotte-compat/src/tests/assertions/legacy_process_assertions.rs`
- `crates/sorotte-compat/src/tests/assertions/legacy_tls_assertions.rs`
- `scripts/compat_live_interop.py`
- `scripts/coverage_profile_lanes.py`
- `scripts/tests/test_compat_live_interop.py`
- `scripts/tests/test_ci_policy.py`
- `docs/evidence/test-coverage/compat-required-live-interop-20260731.md`

Behavior added:

- Central exact opt-in: `SYNCPLAY_REQUIRE_LIVE_INTEROP=1`.
- Missing oracle checkout, unsupported Python, missing TLS dependencies,
  incomplete fixtures, or missing assertions now fail closed in required mode.
- The ordinary optional mode remains unchanged.
- The wrapper pins Syncplay SHA
  `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`.
- Supported CPython is `>=3.11,<3.14`.
- Exact Python requirements are:
  - `twisted==25.5.0`
  - `pyopenssl==25.3.0`
  - `service_identity==24.2.0`
- The tracked fixture inventory is exact: 89 total, comprising 24 protocol
  fixtures, 62 scenario fixtures, and 3 TLS fixtures.
- CI runs a selector-free complete matrix with a closed evidence schema.

Canonical local matrix:

- 143 tests listed.
- 136 tests executed and passed.
- 0 failed.
- 0 optional skips.
- 7 exact ignored fixture writers.
- Duration `47.521699s`.
- Local interpreter CPython 3.13.5; CI remains pinned to Python 3.11.

Missing-oracle checks:

- The wrapper exits 1 with a valid report.
- The direct formerly early-returning Rust test exits 101 in required mode.
- The same direct test remains green in optional mode without the environment
  variable.

Focused validation reported by the slice owner:

- Combined Python policy tests: 53/53.
- Required-live Rust tests: 2/2.
- Focused warning-denied Clippy: passed.
- `actionlint`: passed.
- Rust formatting: passed.
- Python compilation and diff checks: passed.

Evidence transparency:

- An early parser-RED report path was reused while correcting libtest's ignored
  reason suffix accounting.
- The raw log/result was green; the precise report-accounting RED is documented.
- A final distinct evidence bundle was retained.

Remaining work:

1. Recheck the combined workflow and wrapper diff.
2. Commit the implementation.
3. Rerun the complete required-live matrix from committed source so its source
   SHA is the exact implementation commit.
4. Update the evidence file with that committed SHA and final bundle.

## Slice 3: updater transaction storage durability

Status: product defect found and fixed; deterministic fault schedules, Windows
integration, focused validation, and evidence complete.

Files:

- `crates/sorotte-gui/src/bin/sorotte-gui-updater.rs`
- `docs/evidence/test-coverage/updater-transaction-storage-durability-20260731.md`

Important identifier correction:

- The slice owner initially used `TC-UPDATER-001`.
- `TC-UPDATER-001` was already assigned to prepared-file rollback cleanup in
  the central findings.
- The root agent corrected this new directory-sync finding and its exact test
  selector to `TC-UPDATER-002`.
- The exact selector is now
  `tc_updater_002_parent_directory_sync_failure_retains_authenticated_recovery`.

Defect:

- File and journal contents were synchronized, but directory-entry changes did
  not synchronize the parent directory.

Fix:

- Unix opens and synchronizes the parent directory.
- Windows opens a writable directory with `FILE_FLAG_BACKUP_SEMANTICS` and
  synchronizes it through `FlushFileBuffers`.
- Directory synchronization now follows journal/temp creation,
  rename/`ReplaceFileW`, relative directory creation, rollback, cleanup
  deletion, and journal removal.

Coverage added:

- Test-only thread-local one-shot fault seam for Write, Flush, Replace, Remove,
  and ParentDirectorySync.
- Thirteen deterministic storage-failure schedules.
- Assertions for complete old-or-new bytes, authenticated journal selection,
  cleanup, idempotent recovery, and an out-of-scope sentinel.
- A reversible real Windows directory-share denial test.

Focused validation reported by the slice owner before the identifier rename:

- Updater suite: 33/33.
- All 13 deterministic schedules: passed.
- Windows integration runner: exit 0 with exactly two tests listed.
- Focused warning-denied Clippy: passed.
- Rust formatting and slice diff checks: passed.
- No durability fixture roots remained.

Evidence limits are explicit: injected disk-full/access-denied analogues and an
OS-accepted flush are not physical power-loss, controller-cache, or torn-sector
evidence.

Remaining work:

1. Rerun the renamed exact selector:
   `tc_updater_002_parent_directory_sync_failure_retains_authenticated_recovery`.
2. Rerun the complete updater suite and Windows integration check.
3. Commit the implementation.
4. Add `TC-UPDATER-002` as found-and-fixed to the central findings. The current
   known-defect registry should remain empty because the defect is fixed in this
   tranche.

## Slice 4: in-memory framed mpv transcript randomized target

Status: implementation and short campaigns substantially complete, but the
slice needs root review, a missing evidence document, a committed-source
canonical campaign, and full validation.

Files:

- `.github/workflows/rust-fuzz.yml`
- `crates/sorotte-player-mpv/Cargo.toml`
- `crates/sorotte-player-mpv/src/ipc.rs`
- `crates/sorotte-player-mpv/src/lib.rs`
- the 12 corpus files listed in the frozen inventory
- `fuzz/Cargo.toml`
- `fuzz/Cargo.lock`
- `fuzz/fuzz_targets/mpv_framed_transcript.rs`
- `fuzz/run_protocol_fuzz.py`
- `scripts/tests/test_protocol_fuzz_policy.py`

The missing evidence file should be created as:

```text
docs/evidence/test-coverage/framed-mpv-ipc-transcript-coverage-guided-20260731.md
```

Scope and structure:

- Generated bytes stay in local `Vec` values and local parser/session
  functions.
- There are no sockets, process launches, addresses, credentials, persistence,
  privileges, or third-party targets.
- The exact seed corpus contains 12 tracked transcripts.
- The target covers four chunk schedules and five terminal modes, including
  malformed/truncated JSON, response-ID mismatch, duplicate/drop/order cases,
  and the response-barrier behavior.

Focused state reported before the classifier interruption:

- Python policy tests: 20/20.
- Ordinary `sorotte-player-mpv` all-feature compilation: passed.
- Nine focused transcript tests: passed.
- Pinned WSL nightly ASan target build: passed in 27.67 seconds.

Pinned tools:

- Rust toolchain `nightly-2026-07-29`.
- `cargo-fuzz 0.13.2`.
- AddressSanitizer.
- WSL Ubuntu on Linux `6.6.87.2`.
- Campaign Python `3.12.3`.

### Preserved short RED campaign

```text
target/fuzz-ci/mpv-framed-transcript-smoke-20260731-v1
```

Recorded result:

- Status failed; runner exit 1.
- 54 executions.
- 13 new units.
- Peak RSS 57 MiB.
- One preserved failure artifact.
- Artifact aggregate SHA-256:
  `f45139c745413ebd00a97acac618bcc2426d8eddc712a2fbee617fb86ac31d83`.
- Artifact name starts
  `crash-c5663af`.
- Artifact size: 69 bytes.
- Artifact SHA-256:
  `4d41ace74be6233d002d181954c97a674f64751f2819e92fbd1f52cdcea0336f`.
- Source inventory: 64 files, stable.
- Seed inventory: 12 files, stable.
- Evidence-schema errors: none.

Failure:

```text
production outcome Disconnected disagrees with reference ProtocolCorruption
```

The input placed malformed bytes/JSON after a seek event. The working diagnosis
is a target/reference-oracle classification mismatch rather than a product
defect. Confirm that diagnosis against the actual diff before documenting it.
The attempted automatic minimization hit cargo-fuzz/libFuzzer's
`SetMaxInputLen` assertion and produced a zero-byte minimized output. Preserve
both the original artifact and the failed minimization evidence.

### Corrected short GREEN campaign

```text
target/fuzz-ci/mpv-framed-transcript-smoke-20260731-v2
```

Recorded result:

- Status passed; runner exit 0.
- 58,462 executions.
- Average 1,885 executions/second.
- 1,172 new units.
- Peak RSS 442 MiB.
- Failure artifact count: 0.
- Empty-artifact aggregate SHA-256:
  `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945`.
- Final generated corpus: 588 units.
- Source inventory: 64 files.
- Source before/after aggregate:
  `afccf720e1b276254b0fd6309583b6206d8c1095ee0be6f34f786590e5d12e14`.
- Seed inventory: 12 files, stable.
- Evidence-schema errors: none.

This short GREEN bundle is not canonical because its recorded source SHA is the
dirty baseline `2084a24`, not a commit containing the target implementation.

Remaining work:

1. Review the entire slice diff; the subagent was interrupted before it could
   provide a final checkpoint.
2. Confirm that the RED correction preserves an independent, appropriately
   strict oracle rather than weakening it to match production.
3. Rerun the 20 policy tests, nine Rust transcript tests, ordinary all-feature
   compile, warning-denied Clippy, and pinned nightly ASan target build.
4. Write the missing evidence document with both v1 and v2 paths, hashes,
   statistics, correction rationale, and minimization limitation.
5. Commit the bound implementation.
6. Run a fresh 180-second canonical campaign with the implementation commit as
   `--source-sha` and a new output root. Inspect the runner's `--help` and policy
   tests first; the intended shape is:

   ```text
   wsl.exe -d Ubuntu --cd /mnt/c/tmp/sorotte-test-coverage-design bash -lc "python3 fuzz/run_protocol_fuzz.py --target mpv_framed_transcript --toolchain nightly-2026-07-29 --source-sha <implementation-commit> --seconds 180 --seed-corpus crates/sorotte-player-mpv/tests/corpus/framed_ipc_transcript --expected-seed-count 12 --output-root target/fuzz-ci/mpv-framed-transcript-deep-<implementation-commit>-v1"
   ```

7. Require a stable 12-file seed inventory, stable tracked-source inventory,
   zero artifacts, a closed evidence schema, and no unexplained runner errors.
8. Update the evidence file to identify that fresh bundle as canonical.

## Recommended safe continuation order

1. Read this file and the original 2026-07-30 handoff completely.
2. Confirm `git status --short --branch`, the exact branch, HEAD, and the full
   dirty inventory. Do not mutate anything during orientation.
3. Review each slice's diff and evidence, with particular attention to the
   interrupted framed-transcript target and the `TC-UPDATER-002` rename.
4. Rerun each slice's focused tests.
5. Create focused implementation commits. A sensible sequence is:
   - `Add real-mpv owned process recovery`
   - `Require complete live compatibility`
   - `Sync updater transaction directories`
   - `Add framed mpv transcript randomized testing`
6. Run the committed-source required-live compatibility matrix.
7. Run the committed-source 180-second framed-transcript canonical campaign.
8. Complete and update all four evidence documents.
9. Update:
   - `coverage/README.md`
   - `docs/TEST_COVERAGE_STRATEGY.md`
   - `docs/TEST_COVERAGE_FINDINGS.md`
10. Add the found-and-fixed `TC-UPDATER-002` finding and retain an empty current
    known-defect registry.
11. Run full validation:
    - `cargo fmt --all --check`
    - `git diff --check`
    - `actionlint` for changed workflows
    - all affected Python policy/contract suites
    - mutation policy
    - known-defect policy
    - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    - `cargo test --workspace --all-features`
12. After all build-producing gates, rerun the native real-mpv recovery and
    healthy-path campaigns, then update their final evidence digest and bundle
    paths.
13. Commit the evidence and central-document updates in a focused commit.
14. Confirm the worktree contains only intentional files.
15. Push `codex/test-coverage-design`.
16. Verify the live remote branch head equals the final local HEAD.

Do not describe the tranche as complete until all four committed-source
campaigns/evidence records, full gates, focused commits, push, and remote-head
verification have succeeded.

## Suggested continuation prompt

```text
Continue the Sorotte test-coverage work from
C:\tmp\sorotte-test-coverage-design\docs\evidence\test-coverage\test-slices-handoff-20260731-next-four-slices.md.
Read the entire handoff first and work only in
C:\tmp\sorotte-test-coverage-design on codex/test-coverage-design. Preserve
every dirty and generated file. This is bounded defensive QA of our own local
Rust JSON framing/session, updater, GUI/player, and compatibility code.
Generated framed input is processed only in memory, with no network target,
reconnaissance, credentials, persistence, privilege work, or exploitation.
The prior automated cyber flag was a false positive. Finish review of the four
slices, focused implementation commits, committed-source canonical campaigns,
missing evidence, central docs, full validation, the final real-mpv regression,
push, and live remote-head verification.
```
