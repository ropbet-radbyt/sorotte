# Critical boundaries and dependency evidence

The v0.2.9 policy tracks authority, private persistence, process ownership,
untrusted input, and bounded resources. `coverage/critical-boundaries.toml`
names those responsibilities and their Rust module roots. The checker follows
production module declarations, including platform modules and `#[path]`
extractions, and requires every descendant to keep a critical coverage rule
with the same owner. The existing changed-line policy still requires 90% for
critical code and still unions the immutable base and head policies.

Run `python scripts/critical_boundaries.py --repo-root .` after moving a module.
An extraction into an unlisted directory fails this check. Test modules remain
outside the production inventory. This catalog complements review: a wholly
new responsibility needs an explicit root and owner.

## Mutation selection

`coverage/mutation-selection.toml` declares dependency edges between changes
and existing shards. A production or test change selects its package's shards;
feature manifests, selectors, tools, and Cargo.lock are accounted for. Markdown
alone selects none. Relevant changes retain the historical ten-shard
participant-status report set, and scheduled/manual runs select all shards.
The new origin, duplicate-settings, local-clock, and resource-permit shards
require a baseline and 100% viable kills with no misses or timeouts.

The workflow computes the union from immutable base and head Git revisions.
The aggregate recomputes that union independently, requires exactly one report
for every selected shard, and invokes the existing source/test-inventory-bound
verifier. It does not accept a producer's selection list as authority. The
original fixed required report set remains an additional check. A removed
base-selected shard fails instead of silently shrinking the requirement.

Use `python scripts/mutation_ci.py run --repo-root . --policy
coverage/mutation-policy.toml --shard SHARD --results-root target/mutation-ci/SHARD
--output target/verification/mutation-SHARD.json` (one command line). Source
movement must update shard files and exact accepted-unviable identities in the
same change. Compiler failures require actual campaign evidence; they are not
counted as kills. Any later source change requires fresh candidate evidence.

## Dependency gates and release inventory

`deny.toml` checks the locked Rust graph, including build dependencies and all
features, with cargo-deny **0.20.2**. Only crates.io registry sources are allowed;
unapproved registries and Git sources fail. Each scan requires an online
RustSec fetch, then scans offline against the recorded clean database commit.
The report records checker version and executable digest, database URL,
commit, fetch time, raw output digest, effective policy, and input digests.
A missing scanner, failed fetch, incomplete report, or changed input produces
**unavailable** evidence and fails the gate.

Python verification requirements are resolved transitively and scanned with
pip-audit **2.10.1**. The PyPI vulnerability service has no exposed immutable
database revision; reports therefore record that limitation, query time,
resolved versions, and exact response digests. They do not claim RustSec-style
database identity. `coverage/dependency-policy.toml` contains no exceptions.
Any future exception must have an exact advisory identity, rationale, owner,
and expiry; expired entries fail before scanning. Dependabot proposes small
weekly Cargo/Python patch groups and monthly action updates, with no automatic
merge or bypass of compatibility gates.

Run `python scripts/dependency_policy.py scan --ecosystem all --output
target/dependencies/report.json` after installing the pinned scanners. Failed
and unavailable reports are preserved by the workflow, not converted to clean
results. Tool behavior follows the primary [cargo-deny check
documentation](https://embarkstudios.github.io/cargo-deny/checks/index.html),
[advisory policy](https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html),
and [pip-audit documentation](https://github.com/pypa/pip-audit).

GUI/server archives from v0.2.9 include `DEPENDENCIES.json` and
`THIRD-PARTY-NOTICES.txt`. Packaging derives the default-feature normal/build
graph for the exact package and target from locked Cargo inputs and captures
upstream notices. The independent archive verifier hashes actual payload
members, checks the inventory's payload binding and expected target, and
retains the existing executable/package checks. The dependency graph describes
build inputs; it is not a claim that every build dependency is linked into the
binary. Notice collection is an inventory, not a legal compliance conclusion.

`coverage/native-components.toml` separately identifies vendored Lua, SQLite,
ring native code, external media tools, and the container's base image. RustSec
is not asserted to audit independently installed executables or all vendored
C code. The existing server-container Syft SPDX inventory and image-bound
Cosign signatures remain the container evidence contract; archive inventory
does not introduce another signing system.

## Implementation evidence and limits

The live pre-remediation Rust scan found five errors: event-listener 5.4.1
(`RUSTSEC-2026-0221`), quick-xml 0.39.4 (`RUSTSEC-2026-0194` and `0195`),
rustls-pemfile 2.2.0 (`RUSTSEC-2025-0134`), and webbrowser 1.2.1
(`RUSTSEC-2026-0257`). Updating compatible releases and replacing the retired
PEM parser with rustls-pki-types removed those findings without exceptions.
The initial clean scan used RustSec commit
`5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5` (2 September 2026). A separate
scanner-only metadata fixture with an unapproved Git source failed with
`source-not-allowed`; no vulnerable dependency was inserted for a test.

The live Python baseline found advisories in Twisted 25.5.0, pyOpenSSL 25.3.0,
and cryptography 46.0.7. The pins are now Twisted 26.4.0, pyOpenSSL 26.4.0,
cryptography 50.0.1, and service_identity 24.2.0; the live follow-up had no
findings. Pinned Syncplay commit
`d1c5f85af377c960c5a940707c4d01bc84fd9c3f` remains unchanged. Its certificate
display code calls methods removed by modern pyOpenSSL. The local transport
fixture supplies a read-only view backed by cryptography.x509 for those
display methods; Python's normal verified TLS context, CA validation, and
hostname checks still perform authentication. The upstream protocol is not
patched and certificate errors are not bypassed.

Earlier shared-target mutation counts are invalid evidence, including the
previous 10/10 origin and 12/12 duplicate-settings claims in this document.
Independent scratch runs exposed additional settings-section insertion and
resource-ceiling survivors. The implementation now covers those behaviors;
the [0.2.9 ledger](../audits/v0.2.9-implementation.md) identifies the executed
campaigns and their source-bound reports. Two Rust let-chain `&&` to `||`
mutations and two resource-owner `Default` replacements have exact, expiring
accepted-unviable records backed by compiler failures. An unchanged-size byte
reservation explicitly avoids unnecessary atomic updates; equivalent
zero-change accounting is not counted as a killed behavior.

The mutation process-status parser accepts nonzero signed 32-bit failures,
matching the pinned [cargo-mutants process producer](https://github.com/sourcefrog/cargo-mutants/blob/v27.1.0/src/process.rs).
That includes Windows structured-exception exit codes. Zero, booleans,
non-integers, out-of-range statuses, and contradictory outcome records remain
rejected. A platform-status parse error never becomes a passing attestation.

The implementation campaigns also exposed inherited absolute
`CARGO_TARGET_DIR` sharing between concurrent mutation workers. The wrapper
now forces target and intermediate build paths to `target` relative to each
scratch workspace, including both Cargo environment aliases. This follows
[Cargo's build-directory configuration](https://doc.rust-lang.org/cargo/reference/config.html#buildtarget-dir)
and preserves [cargo-mutants' per-worker build isolation](https://mutants.rs/parallelism.html).
Counts from campaigns before this correction are invalid and must not be
used as release evidence; isolated reruns replace them.
