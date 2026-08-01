# Pro-review remediation — 2026-08-01

## Result

The four review findings are implemented as focused, ordinary positive
regressions. `TC-CLI-004` and `TC-CLI-005` were reopened before their fixes and
the executable known-defect registry is explicitly empty again. The loose TLS
compatibility reader is bounded without breaking Certbot-style member links,
the reusable production one-shot protocol reader no longer exists, and the
latest-wins persistence claim is limited to live-service convergence rather
than synchronous durability acknowledgement.

The previous hosted result at
`612917ac8461040549217453bdebfc5001f2378c` predates these changes and is not
merge evidence for this remediation. A fresh exact-head hosted matrix remains
required before the branch is review-ready.

## Defect lifecycle

The branch was clean at `e51e6a93363b43ffcf5188f66421b5bc5c98b1a4` when
the review was applied.

1. Commit `db4c929` reopened `TC-CLI-004` and `TC-CLI-005` and retained two
   exact expected-failure characterizations. The known-defect policy reported
   two defects and two matching characterizations.
2. Commit `cbdb870` implemented the CLI correction, converted both tests to
   ordinary positive regressions, and restored an explicitly empty registry.
3. Commit `efb8c33` bounded loose TLS reads and removed the production
   one-shot protocol-reader wrapper.

The registry transition is intentional evidence. A passing expected failure
was never reported as correct product behavior, and the final source contains
no expected-failure characterization for either defect.

## CLI grammar and privacy

The first 256-case composition campaign independently modeled precedence but
rendered attached short options as `-x=value`. The actual pinned argparse
grammar also accepts a value as the immediate remainder of the short token and
clusters known boolean options. That omitted `-pSECRET`, `-aexample.org:8999`,
`-nAlice`, `-rroom`, `-dg`, and `-gd`; an equals-sign-only diagnostic redactor
could consequently retain and print the accepted `-pSECRET` token.

The production parser now walks each short token character by character:

- `h`, `v`, `d`, and `g` are no-value flags and can cluster;
- `a`, `n`, `r`, and `p` consume the remaining token, optionally after `=`,
  or use the next token according to their required/optional semantics;
- exact `-psn` and `-psn=VALUE` bind the legacy blackhole option, while
  `-psnVALUE` is parsed as `-p` with value `snVALUE`; and
- parser issues store option identity and a boolean attached-value marker, not
  the attached bytes.

The differential invokes the real pinned
`syncplay.ui.ConfigurationGetter.ConfigurationGetter.getConfiguration()` from
`.interop-cache/syncplay-legacy` at
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f`. It disables GUI prompting and
stubs configuration-path lookup/parsing, final checking, saving, and relative
configuration loading. Passwords are projected only as `<redacted>`. Twenty-five grammar
cases cover canonical attached/separated forms, clusters, optional missing
values, and `-p`/`-psn` precedence.

Three actual-process regressions provide a separate privacy and side-effect
boundary. They run accepted `-pCANARY`, `-p=CANARY`, and
`--password=CANARY` version requests, reject an unknown attached canary, and
scan stdout, stderr, and failure text. No canary may appear.

## Final endpoint validation

Host and optional port are now one atomic final occurrence. Specific
`HostArgumentError` variants distinguish an empty host, empty port,
nonnumeric port, out-of-range port, and malformed bracketed IPv6. A later
valid host replaces an earlier invalid occurrence; a malformed final host
replaces an earlier valid occurrence and fails after all configuration layers
compose but before settings persistence, updater/player startup, or network
connection. A valid final host without a port still inherits the lower-layer
port.

The endpoint differential invokes the pinned Python parser and final
validation boundary. Sorotte matches its accepted final-occurrence behavior
except for one documented stricter decision: pinned Python accepts
`[::1]:notaport` by retaining the prior/default port after its bracketed-port
conversion fails, while Sorotte rejects the explicit malformed port. Empty,
nonnumeric, out-of-range, and malformed unbracketed final endpoints otherwise
fail closed as specified by the review.

The actual-process endpoint regression supplies an absent configuration root
and a canary player path. A malformed final endpoint must exit without
creating the root and without reaching player-launch diagnostics.

## Bounded loose TLS compatibility

Manifest-selected immutable generations already rejected oversized members.
The loose compatibility path previously used unbounded `fs::read`. It now:

1. opens the member once, intentionally following a final symlink;
2. requires the opened target to be a regular file;
3. rejects pre-read metadata above 4 MiB;
4. reads at most 4 MiB plus one byte through that same handle;
5. rejects a plus-one result; and
6. rechecks opened-handle type and length before the existing second-capture
   stability comparison.

`loose_tls_snapshot_rejects_every_oversized_member_before_unbounded_allocation`
applies the 4 MiB + 1 boundary to all three required members. The Unix-only
`loose_tls_snapshot_follows_certbot_style_member_symlinks_with_the_same_bound`
builds a `live`/`archive` layout, follows each member link, and requires the
bounded snapshot to remain rustls-loadable. Loose mode remains static or
externally serialized compatibility; two matching captures cannot prove that
three independently published paths came from one generation.

## Stateful protocol-reader containment

The production `read_inbound_protocol_line` convenience function created a
fresh `InboundProtocolLineReader` on every call. It was used only for the
terminal STARTTLS response attempt, but leaving it reusable made recurrence of
the earlier cancellation-loss defect too easy. The wrapper is removed.
STARTTLS now owns a scoped reader explicitly: timeout drops the connection;
success retains the `BufReader` and any prefetched bytes as the transport.
Connected reusable paths continue to own one reader for their complete
session. A source-bound architecture regression rejects reintroduction of the
one-shot name in the connected-session implementation.

## Persistence wording

Room-persistence version arbitration prevents stale queued work from
overtaking newer work while the asynchronous worker continues making
progress. Enqueue is not a synchronous durable commit acknowledgement. A
newer effect can arrive after an older transaction's last currency check; a
process exit before the newer effect is applied or explicitly flushed may
leave the preceding committed state. The central coverage README, strategy,
findings, and the original atomic-TLS continuation evidence now state this
boundary explicitly.

## Local validation

The following remediation gates passed on Windows with Rust 1.97.1:

```powershell
cargo test -p sorotte-cli --all-features
cargo clippy -p sorotte-cli --all-targets --all-features -- -D warnings
cargo test -p sorotte-server --all-features
cargo clippy -p sorotte-server --all-targets --all-features -- -D warnings
python scripts/known_defect_policy.py validate `
  --registry coverage/known-defects.toml `
  --catalog coverage/behaviors.toml
