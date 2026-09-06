# Mutation campaigns

Mutation policy remains in `coverage/mutation-policy.toml`. Execution partitioning
in `coverage/mutation-execution.toml` changes how that work is scheduled. It does
not narrow the mutated files, selected tests, all-feature builds, baseline checks,
zero-survivor/timeout rule, or reviewed compiler-unviable exceptions.

Every PR receives the stable `mutation-required` result. Documentation-only PRs
receive a source-bound `no-applicable-shards` receipt after independently
recomputing the base/head selection. Main pushes, manual runs, and the weekly
schedule use the full selection. A missing, failed or cancelled selection,
preparation or relevant matrix producer fails the aggregate.

## Run a campaign

Use a clean committed candidate and the exact base/head SHAs. The ordinary
whole-shard commands in `DEVELOPMENT.md` remain supported for focused iteration.
Campaigns additionally reject modified, untracked or deleted source/test/policy
inputs because their shared plan must name an immutable candidate.

```powershell
$base = git merge-base origin/main HEAD
$head = git rev-parse HEAD
python scripts/verify.py preflight --phase static --output target/verification/preflight.json
python scripts/mutation_campaign.py prepare --base $base --head $head --output target/verification/mutation-campaign.json
```

The campaign JSON records every selected mutant, its full source span and diff,
the source/test/policy digests, compiler and tool identities, and each chunk's
exact inventory. `scripts/mutation_campaign.py prepare --full` is the full
campaign equivalent. Add `--full` to the run and verify commands for that plan.

List the matrix and run its entries, preserving a separate attempt directory for
each chunk. Parallel jobs may execute independent entries; each retains the
existing policy's worker count and entire test scope.

```powershell
$campaign = Get-Content target/verification/mutation-campaign.json -Raw | ConvertFrom-Json
foreach ($property in $campaign.shards.PSObject.Properties) {
    foreach ($chunk in $property.Value.chunks) {
        python scripts/mutation_campaign.py run --base $base --head $head `
            --campaign target/verification/mutation-campaign.json `
            --chunk $chunk.id --attempt-root "target/mutation-attempts/$($chunk.id)/attempt-1"
        if ($LASTEXITCODE -ne 0) { throw "Mutation chunk failed: $($chunk.id)" }
    }
}
python scripts/mutation_campaign.py verify --base $base --head $head `
    --campaign target/verification/mutation-campaign.json `
    --artifacts target/mutation-attempts --output target/verification/mutation-required.json
```

For an empty selection, verification requires `--mutation-result skipped`.
Hosted verification supplies actual dependency results explicitly. That status
does not replace artifact validation.

## What the finalizer proves

The finalizer regenerates each complete native mutant inventory and checks the
plan's exact deterministic round-robin partition. It requires every selected
chunk exactly once, independently parses the raw structured outcomes, verifies
all logs/diffs/status files against their report hashes, and checks a successful
unmutated baseline for every chunk. The union must include every current mutant
with no duplicate, missing, foreign or stale identity.

A chunk containing only reviewed compiler-unviable mutants is permitted. The
complete shard must still contain viable mutants, and the finalizer reconciles
the original policy's exact unviable multiplicities across all chunks. A reviewed
exception cannot disappear or silently multiply at a partition boundary.

Test listings are independently obtained in the finalizer. Identical
source/compiler/package/features/target/filter/environment requests reuse that
fresh listing within the same finalizer process. Producer-supplied listings and
listings from previous runs are never cache authority. The old fixed
participant-status set remains part of immutable selection; it no longer causes
a second round of verification and inventory builds.

## Progress, failure and retry evidence

Each attempt preserves a source-bound report before starting work. Every phase
streams bounded redacted console output and emits a heartbeat at least every
20 seconds while active. Mutation heartbeats include completed and remaining
counts and the last completed case; process receipts also retain pending and
failing mutant names. Exact inventory parser bytes are separate from redacted
console diagnostics.

The campaign deadline includes inventory preparation and execution, with time
reserved before the hosted job limit for cleanup and artifact upload. Cancellation,
timeout, malformed output and output-limit failures produce incomplete receipts.
They never satisfy the required aggregate. Source drift during preparation,
execution or finalization invalidates the campaign.

Windows commands enter an owned kill-on-close Job Object before launching their
children; POSIX commands own a process group. The wrapper stops and reaps owned
processes and removes its external scratch directory. Inherited absolute Cargo
target/build directories are replaced inside each cargo-mutants worker. A failed
cleanup blocks acceptance. Uncatchable termination or host loss can leave an
initial incomplete receipt; a missing final receipt fails the aggregate.

Do not overwrite an existing attempt. Use another attempt path. Hosted artifact
names include the run attempt and retain earlier evidence. The finalizer selects
one explicit latest attempt per chunk and records earlier attempts; duplicate
reports within the same attempt fail. A completed failed mutation attempt cannot
be erased by an unchanged retry. Correct the candidate and start a new campaign.
An incomplete interrupted attempt can be rerun while retaining its original
diagnostics. The existing nextest fail-on-flaky policy is unchanged.
For a local retry, pass `--attempt 2` and a corresponding fresh attempt directory;
hosted runs obtain that number from `GITHUB_RUN_ATTEMPT`.

## Apparatus checks and performance evidence

```powershell
python -m unittest discover -s scripts/tests -p 'test_mutation*.py'
python scripts/mutation_tool_canary.py --output target/mutation-tool-canary-attempt-1
```

The canary compiles a tiny dependency-free crate with the real pinned
cargo-mutants `27.1.0`, compares native zero-based round-robin inventories, executes
every chunk, and evaluates actual baseline/mutation artifacts. It runs before
selected hosted campaigns, independently of the mocked adversarial harness tests.
It also deliberately weakens a fixture assertion and requires the real producer's
survivors and nonzero exit to be rejected by the same evaluator.

The initial partition targets 48 mutants per chunk, with additional splits for
the slow shards measured in the 0.2.9 cohort. The old 216-mutant server shard
therefore becomes five chunks of 44/43/43/43/43, retaining its complete library
test scope and two workers. Balanced counts are an initial scheduling heuristic;
they do not establish equal runtime. The matrix starts historically expensive
chunks first to limit the tail when runner concurrency is below the matrix size.

Hosted execution allows at most ten mutation chunks at once, leaving capacity
for Rust checks and required aggregates. In the uncapped PR #40 campaign on
`ccb814c`, mutation occupied up to 17 of 20 observed concurrent jobs for that
candidate; the ten-second fuzz aggregate queued for 237 seconds. A concurrent
branch-protection probe also consumed runners, so this is not an account-quota
measurement or an isolated benchmark. This cap can increase
mutation completion time when other capacity is idle. Compare queue time and
total required-check completion across subsequent campaigns before attributing
an overall speedup to it; the mutant inventory and acceptance rules are unchanged.

`mutation-required.json` records per-chunk execution time, unmutated build/test
time, mutant build/test time, fresh finalizer listing executions and historical
whole-job time where available. Compare the slowest chunk, total execution and
additional baseline builds against the referenced cohort. The historical number
includes hosted setup, so it is not directly equivalent to an execution-only
duration. Measure a representative cold and warm hosted campaign before claiming
a new end-to-end performance target.
