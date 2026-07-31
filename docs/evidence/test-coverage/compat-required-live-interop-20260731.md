# Required live Python compatibility accounting — 2026-07-31

## Outcome

The compatibility suite now has a single selector-free required-live entry
point. `SYNCPLAY_REQUIRE_LIVE_INTEROP=1` makes missing oracle, Python process,
Python package, legacy-process, TLS, fixture, and disabled-parity paths fail
closed. When the variable is absent, the existing optional developer behavior
is unchanged.

The final local required-live matrix passed:

- discovered: 143 tests;
- executed and passed: 136;
- failed: 0;
- optional skips: 0;
- ignored: 7 exact fixture-generation tests;
- runtime: 47.394740 seconds.

This was a local, bounded compatibility run. The Python reference processes
and Sorotte test processes used test-owned loopback endpoints only. No remote
server, credentials, reconnaissance, persistence, privilege boundary, or
production data was involved.

## Implemented contract

`scripts/compat_live_interop.py` owns the required-live contract:

1. require an explicit local `SYNCPLAY_LEGACY_ROOT`;
2. verify its Git commit is exactly
   `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`;
3. verify CPython is in the supported `>=3.11,<3.14` family while recording the
   exact observed patch version and executable;
4. parse only exact `name==version` requirement pins and verify installed
   package identities;
5. hash the requirement file, both Python probes, and the complete tracked
   protocol/scenario/TLS fixture inventory;
6. discover the complete and ignored libtest inventories with fixed commands;
7. execute the complete all-feature crate matrix serially with no caller test
   selector;
8. bind every optional skip message to its test and a closed reason code;
9. require the executed, skipped, and ignored inventories to be exhaustive and
   disjoint; and
10. emit a duplicate-key-rejecting, exact-key JSON report plus hashed stdout
    and stderr logs.

The shared Rust mechanism is additive:

- only the exact value `SYNCPLAY_REQUIRE_LIVE_INTEROP=1` enables strict mode;
- shared missing-prerequisite classifiers return false in strict mode;
- the existing legacy fanout and TLS assertion switches are implicitly enabled
  in strict mode;
- missing checkout and Python-spawn errors are wrapped as
  `RequiredLivePrerequisite`, preventing direct tests from matching their old
  optional early-return variants; and
- ordinary mode retains the prior error variants and skip behavior.

The required PR and nightly jobs now run the complete wrapper, validate its
report, and upload the report and logs with `if-no-files-found: error`. The
instrumented compatibility coverage subset also sets required-live mode, but
it remains a coverage-profile supplement rather than the complete matrix.

## Exact prerequisite identities

The canonical local report recorded:

- implementation commit:
  `af494bd7c6323f1c95c770964ba9d6dbae9297aa`;
- committed-run path correction:
  `3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16`;
- canonical source and expected-source commit:
  `3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16`;
- Syncplay oracle commit:
  `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`;
- local Python: CPython `3.13.5` at `C:\Python313\python.exe`;
- `twisted==25.5.0`;
- `pyopenssl==25.3.0`;
- `service_identity==24.2.0`;
- requirements SHA-256:
  `90c4a1cba5530acb2cf3f4dbb9a8ebb84887868cfbb505f0bc483665d1343742`;
- handshake-probe SHA-256:
  `48eeb0675bfb94a58f4031b7ca3c5278a0afce0fbc3484ed15191355e9bd1d88`;
- live-peer-probe SHA-256:
  `dd517be076c2009e4c484a33adc05ecd2ba4bb64bba23d7cd422d45265c6435c`;
- fixture files: 89 total (24 protocol, 62 scenarios, 3 TLS);
- fixture-manifest SHA-256:
  `c6965b679ece27107201b420b398f1bf9e19ef8a54e056165b84fc902ab2a76d`.

The CI jobs continue to request Python `3.11`; the wrapper records the exact
patch version delivered by `actions/setup-python` rather than normalizing it.
The local `3.13.5` result is not represented as a CI `3.11` execution.

## Exact ignored inventory

The seven ignored tests are not missing assertions. They deliberately write
committed trace fixtures and remain outside the non-writing matrix:

- `capture_live_reference_controlled_room_trace_fixtures` — requires Twisted
  and writes fixtures from a live legacy server session;
- `capture_live_reference_state_latency_metrics_trace_fixture` — requires
  Twisted and writes fixtures from a live legacy server session;
