# Targeted mutation proof: client inbound Set ordering

Date: 2026-07-30

Target package: `sorotte-client-core`

Target source:
`crates/sorotte-client-core/src/inbound_order.rs`

Owning test selector: `session::tests::protocol_tests::`

Producer: `cargo-mutants 27.1.0`

## Result

The client inbound `Set` normalizer now has a small, source-bound
zero-survivor mutation proof for command ordering. The unchanged-source
baseline selected 35 existing protocol tests and caught four of five viable
mutations. One equality mutation survived because no test directly observed
how incomplete command-order metadata is completed.

Three ordinary deterministic tests add an explicit oracle for:

- partial wire order followed by missing canonical commands;
- an unknown command retaining its exact wire position;
- canonical order when wire-order metadata is absent;
- no duplication of canonical commands that are already present;
- exact preservation of a complete noncanonical permutation; and
- an unchanged `SetPayload` snapshot accompanying every ordered command.

The final run selected 38 tests and caught all five viable mutations. There
were no misses, timeouts, or compiler-unviable mutations.

No product defect was found or fixed. The baseline survivor was a test-oracle
gap only.

## Why this boundary

`inbound.rs` contains 876 lines and owns unrelated Hello, Set, List, State,
Chat, error, feature, file, and extension normalization. Mutating that whole
file would create a broad, expensive shard whose failures would be difficult
to assign.

The ordering decision was already a self-contained function. It was moved
verbatim into a 26-line, 729-byte module:

```text
crates/sorotte-client-core/src/inbound_order.rs
```

The only production wiring changes are:

1. `lib.rs` declares `mod inbound_order`;
2. `inbound.rs` imports `ordered_set_commands`; and
3. the original function body is removed from `inbound.rs`.

The function signature and body are unchanged apart from the minimum
`pub(super)` visibility needed by its sibling consumer. Before adding any new
tests, all 35 pre-existing tests under
`session::tests::protocol_tests::` passed. This proves the extraction through
the existing end-to-end normalizer and session application path.

An automated ordinal text comparison extracted the original function from
`git show HEAD:crates/sorotte-client-core/src/inbound.rs`, normalized only the
new visibility token, and returned:

```text
moved_function_text_equal=True
```

## Contract and independent oracle

The production helper begins with `SetPayload.command_order`, then appends
each missing known command in canonical compatibility order. Each resulting
name is paired with a clone of the same complete payload so the main
normalizer can take exactly the matching field while retaining wire ordering.

The new tests do not call the production canonical-order table to calculate
their expected results. They use explicit wire-name arrays and compare the
complete output:

| Test | Independent decision proved |
|---|---|
| `set_command_order_completion_appends_only_missing_canonical_commands` | starts with `ready`, an unknown vendor command, and `room`; explicitly expects the remaining nine known names in compatibility order and checks every payload snapshot |
| `set_command_order_completion_uses_canonical_order_without_wire_metadata` | explicitly expects all eleven known wire names in canonical order |
| `set_command_order_completion_preserves_complete_wire_permutation_exactly` | supplies all known names plus an unknown command in a deliberately unusual permutation and requires byte-for-byte name ordering with no additions |

The existing end-to-end
`set_commands_apply_in_wire_order_after_normalization` test remains important:
it proves that the ordered helper output changes observable session state in
wire order rather than testing only a private representation.

## Behavior-preserving source binding

The target source SHA-256 was identical before the baseline and after the
tests were added:

```text
50EB172D41813A9FFA8958A10D138F591DB593341922D909A9A5010AB755B0DB
```

The mutation improvement therefore comes entirely from stronger ordinary
tests. It is not caused by changing production decisions, deleting mutation
sites, reducing the inventory, or changing the selected source.

The final owned protocol test file SHA-256 is:

```text
913FF898345FD29917F0180342F54CBE4E977B633DAF4C2C4D17B21D4F745679
```

## Experiment 0: unchanged-source baseline

Command:

```text
cargo mutants --package sorotte-client-core \
  --file crates/sorotte-client-core/src/inbound_order.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --cargo-test-arg=session::tests::protocol_tests:: \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutants-client-inbound-order-baseline-proof-20260730
```

Artifact:

```text
target/mutants-client-inbound-order-baseline-proof-20260730/mutants.out
```

Elapsed wall time recorded by cargo-mutants: 66.137 seconds.

| Outcome | Count |
|---|---:|
| Inventory | 5 |
| Caught | 4 |
| Missed | 1 |
| Timed out | 0 |
| Unviable | 0 |
| Viable kill rate | 80.00% |

Exact survivor:

```text
crates/sorotte-client-core/src/inbound_order.rs:18:52:
replace == with != in ordered_set_commands
```

