# Qualification fixtures and baseline diagnostics

Candidate `c4688afbd39d642e1d5597a3194f81307abb42b0` passed its full native
qualification, including seven exact Rust canaries, 90 Python cases and ten
StrictPhysical scenarios. Independent review verified the official artifact,
missing-file interactions and complete automatic runner/guest cleanup. The
native evidence remains specific to that source and its sealed bundle.

Other original attempts exposed three separate testing-apparatus problems.
Their failed evidence is retained; no unchanged hosted retry replaces it.

## Cleanup fixture clocks

[Windows preflight job 101464087952](https://github.com/ropbet-radbyt/sorotte/actions/runs/34024924784/job/101464087952)
failed two cleanup tests after their mocked runner DELETE had succeeded. The
fixture replaced the production unregister phase's 15-second grace with 500 ms;
real PowerShell/API-stub and receipt I/O consumed that interval before absence
could be confirmed. The receipt correctly refused to report confirmed removal.

Both unchanged tests pass locally without delay and fail when each mock API call
takes an additional 80 ms, below the fixture's 200 ms individual-call limit.
The repair virtualizes only the four copied phase clocks and three polling
sites. It keeps production grace values, real owned-child timeout/termination,
ownership checks, drain ordering and receipt assertions. A separate short-budget
case requires removal to remain unconfirmed after DELETE until later recovery.

The original artifact and before/after latency reproductions are under
`target/verification/hosted/c4688afb/preflight-windows-failure-attempt-1/`.
Production native scripts are unchanged by this repair.

## Protocol proxy fixture deadline

The separate disposable protection probe, PR #43, failed
[Windows job 101464748766](https://github.com/ropbet-radbyt/sorotte/actions/runs/34025174062/job/101464748766)
while receiving its first ordinary protocol echo. This was a different fixture:
it reduced the client's socket timeout from the usual two seconds to 200 ms,
although the assertion tests frame filtering rather than response latency.

An unchanged-test replay passes normally and reproduces the same timeout when
the first ordinary relay is delayed by 350 ms. The repair keeps the established
two-second socket-fixture deadline and all original filtering/count/trace
assertions. A permanent delayed-relay regression covers that schedule. An
`ExitStack` closes the fixture's sockets and threads even when an assertion
fails. The production proxy and lifecycle deadlines are unchanged.

Original and repaired replay receipts are retained separately at
`target/verification/proxy-fixture-original-attempt-1/` and
`target/verification/proxy-fixture-repaired-attempt-1/`. The injected delay proves
the fixture's sensitivity; it does not establish the original hosted scheduling
or network cause. PR #43's five explicit no-op producers passed, but its failed
Rust preflight does not establish positive protection acceptance.

## Unmutated GUI process abort

Mutation run `34024924759`, attempt 1, failed chunk
`gui-playlist-delivery-fence--2-of-2` before any mutation executed. Its unmodified
GUI baseline printed 36 passing assertions, then exited with SIGABRT. Cargo
returned 101 and cargo-mutants returned 4. The baseline is failed even though its
assertions passed. Its process-abort cause remains unclassified.

Pinned cargo-mutants 27.1.0 represents this baseline result as `Failure`. Our
parser rejected that supported summary as unknown, obscuring the primary
failure. It now recognizes coherent failed baselines and reports the failed
phase, exit result and retained log path/hash. Such outcomes remain errors;
`Failure` cannot count as a caught mutant. Regressions cover failed builds,
tests, timeouts, inconsistent phase results and unsafe artifact paths.

The original official artifact `9986826826`, raw baseline and failed-before /
passed-after parser regression are under
`target/verification/hosted-mutation-fuzz/c4688afb/`. Successful local replays
do not diagnose the original abort or turn its failed attempt into a pass.
