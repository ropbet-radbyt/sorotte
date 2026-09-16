# Native Pause follow-up and release plan

Date: 16 September 2026 (Australia/Sydney).  
Base: `b86696ec6075d2b5bf92bec17d4abc02277847eb`, v0.2.17.  
Branch: `codex/lifecycle-latency-review`. Uncommitted; not release-qualified.

## Outcome

The native mpv Pause reversal is fixed in the tested scenarios. The final
offline Windows Sandbox campaign passed **36 native Play/Pause operations**:
six Play/Pause cycles in each of Media Match off, an empty index and a 98 MiB
index. Each operation had to leave all three real players in the requested
state. Rapid Pause followed by Seek also remained at the requested position in
all three configurations. Collection now fails if those assertions fail.

The playlist improvement remains: with the 98 MiB index, selecting the second
file took **26 / 85 / 85 ms**, and appending while fingerprint work could run
took **6 / 21 / 21 ms** (host / Media Match peer / local-only peer).

Required indexing still took **15.91 seconds** for the differently named peer
copy. Its row appeared in 70 ms; appending while it waited took 6–32 ms across
clients. Both fingerprints were present and the probable candidate opened
paused. Sampled-only evidence still did not grant autoplay verification.

## Three interacting causes

1. **Correction erased an unconfirmed input.** A physical Pause must cover at
   least 100 ms of accepted observations before the classifier treats it as a
   user gesture. Reconciliation could issue Play against the old room state
   after the first sample, erasing the input before confirmation.
2. **Recovery claimed later user input.** A native Play could briefly enter
   recovery. Its stability period then classified an uncommanded Pause as
   technical, even after playback had resumed. Recovery issued another Play.
3. **A heartbeat looked like new authority.** The candidate fence compared the
   complete room snapshot, including position and receipt time. A heartbeat
   carrying the same transport revision could therefore cancel confirmation.

The first failure was reproduced deterministically before the initial fix.
Real-mpv repetition then exposed the recovery and heartbeat cases; each received
its own failing-before regression. This was not a server rejection or a Media
Match setting issue. The earlier unchanged-base Sandbox also reproduced it.

## Ownership retained by the fix

- Client-core continues to classify native intent. A second accepted physical
  observation must establish stability. Waiting does not fabricate one, and an
  unconfirmed candidate expires after the existing one-second deadline.
- Only conflicting Play/unpause corrections are withheld while an eligible
  candidate or accepted local Pause is pending. Unaccepted coordinator command
  records are retired, with reconciliation woken for expiry or supersession.
- Healthy recovery can recognise an uncommanded logical Pause. Cache pauses,
  unresolved recovery seeks, degraded recovery, loading and seek preparation
  remain technical. Command receipts still own system-issued pauses.
- Room, connection and player/media identity fence the candidate. A newer
  canonical transport revision, changed pause owner or replaced player wins.
  Same-revision position heartbeats do not represent new transport decisions.
  Untagged Syncplay state retains its conservative snapshot comparison.
- The GUI does not independently confirm rich-telemetry Pause on another UI
  pump. Its existing native Play/readiness path remains intact.
- Server revision validation, playlist write receipts and Media Match
  confidence/autoplay policy remain in force.

Main implementation:

- [Local intent and candidate fencing](../../crates/sorotte-client-core/src/runtime/playback_coordination/local_intent.rs)
- [Recovery ownership](../../crates/sorotte-client-core/src/playback_coordinator.rs)
- [Native Pause regressions](../../crates/sorotte-client-core/src/runtime/playback_coordination/tests/native_pause_regressions.rs)
- [GUI observation handling](../../crates/sorotte-gui/src/app/runtime_detached.rs)

## Index-job invalidation review

The exact-signature job key previously contained only the path. The retained
lookup could detect a replacement file as missing while the completed job key
still suppressed extraction. The key now includes the index root, settings,
file size, modification time and creation time. Queue-level coverage checks
that replacing the file or changing root/settings retires the old job, while an
unchanged request remains deduplicated. Cache clear continues to retire the job
and retained reader before deleting the index.

## Validation and evidence

