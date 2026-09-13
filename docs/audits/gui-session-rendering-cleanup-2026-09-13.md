# GUI session and rendering cleanup

This pass continues the internal-surface cleanup from PR #73. It combines the
previously deferred session interface removal with unnecessary GUI work and
hidden-window behavior. Syncplay interoperability and settings import remain
supported contracts.

## Playback redraw feedback

The reported symptom was Task Manager GPU usage while playing media on the latest
tagged clients and server: about 7% on one machine and 20% on another. The latest
tag inspected was `v0.2.15`, source
`e4899e10a9485d8652ab7f14e2e347a562f4bfbb`.

The session projection extrapolated room position and attached a new monotonic
clock anchor on every worker poll. Both values participated in snapshot equality.
Every poll could therefore emit another main-window snapshot even with no new
protocol input. Applying that output changed the next `GuiRuntimeInput`, waking
the worker again; runtime output requested another repaint. The worker's 50 ms
idle wait did not cap this feedback because changed input wakes it immediately.

A headless regression using the real session produced 32 main-window snapshots
from 32 unchanged polls before the fix, and zero afterwards. The session now
retains a clock sample keyed by room, source playstate and its received timestamp.
A new source sample, pause, seek or room change invalidates it. The visible clock
still interpolates and requests its existing 100 ms display updates while the
sample is playing and fresh. The existing five-second stale-sample bound remains.
The 16 ms readiness spinner is a separate, temporary animation.

This demonstrates unnecessary GUI work, not a measured GPU reduction. No physical
Sorotte/mpv playback workload was captured during this implementation. The two
reported percentages have not been reproduced. Task Manager reports the busiest
engine for a process; record the process and engine before comparing GUI work
with mpv decoding/rendering. See Microsoft's
[Task Manager GPU explanation](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/).

## Native lifecycle and rendering

The pinned eframe 0.36.2 implementation invokes `App::logic` while a window is
hidden, but can skip `App::ui`. Output draining and automatic pending-operation
completion previously lived in `ui`. They now run in `logic`, including the
successful-update window-close effect. Queued output therefore cannot accumulate
solely because the window is minimized or occluded. Visible user actions still
pump the runtime after they are handled and consume its output before yielding,
requesting a repaint when that output changes shell state.

Headless tests call the actual eframe logic entrypoint with minimized input. They
check queued chat consumption, one-time pending pause dispatch and completion,
and successful versus failed update-launch behavior. They do not launch a native
window or establish physical display behavior.

The native renderer now takes the owned widget tree directly. It no longer walks
it into a second tree and clones that tree again before rendering. Native shell
construction builds only the active Room, Setup or Plugins body. The full tree is
retained for semantic inspection, with stable navigation labels and widget IDs.
Tests compare each active body with its full-tree counterpart and retain the
existing accessibility, layout and clock-timer checks.

The snapshot projection used to occur inside `debug_assert!`, so release builds
omitted its side effect before computing playlist selection. If a remotely
removed row was selected locally, the first update could leave selection on the
first row instead of the room's current row. Projection now runs unconditionally.
A regression applies a real playlist replacement and checks the resulting
selection in the same update. A controlled reproduction that omitted the former
assertion's side effect failed; the unconditional projection passed. This is a
source-level reproduction of the release omission, not a native release run.

## Concrete session ownership

`GuiSessionRuntimeAdapter` had one production implementation and a large set of
successful empty defaults. The owner now holds `GuiClientSession` directly.
The old `GuiClientCoreChatSessionRuntimeAdapter` name, dynamic session interface,
always-true pause capability and redundant forwarding methods are removed.
Session implementation files live under `runtime_stack/client_session`;
player synchronization payloads live in `player_sync_types.rs`.

Tests that supplied synthetic session results now drive the actual session with
Hello, playlist, participant, media-match and playback-barrier messages. They
retain deterministic transport and player boundaries. Read-only observations
record call ordering; three narrow test-only fault modes preserve telemetry,
unpause-finalization and seek-publication failure coverage. There is no alternate
successful session implementation.

The migrated cases cover controlled-room permissions, network receipt timestamps,
active settings surviving unsaved edits, playlist delivery fences, autoplay/EOF,
media matching, long network seek renewal across distinct media generations,
barrier commits and buffering pause/resume, and recovery playback-rate reset.
Several old fixtures bypassed the protocol parser or playback lifecycle. Their
replacements use valid media signatures and actual start acknowledgements and
barrier completion. Mutation path declarations follow the renamed source files;
no kill threshold or required behavior is relaxed.

## Qualification and remaining measurement

Implementation evidence is retained in `target/cleanup-evidence`, with failed
attempts kept separately. Focused rendering, hidden-window and real-session tests
and GUI Clippy passed before integration. The PR records the exact candidate's
full workspace, semantic, hosted, native and package qualification results.
Headless semantic checks do not stand in for required native GUI/playback checks.

Physical measurement remains a separate acceptance step for the reported GPU
symptom. Compare the tagged baseline and candidate release build on the same
machine and media, with the GUI and mpv process IDs recorded separately. Retain
GPU engine, driver, monitor refresh rate, DPI, window visibility, CPU and sampling
interval. Cover idle, paused, playing Room/Setup/Plugins, minimized and restored,
plus seek/buffering and readiness transitions. Observe redraw frequency alongside
GPU utilization so a remaining cause can be distinguished from this repaired
feedback loop. Do not infer a GPU percentage from repaint timer intervals or
Sandbox qualification.

## First native attempt and visible-frame follow-up

The first exact candidate, `9294ceff262eaba2be9f7ca20a44872cf46f31bc`, passed
local workspace, all-feature nextest, doctest and semantic validation. Its
[native attempt](https://github.com/ropbet-radbyt/sorotte/actions/runs/34752177099)
failed while waiting for `drag-window-target.mkv` in the accessibility tree.
Playback qualification was consequently skipped. The failed invocation, report,
job log and exported diagnostics are retained; the host removed the guest,
registration and token, and the watchdog completed. Automatic guest-side runner
unregistration was not attested on that failed attempt.

The threaded headless replay subsequently reproduced the missing entry: the
window switched to Room, but the playlist reverted to its placeholder. The GUI
records the dropped file's directory while the worker opens it. Submitting that
changed GUI input could replace the worker's completed playlist with the older
playlist still present in the GUI snapshot.

Input and output now share one locked handoff. Before accepting input, the handoff
projects any output the GUI has not consumed onto it. Output emitted after input
submission also updates that pending input. The worker takes the reconciled input
before processing its next command batch. This handles both race orders without
keeping an output journal or discarding GUI edits. Drained output is no longer
replayed onto later input, and the GUI's cached comparison snapshot remains
unchanged. Controlled tests cover both orders, draining after submission,
subsequent edits, input coalescing and an already completed configuration save.

A single-pass visible replay also showed that the refactor left output produced
by visible input queued until the next frame. Restoring the end-of-frame drain
and repaint request fixes that regression while preserving hidden-window logic.
Visible tests now keep a real dropped file accessible through subsequent frames
using both the synchronous and threaded real owner. The corrected release
candidate still requires fresh native qualification; the original failure and
the failing threaded replay remain recorded.

The requested tagged patch release is prepared as `v0.2.16`. Its qualification
must bind the corrected PR head and version. The earlier nonpublishing package
run was cancelled before provisioning its Windows guest because it targeted the
superseded version. Release publication follows the repository's qualified-PR
handoff: verify the ordinary merge, then tag the original qualified candidate and
publish its tested artifacts.
