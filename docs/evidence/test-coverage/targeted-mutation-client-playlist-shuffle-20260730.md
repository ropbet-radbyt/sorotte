# Targeted mutation proof: client playlist shuffle and undo decisions

Date: 2026-07-30 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit: `9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614`

Toolchain:

```text
cargo 1.97.1 (c980f4866 2026-06-30)
rustc 1.97.1 (8bab26f4f 2026-07-14)
cargo-mutants 27.1.0
```

Target package: `sorotte-client-core`

Target source:
`crates/sorotte-client-core/src/session/playlist/shuffle_helpers.rs`

Owning test selector:
`session::tests::playlist_tests::shuffle_undo_tests::`

## Result

`client-playlist-shuffle` is the tenth scheduled source-bound mutation shard.
Its final fail-closed campaign selected ten ordinary tests, inventoried 28
mutants, caught every one of the 26 viable mutants, and reconciled the two
compiler-infeasible let-chain mutants. There were zero misses and zero
timeouts, for a 100.00% viable kill rate.

No production source changed. No product defect or source-equivalent mutant
was found. The red results were test-oracle and test-liveness gaps only.

The added deterministic oracles cover:

- undo snapshot no-op, insert, deduplication, replacement, and room isolation;
- absent index, forward match, backward match, final-row, and deliberately
  excluded index-zero target selection;
- exact scope, index, nonce, file-order, and NUL-delimited filename framing in
  the SHA-256 shuffle seed;
- exact wrapping LCG state and returned values;
- empty and singleton shuffle behavior;
- four exact in-place Fisher-Yates permutations; and
- a 512-seed, 17-member permutation stress invariant.

The pre-existing end-to-end regression that toggles between the fully
disjoint `["episode1", "episode2", "episode3"]` and
`["episode4", "episode5"]` playlists remains intact.

## Scheduled fail-closed contract

The checked-in policy is:

```toml
[[shard]]
id = "client-playlist-shuffle"
owner = "client-playlist"
package = "sorotte-client-core"
files = ["crates/sorotte-client-core/src/session/playlist/shuffle_helpers.rs"]
test_target = "lib"
test_filter = "session::tests::playlist_tests::shuffle_undo_tests::"
jobs = 2
timeout_seconds = 60
build_timeout_seconds = 120
minimum_viable_kill_percent = "100.00"
max_missed = 0
max_timeouts = 0
require_baseline = true
```

`.github/workflows/rust-mutation.yml` includes
`client-playlist-shuffle` in its weekly and manually dispatchable matrix. The
existing wrapper pins the producer, inventories the exact test namespace and
mutants, binds the production source before and after the run, reconciles
every structured outcome and artifact, rejects source drift, and fails on a
miss, timeout, stale exception, or unexpected unviable mutant.

## Inventory

The inventory stayed at 28 mutations:

| Function | Mutations |
|---|---:|
| `capture_playlist_undo_snapshot_legacy_compatible` | 3 |
| `local_playlist_target_index_from_changed_playlist_legacy_compatible` | 14 |
| `next_playlist_shuffle_seed_legacy_compatible` | 3 |
| `next_shuffle_state_legacy_compatible` | 2 |
| `shuffle_playlist_slice_in_place_legacy_compatible` | 6 |
| **Total** | **28** |

The inventory contains whole-function replacements, comparison and boolean
reversals, PRNG return substitutions, and Fisher-Yates arithmetic changes.

## Exploratory red campaign

The first complete campaign ran before any test edit:

```text
cargo mutants
  --package sorotte-client-core
  --file crates/sorotte-client-core/src/session/playlist/shuffle_helpers.rs
  --no-config --colors never --no-times --no-shuffle
  --all-features --cargo-arg=--locked
  --cargo-test-arg=--lib
  --cargo-test-arg=session::tests::playlist_tests::shuffle_undo_tests::
  --jobs 2 --timeout 60 --build-timeout 120
  --output target/mutation-exploratory-client-playlist-shuffle-full
```

Artifact:
`target/mutation-exploratory-client-playlist-shuffle-full/mutants.out`

| Field | Value |
|---|---:|
| Selected tests | 4 |
| Total mutants | 28 |
| Viable mutants | 26 |
| Caught | 12 |
| Missed | 12 |
| Timed out | 2 |
| Unviable | 2 |
| Viable kill rate | 46.15% |
| Started UTC | `2026-07-30T10:42:54.9667848Z` |
| Finished UTC | `2026-07-30T10:46:07.2616078Z` |
| Elapsed | 192.295 seconds |

Every red viable outcome is retained. The misses were:

