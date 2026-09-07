# Testing audit evidence

This packet accompanies [the 6 September 2026 audit](../testing-apparatus-audit-2026-09-06.md). It describes source `4000eca69b52003b66e81b6998d15c555e7eb6d1` and PR #32's retained hosted history. It is an apparatus audit, not a new release attestation.

## Retained files

| File | Purpose |
|---|---|
| `hosted-summary.json` | Repository-owned run cohort, execution accounting, final-candidate jobs, failures and release lifecycle producers. |
| `workflow-inventory.json` | All ten current workflows, triggers, dependencies, matrices and step names. |
| `repository-state.json` | Fresh main/tag and branch-rule API observations plus local source/tree identity. |
| `local-validation.json` | Commands, environment boundaries, initial failures and unchanged successful replay of all 767 self-tests. |
| `policy-validation.json` | Raw command results for eight current validators; also preserves an initial mistaken `validate-policy` invocation before the successful `validate` command. That CLI typo is an audit error, not a repository finding. |
| `incident-evidence.json` | Bounded excerpts and hashes from the original hosted mutation and socket-timeout logs, plus the release closure ledger. |
| `source-anchors.json` | Exact source-link paths, line text and source hashes for the report's immutable GitHub links. |
| `document-validation.json` | Report structure, link/anchor, timing-accounting and source-change verification. |
| `collect_hosted.py` | Read-only `gh api` collector for PR #32 and associated Actions attempts/jobs. No dispatch, settings change or publication. |
| `analyze.py` | Recomputes the compact report from the collector's raw index. |
| `validate_report.py` | Checks document links, source anchors, task inventory and preserved source. |

Raw responses and complete logs are under `target/testing-audit/hosted/`; complete local test logs are `target/testing-audit/python-policy.log` and `python-policy-host.log`. These are local diagnostic files, normally ignored by Git. The compact packet preserves the assertions needed to assess the report without checking in large API responses or all successful test output.

## Reproduction

From the exact audited worktree, with Python 3.11+ and an authenticated GitHub CLI:

```powershell
python docs/audits/2026-09-06-testing-evidence/collect_hosted.py
python docs/audits/2026-09-06-testing-evidence/analyze.py
python docs/audits/2026-09-06-testing-evidence/validate_report.py
```

The collector caches raw responses under `target/testing-audit/hosted/`. Existing responses are intentionally immutable local snapshots; use a separate clean worktree/output directory for a later live comparison. The date window is the release cohort, not an open-ended query for current CI health. The analyzer excludes non-repository workflows and does not substitute current rules for the saved rule snapshot.

To replay harness self-tests, install `requirements/ci-policy.txt`, use ordinary Windows process permissions and the normal external system temp directory, and run:

```powershell
python -m unittest discover -s scripts/tests -p 'test_*.py' -v
```

If the checkout has Git ownership restrictions, use process-local `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_0=safe.directory` and the exact checkout path as `GIT_CONFIG_VALUE_0`; do not change global Git trust. The initial restricted replay's repository-local temp override violated the semver wrapper's documented contract, and its process-control environment could not kill an owned test child. The passing replay changed environment only. This is a headless harness test command; it does not run the strict physical GUI suite.

## Accounting boundaries

- The cohort contains 14 PR commits (12 heads with runs) and the actual merge. Three Dependabot runs at the merge SHA are excluded: 77 repository workflows remain.
- 83 attempt responses contain 718 repository job records. Seventeen entries copy earlier executions with new IDs and original timestamps. After deduplication there are 701 execution/skip records. Skips contribute zero execution time.
- The observed 4,772.37 minutes include parallel and self-hosted work. They are not GitHub billing, CPU consumption, operator time or a flake-rate estimate.
- Five Windows and five Linux lifecycle executions at the merge SHA are real separate runs. Carried successful jobs in the server workflow's second attempt are not counted as another execution.
- Failure counts are not bug counts. An aggregate can fail because its producer failed, a cancelled campaign is incomplete, and the release socket timeout caused two lock-poison consequences.
- The policy/model seed replay covers 56 transitions. Full release composition historically required 75 system transitions; current model structure has 78 transitions in total. These are different inventories.
- No physical GUI, privileged storage fault or full Rust/mutation/coverage campaign was run by this audit.
