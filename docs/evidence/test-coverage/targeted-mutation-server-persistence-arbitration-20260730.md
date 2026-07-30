# Targeted mutation proof: server room-persistence arbitration

Date: 2026-07-30  
Branch baseline: `codex/test-coverage-design` at `0748e4a8f07bad4ab30b26b22535ec969c3b10cf`  
Tool: `cargo-mutants 27.1.0` through the fail-closed `scripts/mutation_ci.py` wrapper

## Outcome

The room-persistence desired-state decision boundary now has a bounded,
source-bound mutation ratchet:

- unchanged-source baseline: **3/25 viable mutants caught (12.00%)**,
  22 missed, 2 compiler-unviable, 0 timeouts;
- final: **25/25 viable mutants caught (100.00%)**, 0 missed, 2 independently
  classified compiler-unviable, 0 timeouts;
- the production source SHA-256 was identical before the baseline, after the
  baseline, before the final run, and after the final run:
  `a7348d3346906a494cfb7dfe40b1c377c66248189b705f4a4d327db0c20ba406`;
- no product defect was surfaced.

The implementation was too broad to use
`crates/sorotte-server/src/persistence_actor.rs` as a useful bounded shard.
The existing decisions were therefore extracted, without changing their state
machine, into:

`crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs`

The ordinary deterministic tests live in a separate source file:

`crates/sorotte-server/src/persistence_actor/persistence_arbitration_tests.rs`

Keeping tests outside the mutation target makes the unchanged-source
baseline/final comparison meaningful.

## Decision contract encoded

The extracted object still owns the same per-room tuple:

`(highest_seen_version, desired_effect, unresolved_failure_version)`

and the same `BTreeMap` ordering under the same actor-owned
`Arc<Mutex<...>>`. The tests mechanically specify these decisions:

| Situation | Required disposition |
| --- | --- |
| stats effect reaches room arbitration | classify as not a room effect; caller emits the existing failure |
| first room effect has version `0` | ignore as stale because the initial fence is `0` |
| version is lower than or equal to the highest seen | ignore; retain the desired effect and any unresolved failure |
| version is strictly greater | accept, replace the coalesced desire, advance the fence, clear the superseded failure |
| rooms are updated independently | retain one newest desired effect per room in deterministic room-name order |
| pre-transaction identity is no longer current | do not enter the transaction for the stale snapshot |
| version changes after the write but before commit | report stale and roll the old transaction back |
| current save succeeds | clear desired work and unresolved failure but retain the version fence |
| current delete succeeds | remove the room arbitration entry |
| stale success/failure completes | do not mutate the newer generation |
| current write fails | retain the desired work and record the unresolved failure so a later wake retries it |
| flush/recovery is considered | require every room to have neither desired work nor unresolved failure |
| recovery reporting is considered | additionally require at least one successful apply in that scan |

## Extraction equivalence

The extraction maps each former inline operation directly:

- the enqueue comparison remains `version <= highest_seen_version`;
- accepted work assigns the same three fields in the same order;
- the scan still clones one desired effect from each `BTreeMap` entry;
- pre-transaction currency still requires both the highest version and retained
  desired version to equal the candidate version;
- post-write currency still compares only the room's highest version;
- successful save/delete cleanup and current-generation failure retention are
  unchanged;
- the settled and recovery predicates are the same conjunctions;
- SQLite transaction creation, writes, rollback/commit, worker wake/flush
  handling, event reporting, and degraded/recovered counters remain in
  `persistence_actor.rs`.

This is also checked through the pre-existing integration-level worker suite:
all 9 `persistence_actor::tests::room_worker_` tests passed after extraction,
including queue coalescing, pre/post-arbitration replacement races,
same-connection failure recovery, SQLite full/read-only behavior, and stale
rollback.

## Experiment

Both experiments used this exact bounded scope:

```text
cargo mutants
  --package sorotte-server
  --file crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs
  --no-config --colors never --no-times --no-shuffle
  --all-features --cargo-arg=--locked
  --cargo-test-arg=--lib
  --cargo-test-arg=persistence_actor::persistence_arbitration_tests::
  --jobs 2 --timeout 60 --build-timeout 120
```

The wrapper separately listed the selected tests, listed the mutation
inventory, ran the unmutated baseline, reconciled all raw outcome files, and
bound the report to source hashes before and after execution.

### Unchanged-source baseline

- selected tests: 1;
- test-inventory digest:
  `9ea4504ad8a28cd8e6524fe2de2389f13a466183da0045b9f833ea843e1488b3`;
- mutation inventory: 27 total, 25 viable;
- mutation-inventory digest:
  `32513391f70c4f66c5386634d58a5d8d076001a49fca528feb1354f28cebcde6`;
- outcomes: 3 caught, 22 missed, 2 unviable, 0 timeout;
- viable kill percentage: 12.00%;
- producer interval: `2026-07-30T07:17:57.1256627Z` through
  `2026-07-30T07:20:35.6730399Z` (158.547 seconds);
- expected wrapper status: failed because survivors and unregistered unviable
  outcomes are fail-closed policy violations.

The 22 real survivors covered every missing part of the contract: effect
identity, strict version comparison, desired snapshot, currency conjunctions,
success/failure transitions, settled state, and recovery proof.

### Final

