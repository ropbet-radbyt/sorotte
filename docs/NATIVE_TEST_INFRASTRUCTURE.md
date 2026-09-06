# Isolated native test infrastructure

The native runner controller supports one explicitly assigned trusted job in a
fresh Windows Sandbox. It performs tool and harmless desktop checks before
requesting a registration token. The guest runs the foreground, non-service
Actions listener with `--ephemeral --disableupdate --unattended`. Its job hook
binds repository, source SHA, run ID and attempt; the host independently binds
the queued job ID and later verifies that exact job used its unique runner.

The host maps a read-only bootstrap, a read-only sealed tool bundle and one
fresh output directory. The live checkout, host home, Git credentials and
Cargo caches are never mapped. Networking is enabled for the Actions job;
clipboard, camera, microphone and printer redirection are disabled. The guest
uses short `C:\SorotteCI` and `C:\w` paths, Git's `bin\bash.exe` before any WSL
or `usr\bin` executable, and process-local Git/Cargo/tool configuration.

## Prepare the documented host state

Enable Windows Sandbox on a supported Windows host and complete any required
restart. The opt-in `scripts/enable-windows-sandbox.ps1` records the feature
state and never restarts Windows itself. The modern `wsb.exe` must support
`list`, `start`, `connect`, `stop`, `--raw` and caller-selected UUIDs. The host
needs Python 3.11 or newer and authenticated `gh` with runner administration
permission for `ropbet-radbyt/sorotte`. No existing guest may be running.