- Fourteen focused core regressions cover confirmation, transient edges,
  timeout, command echoes, cache/recovery ownership, heartbeat versus newer
  revision, player replacement and reconnect. Managed and attached players use
  the same hold.
- Formatting and workspace/all-target Clippy with warnings denied passed.
- Full workspace: **4,338 passed, 25 ignored, zero failures** across 48 harness
  summaries. Ignored optional gates are not release qualification.
- All **14 GUI semantic scenarios** passed, including both live Python
  interoperability cases, async reset and playlist indexing feedback:
  [semantic receipt](../../target/latency-review/build-logs/native-pause-semantic.json).
- [Final Sandbox campaign](../../target/latency-review/sandbox-native-pause-final-01/):
  all five probes passed, including the index primitive and required-index case.
  The exact Sandbox ID was stopped and the final inventory was empty.
- [Initial failing regression](../../target/latency-review/build-logs/native-pause-regression-before.log),
  [recovery regression](../../target/latency-review/build-logs/native-pause-recovery-before.log),
  [heartbeat regression](../../target/latency-review/build-logs/native-pause-heartbeat-before.log).
- [First full attempted fix](../../target/latency-review/sandbox-native-pause-fix-01/)
  failed, [recovery tracing](../../target/latency-review/sandbox-native-pause-diagnostic-01/)
  identified its cause, and [the next attempt](../../target/latency-review/sandbox-native-pause-fix-02/)
  exposed the heartbeat race. [The following rerun](../../target/latency-review/sandbox-native-pause-fix-03/)
  passed all six cycles before the final mixed-configuration campaign.
- [Machine-readable summary](../../target/latency-review/native-pause-summary.json)
  and [workspace log](../../target/latency-review/build-logs/native-pause-workspace.log).

The final campaign sealed source diff, untracked Rust sources and binary/tool
hashes. Temporary core tracing was removed before that campaign. Measurements
use debug builds, generated media, null audio/video outputs and effectively
zero RTT. They prove the exercised runtime paths, not native rendering, first
visible frame, WAN behavior, or a statistical latency guarantee. Local workspace
checks ran concurrently with the last Sandbox; use the earlier isolated
comparison for the performance claim.

## Remaining latency work

Other players observed native controls in **82–793 ms** in the final campaign.
The classifier's 100 ms stability requirement is deliberate. The GUI's
one-second state heartbeat is a separate follow-up target: measure and, where
appropriate, promptly flush an already-authorized native transport intent
through the existing session/receipt path. Do not shorten confirmation merely
to hide another scheduling delay.

Cold indexing still performs substantial extraction work on host and receiver.
Prioritize selected media and reuse compatible in-flight/indexed results while
preserving file-version checks and confidence policy. Test NAS/sleeping storage,
real libraries, delayed/reordered player events and 25/100/250 ms RTT with jitter.
The current runs do not justify p95 claims.

## Route to a green PR and v0.2.18

1. Review and commit the worker/reader, signature lifetime, input ownership and
   feedback changes on this branch. Preserve the original dirty checkout and
   the failed experiment evidence. Include the opt-in probes as repeatable
   experiments, with ordinary regressions in their owning crates.
2. Bump the workspace and lockfile package versions to **0.2.18** in the PR and
   update the current architecture/release notes for that candidate. The live
   latest release was verified as v0.2.17; no version or tag has been changed in
   this experiment.
3. Run the clean-head verification wrapper and all required hosted checks on the
   exact PR head: default/all-feature tests, doctests, Clippy, semantics,
   coverage/mutation and the applicable native/package lanes. These local
   uncommitted runs do not satisfy candidate qualification.
4. Run `stable-release.yml` with `publish=false` for that exact candidate.
   Retain lifecycle, archive/updater, container and cold-restoration evidence;
   ordinary PR CI alone does not satisfy `package-required`.
5. After review and authorized merge, verify protected-main promotion and the
   qualified candidate relationship. Then create **v0.2.18** using the qualified
   release subject and publish the retained artifacts. Verify public assets,
   sidecars and container parity independently. A tag should not start a fresh
   unqualified rebuild.

See [release qualification](../RELEASE_QUALIFICATION.md) and
[the testing process](../TESTING_PROCESS.md) for the normative gates.
