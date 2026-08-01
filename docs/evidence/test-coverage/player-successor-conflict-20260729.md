# Player successor-conflict resolution

Date: 2026-07-29

Finding: `TC-PLAYER-001`

## Question

How should the lifecycle graph behave when more than one load attempt claims
the same predecessor and a later observation selects one successor?

## Original failure

Two minimized reducer histories reached the same invariant failure:

```text
attempt predecessor points to another successor
```

The first combined an active external attempt, a submitted commanded
replacement, and a later external observation. The second retained a rejected
successor's backlink, then accepted another submitted attempt. In both cases
the predecessor selected the newest successor while an older attempt still
claimed the same predecessor.

## Chosen rule

Successor selection is exclusive. Immediately before a live predecessor
selects successor `S`, the reducer clears `replaced_attempt` on every other
attempt that still claims that predecessor.

This rule is intentionally narrow:

- the selected successor keeps the reciprocal backlink;
- unselected pending attempts are not failed or retired;
- later physical evidence can still bind an unselected pending attempt;
- terminal predecessors may retain historical incoming provenance only while
  they select no successor;
- no command result, playlist identity, or physical attempt state is invented.

The same reducer helper is called from `ExternalLoadObserved` and
`LoadAttemptAccepted`, the two transitions that select a successor.

## Positive proofs

The former panic characterizations are now ordinary positive regressions:

- `tc_player_001_external_replacement_preserves_reciprocal_links`;
- `tc_player_001_acceptance_detaches_rejected_successor_backlink`.

The adapter adds two complementary ingress proofs:

- `accepted_load_detaches_a_rejected_successor_claim` exercises real adapter
  submission and acknowledgement;
- `mismatched_authoritative_current_terminalizes_predecessor_before_external_admission`
  proves the authoritative snapshot path terminalizes a contradicted
  predecessor before it admits an external current entry.

The unchecked test reducer, causal defect classifier, two `should_panic`
tests, and `coverage/known-defects.toml` entry were removed. `PL-PROP-001`
now requires every generated transition to preserve graph, epoch, ordering,
and at-most-once invariants.

## Executed evidence

```text
cargo test -p sorotte-player-mpv --all-features --lib tc_player_001 -- --nocapture
2 passed

cargo test -p sorotte-player-mpv --all-features --lib \
  authoritative_reconciliation_regression_tests -- --nocapture
9 passed

$env:PROPTEST_CASES = "10000"
cargo test -p sorotte-player-mpv --all-features --lib generated_reducer_input_histories_preserve_contracts -- --nocapture
1 passed; 10,000 generated histories; 3.11 seconds

cargo test -p sorotte-player-mpv --all-features --lib
407 passed; 2 ignored; 8.96 seconds

python -m unittest scripts.tests.test_known_defect_policy
17 passed

python scripts/known_defect_policy.py validate --registry coverage/known-defects.toml --catalog coverage/behaviors.toml --repo-root .
0 defects; 0 characterizations
```

All tests use the ordinary invariant-checking reducer. No expected panic or
defect skip remains. The registry encodes `defect = []` explicitly, and its
policy suite proves that an empty registry is valid only when the executable
characterization inventory is also empty.

## Whole-application gates

```text
cargo clippy --workspace --all-targets --all-features -- -D warnings
passed; 7.33 seconds

cargo test --workspace --all-features
passed; 208 seconds

powershell -ExecutionPolicy Bypass -File scripts/gui-semantic-suite.ps1 -Json
14 / 14 passed; 30 seconds

powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 -Json -TimeoutMs 80000
10 / 10 required scenarios passed; native duration 110,373 ms; stderr 0 bytes
```

The native run ID is `20260729T072511543Z-38900`; its report SHA-256 is
`0c3524e9903ea05b52f4f2d350a76b7ca7bc62812b081305c9f6c7578b2225df`.
