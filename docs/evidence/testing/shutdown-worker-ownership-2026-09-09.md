# Verify bounded shutdown and retained worker ownership

The unmutated server baseline in [PR job 102460119066](https://github.com/ropbet-radbyt/sorotte/actions/runs/34349748551/job/102460119066)
failed `persistence_shutdown_includes_preceding_flush_in_total_budget` on
`86568472cfdd3356457a5856d74f201af6592722`. The isolated child reported
`persistence shutdown deadline exceeded; worker ownership retained in the
unjoined registry`. The test accepted only the other deadline outcome, in
which workers had already joined without claiming durability.

Shutdown gives the caller one total budget, including an earlier flush, and
reserves 100 milliseconds for joining. Operating-system scheduling can exceed
that reserve. The implementation already retains an unfinished worker or its
async cleanup task in the observable registry. The test incorrectly required
this documented path never to occur. The log establishes the missed join
deadline; it does not identify the host's scheduling delay.

The subprocess now accepts only the three explicit durability/owned-cleanup
deadline errors. Success, unrelated errors and unbounded shutdown still fail.
It keeps the SQLite lock held while requiring any retained ownership to drain
within a separate two-second cleanup observation window. It still verifies
concurrent runtime progress, the preceding flush's failed durability result,
database integrity and complete old-or-new persisted documents.

An additional case uses a test-only channel barrier immediately before the
real persistence thread exits. It holds that thread beyond shutdown's deadline,
requires an explicit retained-cleanup result and nonzero observable ownership,
then releases and reaps the worker while SQLite remains locked. The barrier has
its own five-second watchdog and is absent from production builds.

Restoring the old joined-only assertion makes this controlled case fail with
the same retained-ownership error. Restoring the correction passes. The raw
red/green logs and exact source-restoration check are retained in
`target/verification/v0.2.12-release/slow-worker-shutdown-proof/`. All 468 Windows
server library tests passed together with all features. The complete local
verification-tool suite passed 1,209 tests with four registered capability or
platform skips. Hosted qualification remains necessary for the committed PR
revision; these local results are not release authority.
