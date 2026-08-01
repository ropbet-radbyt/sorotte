# Targeted mutation proof: CLI protocol framing

Date: 2026-07-30 (Australia/Sydney)

Branch: `codex/test-coverage-design`

Experiment checkout commit: `8fc81f652d0ca0978150919b91ff6c07d8cb4174`

Producer: `cargo-mutants 27.1.0`

Target package: `sorotte-cli`

Target source:
`crates/sorotte-cli/src/protocol_io.rs`

Scheduled test scope: full package

## Claim

The `cli-framing` shard is a source-bound, zero-survivor mutation ratchet for
the CLI protocol I/O boundary. Its stable final run selected 370 package tests
and caught all 33 generated viable mutants. There were no misses, timeouts, or
compiler-unviable mutants, so no exception was added.

The proof covers mutations generated for:

- the session-owned `InboundProtocolLineReader` accumulator;
- fragmented, coalesced, LF, CRLF, length-limit, EOF, and cancellation
  framing decisions;
- the one-shot inbound compatibility wrapper;
- CRLF protocol writes; and
- pending-frame flush, acknowledgement, failure, and deadline decisions.

No production code changed in this mutation slice. No product defect was
found. The exploratory survivors and timeouts described below were ordinary
test-oracle and test-liveness gaps, and the stronger framing tests distinguish
them without changing the parser.

## Scheduled fail-closed contract

`coverage/mutation-policy.toml` adds this exact policy:

```toml
[[shard]]
id = "cli-framing"
owner = "cli-transport"
package = "sorotte-cli"
files = ["crates/sorotte-cli/src/protocol_io.rs"]
test_target = "package"
test_filter = ""
jobs = 2
timeout_seconds = 60
build_timeout_seconds = 120
minimum_viable_kill_percent = "100.00"
max_missed = 0
max_timeouts = 0
require_baseline = true
```

The weekly and manually dispatchable mutation workflow includes
`cli-framing` in its matrix. The existing wrapper enforces:

- the pinned `cargo-mutants 27.1.0` producer;
- `cargo test --package sorotte-cli --locked --all-features` as the
  unmutated baseline and test inventory;
- one exact source path and matching pre-run/post-run source hashes;
- a non-empty, exact mutation inventory before and after execution;
- one structured outcome and one diff artifact for every mutant;
- 100% viable kill, zero misses, zero timeouts, and a required baseline; and
- rejection of every unexpected or stale compiler-unviable exception.

The full package target is intentional. The source also owns outgoing
protocol-line writes and pending-frame acknowledgement behavior, whose
existing unit tests sit outside the new schedule-test namespace. A selector
limited to the four new framing tests would not be an honest ratchet for the
whole source file.

## Inventory

The final inventory contains 33 mutations:

| Function | Mutations |
|---|---:|
| `InboundProtocolLineReader::read_line` | 24 |
| `read_inbound_protocol_line` | 3 |
| `write_protocol_line` | 1 |
| `flush_runtime_protocol_lines_with_deadline` | 2 |
| `flush_runtime_protocol_lines` | 1 |
| `flush_runtime_protocol_lines_until` | 2 |
| **Total** | **33** |

The inventory includes whole-function return substitutions, comparison
reversals, accumulated-length arithmetic changes, CR/LF index changes,
delimiter-consumption changes, and boolean return substitutions.

## Exploratory red campaign

The first wrapper campaign was deliberately retained rather than overwritten:

```text
python scripts/mutation_ci.py run
  --repo-root .
  --policy coverage/mutation-policy.toml
  --shard cli-framing
  --results-root target/mutation-ci/cli-framing-20260730
  --output target/verification/mutation-cli-framing-20260730.json
```

| Field | Value |
|---|---:|
| Selected tests at preflight | 369 |
| Total mutants | 33 |
| Caught | 26 |
| Missed | 3 |
| Timed out | 4 |
| Unviable | 0 |
| Viable kill rate | 78.79% |
| Started UTC | `2026-07-30T09:50:32.5886842Z` |
| Finished UTC | `2026-07-30T10:02:11.3799869Z` |
| Elapsed | 698.791 seconds |

The exact misses were:

