# Reproducible release qualification

`package-ci.yml` supplies the independent `package-required` merge check on every
PR and main push. It builds and consumes Linux/Windows server archives and the
Windows GUI archive, including updater replacement, success and rollback. It has
no dependency on publication or interactive release authorization. This prevents
the main approval graph from waiting on a publication job that itself needs main
approval.

Publication requires the dedicated Administration-read GitHub App described in
[protection reader setup](PROTECTION_READER_SETUP.md). The normal workflow token
continues to query checks and artifacts. Only protection authorization receives a
short-lived repository-scoped App token; package and native candidate jobs receive
no App credentials. An absent App configuration fails closed.

Stable tags (`v*` and `server-v*`) enter `stable-release.yml`. Individual GUI,
server and container workflows are reusable consumers; they do not each launch
another stable lifecycle campaign. The orchestrator:

1. Requires the exact protected main source and every trusted required check via
   `merge_gate.py authorize-release`. A tag, artifact file, or successful lifecycle
   alone grants no publication authority.
2. Runs one Linux/isolated Windows lifecycle pair and prepares server behavior in
   parallel. Each platform seals its release binaries and optional PDBs into a
   closed bundle. All existing real-player, terminal, missing-file, recovery,
   start-gate and second-client native obligations remain required.
3. Validates the complete cross-platform receipt, downloads the sealed binaries,
   constructs GUI/server archives, and consumes their actual runtime boundaries.
   An extra check compares archive binary hashes with the lifecycle bundle hashes.
4. Independently rechecks authorization before publication, attaches immutable
   archives and sidecars, and compares every published byte through anonymous
   release URLs. Existing assets with different bytes are never overwritten.
5. Attaches a deterministic `sorotte-qualification-<sha>.zip` and sidecar to the
   release. This preserves the compact build/lifecycle/default-workspace/source
   receipts beyond the Actions retention window. Native raw logs remain private;
   their diagnostic projection is distinct from a passing qualification.

Version 2 bundle manifests record exact source files, Cargo inputs, compiler/Cargo/Python
binary hashes, target, default features, release profile, absence of
instrumentation, channel/ref, runner image and OS, resolved OS/Python packages,
media tool binaries, the Windows native driver, and producer run/attempt. The
binary digests identify the actual tested bytes. Source equality or a green run
with the same SHA is insufficient. Legacy bundles without the input closure cannot
authorize reuse. Dev and stable refs/channels retain separate
qualifications. Failed jobs may be retried within the same Actions run while
successful platform jobs and their immutable artifacts are retained.

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

Run `coordinated stable release` manually at the approved main source with
`publish=false` for full native/build/behavior/archive/container qualification
without uploading releases, authenticating to GHCR or signing/pushing an image.
It still requires a provisioned trusted isolated Windows worker. It is not a
shortcut around merge prerequisites. Publication requires a version tag.

Use Actions **Re-run failed jobs** after diagnosing the recorded primary failure.
This retains successful lifecycle producers and the same qualified bundles. A
missing/expired artifact is an error, never permission to find a different green
run. Re-running every job after assets were published can produce a new build
manifest; immutable publication will reject conflicting assets. Preserve the
original successful package/publication artifacts when retrying publication.

## Container identity and latest promotion

The container uses its pinned Debian build/runtime images and retains its own
actual-image protocol, TLS, persistence, non-root, shutdown, SBOM, signature and
anonymous registry checks. Its binary is a distinct build from the host Linux
archive; the shared lifecycle prerequisite is not a claim of container binary
identity. Build timestamps use the source commit time rather than retry time.

The keyless certificate SAN identifies the reusable signer
`publish-server-container.yml@refs/tags/<version>` because Fulcio uses
`job_workflow_ref`. The separately verified Actions run must still originate in
`stable-release.yml`, and source/workflow SHA claims must match the approved source.
The [Fulcio identity contract](https://github.com/sigstore/fulcio/blob/main/docs/oidc.md#github)
distinguishes the reusable signer from the calling workflow.

The `publish sorotte-server container` manual dispatch now only promotes an
existing publication. Select the approved release tag and supply:

- `publication_run_id`: the explicit successful `coordinated stable release` run;
- `approved_digest`: the registry manifest digest from its final gate;
- `version_tag`: the existing version tag from that run.

The consumer verifies repository, source, workflow, event, tag, conclusion and
attempt through the Actions API before downloading evidence. It reruns live
Cosign and anonymous tag/config/layer/SBOM verification, copies only that digest
to `latest`, and then repeats the complete public comparison. It neither rebuilds
the image nor reruns lifecycle qualification. The version, full-SHA and latest
tags must all retain the approved registry digest. The manifest copy disables
automatic index conversion with Docker's
[`--prefer-index=false`](https://docs.docker.com/reference/cli/docker/buildx/imagetools/create/)
and independently checks the resulting digest.

## Evidence limits

The harness tests exercise changed/missing bundles, different source/channel/
profile/features, foreign or incomplete producers, runtime-skipped archives,
wrong published bytes, and failed checks before promotion. Local tests and
workflow validation do not establish a fresh Windows/Linux lifecycle pass or a
successful registry promotion. A hosted dry run and isolated native execution
are still the authorities for those boundaries; no release is published merely
by implementing this apparatus.
