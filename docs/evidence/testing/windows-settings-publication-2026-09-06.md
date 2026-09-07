# Windows settings publication and reader evidence

The documentation-only protection probe [PR 42](https://github.com/ropbet-radbyt/sorotte/pull/42)
exposed a real Windows settings-read failure in
[run 34010712132, attempt 1](https://github.com/ropbet-radbyt/sorotte/actions/runs/34010712132).
The raw-reader regression failed with OS error 2, then passed its diagnostic
retry. The strict flaky-result policy rejected that result. Its Rust sources
matched candidate `8297a56513bffc38d1e462f09f70da671d10dea7`; the earlier ordinary
candidate CI pass therefore did not establish that this race was absent.

## Reproduced boundaries

The retained local probes ran on Windows 11 25H2 with NTFS. Each process owned
its own fixture directory; no process deleted another fixture. Original logs
and failed receipts remain unchanged under `target/verification/`.

| Probe | Observed result | Retained evidence |
| --- | --- | --- |
| Unchanged legacy writer, four parallel test processes | Raw open returned OS2 after 105 total attempts | `ini-atomic-baseline-parallel-1/receipt.json`, `worker-1-iteration-025.log` |
| Ordinary old-file reader held open throughout replacement | Legacy writer failed with access denied after its 250 ms budget; POSIX rename passed | `ini-held-reader-before-fix.log`, `ini-held-reader-after-fix.log` |
| POSIX rename, first parallel campaign | 1,000 executions passed | `ini-atomic-posix-parallel-1/receipt.json` |
| POSIX rename with source-open retry included | Raw open returned OS2 after 875 attempts | `ini-atomic-posix-parallel-2/receipt.json`, `worker-3-iteration-218.log` |
| POSIX rename with read-phase diagnostics | File open, immediate metadata, and immediate reopen all returned OS2; 649 attempts before failure | `ini-atomic-posix-diagnostic-1/receipt.json` |
| Independent native hard-link replacement | Raw open returned OS2 at one worker's round 339; other workers stopped and reaped | `ini-hardlink-probe-1/link-attempt-1/receipt.json` |

The hard-link experiment separately preserved the staging DACL, old-reader
bytes and read-only rejection, but its failed concurrency result disqualified
it as a solution. Its source, compiled binary hashes and primary API references
are retained in `ini-hardlink-probe-1/independent-review.json`.

The [matching upstream report](https://github.com/microsoft/STL/issues/5501)
includes the same symptom and a report that POSIX rename can also exhibit it.
Microsoft's [POSIX rename specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/4217551b-d2c0-42cb-9dc1-69a716cf6d0c)
describes valid old handles and replacement-name opens. These experiments
establish the observed Windows/NTFS boundary; they do not identify the precise
kernel or filesystem-filter implementation responsible for it.

## Product correction and retained diagnostic

Sorotte's readers coordinate with the persistent transaction sidecar. They
cannot decide that settings are absent while a cooperating writer is publishing
them. Read-only and initially missing paths do not create directories or lock
files; a newly appeared sidecar invalidates a provisional unlocked read. The
shared reader and checked locator APIs return busy and read errors. CLI startup
and initial GUI configuration resolution propagate those failures instead of
selecting empty settings or a different configuration root. Internal writer
reads remain under the existing exclusive lock without reacquiring it.

The final writer uses the pinned Rust library's Windows rename implementation,
whose POSIX fallback addresses the independently reproduced held-reader failure.
It preserves the private staging file and existing read-only checks, and retries
only source acquisition/publication within the existing deadline. The exploratory
custom POSIX buffer and hard-link implementation are not needed. This writer is
not presented as a repair for raw lock-free namespace visibility.

`cooperating_readers_observe_complete_documents_through_repeated_replacement`
requires complete old/new bytes through the product reader. The original strict
raw filesystem assertion remains ordinary coverage on non-Windows platforms
and is retained on Windows as
`windows_raw_filesystem_readers_observe_complete_documents_through_replacement`.
It is explicitly quarantined as `IGN-CLI-009`, with upstream tracking and a
review deadline of **2026-10-06**. An expired quarantine fails policy validation.
The raw assertion has no added read retry and its intermittent pass is not
qualification evidence.

A local probe of the synchronized implementation passed 1,000 cooperating-reader
executions and 100 held-reader executions across four independent test processes
in 38.043 seconds, with no retries. The source and executable hashes remained
unchanged throughout that attempt. Its receipt is
`target/verification/synchronized-settings-stress-attempt-1/receipt.json`;
this is pre-commit diagnostic evidence, not hosted candidate authorization.

To reassess the Windows boundary, run the quarantined test explicitly on a
recorded filesystem and OS revision with retries disabled, preserving every
attempt. For example:

```powershell
cargo nextest run --locked -p sorotte-client-app --lib --all-features --run-ignored only --retries 0 --stress-count 1000 -E 'test(=sorotte_ini::transaction_tests::windows_raw_filesystem_readers_observe_complete_documents_through_replacement)'
```

The original reproduction also ran four independent test processes concurrently;
the single-process command above does not reproduce that scheduling pressure.
Product CI must separately pass the shared-reader, reader/writer,
Clear, missing/read-only, path-alias and private-DACL contracts. Filesystem and
power-loss durability beyond the exercised host remain unproven.
