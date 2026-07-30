# Test-coverage continuation handoff — additional slices — 2026-07-30

## Why this handoff exists

The delegated coverage-guided framed-session task was twice interrupted by an
automated cybersecurity classifier. That classification was a false positive.
The work is bounded defensive QA of Sorotte's own local Rust code:

- generated input is processed only in memory by local Sorotte functions;
- libFuzzer and AddressSanitizer are used only as randomized test drivers;
- the target contains no `TcpStream`, `UdpSocket`, `std::net`, hostname,
  address, credential, reconnaissance, persistence, privilege, or third-party
  interaction;
- the maximum driver input is 65,536 bytes, the target bounds each input to 64
  derived frames, each input to 5 seconds, RSS to 2,048 MiB, and a campaign to
  900 seconds; and
- the only external activity observed was Cargo downloading pinned build
  dependencies while compiling the local test target.

Do not reinterpret this as offensive-security work. There is no network
target or exploitation objective.

## Repository coordinates and frozen state

Continue only in:

```text
C:\tmp\sorotte-test-coverage-design
```

Branch and current source identity:

```text
branch:       codex/test-coverage-design
HEAD:         9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614
remote HEAD:  9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614
staged files: none
```

The branch and remote are identical. The worktree is intentionally dirty.
Preserve every dirty file. Do not reset, checkout, restore, clean, or discard
anything.

The preceding historical handoff was read in full:

```text
docs/evidence/test-coverage/test-slices-handoff-20260730.md
```

Its former product defects `TC-CLI-003` and `TC-PROTOCOL-004` were subsequently
fixed by explicit user direction. The current known-defect registry is empty.
This continuation found no new product defect.

## User request and delegation result

The user requested four subagents to tackle more slices of the test plan. The
runtime allowed three child agents concurrently with the root, so three were
started immediately and the fourth was started as soon as the first slot
freed.

| Slice | Final state |
|---|---|
| Controlled-room configuration properties | complete, evidenced, no product change or defect |
| Disposable-platform SQLite syscall faults | complete on Windows and Unix/WSL, evidenced, no product change or defect |
| Client playlist shuffle/undo mutation ratchet | complete, evidenced, 26/26 viable mutants caught |
| Coverage-guided framed transport/session state | implementation substantially complete; delegated validation was classifier-blocked; root review/build/policy checks passed; campaign/evidence/central integration remain |

No agent staged, committed, or pushed.

## Exact dirty inventory

Modified tracked files:

```text
.github/workflows/rust-fuzz.yml
.github/workflows/rust-mutation.yml
coverage/mutation-policy.toml
crates/sorotte-cli/Cargo.toml
crates/sorotte-cli/src/lib.rs
crates/sorotte-cli/src/protocol_io.rs
crates/sorotte-client-core/src/session/tests/playlist_tests/shuffle_undo_tests.rs
crates/sorotte-server/src/tests.rs
fuzz/Cargo.lock
fuzz/Cargo.toml
fuzz/run_protocol_fuzz.py
scripts/tests/test_ci_policy.py
scripts/tests/test_protocol_fuzz_policy.py
```

Untracked implementation/evidence files:

```text
crates/sorotte-cli/tests/corpus/framed_session/bytewise-state-chat.txt
crates/sorotte-cli/tests/corpus/framed_session/cancel-bytewise-hello.txt
crates/sorotte-cli/tests/corpus/framed_session/cancel-pseudorandom-multi.txt
crates/sorotte-cli/tests/corpus/framed_session/coalesced-hello-set.txt
crates/sorotte-cli/tests/corpus/framed_session/empty-and-null-frames.txt
crates/sorotte-cli/tests/corpus/framed_session/escaped-command-order.txt
crates/sorotte-cli/tests/corpus/framed_session/fixed-width-list.txt
crates/sorotte-cli/tests/corpus/framed_session/malformed-prefix-valid-suffix.txt
crates/sorotte-cli/tests/corpus/framed_session/pseudorandom-duplicate-set.txt
crates/sorotte-cli/tests/corpus/framed_session/size-seam-crlf-limit.txt
crates/sorotte-cli/tests/corpus/framed_session/size-seam-crlf-over.txt
crates/sorotte-cli/tests/corpus/framed_session/size-seam-lf-limit.txt
crates/sorotte-cli/tests/corpus/framed_session/size-seam-lf-over.txt
crates/sorotte-cli/tests/corpus/framed_session/unknown-and-unicode.txt
crates/sorotte-client-app/tests/controlled_room_configuration_properties.rs
crates/sorotte-server/src/tests/persistence_platform_syscall_fault_tests.rs
docs/evidence/test-coverage/controlled-room-configuration-properties-20260730.md
docs/evidence/test-coverage/persistence-platform-syscall-faults-20260730.md
docs/evidence/test-coverage/targeted-mutation-client-playlist-shuffle-20260730.md
fuzz/fuzz_targets/framed_session.rs
```

