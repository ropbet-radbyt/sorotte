# Documentation-inclusive hosted closure and CI critical-path evidence — 2026-08-01

Date: 2026-08-01<br>
Branch: `codex/test-coverage-design`<br>
Accepted workflow-bearing head: `612917ac8461040549217453bdebfc5001f2378c`

## Status

The branch's documentation-inclusive implementation and workflow checkpoint is
hosted-green. Workflow run
[`30679354953`](https://github.com/ropbet-radbyt/sorotte/actions/runs/30679354953)
finished successfully at the exact accepted head after GitHub's failed-job
rerun mechanism repeated one Windows server-release job. The final check suite
contained 16 successful checks and the expected schedule-only `nightly-deep`
skip, zero annotations, and nine nonexpired evidence artifacts.

This closes the former “documentation-inclusive hosted acceptance pending”
publication boundary for the coverage implementation and evidence present at
`612917a`. The product-defect registry remains explicitly empty. This result
does not execute or promote the separately documented interactive-runner,
native-Windows mpv endpoint, privileged block-replay, or public-container
capabilities.

## Checkpoint sequence

| Checkpoint | Exact source | Hosted run | Result |
|---|---|---|---|
| Corrected two-map implementation | `dd3012c1bcefa0a68520b063c5ae06f3e1b96f79` | `30639113884` | Every required producer, the 83.03% combined / 80.92% ordinary / 90.79% critical zero-unmapped finalizer, and the aggregate passed. |
| Pre-optimization full matrix | `67525e0969db84a58f715ab0e87846a9370b8aed` | `30674012574` | Passed; 55m07 elapsed included 21m37 queued and 33m30 executing. |
| Parallel required producers | `05926fc123d1acd207bbbdb5600e1da59495e57f` | `30677728038` | Passed in 19m33 with unchanged public required-check identities and evidence contracts. |
| Deduplicated verifier and Node 24 actions | `612917ac8461040549217453bdebfc5001f2378c` | `30679354953` | Final conclusion passed after one failed-job rerun; zero annotations and nine retained artifacts. |

## Critical-path result

The baseline run serialized a 24m33 Windows behavior job, an 8m38 Linux
coverage/diff job, and the final aggregate. Commit `05926fc` split Windows
nextest/doctests, release/package checks, and process coverage into independent
workers, started Linux coverage immediately, and reduced `coverage-diff` to a
small two-artifact policy consumer. The first complete parallel run reduced
observed execution from 33m30 to 19m33 without dropping a command, artifact,
source binding, finalizer check, or public aggregate.

That run made `server-release-verify (windows-latest)` the critical-path
outlier at 19m28. Commit `db3ee47` changed only the CI invocation to
`server-release-verify.ps1 -NoWorkspace`. The standalone script still runs its
full default matrix, while CI skips the duplicate workspace test that the
required locked all-feature Linux and Windows workers already execute. On the
accepted head, the successful Windows verifier rerun took 10m49. This is an
observed result rather than a duration guarantee; the strict server tests,
live compatibility, Clippy, packaging, and release-consumer checks all remain.

## Retained failed attempt

Attempt 1 of `30679354953` ended after 12m28 because
`release_verify_real_python_clients_against_rust_binary` timed out waiting for
the requested playlist state from the legacy Python peer and reported
`observed=[]`. The following TLS test failed only because the shared test lock
was poisoned by that first panic. Every other required producer and the
aggregate had passed. No source or workflow change was made between attempts;
GitHub reran the failed Windows job, and its complete strict matrix passed in
attempt 2. The failed observation remains part of the run history rather than
being described as a clean first attempt or a product finding.

## Node 24 and retained evidence

Commit `612917a` moved the repository's first-party JavaScript actions to
full-SHA-pinned Node 24 majors:

- `actions/checkout` v7.0.1;
- `actions/setup-python` v7.0.0;
- `actions/upload-artifact` v7.0.1; and
- `actions/download-artifact` v8.0.1.

The accepted run emitted zero check annotations, closing the prior Node 20
runtime warnings without suppressing runner diagnostics. Its nine nonexpired
artifacts are:

- `verification-aggregate`;
- `verification-coverage-diff`;
- `verification-linux-merged-coverage`;
- `verification-windows-process-coverage`;
- `verification-lifecycle-contract`;
- `verification-gui-semantic`;
- `verification-compat-live-interop`;
- `nextest-attempts-linux-1`; and
- `nextest-attempts-windows-1`.

## Documentation update validation

The closure-document update passed:

- relative Markdown-link resolution across all nine changed documents;
- all 545 Python policy and infrastructure tests;
- `cargo fmt --all -- --check`; and
- `git diff --check`.

No Rust source, workflow, test, fixture, or generated runtime artifact changed
in the documentation update; all nine files are Markdown. Clippy and workspace
tests were not repeated locally for prose-only edits; the accepted exact
workflow-bearing head above already ran those gates, and any later hosted run
remains authoritative for its own exact source.

## Remaining external execution boundaries

The hosted acceptance above does not change these capability-scoped limits:

1. provision and repeatedly attest the strict physical-input lane on a
   one-job ephemeral interactive Windows runner;
2. repeat the two native GUI HTTP recovery modes with distinct minimum and
   newest supported Windows mpv executables;
3. execute the three nonce-owned `dm-log-writes` replay cuts on a reviewed,
   disposable Linux host with `replay-log`; and
4. execute the complete public GHCR container runtime, restart-persistence,
   SBOM, signature, attestation, logout, and anonymous-digest chain.

Any later implementation or workflow change requires its own exact-head
hosted result. This evidence record closes the historical publication check;
it is not permission to infer unexecuted capability results.
