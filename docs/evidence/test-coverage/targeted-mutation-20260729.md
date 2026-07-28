# Targeted mutation proof: privacy-secret

Date: 2026-07-29 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Checkout commit: `f3964ebc7f7b281b9b78f3bfb243ff65e5122e33`

Producer: `cargo-mutants 27.1.0`

Target: `sorotte-secret`, `crates/sorotte-secret/src/lib.rs`

## Claim

The first bounded mutation shard found a real test-oracle gap, not a product
defect. Seven test-only additions moved the same 44-mutant inventory from
22/43 viable mutants caught to 43/43. No production behavior was changed.

The scheduled implementation now fails closed on a missing or failed baseline,
any missed mutant, any timeout, any new or stale unviable exception, producer
version drift, source changes during execution, pre-run/result inventory
drift, malformed or contradictory JSON, status-file disagreement, missing or
escaping artifacts, weakened Cargo test arguments, and producer exit/result
contradictions.

## Why this target came first

`sorotte-secret` is small, pure, security-sensitive, and already classified as
a critical privacy path. It therefore gives mutation testing a meaningful
oracle-strength experiment without paying the noise and runtime cost of
whole-workspace mutation. It also avoids starting with GUI rendering, FFI,
process entrypoints, or nondeterministic integration code.

The experiment used the official cargo-mutants baseline, file filtering,
timeout, and machine-readable output contracts:

- <https://mutants.rs/baseline.html>
- <https://mutants.rs/skip_files.html>
- <https://mutants.rs/timeouts.html>
- <https://mutants.rs/output.html>

## Experiment 1: expose the weak oracles

The initial command was:

```text
cargo mutants --package sorotte-secret \
  --file crates/sorotte-secret/src/lib.rs \
  --no-config --colors never --no-times \
  --jobs 2 --timeout 60 --build-timeout 120 \
  --output target/mutation-probe
```

The unmutated baseline passed. The run found 44 mutants and completed in
21.33 seconds:

| Outcome | Count |
|---|---:|
| Caught | 22 |
| Missed | 21 |
| Timed out | 0 |
| Unviable | 1 |
| Viable kill rate | 51.16% |

All 21 survivors were missing observations around intended behavior:

| Oracle gap | Survivors |
|---|---:|
| `from_option_names` return and count arithmetic | 3 |
| Safe standalone flag arms other than the already-tested fullscreen arm | 5 |
| Exact option-name aliases, rejection, and match arms | 9 |
| Empty versus non-empty secret | 2 |
| Owned and borrowed conversion preservation | 2 |

There was no evidence of incorrect production behavior. The tests simply did
not distinguish those mutations from the intended implementation.

The initial inventory artifact was 63,240 bytes with SHA-256
`8620d8b95e1a9f327dd2c00a17fb4b89c2207320bd99e9ede1f2e2198d466f28`.
The initial outcome artifact was 84,151 bytes with SHA-256
`d5c1a1e0d01bcd24f60c81ee270846d6176398d81c9d21b97fbc196ccae11a34`.
The pre-test source was Git blob
`1220a8646151a16942e355000adb899a740d0645`.

## Oracle changes

Only `#[cfg(test)]` code changed. The seven new tests encode:

- every approved standalone flag and both fullscreen aliases;
- exact rejection of case, prefix, assignment, spelling, and sensitive-option
  near-matches;
- first-seen ordering and alias deduplication;
- counting of safe, unknown, and duplicate option names;
- the count-only summary contract;
- the distinction between empty and blank-but-nonempty secrets;
- exact preservation through `From<String>` and `From<&str>`.

The focused crate suite passed 11/11 before mutation was repeated.

## Experiment 2: source-bound scheduled contract

The final experiment used the checked-in wrapper:

```text
python scripts/mutation_ci.py run \
  --repo-root . \
  --policy coverage/mutation-policy.toml \
  --shard privacy-secret \
  --results-root target/mutation-wrapper-final \
  --output target/verification/mutation-wrapper-final.json
```

The wrapper verified `cargo-mutants 27.1.0` exactly and constructed the command
itself. The producer invocation disables local cargo-mutants configuration,
selects only the declared package and file, disables shuffle, enables all
features, forwards `--locked` to every Cargo invocation, uses two workers, and
bounds test/build commands to 60/120 seconds.