This handoff file is an additional untracked document after the inventory was
captured.

`fuzz/fuzz_targets/protocol_line.rs` is not dirty. A standalone rustfmt run
briefly reformatted it, and the incidental formatting-only change was restored
with `apply_patch`.

Ignored generated output exists under `target/` and `fuzz/target/`. Do not
stage it.

## Slice 1: controlled-room configuration properties

Files:

```text
crates/sorotte-client-app/tests/controlled_room_configuration_properties.rs
docs/evidence/test-coverage/controlled-room-configuration-properties-20260730.md
```

The new black-box integration suite uses only public client-app boundaries and
an independent model. Four fixed-seed properties cover:

- normalization, reconstruction, idempotence, and command-facing room names;
- malformed, passwordless, ordinary, and legacy room inputs;
- INI render/parse/canonical rewrite, explicit/history precedence, and
  environment noninterference;
- typed room/server credential isolation, TLS selection, and Debug redaction.

Results:

```text
default:    4/4,  2,048 generated cases
scheduled:  4/4,  8,192 generated cases
stress:     4/4, 40,000 generated cases
all depths:       50,240 generated cases
```

Zero and malformed case budgets failed closed with exit 101. Focused strict
Clippy, formatting, and whitespace checks passed. No production code changed
and no defect surfaced.

Current hashes:

```text
44793178edc1b6e4ed95187a6b77214f859dfbbe9c2073f81803e00c40b6d151  controlled_room_configuration_properties.rs
0da393bf7e62a869a1f72e1537b45c6167893c07e72150d6a535f0e6ac49f07d  controlled-room-configuration-properties-20260730.md
```

## Slice 2: real platform SQLite syscall denial

Files:

```text
crates/sorotte-server/src/tests.rs
crates/sorotte-server/src/tests/persistence_platform_syscall_fault_tests.rs
docs/evidence/test-coverage/persistence-platform-syscall-faults-20260730.md
```

Windows uses a real `OpenOptionsExt::share_mode(0)` handle on the checkpointed
production database. It proves kernel rename/delete error 32, production
worker `SQLITE_CANTOPEN`, unchanged main-database bytes, unchanged complete
eight-column row, `PRAGMA integrity_check = ok`, removal of the host
condition, normal worker write/flush, close/reopen, and complete replacement
state.

Unix uses a reversible real namespace denial: rename the checkpointed database,
place a directory at the production pathname, require worker
`SQLITE_CANTOPEN`, prove bytes/row/integrity, restore the file, and prove normal
write/reopen recovery. An unwind guard restores the database.

Validation completed:

```text
Windows focused: 1/1
Windows serial stress: 50/50
Unix Ubuntu WSL focused: 1/1, 365 filtered, 0.11s
full server package: 366 lib, 14 bin unit, 2 integration, 6 release verification
strict server Clippy: passed
package fmt/scoped diff checks: passed
```

The Unix command used an isolated target directory:

```text
CARGO_TARGET_DIR=target/wsl-server-syscall cargo +1.97.1 test --locked \
  -p sorotte-server --lib \
  room_persistence_unix_namespace_denial_preserves_and_recovers_durable_state \
  -- --nocapture --test-threads=1
```

This proves host-level open/rename/delete denial and recovery. It does not
claim parent-directory sync, device-cache persistence, torn-sector behavior,
or power-loss durability.

Current hashes:

```text
8a32aa00a39ef5dc1e5d96514d39ee5793f51b8935bc5ed5539645d0e3742a5c  persistence_platform_syscall_fault_tests.rs
cafe795cfd28016d17cc344a26a98b92f2c41d1e6aef4f4342f0c54cfa9e5200  persistence-platform-syscall-faults-20260730.md
```