python scripts/behavior_evidence.py validate --catalog coverage/behaviors.toml
```

The CLI run passed 376 library tests with eight registered ignores, two
application-boundary integration tests, and three actual-process privacy/
side-effect tests. The server run passed 369 library tests plus 14, two, and
six integration tests; the six-test release verifier included its strict
99-second legacy-peer case. Both warning-denied Clippy gates passed. The known
defect policy reported zero defects and zero characterizations. The expanded
behavior catalog validates at 21 behaviors, 56 exact proofs, and two lanes.

## Clean documentation-bearing head validation

The documentation-bearing implementation head `e23c4f2` passed:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` in
  9.6 seconds;
- `cargo test --workspace --all-features` in 225.4 seconds, with every test
  binary and doctest successful and only the 19 applicable registered ignores
  skipped;
- all 545 Python policy and evidence-infrastructure tests in 27.736 seconds;
- the exact ignored-test, known-defect, and behavior-catalog validators;
- all 14 GUI semantic scenarios;
- local UIA-only native smoke, including the five-item AccessKit menu
  inventory and File -> Exit lifecycle, with zero desktop-input attempts; and
- `scripts/server-release-verify.ps1 -NoWorkspace` in 256 seconds. Only the
  already-passed workspace run was deduplicated; the verifier passed its
  server/compat suites, 21/21 strict live-legacy tests, Clippy, and all six
  serial release scenarios. Its reports are under
  `target/server-release-verify/`.

After every build-producing gate, four mutually exclusive real-mpv contracts
ran against GUI SHA-256
`8ea1ce25575a2aee7329502f0c6ffe630f97a750f225357b8ed508f4b50ea600`
and installed mpv SHA-256
`2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0`
(`mpv v0.41.0-877-ge5486b96d`):

| Contract | Result | Local retained bundle |
|---|---:|---|
| healthy GUI-to-mpv | 13 assertions / 10 artifacts | `target/verification/gui-real-mpv-vertical/20260801T042922947Z-43764` |
| automatic owned-process replacement | 20 assertions / 13 artifacts | `target/verification/gui-real-mpv-owned-process-recovery/20260801T042948946Z-53968` |
| malformed loopback-HTTP recovery | 18 assertions / 11 artifacts | `target/verification/gui-real-mpv-faulting-http-recovery/20260801T043024020Z-21384` |
| valid byte-silent stalled HTTP, run last | 18 assertions / 11 artifacts | `target/verification/gui-real-mpv-stalled-http/20260801T043059675Z-16320` |

The final stalled response remained open and byte-silent for 29,225 ms before
one same-process recovery GET; the player and IPC identities remained stable,
the recovery body completed, and native Exit reaped the owned player and
released the loopback server.

Fresh hosted acceptance remains a separate exact-head publication boundary;
earlier hosted runs are not reused as acceptance for this change.

## Hosted diagnostics and source-mapping correction

