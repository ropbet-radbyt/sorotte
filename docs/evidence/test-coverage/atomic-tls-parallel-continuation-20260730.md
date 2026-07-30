# Atomic TLS and parallel continuation evidence

Date: 2026-07-30

Branch: `codex/test-coverage-design`

Starting commit: `cebc2145e367aa6f9a2bdabd4a5e6c99976490d8`

Platform: Windows, Rust 1.97.1 workspace toolchain

## Objective

This continuation resolves `TC-SERVER-004` completely and advances the
application-wide strategy with qualitatively different transport, process,
parser, reset, persistence, GUI-pump, and archive-consumer tests. Newly
surfaced product defects remain exact expected-failure characterizations; this
slice does not silently normalize or repair them.

## Result matrix

| Stream | Mechanical boundary | Stress / breadth | Result |
|---|---|---:|---|
| atomic TLS reader | strict selector, immutable generation, member authentication, selector recheck, loose double-capture | 9 focused real-filesystem/runtime tests; every member-read switch boundary | `TC-SERVER-004` resolved |
| SWAG TLS publisher | real shell execution with source lineage and selector-replace fault injection | 3 integration cases across successive, mixed, and interrupted publication | green |
| CLI reconnect acknowledgement | production connected-session loop over real TCP generations with State/Ping causal barriers | 4 tests, 20 stress runs, 200 loopback sessions | green |
| client reset completeness | exhaustive projection against a fresh semantic reference plus stale effect completion | 24 seeds; 50 stress runs, 2,400 generated reset cases | `TC-CLIENT-002` |
| server persistence arbitration | service-local scan/arbitration/transaction checkpoints and capacity-one queues | 5 tests; 20 complete actor runs, 340 executions | green |
| updater interruption | real updater child killed while holding the production transaction lock | 11 durable boundaries; 110 terminations and 220 recovery child re-entries | green |
| protocol raw input | production public decoders over escapes, truncation, UTF-8, malformed strings, depth, duplicates, and diagnostics | 8 tests, 654 cases; 100 repetitions, 65,400 case executions | `TC-PROTOCOL-002`, `TC-PROTOCOL-003` |
| media-tool process faults | real exit-zero/nonzero, malformed output, large output, hang, kill/reap, and executable release | 7 tests; 50 repetitions, 350 executions | `TC-GUI-001`, `TC-GUI-002` |
| player IPC process faults | real external child, Windows named pipe, large stdio/IPC, truncation, exit, hang, and handle release | 4 focused tests; 50 stress iterations | green |
| threaded GUI refresh | real owner pump, poisoned legacy getters, contradictory responses, ACK replay, and joined shutdown | 2 tests; 25 repetitions, 50 executions | green |
| server archive consumer | fresh extraction and exact packaged binary with isolated state, live protocol session, Ctrl-C drain, and post-run reauthentication | 33 Python tests; 10 real package executions | green |

## Atomic TLS publication

### Reader contract

The preferred TLS root is:

```text
tls/
  current.json
  generations/
    <generation>/
      privkey.pem
      cert.pem
      chain.pem
```

`current.json` has the closed schema `sorotte-tls-bundle-v1`. It contains one
path-safe generation ID and exact byte length/lowercase SHA-256 identity for
all three required members. The production reader:

1. requires ordinary, non-link/reparse root, generation, selector, and member
   paths;
2. rejects unknown or duplicate manifest fields, path traversal, oversized
   data, noncanonical hashes, and member drift;
3. captures and parses the selector;
4. reads and authenticates only the named immutable generation;
5. rereads the selector and retries if it changed during capture; and
6. passes the exact authenticated bytes to rustls.

The selector-switch test changes `current.json` after member reads 1, 2, and 3.
Every schedule retries and returns complete generation B; no fingerprint
matches a cross-generation combination. Partial and complete-but-unselected
generations remain invisible. A selected unavailable generation leaves the
active runtime context and fingerprint intact, consumes no invalid-generation
retry, and activates after the member becomes available.

When `current.json` is absent, legacy loose files require two identical
consecutive domain-separated captures. This rejects the two observed
mid-capture replacement schedules, but remains explicitly documented as race
reduction: a reader cannot prove the origin of a stable mixed loose directory.

### Publisher contract

`copy-swag-sorotte-certs.sh` no longer overwrites three live target paths. It:

1. resolves `cert.pem`, `chain.pem`, and `privkey.pem` to one Certbot archive
   directory and numeric lineage;
2. copies those immutable sources into a hidden generation staging directory;
3. records exact lengths and hashes, then rehashes the source paths;
4. renames the complete staged directory into `generations/`;
5. invokes the host durability boundary before selection;
6. writes, permissions, and atomically renames a temporary `current.json`; and
7. retains old generations while cleaning only unpublished temporary state.

The executable harness publishes A then B and proves A remains byte-for-byte
unchanged, rejects a mixed Certbot lineage before target creation, and injects
failure at the selector rename. That failure leaves the previous selector
byte-for-byte unchanged and removes temporary artifacts; a later ordinary run
converges on B.

This proves publication ordering, identity, interruption behavior, and reader
atomicity. It does not claim simulation of kernel power loss or storage-device
cache durability; those remain platform/filesystem work.

## Transport, reset, and worker schedules

The CLI continuation carries the client-core acknowledgement oracle through
real connected sessions. Ten logical connection generations prove:

- an emitted but unacknowledged playlist restore re-arms after disconnect;
- a matching echo retires old ownership and a newer local mutation becomes the
  next owner;
- divergent remote authority supersedes emitted ownership;
- capability disablement clears ownership without resurrection; and
- coalesced Hello/empty-playlist keys behave in both valid orders.

State/Ping responses are causal wire-drain barriers; no sleep or
quiet-period-as-absence assertion is used.

The reset oracle destructures every top-level client session/model aggregate,
seeds distinguishable mutable state, and compares reconnect reset with a fresh
reference across 24 seeds. It surfaced `TC-CLIENT-002`: both pause reducer
transactions and degraded local-pause health survive reset. Completing the
stale pre-disconnect position effect sets the replacement session to 391.0 and
emits a pause; later stale pause completions also mutate new-session state.

Server persistence uses service-local callbacks rather than a global
failpoint. The new schedules cover six stale/new/equal save/delete pairs, two
rooms through a capacity-one coalesced wake, replacement after arbitration but
before commit, concurrent replacement of failed work with exact
degraded/recovered events, and stats overflow before transaction entry.

The updater child is terminated at journal header, first/final prepare,
first/middle/final replacement, commit, first/middle/final cleanup, and journal
removal. Every case authenticates the durable journal decision and starts two
independent recovery children. Recovery reaches one complete old/new state,
releases the transaction lock and image handles, is idempotent, and leaves no
transaction/helper artifacts.

## Parser and process defects

The raw protocol matrix surfaced:

- `TC-PROTOCOL-002`: the final duplicate top-level `Set` payload is paired
  with the first/discarded payload's nested execution order; and
- `TC-PROTOCOL-003`: `DecodedMessageLineItem` diagnostics render an untrusted
  credential-bearing unknown command name.

Adjacent positive tests prove the public decoding functions agree over 144
escaped command-key spellings, every character-prefix truncation, seven
malformed UTF-8 classes, 42 malformed escape/control cases, 257 nesting
depths, and typed invalid-payload redaction.

The media-tool child matrix surfaced:

- `TC-GUI-001`: exit-zero empty, invalid-UTF-8, or unrelated output is accepted
  as a healthy tool version; and
- `TC-GUI-002`: a finite 512 KiB stdout producer blocks on the undrained pipe
  and is falsely timed out.

Positive cases prove first-nonempty/unterminated parsing, exit status 23,
bounded timeout, kill/reap, and immediate executable-image release. The
findings ledger records lean and alternative repair designs, but this testing
slice deliberately leaves product behavior unchanged.

The player-mpv process suite independently proves the production IPC worker
drains 256 KiB on each child stdio stream while receiving a fragmented 512 KiB
unsolicited frame, emits one terminal failure/disconnect pair for truncated
response and exit 23, times out a hung request at the configured boundary, and
releases the executable and named-pipe handles after kill/reap.

The GUI owner-pump suite poisons every legacy lifecycle getter and separately
injects contradictory legacy payloads plus a first-ACK failure. Ordered
projection remains atomic, the exact token replays and recovers, no stale
legacy path reaches the shell, and explicit shutdown joins within a bounded
barrier.

## Exact server archive consumption

The server verifier now executes the exact freshly extracted,
manifest-authenticated binary rather than stopping at `--version` and startup.
It removes every inherited `SOROTTE_*` override, uses separate fresh
working/config/state roots, requires a loopback protocol Hello, holds a live
session across production Ctrl-C handling, requires the drain-barrier log and
exit zero within five seconds, checks both SQLite databases, verifies the
binary is unchanged, and reauthenticates the complete package after execution.

Adversarial tests add valid-checksum corrupt/truncated archives, missing
executable, nested extra payload, non-fresh extraction, exact binary-path
plumbing, environment isolation, clean signal, and forced timeout/reap.

## Current registered defects

The fail-closed registry contains exactly five defects and five executable
characterizations:

1. `TC-CLIENT-002` — reconnect reset retains in-flight reducer transactions;
2. `TC-PROTOCOL-002` — duplicate top-level `Set` uses discarded payload order;
3. `TC-PROTOCOL-003` — decoded-item diagnostics expose a credential-bearing
   unknown command;
4. `TC-GUI-001` — version probe accepts unusable successful output; and
5. `TC-GUI-002` — version probe deadlocks on finite output larger than pipe
   capacity.

`TC-SERVER-004` is absent from the registry because its former expected
failure is now ordinary positive regression coverage.

## Integrated validation

The final combined tree passed:

```text
cargo fmt --all -- --check
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python -m unittest discover -s scripts/tests -p "test_*.py" -v
actionlint
python scripts/behavior_evidence.py validate --catalog coverage/behaviors.toml
python scripts/ignored_test_policy.py validate --registry coverage/ignored-tests.toml
python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml \
  --catalog coverage/behaviors.toml \
  --repo-root .
sh -n scripts/copy-swag-sorotte-certs.sh
git diff --check
```

Measured results:

- locked all-feature workspace, including doctests: 268.1 seconds, exit 0;
- warning-denied all-target/all-feature workspace Clippy: 17.0 seconds;
- Python policy/artifact suite: 364/364 in 18.294 seconds;
- publisher integration: 3/3;
- behavior catalog: 20 behaviors, 51 exact proofs, 2 lanes;
- ignored-test registry: 23 exact tests;
- known-defect registry: 5 defects, 5 characterizations;
- actionlint, shell syntax, formatting, and whitespace checks: no findings.