```text
shuffle_helpers.rs:33:9: replace local_playlist_target_index... -> usize with 0
shuffle_helpers.rs:36:28: replace <= with > in local_playlist_target_index...
shuffle_helpers.rs:41:21: replace <= with > in local_playlist_target_index...
shuffle_helpers.rs:43:84: replace == with != in local_playlist_target_index...
shuffle_helpers.rs:51:21: replace > with < in local_playlist_target_index...
shuffle_helpers.rs:55:39: replace < with == in local_playlist_target_index...
shuffle_helpers.rs:55:39: replace < with > in local_playlist_target_index...
shuffle_helpers.rs:55:39: replace < with <= in local_playlist_target_index...
shuffle_helpers.rs:72:9: replace next_playlist_shuffle_seed... -> u64 with 1
shuffle_helpers.rs:90:17: replace == with != in next_playlist_shuffle_seed...
shuffle_helpers.rs:98:9: replace next_shuffle_state... -> u64 with 0
shuffle_helpers.rs:115:63: replace + with * in shuffle_playlist_slice_in_place...
```

The timeouts were:

```text
shuffle_helpers.rs:51:21: replace > with == in local_playlist_target_index...
shuffle_helpers.rs:51:21: replace > with >= in local_playlist_target_index...
```

Both replacements can make the saturating backward scan remain at index zero
forever. The existing fully disjoint undo-toggle regression reached that
state, so the outer 60-second cargo-mutants deadline was initially the only
liveness detector.

The compiler-infeasible outcomes were:

```text
shuffle_helpers.rs:43:17: replace && with || in local_playlist_target_index...
shuffle_helpers.rs:53:17: replace && with || in local_playlist_target_index...
```

Both mutations replace a Rust let-chain `&&` with `||`; Rust permits the `let`
expressions only in an `&&` chain, so neither generated source parses.

The retained baseline `outcomes.json` is 65,060 bytes with SHA-256:

```text
042a5c465d987c0019c15952c99a5b8d4d15ab4cc67d9fff34be94468860f892
```

The original test and production-source Git blobs were respectively:

```text
0d9c2fd00f17f7a3d6b2d16fcb9e7223a24289a9
a6c7529b1f3e730b74bd0e6cb6905f06874c76b8
```

## Deterministic oracle and liveness strengthening

The target-index table supplies the semantic assertions for both former
timeout mutations:

- `> -> ==` skips a required backward match and returns zero instead of two;
- `> -> >=` improperly examines index zero and returns two instead of zero.

The original fully disjoint runtime test must also execute under every
mutant. A narrow test-only completion guard therefore runs only that test
body on a worker and waits up to five seconds:

- healthy execution sends its result and the worker is joined;
- an ordinary assertion panic is relayed to the test thread;
- a non-progress mutation misses the inner deadline and fails the libtest;
  only on that failure path is the stuck `JoinHandle` dropped and therefore
  detached; and
- the cargo-mutants test subprocess then exits and the operating system tears
  down the detached worker.

No production thread, timeout, or implementation changed. A focused
two-mutant replay of the former `> -> ==` and `> -> >=` cases caught 2/2
without an outer cargo-mutants timeout.

The SHA-256 seed expectations were calculated independently from the
documented byte framing and hard-coded as golden values. The PRNG and
Fisher-Yates expectations are likewise explicit constants rather than calls
to a second copy of the production helper.

## Exact accepted-unviable binding

The policy contains one expiring identity:

```toml
[[accepted_unviable]]
id = "client-playlist-shuffle-let-chain-or"
shard = "client-playlist-shuffle"
file = "crates/sorotte-client-core/src/session/playlist/shuffle_helpers.rs"
function = "ClientSession::local_playlist_target_index_from_changed_playlist_legacy_compatible"
return_type = "-> usize"
genre = "BinaryOperator"
replacement = "||"
review_by = "2026-10-31"
```

This is exact under the current validator identity tuple:
`(file, function, return_type, genre, replacement)`. Both generated sites
share that tuple. The final report retains two separate accepted matches with
their complete producer names, including line 43 column 17 and line 53
column 17. `unexpected_unviable` and `stale_accepted_unviable` are both empty.
The policy does not accept another function, return type, mutation genre, or
replacement.

The current tuple does not include source span. That is why one expiring
entry represents both same-function sites; the report's exact
`actual_mutant` names preserve the site-level audit trail.

## Canonical final campaign

The final run used fresh paths after the disjoint regression and completion
guard were stable:

```text
python scripts/mutation_ci.py run
  --repo-root .
  --policy coverage/mutation-policy.toml
  --shard client-playlist-shuffle
  --results-root target/mutation-ci/client-playlist-shuffle-final-20260730
  --output target/verification/mutation-client-playlist-shuffle-final-20260730.json
```

