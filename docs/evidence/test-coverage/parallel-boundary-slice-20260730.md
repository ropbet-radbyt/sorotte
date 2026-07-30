# Parallel boundary-coverage slice — 2026-07-30

Branch: `codex/test-coverage-design`  
Implementation commit: `9ccce00d9997f20d1845f3166a7e58e37cf22a2e`  
Primary experiment platform: Windows x86_64  
Rust toolchain: 1.97.1  

## Scope

Four file-disjoint workstreams ran in parallel:

1. generated and adversarial protocol-codec properties;
2. request-reactive bidirectional mpv IPC faults;
3. exact-child CLI player-process supervision;
4. immutable server-release archive consumption.

The delegated agents were prohibited from editing the shared behavior catalog,
defect registry, findings, or strategy, and from staging or committing. The
primary integration pass reviewed their diffs, separated one positive test
from a newly discovered defect oracle, registered all expected failures, and
ran the combined repository gates.

No surfaced product bug was fixed in this slice. Product findings remain
executable `should_panic(expected = ...)` characterizations and cannot count as
positive behavior evidence.

## 1. Generated protocol codec

The protocol crate now has four shrinkable Proptest properties:

- arbitrary byte-derived strings remain total across every public decoding
  entrypoint and preserve cross-entrypoint success/failure relationships;
- arbitrary bounded recursive JSON matches independent structural and
  compact/pretty-whitespace oracles;
- every supported envelope roundtrips through wire JSON and preserves command,
  message-kind, and Hello-extraction identity;
- generated duplicate composites preserve first command position and final
  payload value.

Seven deterministic specifications cover the configured generation budget,
generator vocabulary, malformed JSON versus malformed envelope boundaries,
line-size and recursion boundaries, duplicate top-level commands, collapsed
nested `Set` payloads, and mixed supported/unknown composites. A minimized
generated composite is retained in
`crates/sorotte-protocol/proptest-regressions/property_tests.txt`.

Stress used `PROPTEST_CASES=10000`. All four properties passed, totaling 40,000
generated cases in 6.63 seconds. The final package suite passed 77/77 tests and
warning-denied all-target Clippy.

### Surfaced finding: TC-PROTOCOL-001

Duplicate nested `Set` members collapse to their final payload values but
retain every occurrence in `command_order`. Server normalization consumes each
field once; client normalization can clone the collapsed payload once per
retained order entry. The same decoded line can therefore execute
differently.

The exact characterization is:

```text
property_tests::known_defect_duplicate_set_members_retain_collapsed_execution_entries
```

with oracle:

```text
collapsed duplicate Set members must appear once in command order
```

The compatible future fix is first-position/final-value deduplication for
nested command order. Rejecting all duplicate JSON keys is safer but requires
an explicit compatibility decision.

## 2. Bidirectional mpv IPC faults

The new request-reactive transport drives the production IPC client, command
serializer, buffered frame reader, worker, actor, and nonblocking completion
boundary. It does not pre-seed a one-way response tape: each peer action is
created only after observing and validating the corresponding request.

The exhaustive model enumerates 343 three-step histories and 1,029 attempted
transitions over:

- byte-at-a-time split success;
- coalesced event/event/response success;
- recoverable server rejection;
- stale duplicate response;
- future response before the matching response;
- read half-close;
- write disconnect.

Its independent oracle checks consecutive request IDs, exact request
correlation, newline framing, event order, at-most-once harvesting, connection
health after recoverable errors, one terminal transition after fatal errors,
and absence of later writes.

A condition-variable-gated delayed response separately proves that a second
nonblocking request cannot overtake, events precede their correlated
completion, token and command identity survive delay, items are harvested
once, and the released connection remains usable. A withheld response proves
one production timeout, one disconnect, bounded completion, fast terminal
reuse, and no later write.

Validation:

- focused suite: 3/3;
- complete player-mpv library: 410 passed, 2 opt-in real-mpv tests ignored;
- strict Clippy and formatting: passed;
- exhaustive model stress: 50/50 runs, 17,150 histories and 51,450 transitions;
- delayed ordering stress: 50/50;
- withheld deadline stress: 25/25.