- `capture_permanent_rooms_file_trace_fixtures` — writes permanent-room
  Python/legacy trace fixtures;
- `capture_persistent_rooms_lifecycle_trace_fixtures` — writes persistent-room
  lifecycle Python/legacy trace fixtures;
- `capture_persistent_rooms_timeout_list_updates_trace_fixtures` — writes
  persistent timeout-list-update Python/legacy trace fixtures;
- `capture_python_fanout_trace_fixtures` — writes Python fanout trace fixtures;
  and
- `capture_python_state_latency_metrics_trace_fixture` — writes the Python
  state-latency trace fixture.

The closed inventory rejects a newly ignored, removed, renamed, duplicated, or
reason-changed test.

## Validation

### Static and policy validation

Passed:

```text
python -m py_compile scripts\compat_live_interop.py scripts\tests\test_compat_live_interop.py scripts\coverage_profile_lanes.py scripts\tests\test_ci_policy.py
python -m unittest scripts.tests.test_compat_live_interop scripts.tests.test_coverage_profile_lanes scripts.tests.test_ci_policy -v
rustfmt --edition 2024 --check <seven changed sorotte-compat Rust files>
cargo clippy --locked -p sorotte-compat --all-targets --all-features -- -D warnings
actionlint -config-file .github\actionlint.yaml .github\workflows\rust-ci.yml
git diff --check
```

The combined Python policy run passed 55 tests. It covers exact requirement
pins, missing Python process, missing package, missing fixture, wrong oracle
revision, TLS/disabled-assertion skip classification, duplicate/extra/missing
JSON keys, contradictory counts, partial execution, partial selectors,
ignored-inventory drift, workflow binding, and the affected coverage-profile
policy. It also proves mixed pass/fail accounting remains globally sorted and
that execution receives the absolute path of the already attested oracle even
when the caller configured a repository-relative path.

The two focused Rust contracts passed:

```text
cargo test --locked -p sorotte-compat required_live -- --nocapture
```

The real `legacy_server_` coverage listing remained exactly 20 tests against
the 143-test all-feature inventory, confirming the updated strict
`filtered_out=123` coverage oracle.

### Required missing-prerequisite proof

With a deliberately absent configured oracle:

```text
SYNCPLAY_REQUIRE_LIVE_INTEROP=1
SYNCPLAY_LEGACY_ROOT=.interop-cache/missing-required-oracle
python scripts/compat_live_interop.py run --repo-root . \
  --output target/verification/compat-required-missing-oracle-final.json
```

The command returned 1. The separately validated report recorded:

- status: `failed`;
- accounting complete: `false`;
- executed: 0;
- skipped prerequisites: 1;
- reason code: `missing-oracle-root`; and
- exact error: `required live prerequisite unavailable:
  missing-oracle-root: configured legacy Syncplay oracle is missing
  syncplayServer.py`.

A direct historical early-return test was also run with required mode and the
same absent oracle:

```text
cargo test --locked -p sorotte-compat --all-features \
  tests::python_protocol_tests::python_interop_roundtrip_returns_server_hello \
  -- --exact --nocapture
```

It failed as intended with exit 101 and
`required live interoperability prerequisite failed`; it did not print a skip
and return success.

With `SYNCPLAY_REQUIRE_LIVE_INTEROP` absent, the same exact test and missing
oracle returned 0 and printed the historical
`skipped due to missing local prerequisites` message. This confirms the
ordinary optional contract was preserved.

### Preserved committed-source RED and path correction

The first execution after the four implementation commits deliberately used
the documented repository-relative oracle path and a fresh output:

```text
target/verification/compat-live-committed-dedb073-v1.json
```

The source-bound report is preserved. It recorded source and expected source
`dedb0736c97561780cdd6250b12704bdfc4ca5c7`, listed 143 tests, and completed
the closed accounting with 75 passed, 61 failed, zero skipped, and seven
ignored in 0.554484 seconds. Every failure had the same prerequisite root:
the wrapper attested `.interop-cache/syncplay-legacy` relative to the
repository but then passed that original relative string to Cargo. The Rust
test process resolved it from its crate working directory and reported the
oracle missing.

This was a wrapper execution-path defect, not a compatibility mismatch. The
fix canonicalizes the attested oracle through `resolve_within(repo_root, ...)`
and passes that absolute path to the child environment. A focused regression
requires the exact absolute value and all three strict-mode switches. The RED
bundle identities are:

```text
eaa6e237207fca2f88cdb0c7a755faed5f6b05cde59d63c3c8e9aba82de1d768  compat-live-committed-dedb073-v1.json
6e89f5295579e814bf4bdfd0a4a26b6eebbd2684e7e5bf1ed79b50356b405153  compat-live-committed-dedb073-v1.stdout.log
124116cd7726aa5111052d11fb7748999c0fcacf7ea3b4b5fdc5ab431c4dfa8e  compat-live-committed-dedb073-v1.stderr.log
```

No RED report or log was overwritten or removed.

### Canonical committed-source live matrix

```text
SYNCPLAY_REQUIRE_LIVE_INTEROP=1
SYNCPLAY_LEGACY_ROOT=.interop-cache/syncplay-legacy
SYNCPLAY_PYTHON_BIN=C:\Python313\python.exe
python scripts/compat_live_interop.py run --repo-root . \
  --output target/verification/compat-live-committed-3cd64ce-v2.json
python scripts/compat_live_interop.py validate \
  --report target/verification/compat-live-committed-3cd64ce-v2.json
```

The fresh report passed closed-schema validation and independently matched
the configured source, local `HEAD`, and expected source at
`3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16`. It recorded:

- listed: 143;
- executed/passed: 136/136;
- failed/skipped/ignored: 0/0/7;
- complete accounting: `true`;
- execution return code: 0;
- runtime: 47.394740 seconds;
- fixture inventory: 89 files with manifest
  `c6965b679ece27107201b420b398f1bf9e19ef8a54e056165b84fc902ab2a76d`;
- report SHA-256:
  `3c6612cd90e53592b2cf6809ed00ba18e24443bcf2557c8e3b7e90c091760961`;
- stdout SHA-256:
  `6a7fc7eb35623f40243aeed714aa0d5244fbbb4cbb2e8ec63dea342c13fd642e`;
- stderr SHA-256:
  `6e8e6cb7e00a08ed9f7119fa6e69dcc19730334c94af5a498eca598d13ba7100`.

The earlier `target/verification/compat-live-local-final.json` run remains
useful historical pre-commit evidence. It is not the canonical source-bound
campaign for this slice.

An earlier exploratory execution had the same green libtest result but failed
the evidence parser because libtest reports ignored tests as
`ignored, <reason>`. The parser was tightened to require the exact ignored
reason suffix; before the exploratory output path was reused, that live log
re-accounted as 135 executed, zero optional skips, and seven ignored. The
distinct `compat-live-local-final` bundle retained the first post-parser-fix
136-test green run; the committed `3cd64ce` bundle above supersedes it as the
canonical campaign.

### Generated framing differential extension

The later generated Rust/Python framing slice added one required-live test
without changing the seven fixture-writer dispositions. Its focused
implementation commit
`e3d8554a61aea9dc1fe8252540e22aff5b134bb6` was then exercised from exact
committed source:

```text
C:\Python313\python.exe scripts/compat_live_interop.py run --repo-root . \
  --output target/verification/compat-live-committed-e3d8554-v1.json
C:\Python313\python.exe scripts/compat_live_interop.py validate \
  --report target/verification/compat-live-committed-e3d8554-v1.json
```

The independently validated report listed 144 tests, executed and passed all
137 non-writing tests, skipped zero, and accounted for the same seven ignored
fixture writers. Its source and expected source both equal `e3d8554`; complete
accounting is `true`, the execution return code is zero, and runtime was
47.920239 seconds. The report SHA-256 is
`ee74f619e51321a775ad9f6b656e1a6e2275d4b629f700ee4086ba41379834ba`.
The exact generated campaign, oracle, stdout/stderr identities, and limitations
are retained in
[`compat-generated-json-framing-differential-20260731.md`](compat-generated-json-framing-differential-20260731.md).

### Final four-slice committed-source matrix

The later four-slice continuation strengthened legacy process synchronization,
added one context-exact delayed permanent-room setter canonicalizer, and leased
legacy server startup ports across threads and processes. Historical report
`target/verification/compat-live-committed-ad410fc-v1.json` is preserved as an
intermediate 147-test checkpoint: 140 passed, seven ignored, zero failed or
skipped, and complete accounting in 48.914050 seconds.

After every implementation and strict-inventory correction was committed, the
canonical command used a fresh output path:

```text
C:\Python313\python.exe scripts/compat_live_interop.py run --repo-root . \
  --output target/verification/compat-live-committed-9f3cb60-v1.json
C:\Python313\python.exe scripts/compat_live_interop.py validate \
  --report target/verification/compat-live-committed-9f3cb60-v1.json
```

The validated report binds source and expected source to
`9f3cb60fbe788575829931b56155f4bc0c19caf0` and the observed/expected pinned
oracle to `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`. It records:

- 149 listed tests;
- 142 executed and passed;
- zero failed and zero skipped;
- the same seven exact ignored fixture writers;
- complete accounting and execution return code zero;
- runtime 48.612783 seconds;
- CPython 3.13.5, Twisted 25.5, pyOpenSSL 25.3, and
  service-identity 24.2; and
- all prerequisite, source, oracle, probe, fixture, inventory, execution, and
  accounting records required by the closed schema.

Canonical bundle identities:

```text
bc3699097ad534930fff7adccb93435c1a1e72d78cc9f127712e65d6722f2793  compat-live-committed-9f3cb60-v1.json
7bd52a6859305a99adf3632cc0d38735855ee3368894db1b5484623d029b65b3  compat-live-committed-9f3cb60-v1.stdout.log
8668abd1e866f904e9415d308885129f9c70491fd2ab80bd746806314eff7dce  compat-live-committed-9f3cb60-v1.stderr.log
```

Two separate default-parallel executions at the port-lease checkpoint also
listed 149, passed 142, ignored seven, and failed/skipped zero. This repetition
is important: it exercised the checkout, delayed-frame, and server-port
coordination under ordinary parallel scheduling rather than a serial-only
mode.

### Exact-head coverage-policy refresh

The later coverage-policy correction changed no compatibility Rust source but
did change the repository head and its source-binding policy. A fresh output
therefore re-ran the complete strict matrix rather than inheriting the
`9f3cb60` result:

```text
SYNCPLAY_REQUIRE_LIVE_INTEROP=1
SYNCPLAY_LEGACY_ROOT=.interop-cache/syncplay-legacy
SYNCPLAY_PYTHON_BIN=C:\Python313\python.exe
C:\Python313\python.exe scripts/compat_live_interop.py run --repo-root . \
  --output target/verification/compat-live-committed-829ab98-v1.json
C:\Python313\python.exe scripts/compat_live_interop.py validate \
  --report target/verification/compat-live-committed-829ab98-v1.json
```

The independently validated report binds both source fields to
`829ab9824d20bc64b03179646c5e182d5c7a4bfb`. It listed 149 tests,
executed and passed all 142 non-writing tests, retained the same seven exact
ignored writers, failed and skipped zero, returned zero, and completed in
48.529611 seconds. Closed prerequisite, oracle, fixture, inventory, execution,
and accounting records remain complete.

Exact-head bundle identities:

```text
be641cf0b556e424aede4adf5b848983c3c6aecade388163882cf7328b30b285  compat-live-committed-829ab98-v1.json
b0b11465dfd99640cc1a2be2e9458b1cc230579d953917d8c6c9876f6bda9ff3  compat-live-committed-829ab98-v1.stdout.log
96d63126df9b96f39864a6a7b322f70bc7014ad9eab9ee5114f26ce458a417a7  compat-live-committed-829ab98-v1.stderr.log
```

### Final integrated gates

After the committed matrix and evidence integration, repository formatting,
`git diff --check`, and actionlint for both changed workflows passed. All 496
Python policy/infrastructure tests passed in 22.380 seconds. The mutation
policy validated 10 shards and 17 exact accepted compiler-unviable identities;
the known-defect policy validated an empty 0-defect/0-characterization
registry. Warning-denied all-target/all-feature workspace Clippy passed in
15.8 seconds, and the complete locked all-feature workspace test suite passed
on its first attempt in 257.5 seconds.

## Limitations

- The local canonical report uses CPython 3.13.5. Hosted workflow run
  `30626889218` separately passed the complete 142-test executable inventory
  with the workflow's Python 3.11 environment; neither result claims every
  supported Python/platform combination.
- The canonical JSON and logs are local ignored `target/verification`
  artifacts. Their exact identities are recorded above; CI will produce and
  upload its own SHA-bound copies.
- The seven trace writers were inventoried but not executed because they mutate
  committed fixtures. Their generated content was not refreshed or claimed.
- The canonical run uses local CPython 3.13.5 and the pinned local oracle. The
  hosted Python 3.11 pass is separate evidence rather than a substitute for
  that source-bound local bundle.