| Attestation field | Value |
|---|---|
| Status | `passed` |
| Producer exit | `0` |
| Checkout HEAD | `9a31b5acfe7e4e0150bdbbe3c31ed7e4155d8614` |
| Configured source dirty | `false` |
| Source bytes | 3,796 |
| Source SHA-256 before/after | `4979708e0ae1b1c79a957333819da87100ec7954e1a381234a078ee6e754a0d5` |
| Selected tests | 10 |
| Test inventory canonical SHA-256 | `97af9847897b6785d684df009d7ffcef07b1ecd817e726cb9efd9222fa67f6cf` |
| Mutation inventory | 28 |
| Mutation inventory canonical SHA-256 | `ca9b8756bd9ccb83420f38a3b6fc8d8c2cefab8e936cd5b98523e57d45ce5d06` |
| Viable mutants | 26 |
| Caught | 26 |
| Missed | 0 |
| Timed out | 0 |
| Accepted unviable | 2 |
| Viable kill rate | `100.00%` |
| Started UTC | `2026-07-30T10:59:57.2724736Z` |
| Finished UTC | `2026-07-30T11:02:03.7103412Z` |
| Elapsed | 126.438 seconds |

The pre-run and producer inventories have the same canonical hash. The
production source hash is identical before and after execution.

## Canonical artifact hashes

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Final attestation report | 18,504 | `3a5f73ce1fa8af16061576721bdabb740dea05173f27d71ad353ff0088d63204` |
| `test-inventory.json` | 1,320 | `4e5a741ce18dcebedef0cd9f37ccebbc029c297cd2437c1b5b7c2a1afb668077` |
| Pre-run `inventory.list.json` | 51,447 | `556b6252f27814bfed32c3e7ef96c1287a85c0ac2d4d0dd3af5b789e14d07e01` |
| Producer `mutants.json` | 51,446 | `de55789eae7d33c80425186cd724492ea81340ec56393c5466a2579cd8a0603b` |
| Producer `outcomes.json` | 65,475 | `1eaa433cc55d388e68c94e3da251abe9405e261d3515213f9c508675b4a5fa55` |

The stable test and enforcement inputs are:

| File | Bytes | SHA-256 |
|---|---:|---|
| `shuffle_undo_tests.rs` | 24,837 | `ef755ec7a6acf2c0d003b7cf3975ec0cf13d45a78a118143c84f1fc0cac717d1` |
| `coverage/mutation-policy.toml` | 11,823 | `009747aaecb820b07399fd81d5ad9a1af8e9ce7dab197459b1931960ca329c38` |
| `.github/workflows/rust-mutation.yml` | 2,416 | `90ebaee035b2f3bd8dabedaad74637f0bcfdbc96562251bc1db93da9ae138a02` |
| `scripts/tests/test_ci_policy.py` | 84,985 | `630d3cb43d67a9cfe4a71a5a3ab71e4d759ebe88da5c1d16b87d6e40394947b8` |

## Validation

All commands ran from
`C:\tmp\sorotte-test-coverage-design` on
`codex/test-coverage-design`.

| Check | Result |
|---|---|
| focused shuffle/undo namespace | 10/10 passed |
| former timeout-mutant replay | 2/2 caught without outer timeout |
| final checked-in policy wrapper | 26/26 viable caught; 2/2 accepted unviable; zero misses/timeouts |
| mutation policy validator | 10 shards and 17 accepted-unviable identities valid |
| CI-policy and mutation-wrapper unit suites | 50/50 passed |
| `cargo test --locked -p sorotte-client-core --all-features` | 724/724 library tests passed; doc tests passed |
| `cargo clippy --locked -p sorotte-client-core --all-targets --all-features -- -D warnings` | passed |
| installed `actionlint` on `rust-mutation.yml` | passed |
| targeted `rustfmt --check` | passed |
| targeted `git diff --check` | passed |

## Defect accounting and limitations

No independent product defect was found. The implementation source remained
unchanged, and no known-defect registry entry is needed.

The exploratory red campaign exposed bounded oracle/liveness gaps:

- twelve observable mutations survived the four prior runtime tests; and
- two non-progress mutations were detected only by the outer timeout.

The stable tests close both classes without weakening the original disjoint
end-to-end regression. The final campaign proves zero viable survivors and
zero outer timeouts for the current 28-mutant inventory.

This proof is deliberately bounded. It does not establish:

- mutation coverage for playlist code outside `shuffle_helpers.rs`;
- statistical randomness or cryptographic unpredictability of shuffle order;
- compatibility with every future legacy client implementation;
- raw protocol framing, server persistence, GUI, or real-player behavior; or
- coverage beyond the exact cargo-mutants 27.1.0 inventory.

The scheduled lane is a weekly/manual ratchet. The wrapper binds production
source and artifact hashes; the checked-in test-file hash above supplements
its canonical test-name inventory.