No product defect surfaced. Kernel named-pipe fragmentation/partial writes,
request-ID rollover, real-mpv process faults, and attachment replacement remain
outside this transport-boundary harness.

## 3. CLI process supervision

The process tests spawn the exact Rust test executable in a selected child
role and coordinate through filesystem barriers. They do not identify success
by process name, arbitrary sleep, or an unrelated child.

Positive proofs cover:

- missing managed binary rejection before ownership is created;
- contextual external spawn failure with redacted arguments;
- exact early-exit status handoff;
- unmanaged lifetime transfer without parent-side termination;
- child stdout/stderr containment;
- managed guard kill, wait, reap, and IPC artifact removal behind a bounded
  parent channel;
- idempotent cleanup after the child was already reaped.

The review pass removed an inherited-stdin assertion from the positive
stdout/stderr test. That assertion described the product defect and would have
made the positive proof fail after the defect was repaired; stdin is now owned
only by its expected-failure characterization.

Validation:

- managed repeated stress: 100/100;
- external repeated stress: 260/260;
- final CLI library: 346 passed with 8 pre-existing ignored tests;
- CLI integration: 2/2;
- strict Clippy, formatting, and diff checks: passed.

### Surfaced finding: TC-CLI-001

Managed attachment does not observe an owned child exiting during IPC polling,
so it waits through the complete connection deadline. The exact
characterization configures a 300 ms production deadline around an
immediately exiting child and requires an early result:

```text
mpv_startup::managed_process::process_supervision_tests::
known_defect_managed_attach_waits_full_deadline_after_child_exit
```

Oracle:

```text
managed attach must stop retrying when its child exits
```

The proportional fix is `Child::try_wait` before each retry wait, returning
the observed exit status while retaining the current guard cleanup.

### Surfaced finding: TC-CLI-002

Unmanaged external launch nulls stdout and stderr but inherits the CLI stdin
handle. A nested coordinator proves the child receives a parent stdin token.

Exact characterization:

```text
tests::mpv_startup::external_launch::
known_defect_external_launch_inherits_cli_stdin
```

Oracle:

```text
external launch must not inherit the CLI stdin handle
```

The lean complete fix is `.stdin(Stdio::null())` on both managed and unmanaged
player commands.

## 4. Immutable server release artifact

`package-server-release.ps1` now writes `manifest.json` inside the primary
archive. It binds:

- schema version;
- package version;
- platform and architecture;
- exact 40-character source commit SHA;
- path, byte size, and SHA-256 for the executable, README, server release
  guide, and license.

The independent Python consumer locates exactly one primary archive, verifies
the canonical adjacent checksum, and rejects:

- missing, duplicate, stale, or unexpected upload-directory entries;
- absolute, drive-qualified, traversal, backslash, non-normalized, duplicate,
  or case-colliding member paths;
- ZIP encryption, symlinks, and special file types;
- TAR symlinks, hardlinks, sparse files, devices, and FIFOs;
- archive, per-file, and expanded-size bound violations;
- missing, extra, or empty payload files;
- duplicate JSON keys or an open-ended manifest schema;
- version, platform, architecture, source SHA, size, or digest drift;
- invalid optional symbols archive/checksum shape;
- an archive that changes while being consumed.

Extraction is member-by-member into a fresh temporary directory; neither
`extractall` nor an archive-provided destination is used. The exact extracted
binary must report the packaged version, bind a loopback-only server, accept a
protocol Hello, and return a server Hello before bounded termination. Success
and failure reports are written atomically. The release workflow pins all
actions to reviewed commits, performs this consumer step after packaging and
before upload, uploads the verification report even on failure, and uploads
the same artifact-directory files only after verification succeeds.

The synthetic suite has 26 tests across positive ZIP/TAR/symbols cases and
checksum, path, type, inventory, schema, provenance, digest, size, report, and
workflow-policy adversaries.