```text
crates/sorotte-cli/src/protocol_io.rs:108:60:
replace + with * in InboundProtocolLineReader::read_line

crates/sorotte-cli/src/protocol_io.rs:113:90:
replace == with != in InboundProtocolLineReader::read_line

crates/sorotte-cli/src/protocol_io.rs:113:85:
replace - with / in InboundProtocolLineReader::read_line
```

The exact timeouts were:

```text
crates/sorotte-cli/src/protocol_io.rs:86:9:
replace InboundProtocolLineReader::read_line with Ok(Some(String::new()))

crates/sorotte-cli/src/protocol_io.rs:86:9:
replace InboundProtocolLineReader::read_line with Ok(Some("xyzzy".into()))

crates/sorotte-cli/src/protocol_io.rs:125:42:
replace + with - in InboundProtocolLineReader::read_line

crates/sorotte-cli/src/protocol_io.rs:125:42:
replace + with * in InboundProtocolLineReader::read_line
```

The two constant-`Some` replacements never consumed input or reached EOF. The
two delimiter arithmetic replacements could leave the delimiter unconsumed.
The original helper read until `None`, so those mutants produced an unbounded
stream of apparent frames and were classified only by cargo-mutants' timeout.

The three misses altered only the maximum-line-length guard. The existing
small valid frames remained far below the limit:

- multiplying instead of adding the retained prefix and current newline
  offset needs a deliberately chosen split at the size boundary;
- reversing the same-buffer CR predicate needs exact-limit CRLF in one
  buffer; and
- dividing the newline index by one instead of subtracting one inspects the
  newline rather than the preceding CR, which needs the same exact-limit
  single-buffer oracle.

This red aggregate is exploratory evidence, not the canonical homogeneous
test-source attestation. The framing-test file was strengthened while this
campaign was still finishing. The wrapper binds the production source and
the selected test names but does not hash all test source bytes. Its report
therefore proves the seven exact observed outcomes and the unchanged
production source, but this document does not overclaim a single immutable
test-file provenance for the aggregate 26/3/4 result. The stable final
campaign below is the canonical claim.

The exploratory report is 66,241 bytes with SHA-256:

```text
e6a1263fd1b21aeebe895e14c48c4ffcb7fe7b2b7390f585446e64f8dd7e3a06
```

## Test-only oracle strengthening

`crates/sorotte-cli/src/tests/framed_transport_schedules.rs` now makes every
read schedule finite by deriving the expected number of wire frames from the
input, reading exactly that many frames, and performing one explicit EOF
probe. A mutant that returns extra frames or fails to consume a delimiter now
fails promptly instead of relying on a wall-clock timeout.

The deterministic
`split_lf_and_crlf_payload_limits_use_exact_accumulated_length` test adds
exact-limit and `MAX + 1` cases for LF and CRLF, including:

- exact-limit CRLF in one scheduled chunk;
- over-limit CRLF split after one payload byte;
- exact-limit LF split at the last payload byte; and
- over-limit LF split at the last payload byte.

Those cases independently distinguish accumulated prefix arithmetic,
same-buffer versus retained-buffer CR handling, and the exact
`line_len > MAX` boundary. The complete focused namespace passed 4/4 tests.

The stable test file is 16,186 bytes with SHA-256:

```text
086d7366bf8c1451f1662685b4cd3e1fb4423ae74319a997941d5f124bc0d54b
```

## Canonical stable campaign

The final campaign used a fresh result root after the stronger test file was
stable:

```text
python scripts/mutation_ci.py run
  --repo-root .
  --policy coverage/mutation-policy.toml
  --shard cli-framing
  --results-root target/mutation-ci/cli-framing-final-20260730
  --output target/verification/mutation-cli-framing-final-20260730.json
```

Producer result:

```text
Found 33 mutants to test
ok       Unmutated baseline
33 mutants tested: 33 caught
mutation shard cli-framing: 33/33 viable mutants caught (100.00%)
```

