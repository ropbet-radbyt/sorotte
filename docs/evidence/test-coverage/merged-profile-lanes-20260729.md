# Merged behavioral coverage profiles

Date: 2026-07-29

Branch: `codex/test-coverage-design`

Implementation parent: `d35edfd`

Rust: `1.97.1-x86_64-pc-windows-msvc`

cargo-llvm-cov: `0.8.4`

Legacy Syncplay: `d1c5f85af377c960c5a940707c4d01bc84fd9c3f`

## Claim

The required Linux coverage producer now collects compatible raw LLVM profiles
from three independently checked execution lanes before exporting its JSON and
native text views:

1. locked, all-feature workspace tests;
2. the complete 14-scenario GUI semantic suite;
3. the four exact strict live-TLS compatibility tests.

`scripts/coverage_profile_lanes.py` owns the commands, producer version,
pinned reference revision, instrumentation environment, behavioral oracles,
profile reset/inventory, logs, and final merge check. Before execution it
removes only generated `target/**/*.profraw` and `*.profdata` inputs and
attests that none remains. A command that exits zero but does not create or
change a raw profile fails. A semantic run that reports fewer or different
scenarios fails. The compatibility lane fails on a skipped prerequisite,
ignored test, unexpected selector, or filtered-count drift. The coverage
finalizer hashes and validates this report before it can mark profile
generation successful.

This does not claim that native Windows GUI execution or the complete strict
legacy fanout matrix is merged. Native execution remains separate because its
interactive Windows boundary is not compatible with the hosted Linux lane.
The complete strict fanout matrix remains red for six findings documented
below and is not represented as passing coverage.

## Fail-closed experiments

### Raw-profile location

The first collector prototype examined only `target/*.profraw`. A complete
instrumented workspace run passed, but cargo-llvm-cov had written its profiles
under `target/llvm-cov-target/`; the collector correctly failed for a missing
fresh profile delta. The inventory now recursively hashes every `.profraw`
below `target`, and a unit test proves a nested profile is detected.

### Stale semantic binary

Applying `cargo llvm-cov show-env` to an ordinary `cargo run` initially reused
an older binary in `target/debug`. All 14 semantic scenarios passed, but the
profile inventory did not change, so the collector failed instead of claiming
semantic coverage. External instrumented lanes now set
`CARGO_TARGET_DIR=target/llvm-cov-target`, forcing them to use the same isolated
instrumented build graph as cargo-llvm-cov. A manual replay then passed 14/14
and created fresh raw profiles.

### Stale profile contamination

The first successful merged report found 193 profiles before its workspace
lane and added 36. That proved lane freshness but allowed older local
experiments to influence the final percentage. The resulting 77.92% LLVM
line-instance number was rejected as final evidence.

Schema version 2 now records a `fresh-profile-reset`, restricts deletion to
generated profile extensions below the repository `target` directory, rejects
symlinks, verifies zero remaining inputs, requires the workspace lane to start
at zero, requires every subsequent profile count to be continuous, hashes
profile content rather than trusting timestamps, and forbids a later lane from
removing an earlier lane's profile. The first clean replay removed 229 raw
profiles and one merged profile. The exact-final replay then removed that
trial's 36 raw profiles and one merged profile before recreating and merging
exactly 36 current-run profiles. Unrelated target files and compiled artifacts
were preserved.

### Intermittent player event loss

One complete parallel instrumented workspace run failed:

```text
adapter::player_adapter::nonblocking_maintenance_tests::
property_between_heartbeat_ack_and_response_remains_full_pump_visible

left:  []
right: ["property-change"]
```

The failed lane ran for 78.809 seconds and created 22 fresh profiles. Its
stdout SHA-256 was
`f3f3ce381d986ecaf178ccaa1bd71d0d85397c11be4799e431da63b22512c326`;
its stderr SHA-256 was
`083499ab87d0b3558b8bdd02698a407dff1b2fc0788e29dee29599afe6a19181`.

Fifty ordinary exact replays, twenty instrumented exact replays, and five
instrumented full-package replays did not reproduce it. A later complete
instrumented workspace run passed. The collector has no retry, so the original
red result is not normalized away. The unresolved ordering observation is
tracked as `TC-PLAYER-003`.

### Complete strict legacy fanout

The proposed broad compatibility profile was tested before narrowing it:

```powershell
$env:SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY = "1"
$env:SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY = "1"
cargo test --locked -p sorotte-compat --all-features -- --nocapture
```

The durable replay completed in 88.98 seconds:

```text
129 passed; 6 failed; 9 ignored; 0 filtered out
```