With the replacement, incomplete command metadata no longer receives the
missing canonical slots. Existing end-to-end tests remained green because
their decoded payloads already listed every populated command and did not
inspect completion of absent slots. This is an oracle gap, not evidence of an
application bug.

The complete five-mutant inventory contains:

- three whole-function vector substitutions;
- deletion of the negation guarding append; and
- replacement of equality with inequality in the already-present check.

## Test-only improvement

Three tests were added to
`crates/sorotte-client-core/src/session/tests/protocol_tests.rs`.
No production behavior was changed after the baseline.

Focused result:

```text
running 38 tests
test result: ok. 38 passed; 0 failed; 0 ignored
```

The three new tests were also executed 50 times serially:

```text
focused stress passed:
iterations=50
test_executions=150
elapsed_seconds=20.529
```

No sleeps, wall-clock comparisons, network operations, random seeds, or
scheduler races are involved.

## Experiment 1: final mutation proof

Command:

```text
cargo mutants --package sorotte-client-core \
  --file crates/sorotte-client-core/src/inbound_order.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --cargo-test-arg=session::tests::protocol_tests:: \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutants-client-inbound-order-final-20260730
```

Artifact:

```text
target/mutants-client-inbound-order-final-20260730/mutants.out
```

Elapsed wall time recorded by cargo-mutants: 88.742 seconds.

| Outcome | Count |
|---|---:|
| Inventory | 5 |
| Caught | 5 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 0 |
| Viable kill rate | 100.00% |

The inventory stayed at five and the target source hash stayed identical.
No accepted-unviable exception is required.

## Proposed scheduled shard

This slice intentionally does not edit the shared mutation policy or
workflow. The exact proposed `coverage/mutation-policy.toml` stanza is:

```toml
[[shard]]
id = "client-inbound-order"
owner = "client-protocol"
package = "sorotte-client-core"
files = ["crates/sorotte-client-core/src/inbound_order.rs"]
test_target = "lib"
test_filter = "session::tests::protocol_tests::"
jobs = 2
timeout_seconds = 60
build_timeout_seconds = 120
minimum_viable_kill_percent = "100.00"
max_missed = 0
max_timeouts = 0
require_baseline = true
```

The workflow matrix entry should use `client-inbound-order`. The scheduled
preflight should discover exactly 38 tests at this source state and fail
closed on an empty selector, empty mutation inventory, source-hash drift,
miss, timeout, or artifact-count mismatch.

## Integrated checked-in policy replay

After adding the proposed shard to the shared eight-shard policy, the exact
checked-in wrapper command passed:

```text
python scripts/mutation_ci.py run --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard client-inbound-order \
  --results-root target/mutation-ci/client-inbound-order-20260730 \
  --output target/verification/mutation-client-inbound-order-20260730.json
```

The final attestation again selected the 38-test namespace, passed its
unmutated baseline, and caught 5/5 viable mutants with zero misses, timeouts,
unviables, or exceptions. Its source-before and source-after SHA-256 values
both equal `50eb172d41813a9ffa8958a10d138f591db593341922d909a9a5010ab755b0db`.
The 9,245-byte report has SHA-256
`86ef4313a996e21dac3d726324752a569573432dd1be46d4034491086dc313cf`.

## Validation

All commands were run from
`C:\tmp\sorotte-test-coverage-design` on
`codex/test-coverage-design`.

| Check | Result |
|---|---|
| original-versus-extracted function text | identical after normalizing `pub(super)` |
| post-extraction, pre-test owning selector | 35/35 passed |
| final owning selector | 38/38 passed |
| focused serial stress | 150/150 test executions passed |
| final cargo-mutants inventory | 5/5 viable mutations caught |
| `cargo test --locked -p sorotte-client-core --all-features` | 718/718 library tests passed; doc tests passed |
| `cargo clippy --locked -p sorotte-client-core --all-targets --all-features -- -D warnings` | passed |
| targeted `rustfmt --check` | passed |
| targeted `git diff --check` | passed |

## Defect accounting and limitations

No new defect was surfaced. Accordingly, there is no expected-failure test
or defect-registry entry from this slice.

`TC-CLI-003` remains open and unchanged. That defect is in the CLI transport
read-cancellation/framing boundary. This shard starts after protocol decoding
inside client-core and neither exercises nor repairs that boundary.

This proof is deliberately narrow. It does not establish:

- correctness of raw JSON command-order scanning in `sorotte-protocol`;
- cancellation safety or fragmented framing in `sorotte-cli`;
- semantics of every individual inbound command;
- all permutations of duplicated raw JSON keys; or
- mutation coverage for the remainder of `inbound.rs`.

Those behaviors retain their separate parser corpus, raw-loopback, defect,
and protocol/session evidence. This shard proves only the five mutations
generated for the extracted client inbound ordering decision and the
ordinary tests selected above.
