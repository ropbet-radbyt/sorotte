# Nextest flake and subprocess-leak evidence — 2026-07-28

This is a durable, sanitized record of the experiments used to harden the
required Rust workspace runner. The updater implementation and its test were
not changed after the runner exposed the intermittent behavior.

## Provenance and enforced command

- Tool: `cargo-nextest 0.9.137 (75ddba7e9 2026-05-26)`
- Commit: `75ddba7e911b44c5c0700dac0415d824403de9bd`
- Windows release archive SHA-256:
  `88c746b41b1e96165028ef90b9dac5d37eb923e4e00aee6b9080a038f1ac2705`
- Workspace profile: `.config/nextest.toml` profile `ci`
- Leak contract: an inherited subprocess handle still open after 500 ms is a
  failed test result.
- Retry contract: one retry means two total attempts; a later pass never
  converts the required check to green.

The checked wrapper constructs this command rather than accepting arbitrary
test arguments:

```text
cargo nextest run --locked --workspace --all-features --profile ci \
  --retries 1 --no-fail-fast --status-level leak \
  --final-status-level fail --flaky-result fail
```

The wrapper independently validates the checked profile, exact cargo-nextest
version, producer exit code, JUnit attempt elements, and nonzero testcase
count. CI always retains the console log, JUnit, and policy report.

## Real workspace observation

The intermittent selector is:

```text
binary: sorotte-gui::updater_self_replacement_windows
test:   running_installed_updater_recovers_interrupted_replacement_and_restarts
```

The first diagnostic full-workspace run completed 3,458 tests with 21 skipped
and reported this result:

```text
LEAK [0.919s] sorotte-gui::updater_self_replacement_windows
  running_installed_updater_recovers_interrupted_replacement_and_restarts
```

That run returned zero under the profile then in use, proving that merely
displaying `LEAK` was not a blocking contract. A second full run with the
hardened 500 ms fail-on-leak profile completed the same 3,458 tests without
reproducing the leak and returned zero. The different outcomes establish
intermittency; the clean rerun does not supersede the first observation.

A final run through the exact checked wrapper reproduced the behavior and
proved the new policy catches it:

```text
TRY 1 LKFAIL [1.161s] sorotte-gui::updater_self_replacement_windows
  running_installed_updater_recovers_interrupted_replacement_and_restarts
TRY 2 PASS [1.127s]
  - test configured to fail if flaky
FLKY-FL 2/2 ...
```

cargo-nextest returned `100`, and the wrapper returned `1`. The JUnit report
contained 3,458 testcases, one final failure, and one `flakyError` attempt.
The policy report independently recorded all three violations: nonzero
producer exit, pass-after-fail, and final test failure. This is the desired
classification: the retry provides a second evidence set but cannot hide
either the inherited-handle leak or its nondeterminism.

## Controlled inherited-handle proof

A temporary synthetic test launched a child process that inherited the
libtest output handle and kept it open beyond the 500 ms limit. With the exact
CI profile, both attempts failed:

```text
producer_exit=100
TRY 1 LKFAIL [0.520s]
TRY 2 LKFAIL [0.521s]
Summary: 0 passed, 1 failed (1 due to being leaky)
```

JUnit encoded the first attempt as `<error>` and the retry as
`<rerunError>`. The wrapper's adversarial tests also prove that either element
is rejected even if a producer were to return zero. A separate controlled
fail-then-pass test returned `100` and emitted both the original failure and
flaky attempt, proving the same fail-closed behavior for ordinary assertion
flakes.

The synthetic fixture lived only under `target/` and is not part of the
application. This note retains only non-secret, decision-relevant output.