`verification/windows-native-guest.json` is the reviewed guest profile. It
names Rust 1.97.1, Python 3.12.10, MSVC 14.29.30133, SDK 10.0.19041.0 and Actions
runner 2.337.0. The runner archive hash matches the [published runner checksum](https://github.com/actions/runner/releases/tag/v2.337.0).
Every copied tool file is also hashed. Updating a compiler or tool requires
reviewing the profile and producing a fresh bundle; an existing sealed bundle
cannot silently adopt a changed download. Rustup's download is content pinned:
a replaced upstream file is rejected rather than trusted by URL alone.

There are two preparation paths. To import a previously reviewed portable
tool layout (including the retained release tooling), run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native-runner-prepare.ps1 `
  -PortableToolsRoot C:\reviewed-tools `
  -OutputDirectory target\native-runner-inputs\candidate
```

For a fresh host, install the profile's MSVC/SDK and standard portable tools,
copy `verification/windows-native-tool-sources.example.json` to a local file,
and set its eight explicit source paths. Python must be a dedicated full
3.12.10 runtime prepared with the pinned pip and legacy interop dependencies
from `requirements/verification-constraints.txt` and
`requirements/legacy-python-interop.txt`. Package installation is an explicit
input-preparation step. Readiness never installs packages or accesses a package
index. Collection copies the selected compiler/SDK/runtime components and only
that pinned dependency closure, including its distribution metadata and native
extensions; unrelated packages and `.pth` startup hooks are excluded.

```powershell
python scripts/native_runner_bundle.py collect-installed `
  --sources target\native-tool-sources.json `
  --bundle target\native-runner-inputs\candidate
python scripts/native_runner_bundle.py validate `
  --bundle target\native-runner-inputs\candidate
```

The output includes `tools-manifest.json`, its digest and a closed tool-file
inventory, shared Python readiness probe and requirements contract. Both host
and guest check it. Preparation and validation execute the selected isolated
interpreter, verify `python -m pip`, package versions and required native
interop imports (including `zope.interface`). The guest repeats the probe at
the exact Actions tool-cache path before publishing `x64.complete` or accepting
a registration token. An embedded interpreter without pip is rejected before
publication. Cached downloads must match their
reviewed hashes; failed downloads leave no partial file or registration.
Guest preflight compiles and executes harmless C/Rust programs, checks Python
and Git Bash, then checks Explorer, the foreground session, input-desktop
access and at least 1800x1200 physical pixels. It records the actual Windows
version. Keep the Sandbox viewer connected for the whole native job.

## Run one trusted job and verify cleanup

For a reviewed PR candidate, one command dispatches its exact source, waits
up to ten minutes for hosted applicability, then provisions and tears down
one guest. An already queued exact-source main run is used without creating
a duplicate dispatch:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native-runner-qualify.ps1 `
  -ReviewedRef codex/reviewed-candidate -SourceSha <full-40-hex-sha> `
  -BundleDirectory target\native-runner-inputs\candidate
```

The ref must exist in this repository and resolve to the explicitly supplied
source. Running this command is the maintainer's approval to execute that
reviewed code in the isolated guest. It requires current repository write,
maintain or administrator authority. It retains a request record even if
queueing or provisioning fails. For a fork PR, first review the source and
make that exact commit available through a trusted repository ref; arbitrary
PR execution does not receive a native runner.

After a provisioning failure the wrapper saves the failure before cancelling
the exact still-active native run it selected. This releases native workflow
concurrency; it never cancels a different source or an ambiguous run. The
controller remains responsible for exporting diagnostics and verifying guest
and registration cleanup.

Dispatch `gui-native-interactive.yml` at the exact committed trusted source,
using `gh workflow run --ref <reviewed-ref>` with `source_sha` equal to that
ref's commit. The pre-checkout gate requires `source_sha == GITHUB_SHA`;
a dispatch on main cannot substitute another checkout through its input.
Select `native_dpi=none`, `96`, `144`
or `192`. Use the resulting run/attempt and queued native job ID; the
controller rejects foreign repositories, PR events, changed sources, stale
attempts, a nonqueued job, another registered native runner or competing
queued native jobs. For stable qualification, use the explicitly authorized
run and job from the release orchestrator instead.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native-runner-sandbox.ps1 `
  -BundleDirectory target\native-runner-inputs\candidate `
  -SourceSha <full-40-hex-sha> -RunId <run-id> -RunAttempt <attempt> -JobId <job-id>
```

The controller does not infer permission from the newest green run. This
command is the operator's explicit candidate/job authorization. Registration
credentials are handed off only after guest readiness, consumed once and
removed in normal and recovery cleanup. They are not part of diagnostic
exports. Host receipts separately record provisioning, queue and actual job
execution time, job conclusion, manifest identity, automatic unregistration
and verified guest removal. A successful controller exit requires all of
those cleanup obligations, including automatic unregistration.

A hidden independent watchdog detects controller exit or a bounded timeout
and runs cleanup for only its saved instance UUID. Its final observation must
record `watchdog-completed`; resource absence alone cannot hide a subsequent
watchdog error. Fault acceptance also verifies that the exact recorded watchdog
process has exited. After host reboot, API
outage, watchdog failure or an interrupted cleanup, use the retained receipt:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native-runner-sandbox.ps1 `
  -CleanupOnly -InstanceId <saved-instance-uuid>
```

Recovery removes only that instance's token handoff, guest and uniquely named
runner, then re-queries both inventories. It exports evidence first, validates
the exact source/run/attempt/job/runner, and requests cancellation while the
guest can still acknowledge it. After a 30-second normal cancellation window,
an unresponsive exact run may use GitHub's documented
[force-cancel endpoint](https://docs.github.com/en/rest/actions/workflow-runs#force-cancel-a-workflow-run).
Cancellation requests never substitute for observing a completed job and an
idle runner. A completed successful native job instead gets its automatic
unregister window without cancelling the enclosing release workflow.

Drain and unregister have separate 90-second and 30-second API budgets;
each API child has at most 10 seconds plus bounded process/stream termination.
Each evidence exporter also has a 10-second process limit and records an
explicit unavailable result on timeout. GitHub or exporter failure cannot
prevent the separate guest-stop attempt. Token removal is retried after the
guest stops, and a remaining token makes cleanup fail. Manual unregister is
recorded before DELETE and cannot become automatic-unregister evidence on a
later recovery. Private API captures retain status, original errors and each
ownership observation without publishing credentials or raw guest output.

Guest-stop, token-removal and unregister failures
are independent; an unconfirmed result makes cleanup fail. Repeating
recovery retains separate diagnostic attempts. It does not delete another
guest, runner or a previous evidence directory.

## Failure evidence and independent readiness

`native_failure_evidence.py` exports structured causal traces, final observed
state, per-mode outcomes, PID and stable opaque endpoint identities. It
withholds raw logs, screenshots, configuration and runner credentials; private
paths, URLs and credential fields cannot be copied unchanged. Malformed,
oversized or missing records are explicitly unavailable. Diagnostics always
say `authoritative=false`; they cannot replace a passing lifecycle attestation.
The controller exports before guest removal and retains final cleanup in a
separate receipt. The guest's finalizer provides a host-readable fallback when
the Actions upload does not finish. If interruption prevents that export, the
host records unavailable evidence instead of inventing a native result.

Native Actions uploads contain only the projected directory and use distinct
source/run/attempt identities with 90-day retention. Preserve the compact
controller receipt, safe diagnostic, tool manifest and source-bound release
receipt in the release's durable archive; Actions retention is not permanent
release history. Raw local screenshots can be reviewed privately before a
deliberate publication decision.

Before spending the isolated worker's time on physical input, run:

```powershell
python scripts/native_harness_canary.py --output target\verification\native-readiness.json
```

The non-desktop gate executes each test in
`coverage/native-harness-canaries.json` by exact name and rejects zero or
ignored execution. It replays exact counter acknowledgements, actual-server
wire exchanges, fragmented frames, accepted-socket modes, early replacement
events, malformed observations and the delayed Lua resource-open schedule.
The report validators retain adversarial missing-file, foreign PID/endpoint,
stale generation, missing interaction and undo/transport assertions. These
checks preserve the separate actual-server, real-player and independent
lifecycle-oracle authorities. The fake's immediate acknowledgement is an
allowed fixture simplification; the actual server can piggyback that exact
counter on its next state publication.

## Scheduled and manual responsibilities

`coverage/assurance-capabilities.json` records owners, commands, environments,
cadence, freshness and the last verified source/date. Missing evidence stays
visible. The native workflow requests a full inventory and measured 144-DPI
profile weekly from trusted main. It still requires an authorized isolated
runner; a queued request does not prove provisioning or execution. Selected
trusted dispatch supports the separate 96/192-DPI profiles. A real
`GetDpiForWindow` measurement must match each requested profile. Application
zoom does not stand in for native DPI. Six captures require visual inspection
for clipping, glyph fallback, focus, contrast and scroll reachability.

The screen-reader owner must record reader/version, OS/build, source/binary
digest, DPI and actual task observations: connect; identify room/member/playback
state; edit and recover a validation error; navigate both long lists; open and
cancel a modal; recover missing media; exit. Retain what was announced and
whether the task was completed. UIA enumeration alone cannot pass this task.
No current 96/192-DPI or screen-reader proof is claimed by the harness tests.

Privileged persistence power-loss checks belong to a disposable Linux storage
worker. Start with `python scripts/persistence_power_loss_harness.py --plan-json`
and `--preflight`; the owner must provision the documented device-mapper/replay
prerequisites before the explicitly confirmed `--run`. Fixture generators are
maintenance work and must never be scheduled to rewrite trusted oracles.

## Required native authority and bounded waiting

`native-required.yml` always produces the `native-required` check for PRs
and main pushes on an unprivileged hosted worker. Its plan must match both
the externally supplied event base and exact candidate SHA. A documentation
change gets an explicit no-applicable-native-work receipt. Applicable work
requires GitHub to report one successful physical Windows job from
`gui-native-interactive.yml` at that exact source and attempt; local reports,
diagnostic exports, older sources and equal trees cannot satisfy it.

Main pushes automatically run hosted applicability before queueing their
native job. This has no dependency on release authorization and does not
expose an arbitrary PR checkout to a self-hosted runner. PR candidates use
the review-and-provision command above. The producer's original actor and
rerun actor must both currently have repository write authority; automated
bot identities are not accepted as a maintainer approval.

The required check waits at most 90 minutes, with a pending explanation every
30 seconds. Missing capability fails with the dispatch/provision command;
a completed failing latest producer fails immediately and keeps its original
diagnostics. Repair or provision the exact candidate, then rerun the required
check. It never searches backward for an earlier green run after a later
failure. The published decision records the producer run, attempt, job and
runner IDs and approved actors. GitHub's permission lookup uses the
[repository metadata read endpoint](https://docs.github.com/en/enterprise-cloud%40latest/rest/collaborators/collaborators#get-repository-permissions-for-a-user).

## Validation boundary for this change

Repository self-tests cover corrupt/failed downloads, manifest mutation,
source/run/attempt/job mismatches, host refusal, native failure projections,
private credential/path canaries, interrupted evidence and workflow policy.
The actual-server conversation and 22 native Rust unit/socket tests were run
on Windows. Initial conversation failures were test assumptions (username
length and acknowledgement publication timing), corrected against observed
server behavior; they were not product defects.

Actual disposable-Sandbox fault drills were attempted on trusted source
`8297a56513bffc38d1e462f09f70da671d10dea7`. The cancellation drill
[run 34010550587](https://github.com/ropbet-radbyt/sorotte/actions/runs/34010550587)
completed its expected fault, safe export and independently verified guest,
runner and process cleanup. Cold guest preparation took 80.87 seconds; the
guest reported a 3050x1668 input desktop. This is neither a DPI assertion nor
a full native suite pass.

The controller-interruption drill
[run 34010920508](https://github.com/ropbet-radbyt/sorotte/actions/runs/34010920508)
failed: cleanup stopped the guest before the cancelled job drained, and
GitHub rejected DELETE with HTTP 422 because the runner was busy. Its watchdog
and bounded fallback both failed. GitHub eventually completed cancellation
and the runner disappeared, independently verified alongside zero owned
guests and processes; this later cleanup does not retroactively pass the
drill. Original receipts and errors remain under
`target/verification/native-acceptance-drills/interrupt-01c1bcf0-a81a-417b-b566-14d3c06430e9`.
The repair has real PowerShell recovery/process regressions but requires a
new committed-source cancellation/interruption run and full positive native
qualification. Host-reboot recovery and actual DPI/screen-reader profiles
remain separate acceptance obligations. No native action ran on the user's
active desktop.

A subsequent interruption of source
`7cd70f49aa255968dbdbe9c38a863041bd9ec2fd` in
[run 34015580980](https://github.com/ropbet-radbyt/sorotte/actions/runs/34015580980)
drained the job and removed all owned resources within the deadline, then the
watchdog reported an unset `$LASTEXITCODE`. Its test stub had manufactured that
native-process variable even though recovery invokes a PowerShell script. The
realistic fixture reproduces the reporting failure. Recovery now checks the
script's success and records explicit completion; exception and nonzero-exit
regressions preserve failure behavior. The original drill and independent review
remain in `target/verification/native-acceptance-drills/interrupt-3004fd38-7fbe-4da3-b814-f6fe953f4efe`.
That resource-cleanup proof is retained, and final fault acceptance requires a
new run with a successful watchdog completion record.

The first full positive attempt of source
`67679a7a112b78e9c591aec45f27194ab9f55ce4` failed in
[run 34016530011](https://github.com/ropbet-radbyt/sorotte/actions/runs/34016530011)
before any native scenarios ran: the retained portable Python lacked pip, but
the old standard-library probe had marked its Actions cache complete. The
original `17f1bbca...` input bundle and failed attempt remain preserved. The
controller exported diagnostics, drained the failed job and removed its guest,
runner and token files; the watchdog recorded completion. That cleanup does
not qualify native behavior. The shared offline runtime probe closes this
pre-registration gap; a fresh supported-runtime bundle and positive run are
required.
