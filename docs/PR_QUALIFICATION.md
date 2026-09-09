# Qualify before merge, promote the tested candidate

The application qualification decision belongs to the pull request. Repeating a
long application campaign after merge makes an intermittent fixture failure a
release repair, and using different PR-head, prospective-merge and release builds
makes a green PR an incomplete answer.

The replacement flow keeps these subjects explicit:

1. Update the PR branch to include current main. Test that exact head, including
   the complete nonpublishing stable-release campaign, while the PR is open.
2. Merge only after the ordinary checks and the candidate campaign pass. A
   lightweight main gate verifies the actual merge parents, unchanged source tree,
   current PR checks, exact candidate producer and immutable artifact identities.
   It does not execute application tests.
3. Tag the exact qualified head, which is now an ancestor of main with the same
   source tree. This preserves the source SHA recorded in the tested binaries;
   the merge commit remains a separately recorded integration identity.
4. Publish the retained archives and container image. Recheck source protection,
   qualification provenance and artifact digests, then verify public bytes and
   signatures. Do not rebuild or rerun application qualification after merge.

An unrelated commit with an equal tree is insufficient. A changed merge result,
outdated base, failed latest attempt, missing artifact, different tool/build
inputs or untrusted producer blocks promotion. A new candidate must be qualified
before it is merged; the publisher must never silently fall back to a fresh
application test campaign.

Application qualification still cannot prove that a timing-dependent defect does
not exist. Controlled fault and contention tests are required for known failure
paths. Scheduled assurance retains independent repeated execution without
changing the recorded PR qualification decision or publishing releases.

Acceptance for this change includes a complete candidate campaign while the PR
is open, green required PR checks, main promotion with no application execution,
publication of exactly the retained candidate bytes, and independent public
verification. Timing records must distinguish PR queue/setup/execution, main
promotion and publication; no feedback-time improvement is claimed from workflow
structure alone.

The implementation has passed local pipeline regressions, default workspace
tests, all-feature client tests, Clippy, static preflight and workflow validation.
Hosted acceptance is pending on the current PR; these local checks do not claim
a completed candidate campaign or publication.

The first hosted revision was stopped by Linux PR preflight: the fuzz cleanup
canary read procfs state `X` for its killed descendant, but the assertion accepted
only `Z`. The [Linux process-state contract](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html)
defines `X` as dead and `Z` as zombie. Fuzz and mutation cleanup observations now accept both terminal
states while still rejecting running, sleeping, stopped, idle and unknown states
and preserving other procfs read errors. The actual timeout/descendant test
remains required on Linux. This failure stayed in the open PR; main and release
were not changed.
