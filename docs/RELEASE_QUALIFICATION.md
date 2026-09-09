# Reproducible release qualification

`package-ci.yml` supplies the `package-required` merge check on every PR.
It builds and consumes Linux/Windows server archives and the
Windows GUI archive. The GUI runtime check exercises updater replacement and
rollback when its environment supports installation; an elevated process instead
verifies launch and updater refusal before mutation. Its report records which
path ran. Before this required check can pass, the same exact PR head must also
complete the full nonpublishing stable candidate campaign. Neither campaign
depends on publication permission. See [the PR qualification contract](PR_QUALIFICATION.md).

The required native PR job builds default-feature release binaries and exercises
the same Windows playback qualification action used below, including real-mpv
HTTP faults/stalls, process recovery and second-client status. A failed or skipped
suite blocks `native-required`. This brings those regressions before merge while
preserving maintainer-authorized, isolated desktop execution. PR-head evidence
remains bound to the tested head. Main verifies that its ordinary two-parent
merge incorporated that exact head and base without changing the tree; the
release tag names the original qualified head. The merge SHA is recorded
separately and is never substituted into binary provenance.

Publication requires the dedicated Administration-read GitHub App described in
[protection reader setup](PROTECTION_READER_SETUP.md). The normal workflow token
continues to query checks and artifacts. Only protection authorization receives a
short-lived repository-scoped App token; package and native candidate jobs receive
no App credentials. An absent App configuration fails closed.

Both PR candidate dispatches and stable tags (`v*` and `server-v*`) enter
`stable-release.yml`, through disjoint job graphs. Before merge, the orchestrator:

1. Requires a maintainer-dispatched, open, up-to-date repository PR whose head is
   the exact workflow source. No publication or protection-reader credentials
   are supplied to candidate jobs.
2. Runs one Linux/isolated Windows lifecycle pair and prepares server behavior in
   parallel. Each platform seals its release binaries and optional PDBs into a
   closed bundle. All existing real-player, terminal, missing-file, recovery,
   start-gate and second-client native obligations remain required.
3. Validates the complete cross-platform receipt, downloads the sealed binaries,
   constructs GUI/server archives, and consumes their actual runtime boundaries.
   An extra check compares archive binary hashes with the lifecycle bundle hashes.
4. Exports the tested container, proves cold restoration, and uploads all release
   artifacts. A separate worker downloads the actual uploads using the same
   byte/digest validation as publication, collects the archives and restores the
   image while it is absent locally. Handoff failure blocks candidate completion.
5. Seals the immutable artifact IDs/digests, original producer run/attempt, PR
   head/base and policy in the candidate manifest. It retains a deterministic
   `sorotte-qualification-<sha>.zip` and sidecar containing the compact
   build/lifecycle/default-workspace/source receipts. Native raw logs stay private.

After merge, `main-qualification.yml` checks the exact PR, merge parents, equal
tree, seven trusted PR checks and candidate artifact identities. Its
`main-qualified` result performs no application execution. Tag publication then
rechecks current main protection, this trusted main producer, PR authority and
candidate artifacts through `candidate_authority.py authorize-release`.
`publish-qualified-archives.yml` attaches the original archives and compares every
public byte; `publish-server-container.yml` restores, pushes and signs the exact
saved image. Neither rebuilds nor reruns application tests. Conflicting existing
assets are never overwritten. The durable qualification archive preserves the
original pre-merge source authorization; publication authorization is retained
separately. A tag or local JSON file alone grants no publication authority.

Version 2 bundle manifests record exact source files, Cargo inputs, compiler/Cargo/Python
binary hashes, target, default features, release profile, absence of
instrumentation, channel/ref, runner image and OS, resolved OS/Python packages,
media tool binaries, the Windows native driver, and producer run/attempt. The
binary digests identify the actual tested bytes. Source equality or a green run
with the same SHA is insufficient. Legacy bundles without the input closure cannot
authorize reuse. Dev and stable refs/channels retain separate
qualifications. Failed jobs may be retried within the same Actions run while
successful platform jobs and their immutable artifacts are retained.