The result completed in 20.57 seconds:

| Outcome | Count |
|---|---:|
| Unmutated baseline | Passed |
| Caught | 43 |
| Missed | 0 |
| Timed out | 0 |
| Accepted unviable | 1 |
| Viable kill rate | 100.00% |

The pre-run inventory and result inventory were exactly equal. Importantly,
the inventory bytes and SHA-256 were also identical to Experiment 1, proving
that the improvement did not come from selecting fewer mutations.

The final source binding and primary artifact bindings were:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Configured Rust source | 10,605 | `9b84fb16293495b409ec8bccfe110e725a11a178eba1100e9002dc2e12bd020b` |
| `mutants.json` | 63,240 | `8620d8b95e1a9f327dd2c00a17fb4b89c2207320bd99e9ede1f2e2198d466f28` |
| `outcomes.json` | 89,612 | `2683d325efe294c0dc5bd7de3f2027e8aaa292739ae57632713be066d67eb9fd` |
| Wrapper report | 19,907 | `afeeabb671aea3613f9a59214b251b359a3231f76e592e67b3e30545a3fbfdb9` |

The canonical normalized inventory digest was
`a2fd3d151aefcf5c6ac8713e7bf9842c6f0e8ce3b0ba42206683394ed32f3a89`.
The configured source hash was identical before and after the run.

The local checkout correctly records `configured_sources_dirty: true` because
this experiment ran before committing the new test-only branch changes. The
full source hash above, the equal pre/post binding, and the exact inventory are
the authority for this local proof. The scheduled workflow runs from a fresh
read-only checkout and will normally report a clean configured source.

## The one unviable mutation

The exact compiler-infeasible mutant is:

```text
crates/sorotte-secret/src/lib.rs:48:9: replace \
RedactedCommandArgs::from_count -> Self with Default::default()
```

`from_count` is a `const fn`; cargo-mutants inserts a non-const
`Default::default()` call, so Rust rejects the mutant before tests can execute.
This is not treated as a survivor. The policy keys the exception to the stable
file/function/return-type/genre/replacement identity rather than its line
number, requires it to be present exactly once, and expires its review on
2026-10-31. A new unviable mutant or disappearance of this one both fail.

## Mechanical enforcement

`scripts/mutation_ci.py` validates strict TOML and owns the producer command.
It inventories before running and then verifies:

1. exact tool version and current Git/source binding;
2. exact equality of the pre-run and result inventories;
3. exactly one successful baseline and one outcome per mutant;
4. coherent build/test phases using `--locked` and `--all-features`;
5. structured counts against all four status text files;
6. safe, present, uniquely referenced log and diff artifacts;
7. source hashes before and after mutation;
8. the 100% viable kill ratchet and exact expiring unviable set;
9. zero/nonzero producer exit coherence.

The report hashes the raw inventory, outcomes, lock, status, log, and diff
artifacts without publishing the lock file's hostname or username fields in
the report. Twenty-six focused Python tests exercise the successful path and
adversarial schema, traversal, duplicate-key, source-drift, inventory,
summary, phase, policy-expiry, producer-exit, and tool-version cases. The main
CI policy suite separately binds the workflow, pinned Actions, schedule,
command, artifact upload, and exact repository policy.

## Finding disposition and limits

No product bug surfaced in this slice, so there is no product fix or
known-defect entry. The 21 initial survivors are closed as test-oracle gaps by
the repeat experiment. The one unviable mutant is a compiler/tooling
constraint, not an application finding.

This proof covers one pure privacy module and should not be generalized to the
workspace. It does not exercise concurrency schedules, I/O failures, unsafe
code, process boundaries, GUI behavior, or equivalent mutations that
cargo-mutants does not generate. The next high-value shards should remain
bounded and independently timed:

1. protocol key-order parsing and redaction;
2. authorization and persistence arbitration;
3. lifecycle reducer and attachment/generation fences;
4. config precedence and atomic-state decisions.

Each should start with an observed baseline, add tests without altering
surfaced product defects, and earn a 100% or explicitly reviewed meaningful
kill ratchet before becoming scheduled evidence.
