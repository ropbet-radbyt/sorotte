# Server container build/load/publication contract — 2026-07-31

## Scope and outcome

This slice replaces the former one-step `build-push-action` publication path
with a fail-closed, build-once consumer chain for the `sorotte-server`
container. It is bounded to the repository's own server image, a test-owned
Docker container and bind-mounted state directories, loopback protocol/TLS
traffic, and the configured GHCR repository. It does not add a general image
publisher.

The offline policy/unit suite is green. No image was built, loaded, run,
pushed, signed, attested, or queried from GHCR on this Windows host because
Docker, Syft, and Cosign are not installed. Those capability-bearing phases
remain CI-owned and are deliberately not reported as executed here.

## Defect closed in policy

The previous workflow sent a fresh BuildKit result directly to GHCR with
`push: true`. It did not:

- load and consume the image that would be published;
- prove the real non-root entrypoint, a protocol-accepted persistence write,
  graceful stop/restart restoration, TLS protocol Hello/drain, or exact
  SQLite state/integrity;
- generate or attest an SBOM;
- keylessly sign the resulting registry digest;
- compare authenticated push results with anonymous public GHCR tag/digest
  results; or
- pin its actions, Dockerfile frontend, or base-image identities immutably.

The new workflow has one `docker/build-push-action` invocation with
`load: true` and `push: false`. Registry login occurs only after local runtime
smoke and SBOM verification pass. The publication helper can only tag and push
the already-inspected daemon image; it contains no image-build operation.

## Identity chain

The final gate keeps distinct identities and cross-binds them:

1. `github.sha` is recorded in the exact full-SHA tag and OCI
   `org.opencontainers.image.revision` label.
2. Docker's local image ID is treated as the image-config digest. Local
   inspection also records the ordered RootFS diff IDs, source label,
   non-root user, entrypoint, command, OS, and architecture.
3. Both test-owned container scenarios use that local tag:
   - plaintext loopback writer and watcher sessions synchronize acceptance of
     a distinctive two-file playlist, index, and paused position through real
     fanout; SIGINT then drains both live clients, the exact nine-column raw
     `persistent_rooms` row and `PRAGMA integrity_check` are inspected, and a
     second container from the same loaded tag and bind-mounted state
     directory must restore Hello, playlist, index, and periodic playstate
     before another graceful drain;
   - plaintext STARTTLS negotiation requires the exact `send` request and
     `true` acknowledgement at a clean framing boundary, then the
     certificate-authenticated TLS loopback performs a bounded real Hello and
     live-session drain. A duplicate TLS restart would add no persistence
     identity proof beyond the plaintext restart.
   In each stop, the verifier sends `SIGINT` directly to the image entrypoint,
   requires both `docker wait` and the inspected container state to report a
   stopped, non-dead exit with code zero, no daemon error, and no OOM kill, and
   proves EOF on every still-open client. The plaintext phase additionally
   checks SQLite integrity and exact restoration from the same loaded image
   after restart. Those process, transport, actor,
   and persistence outcomes are the graceful-shutdown proof. Retained Docker
   logs remain required diagnostics: every started container must retain the
   startup listener record, and every scenario that completes its SIGINT stop
   must also retain the graceful-shutdown record. A failure before the stop
   boundary preserves its primary error while still reporting any separate log
   capture or container-removal failure.
4. The commit-pinned `anchore/sbom-action` invokes Syft 1.44.0 with the exact
   local test tag. Syft's SPDX 2.3 projection does not provide a guaranteed,
   stable Docker config-ID and RootFS identity field for this verifier to
   require. The contract therefore binds the controlled action input between
   two daemon inspections: after SBOM generation, the verifier re-inspects
   the exact runtime-report tag and requires the complete config ID, source
   URL/SHA, ordered RootFS diff IDs, platform, entrypoint, user, command, and
   created-label identity to equal the pre-SBOM runtime inspection. Duplicate
   JSON keys, SPDX structure, the exact `Tool: syft-1.44.0` creator, package
   presence, and the exact SBOM byte hash are also recorded before registry
   login.
5. Every pushed tag must report the same registry manifest digest. A divergent
   or missing digest stops publication.
6. Cosign 3.0.6 signs that exact `image@sha256:...` and attests the exact SPDX
   predicate. Verification is constrained to the exact workflow URI, GitHub
   OIDC issuer, repository claim, source SHA claim, and signed source/workflow
   source annotations.
7. After `docker logout`, the verifier obtains an anonymous pull token and
   queries each tag plus the digest reference through the GHCR Distribution
   API. Manifest and config bytes are rehashed. All references must return the
   same manifest bytes/digest; the manifest config digest must equal the
   tested local image ID; config labels, runtime contract, ordered layer
   inventory, and RootFS diff IDs must still match.
8. The always-run final gate rejects a missing, skipped, failed, stale, or
   cross-identity-divergent runtime, SBOM, push, signature, attestation, or
   public-verification phase.