The repository's `.gitattributes` fixes text checkouts to LF across Linux,
hosted Windows and the isolated native runner. Image files and the retained
fuzz corpora explicitly disable text conversion. Release input records hash
actual checked-out bytes; the same commit with different working-tree line
endings cannot authorize bundle reuse. The checkout regression exercises both
Git `core.autocrlf` settings and preserves framing and binary corpus bytes.
An input mismatch reports the affected path count and up to ten names, without
logging file contents; the original qualified input inventory remains in the bundle.
The workspace receipt checks cleanliness before and after executing tests. A
dirty checkout reports its status-entry count and up to ten entries without file
contents. Passing test assertions cannot certify a changed checkout. Tests must
own and remove their temporary configuration directories, including cache locks
created by constructors and other indirect operations.

## Prepare the release version

Before committing or qualifying a release candidate, update
`[workspace.package].version` in `Cargo.toml` and the corresponding workspace/path
package versions in both `Cargo.lock` and `fuzz/Cargo.lock`. Preserve all
third-party dependency versions and checksums; the private `sorotte-fuzz` harness
keeps its independent version.

Update `[release]` in `coverage/current-architecture.toml` with the same version
and the exact previously released base commit. Leave fixing and hosted evidence
explicitly pending until recorded; retain older boundary-local results and
remaining-work notes as historical evidence. Regenerate the index and validate
the complete preparation before pushing or starting native qualification:

```powershell
python scripts/architecture_index.py --write
cargo metadata --locked --format-version 1
cargo metadata --locked --manifest-path fuzz/Cargo.toml --format-version 1
python scripts/verify.py preflight --phase static --output target/verification/release-version-preflight-attempt-1.json
```

Use a fresh output path for every attempt and preserve failures. Locked metadata
validates dependency inputs; the full static preflight also checks version-bound
catalogs and generated documentation. After committing, require fresh exact-source
hosted and native qualification for the release candidate.

## Local and coordinated server stages

The supported standalone default remains full verification:

```powershell
./scripts/server-release-verify.ps1
```

Preparation validates the configured or bootstrapped legacy checkout against
`coverage/verification-tools.toml`, requires a clean tree and Python imports, and
does not compile. Behavior independently revalidates preparation:

```powershell
./scripts/server-release-verify.ps1 -Stage Prepare
./scripts/server-release-verify.ps1 -Stage Behavior
./scripts/package-server-release.ps1
python scripts/verify_server_release_artifact.py --artifacts-dir target/server-release/artifacts --expected-source-sha <full-sha> --report target/server-release/artifact-verification.json
```

Coordinated qualification calls `release_qualification.py workspace` to execute
and receipt `cargo test --locked --workspace` once per platform. The behavior
stage accepts `-WorkspaceReceipt <file> -ReceiptRunId <run-id>` only for the same
source, compiler, platform, default features, test profile, ordinary
instrumentation and trusted producer. All-features receipts cannot suppress this
obligation. Package-only server/compatibility tests remain separate because
workspace feature unification can change their dependency inputs. Live Python
interop, Clippy and the dedicated server release matrix also remain required.
`-NoWorkspace` retains its explicit specialist-use behavior; release orchestration
uses the validated receipt instead.

Archive construction accepts `-QualifiedBundle <dir> -QualificationReceipt <file>`
with the qualification run identity. It validates the complete input closure
before copying binaries and disables rebuilding. The ordinary standalone
packagers still build by default. Every Cargo build/test/Clippy path uses locked
dependency resolution.

## Dry run and retry boundaries

Update the PR branch to include current main, then dispatch its exact reviewed
branch while the PR is open:

```powershell
gh workflow run stable-release.yml --ref <reviewed-branch> -f publish=false -f pull_request_number=<number>
```

This runs full native/build/behavior/archive/container and delivery qualification
without publishing. Provision the trusted isolated Windows worker for that run.
The ordinary native-required campaign and this candidate campaign both belong
to the PR phase. Required checks reject an outdated base or different source.
After a normal two-parent merge and successful `main-qualified`, create the
version tag at the qualified PR head. Its tree must equal current main.

