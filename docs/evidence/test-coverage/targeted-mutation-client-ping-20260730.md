# Targeted mutation proof: client ping and RTT arithmetic

Date: 2026-07-30 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit:
`c4ad56e0bd4bf363f0ec86b605326191bc8073b2`

Producer: `cargo-mutants 27.1.0`

Target: `sorotte-client-core`,
`crates/sorotte-client-core/src/ping.rs`

Scheduled test scope: library target,
`session::tests::ping_tests::` namespace

## Claim

The `client-ping` shard turns legacy-compatible client RTT validation,
moving-average calculation, forward-delay estimation, and wall-clock
delegation into a zero-survivor mutation ratchet. The final source-bound run
inventoried eight selected tests and 47 mutations. It caught all 47 viable
mutations, with no misses, timeouts, unviable mutations, or exceptions.

No application defect was found or fixed. The unchanged-source baseline
exposed nine missing behavior oracles. Eight were closed directly by tests.
The ninth was an algebraically equivalent `<` to `<=` replacement in the
forward-delay branch. A behavior-preserving normalization expresses the same
formula as a base delay plus a nonnegative client/server delta, removing the
redundant mutation point without a waiver.

## Why this target

`ping.rs` is a compact timing boundary whose outputs directly affect playback
position projection and desynchronization decisions. It owns:

- requiring both `clientLatencyCalculation` and `serverRtt`;
- rejecting non-finite timestamps and RTTs;
- rejecting negative server and calculated client RTTs while accepting zero;
- calculating client RTT from the wall-clock observation time;
- retaining the latest client and server RTTs;
- initializing and updating the legacy `0.85 / 0.15` moving average;
- adding only a positive client-minus-server RTT delta to half the average;
- obtaining current Unix epoch time through both public wrapper paths.

Line coverage alone cannot distinguish most of these decisions. A single
boolean or arithmetic replacement can leave the code compiling and can retain
plausible-looking latency values while materially changing synchronization.

## Experiment 0: unchanged-source baseline

The existing namespace contained five tests. The unchanged source was probed
with:

```text
cargo mutants --package sorotte-client-core \
  --file crates/sorotte-client-core/src/ping.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --cargo-test-arg=session::tests::ping_tests:: \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutants-client-ping-baseline-20260730
```

Result:

| Outcome | Count |
|---|---:|
| Caught | 43 |
| Missed | 9 |
| Timed out | 0 |
| Unviable | 0 |
| Viable kill rate | 82.69% |

The nine survivors mapped cleanly to missing behavior:

| Missing oracle | Surviving mutation |
|---|---|
| public live-clock inbound wrapper | `observe_inbound_state` to `()` |
| either non-finite input is sufficient to reject | finite-input `\|\|` to `&&` |
| zero server RTT remains valid | server RTT `< 0` to `<= 0` |
| zero calculated client RTT remains valid | client RTT `< 0` to `<= 0` |
| exact client/server RTT equality | forward branch `<` to `<=` |
| no-positive-delta forward-delay branch | average `/ 2` to `% 2` |
| no-positive-delta forward-delay branch | average `/ 2` to `* 2` |
| public client timestamp wrapper | return `1.0` |
| Unix wall-clock helper | return `1.0` |

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Baseline `mutants.json` | 85,955 | `21aa085e224a6e247aed8d6dffcda603f350bf0f62be728ff544869bba59b373` |
| Baseline `outcomes.json` | 113,826 | `dd5bac043118e93090b7d555c943af3703d87b303b7efecf1a8797336fd1bd9d` |

## Added behavior oracles

The focused namespace now contains eight tests. Four areas received new or
strengthened coverage:

1. `client_ping_metrics_legacy_compatible_ignores_incomplete_and_invalid_inputs_atomically`
   seeds all three metrics, then proves that missing ping objects, missing
   required fields, NaN, positive and negative infinity, negative server RTT,
   a future client timestamp, and non-finite observation times preserve the
   complete previous metric snapshot.
2. `client_ping_metrics_legacy_compatible_accepts_zero_and_equality_boundaries`
   proves that zero server RTT, zero calculated client RTT, and equal client
   and server RTT are accepted and have the intended forward-delay values.
3. `client_ping_metrics_legacy_compatible_applies_multi_sample_moving_average`
   uses three hand-calculated samples. It distinguishes the latest RTTs from
   the historical average, proves both `0.85` and `0.15` weights, and selects
   the branch where the server RTT exceeds the client RTT.
4. `client_ping_metrics_legacy_compatible_wall_clock_entry_points_report_unix_time`
   brackets the production helper, public client timestamp wrapper, and
   public inbound-state wrapper with an independently read `SystemTime`.
   The two-second tolerance accommodates scheduling and wall-clock adjustment
   while still deterministically rejecting constant-return mutations.

The wall-clock behavior did not require a test-only production seam. The
arithmetic tests pass explicit observation times and are fully deterministic;
the one production-clock smoke compares against the same operating-system
clock at the call boundary and passed 100/100 serial stress iterations.

## Experiment 1: new oracles against unchanged arithmetic

The 52-mutant experiment was repeated after adding the tests:

| Outcome | Count |
|---|---:|
| Caught | 51 |
| Missed | 1 |
| Timed out | 0 |
| Unviable | 0 |
| Viable kill rate | 98.08% |

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Pre-normalization `mutants.json` | 85,955 | `21aa085e224a6e247aed8d6dffcda603f350bf0f62be728ff544869bba59b373` |
| Pre-normalization `outcomes.json` | 114,063 | `bddc7ba9cd5d5f5a8659cfea82b0d8a6490349552d793d6b3dc813e991626fe2` |

The sole survivor replaced:

```text
server_rtt < current_rtt
```

with:

```text
server_rtt <= current_rtt
```

The predicates differ only when the already validated finite values are
equal. At equality, `current_rtt - server_rtt` is zero, so the two branches
both yield `average_rtt / 2`. No test can legitimately distinguish them.

## Behavior-preserving formula normalization

The conditional:

```text
if server < client {
    average / 2 + (client - server)
} else {
    average / 2
}
```

is now expressed as:

```text
average / 2 + max(client - server, 0)
```

The preceding guards guarantee finite client and server RTTs and reject
negative values. For `client > server`, `max(client - server, 0)` is the same
positive delta as the original first branch. For `client <= server`, it is
zero, which is the original second branch. The equality regression test
anchors the exact boundary. This removes duplicate division expressions and
the equivalent comparison mutant while retaining the observable arithmetic
contract.

## Final attested run

The checked-in wrapper constructed, executed, and reconciled the exact shard:

```text
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard client-ping \
  --results-root target/mutation-ci/client-ping-20260730 \
  --output target/verification/mutation-client-ping-20260730.json
```

Result:

```text
Found 47 mutants to test
ok       Unmutated baseline
47 mutants tested: 47 caught
mutation shard client-ping: 47/47 viable mutants caught (100.00%)
```

| Attestation field | Value |
|---|---|
| Status | `passed` |
| Configured source dirty | `true` |
| Source SHA-256 before/after | `769fcc9759fa497947cc783a2289e242fa3db4de03cd146d5eeb8c7e74e77740` |
| Selected tests | 8 |
| Test inventory canonical SHA-256 | `885f200b885f5fb814d9480209f9b4c8f5fc8b0f5ee6bf09a5ea952974e74442` |
| Mutation inventory | 47 |
| Mutation inventory canonical SHA-256 | `cd178c9b89b05ac32c1d06d4100e2c8165b895b3a8a1d5b167b8e5a1455bf91d` |
| Viable mutants | 47 |
| Caught | 47 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 0 |
| Accepted exceptions | 0 |
| Viable kill rate | `100.00%` |

The source-before and source-after hashes are equal. The report records the
source as dirty because this proof was deliberately run before the slice was
committed; the source binding, equal inventory digests, and raw-artifact
digests bind the result to the exact tested source.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Attestation report | 22,668 | `304e8c490d239b07759bf1e788c64b658c6bf241b061273e922d8f47ec6984e2` |
| `test-inventory.json` | 901 | `92f5cc734d1bc256ec562a9e7bc85ed80852172221723aa40eeb98dd56c26214` |
| Pre-run `inventory.list.json` | 78,486 | `12b2cc52b2c1fb38811140c49fe86ce4cc6bffec11c2aad6b558456dc393ff10` |
| Producer `mutants.json` | 78,485 | `e3bf52ed69fba68f173b7ba1b89b14f6d52feb15a7c32a09e5dd205c8d5c56f4` |
| Producer `outcomes.json` | 103,197 | `20ff64d7926d2ae0f4d5fd95ccff790fe4d5b08baef678c2beb8117b3309ae60` |

## Scheduled enforcement

`.github/workflows/rust-mutation.yml` schedules `client-ping` as the sixth
bounded shard. Its policy owns:

- one `sorotte-client-core` source file;
- the library target and exact ping-test namespace;
- two mutation jobs;
- 60-second per-test and 120-second build timeouts;
- a required unmutated baseline;
- a non-empty, namespace-confined test inventory;
- a non-empty, source-bound mutation inventory;
- 100% viable kill threshold;
- zero misses, timeouts, and accepted exceptions.

The shard is scheduled rather than added to pull-request latency. Source
drift, a selector escape, an empty inventory, an artifact mismatch, or any new
survivor fails closed.

## Validation

The completed slice passed:

- focused ping namespace: 8/8;
- focused serial stress: 100/100 iterations;
- final mutation attestation: 47/47 viable mutants caught;
- complete `sorotte-client-core` all-feature suite: 715/715;
- `cargo clippy --locked -p sorotte-client-core --all-targets
  --all-features -- -D warnings`;
- mutation wrapper and CI policy tests: 50/50;
- mutation policy validation: 6 shards and 14 exact accepted-unviable
  compiler mutations;
- `actionlint .github/workflows/rust-mutation.yml`;
- `cargo fmt --all -- --check`;
- `git diff --check`.

## Limits

This shard proves the generated mutations in `ping.rs` through the focused
unit namespace. It does not prove monotonic time, operating-system clock
quality, network timestamp authenticity, cross-host clock synchronization,
timer precision, scheduler latency, long-run floating-point error, playback
quality under real jitter, or mutations cargo-mutants does not generate.
Those remain separate simulation, integration, and system-test concerns.