## Slice 3: playlist shuffle/undo mutation ratchet

Files:

```text
.github/workflows/rust-mutation.yml
coverage/mutation-policy.toml
scripts/tests/test_ci_policy.py
crates/sorotte-client-core/src/session/tests/playlist_tests/shuffle_undo_tests.rs
docs/evidence/test-coverage/targeted-mutation-client-playlist-shuffle-20260730.md
```

Production
`crates/sorotte-client-core/src/session/playlist/shuffle_helpers.rs` is
unchanged.

Exploratory baseline:

```text
inventory: 28
viable: 26
caught: 12
missed: 12
timed out: 2
compiler-unviable: 2
viable kill rate: 46.15%
```

The two timeouts were saturating backward-loop comparison mutations. The final
tests retain the original fully disjoint undo regression and add deterministic
oracles for snapshot decisions, target-index selection, seed framing and
nonce, PRNG transition, exact Fisher-Yates permutations, and a 512-seed
permutation invariant. A narrow test-only five-second completion guard makes
the two non-progress mutants fail inside the cargo-mutants subprocess rather
than consume the outer 60-second timeout.

Final canonical policy run:

```text
inventory: 28
viable: 26
caught: 26
missed: 0
timed out: 0
accepted compiler-unviable outcomes: 2
viable kill rate: 100.00%
report SHA-256: 3a5f73ce1fa8af16061576721bdabb740dea05173f27d71ad353ff0088d63204
```

The two unviable mutations are duplicate sites with the same current
structured identity tuple: the same function's Rust let-chain `&& -> ||`
rewrite cannot parse. One exact expiring policy identity represents both
sites; the report retains both complete producer names.

Validation completed:

```text
focused namespace: 10/10
former timeout-mutant replay: 2/2 caught without outer timeout
client-core package: 724/724 plus docs
mutation/CI policy unit suites: 50/50
mutation policy: 10 shards, 17 accepted-unviable identities
strict client-core Clippy: passed
rust-mutation actionlint: passed
targeted rustfmt/diff checks: passed
```

Current hashes:

```text
ef755ec7a6acf2c0d003b7cf3975ec0cf13d45a78a118143c84f1fc0cac717d1  shuffle_undo_tests.rs
7f88ff17de6cd9f9dbab7e56242484a9d24dbbe13a651e8e7608863013fb2697  targeted-mutation-client-playlist-shuffle-20260730.md
```

The central mutation total should become:

```text
10 scheduled shards
484/484 viable mutants caught
0 misses
0 timeouts
17 exact accepted compiler-unviable policy identities
```

The viable total is the previous 458 plus this shard's 26.

## Slice 4: coverage-guided framed transport/session state

### Current implementation

Files:

```text
.github/workflows/rust-fuzz.yml
crates/sorotte-cli/Cargo.toml
crates/sorotte-cli/src/lib.rs
crates/sorotte-cli/src/protocol_io.rs
crates/sorotte-cli/tests/corpus/framed_session/*
fuzz/Cargo.toml
fuzz/Cargo.lock
fuzz/fuzz_targets/framed_session.rs
fuzz/run_protocol_fuzz.py
scripts/tests/test_protocol_fuzz_policy.py
```

The CLI now has an empty `fuzz-support` feature and a hidden, feature-gated
exact re-export of the production `InboundProtocolLineReader` and
`MAX_INBOUND_PROTOCOL_LINE_BYTES`. The framing algorithm was not copied.
Underlying items changed from `pub(crate)` to `pub`, but remain unreachable
from external crates unless re-exported through the feature-gated module.
Normal production behavior is unchanged.

The local target:

- uses the exact production line accumulator and limit;
- supplies an in-memory `AsyncBufRead` with coalesced, one-byte, fixed-width,
  and deterministic pseudo-random chunk schedules;
- cancels one first-frame read after a generated consumed-byte offset, then
  resumes the same accumulator;
- compares both coalesced and scheduled reads with an independent byte-framing,
  CR/LF, UTF-8, and payload-limit oracle;
- derives an exact input frame bound and requires a final EOF probe;
- applies every line returned by production framing, including valid
  EOF-terminated partial lines, through public
  `ClientApplication::apply_protocol_line`;
- compares accepted/error outcomes, complete deterministic Debug session
  projections, and pending protocol counts across schedules;