| Attestation field | Value |
|---|---|
| Status | `passed` |
| Producer exit | `0` |
| Checkout HEAD | `8fc81f652d0ca0978150919b91ff6c07d8cb4174` |
| Configured source dirty | `false` |
| Source bytes | 21,060 |
| Source SHA-256 before/after | `744409f99791e9db45edca48e372677e583ef1427d298b0fbaa7617f8bc25845` |
| Selected tests | 370 |
| Test inventory canonical SHA-256 | `70a6f6e4d7847121e1d12f07c29b5770ff8c7ef450345aa70b3a691672e44c36` |
| Mutation inventory | 33 |
| Mutation inventory canonical SHA-256 | `242223eeb003a0ee4390adaabca551d06f93e53ecb013c06aa7b494bea94ba38` |
| Caught | 33 |
| Missed | 0 |
| Timed out | 0 |
| Unviable | 0 |
| Viable kill rate | `100.00%` |
| Started UTC | `2026-07-30T10:03:22.7303804Z` |
| Finished UTC | `2026-07-30T10:13:02.4204955Z` |
| Elapsed | 579.690 seconds |

The pre-run and producer mutation inventories have the same canonical hash.
The target production source is clean relative to the experiment commit and
has the same byte count and SHA-256 before and after the run. Repository-wide
dirty state from the four concurrent test slices is outside this exact source
binding.

## Canonical artifact hashes

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Final attestation report | 65,338 | `f60c952ce3f9f1c87761666f565f6b87c27988a985dcd9f83c3ca14ebf0b30dd` |
| `test-inventory.json` | 47,702 | `2d4b54e7dc468898564edfb943b09ae23d93a318ffdb28f665d55bec4803c608` |
| Pre-run `inventory.list.json` | 68,405 | `3a8d4a260c39e5952b45e56b076047f2f9c36697d2cc8033fdb5b775fd69eaa9` |
| Producer `mutants.json` | 68,404 | `cac0d2e4489181683d41c60baed0ee195cb0b8015cd4650e32f61143b727d861` |
| Producer `outcomes.json` | 69,466 | `43f88416ad1ccdebf727e069611cb254ef65eb4eb434f498a5e14fd2926ad080` |

The checked-in enforcement inputs at evidence time are:

| File | Bytes | SHA-256 |
|---|---:|---|
| `coverage/mutation-policy.toml` | 10,840 | `6f6bb69c35036368c30d5e0a72962f5dd7a128ed4f88df4e1864f2dd6e08f14b` |
| `.github/workflows/rust-mutation.yml` | 2,380 | `015772100b444bdf84892cab33f6e742a51027bd36f9d5e52cbc63cc4bd4b31b` |
| `scripts/tests/test_ci_policy.py` | 82,828 | `382b3a1790b510b5bb4b540155867bfa88a4da82be48af0a399146d0129414ac` |

## Validation

All commands ran from
`C:\tmp\sorotte-test-coverage-design` on
`codex/test-coverage-design`.

| Check | Result |
|---|---|
| focused framing schedule namespace | 4/4 passed |
| checked-in mutation wrapper | 33/33 caught; zero misses, timeouts, and unviables |
| mutation policy validator | 9 shards and 16 exact accepted-unviable entries valid |
| CI-policy and mutation-wrapper unit suites | 50/50 passed (13 + 37) |
| installed `actionlint` on `rust-mutation.yml` | passed |
| targeted `git diff --check` | passed |

## Defect accounting and limitations

No product defect or compiler-unviable mutation was found. No production
source, known-defect entry, or accepted-unviable policy entry changed.

The exploratory red campaign found the independent harness/oracle defect
`TC-HARNESS-017`:

- three observable size-boundary mutations survived;
- four non-progress mutations were detected only by timeout.

The stable deterministic tests close both classes, and the final campaign
confirms every one of the 33 generated mutants is caught without a timeout.

This proof is deliberately bounded. It does not establish:

- mutation coverage for CLI sources other than `protocol_io.rs`;
- arbitrary transport correctness beyond the generated cargo-mutants
  inventory and the deterministic framing schedules;
- kernel TCP behavior, TLS, or a live external server;
- fuzz coverage of framed session state; or
- native GUI/player behavior.

The scheduled run is a weekly and manual ratchet rather than a pull-request
latency gate. Its evidence binds the targeted production source and outcome
artifacts. The final test-source SHA-256 above supplements the wrapper's
test-name inventory; the current wrapper does not independently bind every
selected test file's bytes.