### Clean-commit real-package experiment

The implementation commit was clean before production packaging. The packaging
script ran its own locked release build and produced the normal Windows
archive plus optional symbols archive. The consumer then extracted and
executed those exact bytes.

```text
source SHA:
  9ccce00d9997f20d1845f3166a7e58e37cf22a2e

primary archive:
  sorotte-server-0.2.4-windows-x86_64.zip
  4,137,875 bytes
  sha256 ab30a88a9ec3f35164b012ab31f1eac9b03b2f241bd13995f572ce4aac526210

checksum file sha256:
  580b257aa220c4bd262e05fa21f29a4b306620b8472d6fcc42d1a56b57c71050

manifest sha256:
  094592247069f9916ca6c58502746f179c6926264e7a954010a3b3a67cf6a702

extracted binary:
  9,441,280 bytes
  sha256 6f26636a38a493b54fc0333a62386412bdf7721eb087e869b5abf46d3162b5e9
  version output "sorotte-server 0.2.4"

symbols archive sha256:
  59226365fcc9f1790c8d5ae42779e8914e498d740d5d9212710934fb6caba31a

runtime:
  loopback-only = true
  protocol Hello received = true
  elapsed = 286 ms
```

The machine-readable report remains under ignored `target/server-release` for
the local run; release CI uploads the equivalent current-run report.

This proof is deliberately outside `coverage/behaviors.toml`: the current
evidence runner accepts exact Rust tests and GUI semantic scenarios only, while
the artifact consumer is a tag/manual release gate on both Linux and Windows.
Its ordering and immutable upload path are nevertheless checked by the Python
policy suite. Pretending it was a Rust lifecycle proof would weaken the
catalog's type boundary.

## Integrated policy and validation

Three positive behavior IDs and eleven exact Rust proofs were added:

- `NET-CODEC-001` — four generated codec proofs;
- `PL-IPC-003` — exhaustive model, gated delay, and terminal deadline;
- `PL-PROC-001` — managed cleanup, already-exited cleanup, unmanaged
  ownership, and spawn redaction.

The catalog validates at 20 behaviors and 51 proofs. The known-defect registry
validates at five defects and seven exact characterizations, including the
three findings from this slice.

Final combined commands:

```text
cargo fmt --all --check
python -m unittest discover -s scripts/tests -p "test_*.py" -v
python scripts/behavior_evidence.py validate --catalog coverage/behaviors.toml
python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml \
  --catalog coverage/behaviors.toml \
  --repo-root .
powershell -File scripts/package-path-boundary-tests.ps1
powershell -File scripts/release-publication-policy-tests.ps1
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Outcomes:

- formatting: passed;
- complete Python policy/harness suite: 321/321, including 26/26 focused
  artifact tests;
- behavior catalog and known-defect registry: passed;
- Windows package path/reparse/hardlink suite: passed;
- publication policy suite: passed;
- complete locked all-feature Rust workspace, including release verification:
  passed in 209.4 seconds;
- warning-denied all-target/all-feature workspace Clippy: passed in 8.9
  seconds.

The pinned `actionlint` command could not run locally because Go is not
installed. This is an environment limitation, not a green result. The Python
CI-policy suite independently passed the release workflow's action pin,
ordering, source-SHA, report, and upload-path assertions; the normal Linux CI
job still runs pinned actionlint.

## Remaining boundaries

This slice does not claim:

- raw socket-byte fuzzing below the protocol crate's `&str` API;
- kernel named-pipe fragmentation or partial-write injection;
- simultaneous in-flight IPC response reorder, which production currently
  prevents by permitting one in-flight command;
- managed attachment success without the existing real-mpv smoke;
- OS hard-kill, kill-permission, power-loss, filesystem-cache, or disk-full
  semantics;
- GUI archive, server container, SBOM, provenance signature, or public
  registry/release digest verification;
- fixes for TC-PROTOCOL-001, TC-CLI-001, or TC-CLI-002.