Manual workflow run `30684423737` was bound to exact head
`c314374f4919dbd7414e34b6be59fae632a0af8b` and explicit base
`f3964ebc7f7b281b9b78f3bfb243ff65e5122e33`. Every behavior-producing job
passed on attempt 1: Linux and Windows all-feature tests, lifecycle, semantic,
complete pinned-Python compatibility, generated Media Match, minimum/newest
source-built mpv, Windows package checks, both coverage producers, both strict
server verifiers, and the Windows behavior aggregate. The schedule-only
nightly job was the sole expected skip.

Coverage-diff job `91328082388` retained a real fail-closed diagnostic. Its
two source-bound maps measured 82.53% combined, 80.51% ordinary, and 90.79%
critical coverage, but one ordinary changed Rust line had no LLVM mapping:

```text
crates/sorotte-cli/src/client_args/types.rs:78
attached_value_present: true,
```

The containing redaction arm executed and its neighboring lines were covered;
LLVM emitted no region for the second field of the multiline match pattern.
The verification aggregate failed only because coverage-diff is required. The
downloaded evidence is retained locally under
`target/hosted/30684423737/coverage-diff/`.

Commit `a09698f54b48b48e731949ca9566062b8ae528cf` keeps the same structural
privacy behavior but makes the attached/unattached choice an explicit boolean
branch. The behavior proof now asserts both diagnostics. A focused
cargo-llvm-cov 0.8.4 replay maps the formerly absent field line with 18 hits,
the attached redaction line with 17 hits, and the unattached identity line
with one hit.

A second fresh workflow, run `30685217448` at exact head
`d35c3f06036ddf2c7237395ce18e853326d50ec6`, confirmed the original field
was mapped. Every behavioral and platform producer again passed, including
both coverage producers, Windows and Linux all-feature tests, both strict
server verifiers, compatibility, semantic, lifecycle, packaging, and both mpv
lanes. Coverage-diff job `91330310651` then retained one remaining unmapped
line:

```text
crates/sorotte-cli/src/client_args/types.rs:78
..
```

Rustfmt had placed the multiline match rest pattern on its own line. The
conservative scanner correctly refused to treat that punctuation-only line as
non-executable without a canonical source map. The downloaded evidence is
retained locally under `target/hosted/30685217448/coverage-diff/`.

Commit `d51562a6d101dee3be571446926c535ca33b34fc` replaces the multiline
unknown-option match fields with a dedicated secret-free payload and removes
both mapping ambiguities. The complete CLI suite and warning-denied all-target
Clippy remain green. A fresh cargo-llvm-cov 0.8.4 replay through the committed
policy classified 42 changed Rust lines, found 18 coverable and 15 covered
lines, and reported zero unmapped lines (83.33% focused coverage). The three
uncovered lines are ordinary coverable branches, not absent source mappings.
`TC-HARNESS-049` records both diagnostics; the unmapped-line policy was not
weakened, bypassed, or given a formatter waiver.

Exact-head run `30685859358` at
`cca386a256f9e6493f0587f85de64a501a18c003` then passed all 16 required jobs
on attempt 1. The schedule-only nightly lane was the sole expected skip.
Coverage-diff passed at 82.49% combined, 80.47% ordinary, and 90.79% critical
coverage with zero unmapped lines, and the required aggregate passed all 21
catalog behaviors. The successful coverage and aggregate artifacts are
retained locally under `target/hosted/30685859358/`.

## PR-triggered fuzz bootstrap diagnostic

Opening the ready PR triggered protocol-fuzz run `30686290291` at the same
exact head. Jobs `91332682574`, `91332682583`, and `91332682597` all failed in
the installer step before compiling a fuzz target. The pinned
`taiki-e/install-action` v2.85.2 revision did not recognize the independently
pinned `cargo-fuzz@0.13.2`; its disabled fallback correctly made that mismatch
fatal. The missing-artifact errors followed from the same early bootstrap
failure.

Commit `b0ae982dab9d0d361d4caf46d95bef686fa6ecd6` removes the unsupported
installer only from the fuzz jobs and uses the dated nightly Cargo directly:

```text
cargo +nightly-2026-07-29 install cargo-fuzz --version 0.13.2 --locked
```

The subsequent exact runtime-version check remains required. The executable
workflow policy rejects any installer action or action inputs and binds the
toolchain, crate version, and lock constraint in all three jobs. Actionlint,
20 protocol-fuzz policy tests, and 19 central CI-policy tests pass locally.
`TC-HARNESS-050` retains the diagnostic; fresh PR checks and a fresh exact-head
matrix remain required after this workflow-only correction.

## Safety and scope

Generated framed and argument input is processed only in memory. The Python
parser probe uses the local pinned checkout with side effects replaced by test
stubs. Process tests use version/error exits or fail before startup and verify
that no configuration directory or player launch occurs. TLS tests use only
nonce-owned temporary files. No public network target, reconnaissance,
credentials, persistence mechanism, privilege operation, or exploitation is
involved.