- selected tests: 7;
- test-inventory digest:
  `565ea3852fdbd8d06960a99e66ea91b0080c823cd1e07bfb29ca16bf31ce56e5`;
- mutation inventory: the same 27 mutants with the same inventory digest;
- outcomes: 25 caught, 0 missed, 2 accepted unviable, 0 timeout;
- viable kill percentage: 100.00%;
- producer interval: `2026-07-30T07:22:53.0297902Z` through
  `2026-07-30T07:25:39.1292778Z` (166.099 seconds);
- wrapper status: passed;
- raw `outcomes.json` SHA-256:
  `20430935f6f5e0658a23084e729c84490a6b5def83c62ea7d5f8c36503e6b520`;
- raw `mutants.json` SHA-256:
  `f8ce39875976f2a695da93ff69c76cdf97fb406aa50cab44465d641094fc067c`.

The seven tests are ordinary `#[test]` cases. They use no wall clock, sleep,
filesystem, random input, or scheduler.

## Independently proven compiler-unviable mutants

These are not equivalent survivors. Cargo-mutants generated programs that
cannot type-check, and the retained compiler logs show the exact Rust error in
both library build contexts:

1. `RoomPersistenceArbitration::enqueue -> RoomEffectEnqueueDisposition` with
   `Default::default()`
   - compiler result: `E0277`;
   - reason: the semantic three-way disposition enum intentionally has no
     `Default` implementation. Choosing accepted, stale, or not-room as a
     default would silently encode an unsafe routing decision.

2. `RoomPersistenceArbitration::desired_effects -> Vec<ServerPersistenceEffect>`
   with `vec![Default::default()]`
   - compiler result: `E0277`;
   - reason: `ServerPersistenceEffect` intentionally has no `Default`;
     constructing a save/delete/stats effect requires a real identity and
     payload.

Both identities are exact, source-bound, expire on 2026-10-31, and must become
stale-policy failures if the producer stops generating them.

## Proposed policy integration

Exact shard:

```toml
[[shard]]
id = "server-persistence-arbitration"
owner = "server-persistence"
package = "sorotte-server"
files = ["crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs"]
test_target = "lib"
test_filter = "persistence_actor::persistence_arbitration_tests::"
jobs = 2
timeout_seconds = 60
build_timeout_seconds = 120
minimum_viable_kill_percent = "100.00"
max_missed = 0
max_timeouts = 0
require_baseline = true
```

Exact expiring compiler-unviable identities:

```toml
[[accepted_unviable]]
id = "server-persistence-arbitration-enqueue-default"
shard = "server-persistence-arbitration"
file = "crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs"
function = "RoomPersistenceArbitration::enqueue"
return_type = "-> RoomEffectEnqueueDisposition"
genre = "FnValue"
replacement = "Default::default()"
reason = "cargo-mutants requests Default for a semantic three-way disposition enum that intentionally has no safe default"
review_by = "2026-10-31"

[[accepted_unviable]]
id = "server-persistence-arbitration-effect-default"
shard = "server-persistence-arbitration"
file = "crates/sorotte-server/src/persistence_actor/persistence_arbitration.rs"
function = "RoomPersistenceArbitration::desired_effects"
return_type = "-> Vec<ServerPersistenceEffect>"
genre = "FnValue"
replacement = "vec![Default::default()]"
reason = "cargo-mutants requests Default for a persistence effect whose required room or snapshot identity has no valid default"
review_by = "2026-10-31"
```

## Integrated checked-in policy replay

After adding the proposed shard and both exact exceptions to the shared
eight-shard policy, the checked-in wrapper passed:

```text
python scripts/mutation_ci.py run --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard server-persistence-arbitration \
  --results-root target/mutation-ci/server-persistence-arbitration-20260730 \
  --output target/verification/mutation-server-persistence-arbitration-20260730.json
```

The final attestation passed its unmutated baseline and reconciled the same 27
mutants as 25 caught plus the two exact accepted compiler-unviables, with zero
misses or timeouts. Its source-before and source-after SHA-256 values both
equal `a7348d3346906a494cfb7dfe40b1c377c66248189b705f4a4d327db0c20ba406`.
The 17,558-byte report has SHA-256
`840a478b8146fdff35e7a763479b22137644f29c8578c676876978003a825ff8`.
The complete policy validates eight shards and 16 accepted-unviable
identities.

## Validation

- focused selector: 7/7 passed;
- focused selector stress: 50 serial repetitions, 350/350 test executions,
  13.409 seconds;
- pre-existing room-worker suite: 9/9 passed in 0.42 seconds;
- complete all-feature server package:
  - 365/365 library tests;
  - 14/14 binary unit tests;
  - 2/2 server-binary integration tests;
  - 6/6 release-verification tests;
  - 0 doc tests;
  - 118.182 seconds;
- `cargo clippy --locked --all-features -p sorotte-server --all-targets -- -D warnings`:
  passed in 5.025 seconds.

## Limits

This shard proves the in-memory arbitration state machine for the exact source
hash above. It does not independently prove SQLite atomicity, operating-system
ACL behavior, disk durability after kernel or power loss, worker channel
scheduling, or broadcast receiver delivery. Those are deliberately exercised
by the existing worker fault/race/crash suites rather than duplicated in this
pure decision selector.