The local stdout log was 22,000 bytes with SHA-256
`e3e3514efc553296f02f25d21dfca4839d304841e20b5ba36615c6a1b2042b59`.
The stderr log was 4,519 bytes with SHA-256
`4d8bc3a688368c50518c1119cf087e1a40a8c49f5016ffbbd80f569feba677d4`.
The six failures are:

| Finding | Scenario | Observed divergence |
|---|---|---|
| `TC-COMPAT-001` | username conflict | legacy `alice_` user output had no Rust match at step 1 |
| `TC-COMPAT-002` | persistent-room lifecycle | Rust emitted an additional `playlistIndex(0)` at step 1 |
| `TC-COMPAT-003` | controlled-room permissions | Rust emitted `playlistIndex(0)` to both clients at step 7, producing four outputs instead of two |
| `TC-COMPAT-004` | permanent-rooms file | Rust emitted an additional `playlistIndex(0)` at step 1 |
| `TC-COMPAT-005` | persistent-room timeout list updates | legacy connection aborted with Windows error 10053 |
| `TC-COMPAT-006` | state periodic timeout | legacy connection aborted with Windows error 10053 |

These are existing executable parity tests, not new expected-failure
characterizations. The required coverage profile therefore uses the already
required strict live-TLS selector only. That exact lane proves four passed,
zero failed, zero ignored, and 140 filtered tests; it does not imply full
fanout parity.

## Successful end-to-end attestation

The final collector run independently validated this report:

```text
target/verification/coverage-profile-lanes.json
```

| Lane | Result | Duration | Profiles before | Profiles after | Fresh delta |
|---|---:|---:|---:|---:|---:|
| workspace all features | pass | 180.969s | 0 | 34 | 34 |
| GUI semantic | 14/14 | 8.613s | 34 | 35 | 1 |
| strict live TLS | 4/4 | 1.101s | 35 | 36 | 1 |
| LLVM merge check | pass | 1.598s | 36 | 36 | 0 |

Every row attested `profile_removed_count=0`; the merge row also attested zero
raw-profile content changes.

The workspace stdout/stderr digests were
`8b59e0ad3cbdf59668c9a68c502cdf2b53250f3c2e84fa27418cf82b8728d166`
and
`d949402df6244a11dd2270bd5f0c396444abb6b4e4c218a0013eeb83e636cee1`.
The semantic stdout digest was
`a7dff340e31a92f50d611f35ec373a2862f05f20a44ef165ec84162f907ec54a`.
The strict TLS stdout/stderr digests were
`865eb879f0ee29d60969c4f46aa8efc8b964eefdbb4e104619e40c48352603f9`
and
`19b7d0ce6709186cacd5f8d333fdf76228f67c0d8cc6188795a585d1bb0789e8`.
The schema-2 report is 16,012 bytes with SHA-256
`41dc1a4fd3bad5a06a87823aebc4e658faf9006ce3c4378b7f3b989605fbd349`.

The final `cargo llvm-cov report --summary-only` merge reported:

```text
TOTAL regions:   184641 / 239055 = 77.24%
TOTAL functions: 11611 / 15070  = 77.05%
TOTAL lines:     148209 / 190067 = 77.98%
```

The percentage is diagnostic. The required pull-request policy continues to
use the source-bound unique changed-line map and its separate ordinary and
critical thresholds.

Both downstream producer views were then exported from the same 36 profiles
and accepted by `scripts/llvm_cov_line_map.py`. The canonical map contains 395
files and 145,016 of 183,712 covered unique physical lines (78.936596%); it
retains the LLVM 148,209 of 190,067 line-instance result (77.977240%)
separately.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| LLVM JSON | 13,602,319 | `b1745db8437e98142200cd12481dee2473b293f41458f63778b51a4d40ea943f` |
| LLVM native text | 13,666,763 | `a2ee83518c11f5095805a70d9354887db46da6dbd4f7ef111d1f60db63feb434` |
| source-bound line map | 8,987,670 | `492995ccbc2130853d724d44fd7524ce276c9aaaf97e5c46947ab13aec2721d3` |

The complete Python infrastructure and workflow-policy suite passed all 284
tests in 11.046 seconds after the schema-2 reset and finalizer binding were
added.

## Reproduction

Provision the pinned producer, legacy Python requirements, and exact reference
checkout, then run:

```powershell
$env:SYNCPLAY_LEGACY_ROOT = ".interop-cache/syncplay-legacy"
python scripts/coverage_profile_lanes.py run `
  --repo-root . `
  --output target/verification/coverage-profile-lanes.json
python scripts/coverage_profile_lanes.py validate `
  --report target/verification/coverage-profile-lanes.json
```

The report is replaced with a failed document when setup, execution,
instrumentation, oracle validation, log hashing, or profile merging fails.
Coverage export is allowed only after the report validates.
