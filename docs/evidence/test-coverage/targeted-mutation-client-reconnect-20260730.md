# Targeted mutation proof: client reconnect and state acknowledgement

Date: 2026-07-30 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit: `dccb319d28766a97f79c04e70ebff575c1396fe6`

Producer: `cargo-mutants 27.1.0`

Target: `sorotte-client-core`,
`crates/sorotte-client-core/src/session/reconnect.rs`

Scheduled test scope: `--lib session::tests::`

## Claim

The `client-reconnect-state` shard turns reconnect reset, inbound state
reconciliation, and `ignoringOnTheFly` acknowledgement fencing into a
zero-survivor mutation ratchet. The final source-bound run inventoried 445
selected tests and 32 mutations. It caught all 30 viable mutations, with no
misses or timeouts. Two generated `|| let` replacements cannot parse and are
matched by exact, expiring policy entries.

No application defect was found or fixed. The surviving mutations exposed
four missing behavior oracles:

- a reconnect reset must preserve the canonical V2 readiness projection only
  when its room matches the reconnecting room;
- position and paused are jointly required before either ping-only or
  telemetry-backed reconciliation may apply an inbound playstate;
- a pause-only local state change must enter acknowledgement fencing even
  when it is not also a seek;
- without fresh player telemetry, a pending client acknowledgement must keep
  remote state fenced and keep advertising the client counter.

The slice also closes an attestation gap: a valid cargo-mutants baseline can
otherwise pass while a typo or substring collision selects zero tests. The
wrapper now inventories tests before mutations, requires at least one test,
requires every focused selector to remain inside the configured namespace,
and records the exact selector list and canonical digest.

## Why this target

`reconnect.rs` is a compact, high-consequence state-machine boundary. It owns:

- clearing connection-scoped state while preserving reconnect restoration
  intent;
- preserving acknowledgement-fenced playlist state across transport loss;
- preserving room-scoped canonical readiness;
- rejecting incomplete inbound playstate updates;
- applying inbound state only after the local ignore counter is acknowledged;
- emitting and retiring server/client ignore counters;
- classifying pause or seek changes as state changes.

These decisions are easy to weaken with a one-token boolean change while the
application still compiles and broad happy-path tests continue to pass.
Mutation testing is therefore a better mechanical guard than line coverage
alone.

## Experiment 0: reconnect-only selector

The first unchanged-source probe used the 76 tests below
`session::tests::reconnect_tests::`:

```text
cargo mutants --package sorotte-client-core \
  --file crates/sorotte-client-core/src/session/reconnect.rs \
  --no-config --colors never --no-times --no-shuffle \
  --all-features --cargo-arg=--locked \
  --cargo-test-arg=--lib \
  --cargo-test-arg=session::tests::reconnect_tests:: \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutants-client-reconnect-probe-20260730
```

| Outcome | Count |
|---|---:|
| Caught | 10 |
| Missed | 20 |
| Timed out | 0 |
| Unviable | 2 |
| Viable kill rate | 33.33% |

This proved that the selector was too narrow: `reconnect.rs` also owns state
reconciliation behavior whose established tests live in sibling session test
modules.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `mutants.json` | 71,396 | `314cda60d03d8cc98cd156afcbb050e7eaaf8c68bca5de040ec1856a1fc3d01b` |
| `outcomes.json` | 72,214 | `bd6dc515ee22ec9eed734f3c5d49aaa79a5c7d7b74bdf6e40a0129a4608fd148` |

## Experiment 1: owning session selector

Expanding only to the owning `session::tests::` namespace caught 23 of 30
viable mutations. Seven survivors remained:

| Decision | Surviving mutation |
|---|---|
| same-room readiness preservation | `==` to `!=` |
| complete ping-only playstate | `&&` to `||` |
| ping-only application while client ack is pending | `&&` to `||` |
| emit ignore payload for a pending client counter | `!=` to `==` |
| include the pending client counter | `!=` to `==` |
| complete telemetry-backed playstate | `&&` to `||` |
| pause or seek constitutes a local state change | `||` to `&&` |

| Outcome | Count |
|---|---:|
| Caught | 23 |
| Missed | 7 |
| Timed out | 0 |
| Unviable | 2 |
| Viable kill rate | 76.67% |

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `mutants.json` | 71,396 | `314cda60d03d8cc98cd156afcbb050e7eaaf8c68bca5de040ec1856a1fc3d01b` |
| `outcomes.json` | 72,138 | `8e59fdac72b1a1e8d62bd555850f07269c23ff10ef7fcc26017ab6b17f9098a2` |

## Added behavior oracles

Four deterministic tests distinguish all seven survivors:

1. `reconnect_reset_preserves_same_room_canonical_v2_ready_projection`
   installs an acknowledged Ready snapshot, resets for reconnect, and checks
   that the local user remains canonically ready in that same room.
2. `ping_only_reconcile_rejects_partial_playstate_updates` proves that a
   position without paused does not replace the last complete room state when
   local telemetry is absent.
3. `telemetry_reconcile_rejects_partial_playstate_updates` proves the same
   invariant when fresh local telemetry selects the full reconcile path.
4. `ping_only_reconcile_preserves_pending_local_state_until_client_ack`
   creates a pause-only local transition, proves it enters the client-counter
   fence, removes fresh telemetry, and proves a complete remote state remains
   blocked while the response continues to advertise that counter.