Anonymous manifest/config reads use six bounded attempts with exponential
backoff for registry eventual consistency. Authentication failures,
non-public packages, non-image manifests, malformed/duplicate-key JSON,
unexpected schemas, and exhausted retries fail closed.

Registry logout is itself `always()` so credentials are removed after any
post-login failure. The anonymous comparison is explicitly `success()`, while
the final gate remains `always()` and cannot pass without every closed report.
Evidence upload also runs always and treats a missing artifact path as an
error rather than silently warning.

## Immutable inputs

The workflow action pins are full commits:

- `actions/checkout` v7.0.1:
  `3d3c42e5aac5ba805825da76410c181273ba90b1`
- `docker/setup-buildx-action` v3.11.1:
  `e468171a9de216ec08956ac3ada2f0791b6bd435`
- `docker/metadata-action` v5.9.0:
  `318604b99e75e41977312d83839a89be02ca4893`
- `docker/build-push-action` v6.19.0:
  `ee4ca427a2f43b6a16632044ca514c076267da23`
- `anchore/sbom-action` v0.24.0:
  `e22c389904149dbc22b58101806040fa8d37a610`
- `docker/login-action` v3.6.0:
  `5e57cd118135c172c3672efd75eb46360885c0ef`
- `sigstore/cosign-installer` v4.1.2:
  `6f9f17788090df1f26f669e9d70d6ae9567deba6`
- `actions/upload-artifact` v7.0.0:
  `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a`

The Dockerfile frontend and both base images are pinned by OCI index digest:

- `docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89`
- `rust:1.97.1-bookworm@sha256:77fac8b98f9f46062bb680b6d25d5bcaabfc400143952ebc572e924bcbedc3fa`
- `debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818`

The initial pins were resolved from the official action repositories and
container registries on 2026-07-31; the Node 24 checkout and artifact pins were
refreshed on 2026-08-01. Updating a dependency requires an intentional pin
change and rerun of this contract.

## Offline validation executed

```text
python -m unittest scripts.tests.test_server_container_verification -v
  Ran 34 tests
  OK

python -m py_compile scripts/verify_server_container.py \
  scripts/tests/test_server_container_verification.py
  PASS

actionlint -config-file .github/actionlint.yaml \
  .github/workflows/publish-server-container.yml
  PASS (actionlint 1.7.12)

git diff --check -- .github/workflows/publish-server-container.yml \
  Dockerfile.server
  PASS

git diff --no-index --check -- NUL <each new Python/evidence file>
  no whitespace diagnostics
  exit 1 is expected because each comparison contains added content
```

The 34 tests cover closed/duplicate-free report schemas, exact tag/source
identity, local image/config inspection, entrypoint and label drift, the
required accepted/restarted/restored plaintext state and raw SQLite row
evidence, bounded TLS Hello/drain evidence, SPDX/Syft structure, post-SBOM
daemon reinspection, substituted config/source/RootFS rejection, substituted
valid-SBOM byte rejection, build-free push behavior, multi-tag digest
agreement, bounded public-registry retries, manifest/config rehash and
cross-binding, Cosign signature annotations, attestation subject and exact
SPDX predicate binding, all-phase final-gate rejection, immutable action/base
pins, duplicate/extra runtime-scenario rejection, exact pinned-Syft creator
rejection, fail-closed logout/upload conditions, and workflow step ordering.

`ruff` and `black` were not installed, so their checks were not run. Python
compilation and the repository's unittest conventions were used instead.

## Hosted Docker shutdown-proof follow-up — 2026-08-01

Manual publication run
[`30690430335`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30690430335)
and its retry reported a missing final shutdown text line before registry
login. A first correction retried the post-exit `docker logs` snapshot, but run
[`30692117813`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30692117813)
proved that the line remained absent across 20 snapshots. The log assertion
ran from `finally`, so it masked the earlier scenario error that prevented the
test from ever sending SIGINT.

Run
[`30693330321`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30693330321)
made startup-log capture sufficient only for a pre-stop failure and exposed
the primary defect: the smoke harness requested 28- to 30-character usernames
while the server's production default is 16 characters. Sorotte correctly
canonicalized those identities, but the join matcher waited for the impossible
untruncated watcher name until its bounded deadline.

The identity correction uses distinctive usernames within the production limit,
rejects an overlong test identity before opening a socket, and requires the
Hello response to echo the exact requested username and room. Cleanup now
preserves the primary scenario failure and appends any independent diagnostic
or removal failure. The shutdown log marker remains mandatory after a
completed SIGINT stop, together with the direct clean-state, session-drain,
SQLite-integrity, and same-image restart evidence.

Exact-merge publication run
[`30694770477`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30694770477)
then completed the plaintext write, clean SIGINT stop, and raw persistence
inspection, but the same-image restart failed because the host inspection
opened the WAL-mode database with SQLite `mode=ro`. SQLite created `-wal` and
`-shm` files in the bind-mounted live state as the GitHub runner user. The
non-root image user (UID 10001) could not write that host-owned shared-memory
file, so schema initialization correctly failed with `attempt to write a
readonly database`.

