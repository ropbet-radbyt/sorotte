# Windows process coverage profile experiment — 2026-07-30

## Result

The Windows/MSVC process-profile extension passed a real local experiment on
Windows. Fifty exact, non-interactive tests ran across five behavioral lanes.
Every execution lane produced a fresh or changed raw LLVM profile, all 34
retained raw profiles merged successfully, and the final merge check left the
raw-profile inventory unchanged. The profile count is an observed inventory,
not a brittle expected constant: exact tests, filtered counts, per-lane fresh
deltas, inventory continuity, and the final merge are the enforced contracts.

This is a separate `windows-x86_64-msvc` coverage domain. It is intentionally
not raw-profile-merged with the Linux artifact. Interactive Windows UI
Automation is also intentionally excluded: native smoke remains an
uninstrumented, separately attested test signal.

No product defect surfaced in this experiment.

## Producer and source identity

- source HEAD: `dccb319d28766a97f79c04e70ebff575c1396fe6`
- source working-tree SHA-256:
  `9653f61d4246864a73f82186df71a63ba2541ba654ca60b0754d3a8a50f641b3`
- source state: dirty, with ten untracked files captured by path, size, and
  content digest in the attestation
- `cargo-llvm-cov`: `0.8.4`
- Rust: `1.97.1`
- rustc commit: `8bab26f4f68e0e26f0bb7960be334d5b520ea452`
- rustc host: `x86_64-pc-windows-msvc`
- LLVM: `22.1.6`
- isolated target: `target/llvm-cov-windows-process`

The dirty state is expected in the shared implementation worktree. The report
does not silently call it clean: it binds the committed HEAD, the binary Git
diff digest, and each untracked file's content digest into a recomputable
working-tree digest. CI checkouts are expected to attest `is_clean: true`.

## Exact behavioral inventory

| Lane | Passed | Filtered out | Profiles before → after | Fresh deltas | Duration |
| --- | ---: | ---: | ---: | ---: | ---: |
| updater transaction/process | 30 | 0 | 0 → 23 | 23 | 3.311 s |
| installed-updater self-replacement | 2 | 0 | 23 → 28 | 5 | 1.488 s |
| mpv Windows named-pipe faults | 8 | 415 | 28 → 29 | 1 | 0.432 s |
| mpv external child-process faults | 3 | 420 | 29 → 32 | 3 | 0.451 s |
| media-tool child-process faults | 7 | 1116 | 32 → 34 | 2 | 1.099 s |
| final merge check | n/a | n/a | 34 → 34 | 0 | 1.074 s |

The first lane establishes cargo-llvm-cov's owned profile/build domain. Later
Cargo processes inherit the exact `cargo llvm-cov show-env` instrumentation
contract and use that owned build directory. The collector rejects:

- zero selected tests;
- a missing, extra, duplicated, ignored, or skipped test;
- a changed filtered-test count;
- a non-zero command exit;
- a profile lane with no fresh/changed non-empty `.profraw`;
- deletion or discontinuity of a preceding lane's profiles;
- a profile outside the isolated Windows target;
- a merge that changes raw-profile inventory;
- producer, source, command, selector, environment, schema, or boundary drift.

## Coverage artifacts

The retained local experiment files are ignored build artifacts under
`target/`; the scheduled workflow emits equivalent canonical artifact names.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `target/verification/coverage-windows-process-lanes-local-20260730.json` | 28,260 | `272a66ed111e58cacbb51ee190e0fa656d1ea208e15d10db868363b54d57ec2a` |
| `target/coverage-windows-process-local-20260730.json` | 11,320,751 | `91fd4f25480d44b89eaee10806cab443bf23f8faacf7297143f89e4d5cc3805d` |
| `target/coverage-windows-process-local-20260730.txt` | 12,553,608 | `a2b7bc6b515a30505534ed0422e8b4c7c6ec79ff59de5ac4e2644146c5293d2d` |
| `target/coverage-windows-process-line-map-local-20260730.json` | 7,638,931 | `f12be04eef6a0e0796d898ce37f044317a1cff48a89b24caafb609d76e6942e4` |

The source-bound physical line map accepted the Windows LLVM exports and
reported 2,054 covered physical lines out of 156,297 (1.314165%). The
unaltered LLVM line summary was retained separately: 2,088 out of 161,978
(1.289064%). These are narrow process-harness profiles, not an application-wide
coverage percentage.

## Experiments that shaped the implementation

Two deliberately retained failure observations prevented a misleading
producer:

1. Direct `show-env`-instrumented Cargo tests produced fresh profiles and all
   50 tests passed, but `cargo llvm-cov report` rejected the set because no
   cargo-llvm-cov-owned profile root existed. A pile of `.profraw` files was
   therefore proven insufficient evidence of mergeability.
2. A cumulative `cargo llvm-cov --no-clean --no-report` attempt was rejected
   by cargo-llvm-cov 0.8.4 because those flags are mutually exclusive.

The final producer follows the already-proven ownership pattern: one owned
`cargo llvm-cov --no-report` lane, followed by externally selected Cargo lanes
using `show-env`, followed by an explicit merge. This passed end to end in
94.2 seconds while the isolated instrumented artifacts were being populated.
The final source-bound replay against warm artifacts completed in 10.8 seconds.

## Verification

The following checks passed:

```text
python -m unittest scripts.tests.test_coverage_windows_process_lanes -v
13 tests passed

python -m unittest \
  scripts.tests.test_coverage_windows_process_lanes \
  scripts.tests.test_windows_process_coverage_workflow -v
17 tests passed

python -m unittest scripts.tests.test_ci_policy -v
13 tests passed

python -m unittest \
  scripts.tests.test_coverage_profile_lanes \
  scripts.tests.test_coverage_ci_guard \
  scripts.tests.test_coverage_windows_process_lanes \
  scripts.tests.test_windows_process_coverage_workflow -v
63 tests passed

C:\Users\shaun\go\bin\actionlint.exe \
  .github/workflows/rust-coverage.yml
passed

python scripts/coverage_windows_process_lanes.py validate \
  --report target/verification/coverage-windows-process-lanes-local-20260730.json
passed
```

The real experiment also successfully exported cargo-llvm-cov JSON and native
text, then passed both through `scripts/llvm_cov_line_map.py`.
The scheduled upload step uses `if: always()`, so a producer failure still
retains the wrapper-controlled failed report and any logs already written;
missing required artifacts remain an upload error rather than a silent skip.