Use Actions **Re-run failed jobs** after diagnosing the recorded primary failure.
During candidate qualification this retains successful lifecycle producers and
the same qualified bundles. The final candidate manifest records the current
attempt while container evidence retains its actual producing attempt. A
missing/expired artifact is an error, never permission to find a different green
run. If a completed artifact would conflict with an immutable upload on a retry,
start a new nonpublishing candidate dispatch while the PR is still open. Never
rebuild after merge to repair qualification. Publication retries use the original
successful candidate artifacts; they only repeat authorization, delivery and
public verification. Missing or expired candidate artifacts require a new PR
qualification, not a tag-time application run.

## Container identity and latest promotion

The container uses its pinned Debian build/runtime images and retains its own
actual-image protocol, TLS, persistence, non-root, shutdown, SBOM, signature and
anonymous registry checks. Its binary is a distinct build from the host Linux
archive; the shared lifecycle prerequisite is not a claim of container binary
identity. Build timestamps use the source commit time rather than retry time.
The metadata step reads the requested commit's Unix timestamp and renders it as
UTC with a `Z` suffix. The image verifier still rejects offset timestamps and
other noncanonical values. The actual workflow command is tested against Git
commits with different timezone offsets and a missing source revision.

The keyless certificate SAN identifies the reusable signer
`publish-server-container.yml@refs/tags/<version>` because Fulcio uses
`job_workflow_ref`. The separately verified Actions run must still originate in
`stable-release.yml`, and source/workflow SHA claims must match the approved source.
The [Fulcio identity contract](https://github.com/sigstore/fulcio/blob/main/docs/oidc.md#github)
distinguishes the reusable signer from the calling workflow.

The `publish sorotte-server container` manual dispatch now only promotes an
existing publication. Select qualified `main` as the workflow ref and supply:

- `publication_run_id`: the explicit successful `coordinated stable release` run;
- `approved_digest`: the registry manifest digest from its final gate;
- `version_tag`: the current latest stable release tag from that run.

`container_promotion.py` authenticates the tool revision as the exact current
protected `main`, including its trusted `main-qualified` check. It separately resolves
the published source from the explicit original producer, requires an annotated
tag naming that source, and verifies its original trusted PR checks and unchanged
merge. Older published releases retain their historical main-push check contract. The
published source must be an ancestor of the tool revision and its tag must still
be GitHub's latest stable release. This permits a reviewed promotion-tool repair
without changing the published binaries, tag or original signing identity.

The initial authorization selects the exact artifact before download. After
fresh signature and registry checks, a callback repeats the authority checks
immediately before assigning `latest` and rejects changes to the release, tag,
source, operator or producer identity. Both authorizations and their original
API evidence are retained. These are fresh observations; a saved receipt alone
cannot authorize the mutation. The original release/source certificate remains
separate from the recorded promotion workflow/source revision.

The consumer verifies repository, source, workflow, event, tag and conclusion
through the Actions API, then enumerates every job execution in that explicit
run. It selects the latest actual container job and its exact artifact ID; the
container's producing attempt can precede the workflow's final attempt when only
a later attachment job was retried. A newer failed container execution, ambiguous
producer or missing artifact is an error, never permission to use an older green
execution. Promotion evidence retains the original API responses and
both attempt identities. It reruns live
Cosign and anonymous tag/config/layer/SBOM verification, copies only that digest
to `latest`, and then repeats the complete public comparison. It neither rebuilds
the image nor reruns lifecycle qualification. The version, full-SHA and latest
tags must all retain the approved registry digest. The manifest copy disables
automatic index conversion with Docker's
[`--prefer-index=false`](https://docs.docker.com/reference/cli/docker/buildx/imagetools/create/)
and independently checks the resulting digest.

Subprocess regressions exercise separate stdout and stderr through actual
processes, including Cosign-style diagnostic banners, malformed output and
nonzero exits. The strict parsers consume stdout only; diagnostics remain
available separately. The promotion path also verifies tag/push ordering and
rejects a failed final authorization before any manifest assignment. These
checks run in PR validation; they use local tool fixtures and do not publish.

## Evidence limits

The harness tests exercise changed/missing bundles, different source/channel/
profile/features, foreign or incomplete producers, runtime-skipped archives,
wrong published bytes, and failed checks before promotion. Local tests and
workflow validation do not establish a fresh Windows/Linux lifecycle pass or a
successful registry promotion. A hosted dry run and isolated native execution
are still the authorities for those boundaries; no release is published merely
by implementing this apparatus.