The final correction opens the already-stopped database with SQLite
`mode=ro&immutable=1`. This retains exact row and integrity inspection without
creating or changing WAL sidecars in the state directory that the second
container must reuse. A WAL-mode regression now proves both generic integrity
and exact-row inspection leave `-wal` and `-shm` absent.

Publication run
[`30695636736`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30695636736)
then proved both plaintext container lifecycles and reached the TLS scenario.
It failed with `SSL: WRONG_VERSION_NUMBER` because the smoke client attempted
direct TLS on Sorotte's plaintext STARTTLS listener; the retained server log
showed the TLS ClientHello being correctly rejected as an invalid plaintext
protocol line.

The STARTTLS correction now sends the canonical
`{"TLS":{"startTLS":"send"}}` plaintext frame, requires the exact string
`{"TLS":{"startTLS":"true"}}` acknowledgement with no buffered bytes beyond
its line boundary, and only then starts the certificate-verified TLS handshake.
The closed runtime report records `startTls: true` in addition to the cipher,
TLS version, and peer-certificate digest. All five failed runs skipped registry
login, push, signing, and attestation, so none could mutate GHCR.

Exact-merge publication run
[`30696424760`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30696424760)
then passed both plaintext lifecycles, the certificate-verified STARTTLS
scenario, SBOM generation and binding, registry login, and the exact tested
image push. It recorded both
`sha-9042f944d10b5506587c2427832839180dc02240` and `latest` at
`sha256:a19de4aa20beaf213876ebae7d4d94644756f27b37795c4660a0bd62ad351f64`.
Unlike the preceding fail-before-login runs, this run did mutate GHCR. It did
not complete publication assurance: pinned Cosign 3.0.6 rejected the singular
`cosign sign --annotation` option before creating a signature or attestation,
and the always-run final gate rejected the missing downstream reports.

Cosign 3.0.6 exposes the long option as `--annotations` for both `sign` and
`verify`. The workflow now uses that exact interface at both boundaries, and
the policy regression requires both signed claims while rejecting the invalid
singular spelling. A later all-green run remains required to supersede the
unsigned publication and satisfy the completion criteria below.

Publication run
[`30697387744`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30697387744)
proved the corrected Cosign 3 interface: build, both real runtime boundaries,
STARTTLS, SBOM binding, push, keyless signature and SPDX attestation creation,
and exact certificate/annotation verification all passed. It pushed
`sha-776212abba6f336bee634130b836c912aaf130d6` and `latest` at
`sha256:aea3984f49e2a8d6d328b6318481790ee7ce5320cacd8d57c8a41f7e5bb9847c`.
The anonymous cross-binding phase still failed before its registry queries
because Cosign 3 represents the verified signature's `docker-reference` as
the exact digest-qualified `image@sha256:...`, while the evidence parser only
accepted Cosign's legacy bare-image identity.

The parser now accepts precisely those two canonical verified forms: the
expected bare image or that same image qualified by the already-validated
expected digest. It also requires either the legacy Cosign signature type or
the Cosign 3 signature URI, and regressions reject a tag, foreign repository,
wrong digest, or unknown signature type. The attestation emitted by the same
run already matched the existing exact image, digest, and SPDX-predicate
contract. The final gate correctly remained failed until a later run can
complete anonymous tag/config comparison and produce every passed report.

Follow-up local validation:

```text
python -m unittest scripts.tests.test_server_container_verification -v
  Ran 47 tests
  OK

python -m unittest discover scripts/tests -v
  Ran 561 tests
  OK

cargo test -p sorotte-server --all-features --locked
  369 tests passed

python -m py_compile scripts/verify_server_container.py \
  scripts/tests/test_server_container_verification.py
cargo fmt --all --check
git diff --check
  PASS

verify-publication and final-gate replay over the retained run 30697387744
signature, attestation, SBOM, runtime, and push reports plus anonymous GHCR
  PASS; all three public references resolved to the recorded digest and the
  reconstructed final gate passed
```

## CI-owned completion criteria

A successful run of `.github/workflows/publish-server-container.yml` is
required before claiming container publication assurance. Every such run must
produce and retain:

- plaintext write and restart logs, the TLS log, both SQLite state sets, and
  `runtime-report.json` with accepted/restored state plus both exact raw room
  rows;
- `sbom.spdx.json` and `sbom-report.json`;
- `publish-report.json` naming one digest for every intended tag;
- Cosign signature and SPDX-attestation verification JSON;
- anonymous `publication-report.json`; and
- a passed `final-gate-report.json`.

The GHCR package must be anonymously pullable or the public comparison fails.
The runtime lane currently proves only `linux/amd64`, and its TLS fixture proves
the server TLS boundary rather than public-PKI issuance. No local Docker build
or run, Syft generation, Cosign signing, or signature-transparency result is
claimed by this evidence note; those phases remain CI-owned. The supplementary
anonymous GHCR replay consumes retained CI evidence and does not replace the
required all-green hosted workflow run.