The assertions use public state transitions for the behavior under test. The
single direct model adjustment removes stale local telemetry to select the
ping-only branch while deliberately retaining the acknowledgement fence.

## Fail-closed selector and inventory policy

Before invoking cargo-mutants, `scripts/mutation_ci.py` now runs:

```text
cargo test --package <policy package> --locked --all-features \
  <policy target and filter> -- --list --format terse
```

The wrapper:

- constructs the command itself from the validated shard;
- rejects a failed inventory command;
- rejects malformed non-test output;
- rejects zero selected tests;
- treats a focused filter as a namespace prefix and rejects every selector
  outside it, preventing libtest substring collisions;
- stores stdout, stderr, and parsed selectors in the mutation evidence root;
- records the selected count, exact selectors, and canonical SHA-256 in the
  final report;
- performs this before cargo-mutants inventory or mutation execution.

Adversarial unit tests prove that zero tests stop the runner after tool
verification and that zero mutants stop it before mutation execution. The
existing exact mutation command, source-before/source-after hashes, pre-run
mutation inventory, viable-mutant accounting, artifact reconciliation, and
zero-survivor/zero-timeout thresholds remain in force.

## Final attested run

```text
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard client-reconnect-state \
  --results-root target/mutation-ci/client-reconnect-state-20260730 \
  --output target/verification/mutation-client-reconnect-state-20260730.json
```

Result:

```text
Found 32 mutants to test
ok       Unmutated baseline
32 mutants tested: 30 caught, 2 unviable
mutation shard client-reconnect-state: 30/30 viable mutants caught (100.00%)
```

| Attestation field | Value |
|---|---|
| Status | `passed` |
| Configured source dirty | `false` |
| Source SHA-256 before/after | `b1c3f6aff39e6c3b23c2f46b0508f646f60d9c0af2f51a850e1964d81319d863` |
| Selected tests | 445 |
| Test inventory canonical SHA-256 | `6a8594a5813ad2fd3327d53ee2873176b20286f583e03f8b823711b02b9c69eb` |
| Mutation inventory | 32 |
| Mutation inventory canonical SHA-256 | `e25b796597c46c433d1fb8f150bb7c8108e8d9aef81482a0083bd0b6309c8751` |
| Viable mutants | 30 |
| Caught | 30 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 2 |
| Viable kill rate | `100.00%` |

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Attestation report | 74,336 | `386ce3478571e67d6598f8f6fdaa93d596467316a85868827d07e8254a0b97e4` |
| `test-inventory.json` | 54,913 | `472337931ca05aecfd0e9d71de9927a1598a7a180b992d7883a1c5df2373390f` |
| Pre-run `inventory.list.json` | 71,397 | `c64707054fa64f4f029a529642dcb897d5945edb635411069a0919514ef68740` |
| Producer `mutants.json` | 71,396 | `314cda60d03d8cc98cd156afcbb050e7eaaf8c68bca5de040ec1856a1fc3d01b` |
| Producer `outcomes.json` | 72,305 | `3adfa2be1300c4547873ea5f5c01bc6524b34702f260510aedb6145b3e77ed1f` |

The two accepted unviables are the same cargo-mutants rewrite class in two
functions: replacing the `&&` in a Rust let-chain with `||`. Rust permits a
let expression only in an `&&` chain, so those generated forms fail to parse.
Both exceptions are identity-bound to file, function, return type, mutation
genre, and exact replacement, and expire for review on 2026-10-31.

## Scheduled enforcement

`.github/workflows/rust-mutation.yml` now schedules
`client-reconnect-state` alongside the existing bounded shards. Its policy is:

- one source file;
- the `sorotte-client-core` library target;
- the owning `session::tests::` namespace;
- two mutation jobs;
- 60-second per-test timeout;
- 120-second build timeout;
- required unmutated baseline;
- 100% viable kill threshold;
- zero misses and zero timeouts.

This is intentionally a scheduled ratchet rather than a pull-request latency
gate. Any new viable survivor, timeout, empty test selection, empty mutation
inventory, selector escape, stale exception, source drift, or contradictory
producer artifact fails closed.

## Validation

The completed slice passed:

- `python scripts/mutation_ci.py validate --repo-root . --policy
  coverage/mutation-policy.toml --shard client-reconnect-state`:
  4 shards and 11 exact accepted-unviable entries validated;
- `python -m unittest scripts.tests.test_mutation_ci
  scripts.tests.test_ci_policy`: 50 tests passed;
- `cargo test -p sorotte-client-core --lib 'session::tests::' --
  --nocapture`: 445 tests passed;
- `cargo clippy -p sorotte-client-core --all-targets --all-features --
  -D warnings`: passed;
- `cargo fmt --all -- --check`: passed;
- `actionlint .github/workflows/rust-mutation.yml`: passed using the installed
  `C:\Users\shaun\go\bin\actionlint.exe`;
- `git diff --check` over the owned tracked files: passed.

The retained attestation was also re-read without rerunning the long
experiment. Its source-before and source-after hashes are equal and match the
current configured source; all 445 selectors remain inside
`session::tests::`; both the 32-item mutation inventory and outcome total match
the report; and the on-disk `mutants.json` and `outcomes.json` hashes match the
attested artifact hashes.