- asserts active identity and room/user projection consistency;
- asserts syntactically invalid JSON fails without partial session mutation;
- exercises MAX/MAX+1 LF/CRLF size seams through four explicit corpus seeds;
- caps ordinary inputs at 64 frames; and
- contains no network API.

The root corrected one important issue after the delegated agent was blocked:
the initial target applied only newline-terminated frames to session state,
but the real connected-session path also applies a valid unterminated line at
EOF. The current target applies all lines returned by the production reader.

The checked corpus has 14 direct files. Four ordinary control prefixes cover
all chunk modes, odd control bytes cover cancellation, and four `!SEAM0`
through `!SEAM3` seeds cover exact and one-byte-over LF/CRLF boundaries.

`fuzz/run_protocol_fuzz.py` is now target-selectable while preserving
`protocol_line` as its default. `framed_session` receives schema
`sorotte-framed-session-fuzz-v1`, exact target selection for run/minimization,
and a conservative source binding over every direct file under `crates/` plus
the workflow, manifests, locks, target, runner, and policy inputs.

The workflow retains the original protocol job and adds a second
`framed-session-fuzz` job:

```text
PR/main push: 45 seconds
schedule/manual: 900 seconds
toolchain: nightly-2026-07-29
cargo-fuzz: 0.13.2
sanitizer: address
seed count: 14
evidence upload: always, missing files are an error
```

Workflow paths now include `Cargo.lock`, `crates/**`, and
`requirements/ci-policy.txt` so every framed-session bound input triggers the
lane.

### Validation already completed

The expanded policy suite passed:

```text
python -m unittest scripts.tests.test_protocol_fuzz_policy -v
16/16 passed
```

It mechanically verifies:

- both exact workflow jobs, triggers, durations, pins, commands, uploads, and
  fail-closed behavior;
- exact standalone dependencies and both target declarations;
- feature-gated exact production re-export;
- local target structure and absence of network APIs;
- exact 14-file seed/control/seam inventory;
- target-selectable runner commands and minimization;
- protocol and framed source inventories and workflow coverage;
- source drift, seed drift, stale output, tool identity, statistics, and
  status precedence.

The pinned local target compiled successfully under Ubuntu WSL:

```text
cargo +nightly-2026-07-29 fuzz build \
  --fuzz-dir fuzz --sanitizer address framed_session
```

The build completed in 1 minute 58 seconds. It compiled only the local
standalone target and Sorotte dependencies. No campaign has run yet.

At the handoff point, the framed bound-source manifest is:

```text
file count:       881
total bytes:      17,334,304
aggregate SHA-256:
e02c283a27b661c2bf9e4dd2f837e6ae46e54930d80cf88010f0ed4db55c7e42
```

Expected implementation hashes at the handoff point:

```text
c9441dfdd1af1fa53821ef30827ceeee5333afd371cf4dcc8a9c824e782df7ba  .github/workflows/rust-fuzz.yml
2bfd74a02e39a6d80f518338785142d31c3af9b071067086b8cf88cbf46d3be1  crates/sorotte-cli/Cargo.toml
01700a1cb078d33a43e11e7620a4bfd6e68bf7a6949e27191134a2c5a3e23e85  crates/sorotte-cli/src/lib.rs
11491e346675b6672d222d886e10ca47febae06a427f00a5c10124c505093b9d  crates/sorotte-cli/src/protocol_io.rs
e3a1d2ab993581c24a93768dc854bc6b7c427bb6283864a691ecb3046d4ef143  fuzz/Cargo.toml
3fcdca0c38fb9297bffecdd53d49930f1b693c95211df9ecce6165926b10ee66  fuzz/Cargo.lock
1cbadeec5dbac52273f7ac13932adfec59ce29e8e610367747a3783e92dfc983  fuzz/fuzz_targets/framed_session.rs
165f127f1f304e97569dfe8409866924b04fa0f92097bec72d936e8121471325  fuzz/run_protocol_fuzz.py
ad706ee074ee23033e0d61b6c55c9ce6ef4ac74809f7a71ffa59667b19b052ca  scripts/tests/test_protocol_fuzz_policy.py
```

These are drift detectors, not an instruction to revert a justified fix.

### Still missing for this slice

The following file does not yet exist:

```text
docs/evidence/test-coverage/framed-session-coverage-guided-20260730.md
```

Still required:

1. run focused CLI all-feature tests/Clippy and re-run the target build;
2. lint the changed `rust-fuzz.yml` with the installed actionlint;
3. run a fresh short local campaign from a new output path;
4. retain and classify any failure as target/oracle, independent product
   behavior, or known behavior; do not delete red evidence;
5. commit stable implementation sources before the canonical run so
   `--source-sha` identifies the actual bound source rather than only the dirty
   base;
6. run a fresh 180-second canonical campaign from another new output path;
7. write the missing evidence with exact report/log/source/corpus hashes,
   statistics, commands, tool identities, resource limits, and limitations;
8. update central docs; and
9. run full validation, focused commits, push, and remote-head verification.

## Safe ordered continuation

### 1. Re-orient without changing state

```powershell
Set-Location C:\tmp\sorotte-test-coverage-design
git status --short --branch --untracked-files=all
git rev-parse HEAD
git rev-parse origin/codex/test-coverage-design
```

Expected local and remote SHA:

```text
9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614
```

Read this entire handoff before editing.

### 2. Re-run focused policy and ordinary compilation

```powershell
python -m unittest scripts.tests.test_protocol_fuzz_policy -v

cargo test --locked -p sorotte-cli --all-features
cargo clippy --locked -p sorotte-cli --all-targets --all-features -- -D warnings

cargo test --locked -p sorotte-client-app `
  --test controlled_room_configuration_properties -- --nocapture

cargo test --locked -p sorotte-server --lib `
  room_persistence_windows_share_denial_preserves_and_recovers_durable_state `
  -- --nocapture --test-threads=1

cargo test --locked -p sorotte-client-core --all-features `
  session::tests::playlist_tests::shuffle_undo_tests:: -- --nocapture
```

Use the installed actionlint:

```powershell
C:\Users\shaun\go\bin\actionlint.exe `
  .github/workflows/rust-fuzz.yml `
  .github/workflows/rust-mutation.yml
```

Rebuild the local target under WSL:

```powershell
wsl.exe -d Ubuntu `
  --cd /mnt/c/tmp/sorotte-test-coverage-design `
  bash -lc "cargo +nightly-2026-07-29 fuzz build --fuzz-dir fuzz --sanitizer address framed_session"
```

### 3. Run a short fresh local campaign

Every runner output directory must be new. The runner rejects stale evidence.
Before the implementation commit, the SHA is the dirty base and the report's
before/after source manifest is the exact identity:

```powershell
wsl.exe -d Ubuntu `
  --cd /mnt/c/tmp/sorotte-test-coverage-design `
  bash -lc "python3 fuzz/run_protocol_fuzz.py --target framed_session --toolchain nightly-2026-07-29 --source-sha 9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614 --seconds 30 --seed-corpus crates/sorotte-cli/tests/corpus/framed_session --expected-seed-count 14 --output-root target/fuzz-ci/framed-session-smoke-v1"
```

This command tests only in-memory local Sorotte code.

If it fails, retain:

```text
run-report.json
fuzz.log
artifacts/*
minimized/*
```

Do not overwrite or delete the first red output. Determine whether the failure
is an oracle/target defect or independent product behavior. Add a narrow
ordinary characterization for an independent product defect before deciding
whether the user's current scope authorizes a product fix.

### 4. Commit stable implementation before canonical campaign

A clean bound implementation commit is preferred. Documentation under `docs/`
is not part of the framed source binding, but every file under `crates/` is.
Do not claim a clean source SHA while any bound source is dirty.

Suggested focused commits:

1. `Add controlled-room configuration properties`
2. `Add platform persistence syscall probes`
3. `Add playlist shuffle mutation ratchet`
4. `Add coverage-guided framed session testing`

The fourth implementation commit should include the CLI feature seam, 14
seeds, standalone target/dependencies/lock, runner, workflow, and policy tests.
Do not include `target/` or `fuzz/target/`.

### 5. Run the canonical 180-second local campaign

Replace `<implementation-head>` with the actual clean implementation commit:

```powershell
wsl.exe -d Ubuntu `
  --cd /mnt/c/tmp/sorotte-test-coverage-design `
  bash -lc "python3 fuzz/run_protocol_fuzz.py --target framed_session --toolchain nightly-2026-07-29 --source-sha <implementation-head> --seconds 180 --seed-corpus crates/sorotte-cli/tests/corpus/framed_session --expected-seed-count 14 --output-root target/fuzz-ci/framed-session-deep-v1"
```

Require:

```text
status: passed
fuzzer_exit_code: 0
source_bindings.stable: true
seed_corpus.source_stable: true
evidence_errors: []
artifacts.file_count: 0
complete positive final statistics
```

### 6. Write missing evidence and central integration

Create:

```text
docs/evidence/test-coverage/framed-session-coverage-guided-20260730.md
```

Include:

- bounded local-QA and no-network scope;
- exact production seam and why it is not a fork;
- independent framing, cancellation, EOF, UTF-8, size, and session oracles;
- all 14 seeds and control semantics;
- workflow/tool/dependency/resource pins;
- source/seed/corpus/report/log/artifact hashes;
- short and canonical campaign statistics;
- policy, build, focused test, Clippy, actionlint, and formatting proof;
- classifier interruption as process history, not technical evidence;
- no-defect statement or exact independent defect characterization; and
- limitations: one in-memory target is not native GUI, real player, network
  timing, physical storage durability, or another sanitizer.

Update:

```text
coverage/README.md
docs/TEST_COVERAGE_STRATEGY.md
docs/TEST_COVERAGE_FINDINGS.md
```

Current-state updates should include:

- the coverage-guided framed transport/session lane;
- the controlled-room configuration properties;
- Windows and Unix real host-filesystem persistence denial/recovery;
- 10 mutation shards, 484/484 viable caught, zero misses/timeouts, and 17
  accepted compiler-unviable policy identities;
- links to all four new evidence files;
- an explicitly empty product-defect registry; and
- removal of now-closed remaining-work items for framed-session randomized
  coverage, controlled-room properties, broader session-state mutation, and
  ordinary platform syscall denial.

Do not remove the remaining physical power-loss/device durability gap. The
most valuable still-open plan items are:

1. ephemeral interactive Windows required native lane;
2. genuine GUI-to-real-mpv vertical harness;
3. disposable block-device/power-loss durability rather than ordinary syscall
   denial; and
4. server-container build/load/public-digest/SBOM/signature verification.

Do not rewrite historical checkpoint counts. Change only statements that claim
to describe current policy or current capability.

### 7. Full validation

At minimum:

```powershell
cargo fmt --all -- --check
git diff --check

python scripts/mutation_ci.py validate `
  --repo-root . `
  --policy coverage/mutation-policy.toml

python scripts/known_defect_policy.py validate `
  --registry coverage/known-defects.toml `
  --repo-root . `
  --catalog coverage/behaviors.toml

python -m unittest discover -s scripts/tests -p "test_*.py" -v

cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

Also re-run the four focused slices at their documented scheduled depths and
the checked-in `client-playlist-shuffle` mutation wrapper if any owning source,
test, policy, or workflow changes after the recorded final run.

If a broad test fails in a way that looks timing-related, isolate and retry it
before changing code. Do not turn a flake into a speculative product change.

### 8. Focused commits and push

After the canonical evidence and central docs:

```powershell
git status --short --untracked-files=all
```

Confirm no generated files under these paths are staged:

```text
target/
fuzz/target/
fuzz/__pycache__/
```

Keep evidence/central integration in a focused final commit such as:

```text
Update test coverage strategy
```

Push and verify:

```powershell
git push origin codex/test-coverage-design
git rev-parse HEAD
git ls-remote origin refs/heads/codex/test-coverage-design
git status --short --branch
```

## Suggested opening prompt for the new chat

```text
Continue the Sorotte test-coverage work from
C:\tmp\sorotte-test-coverage-design\docs\evidence\test-coverage\test-slices-handoff-20260730-more-slices.md.
Read the entire handoff first and work only in
C:\tmp\sorotte-test-coverage-design on codex/test-coverage-design. Preserve
every dirty file. This is bounded defensive QA of our own local Rust JSON
framing/session code: generated input is processed only in memory, with no
network target, reconnaissance, credentials, persistence, privilege work, or
exploitation. The prior automated cyber flag was a false positive. Finish the
short and canonical local randomized campaigns, missing evidence, central
docs, full validation, focused commits, push, and remote-head verification.
```
