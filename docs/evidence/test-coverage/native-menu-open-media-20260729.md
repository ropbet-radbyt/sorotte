# Native menu and Open Media evidence — 2026-07-29

This record closes TC-NATIVE-001 with a real interactive Windows experiment.
It supplements, rather than rewrites, the preserved failing baseline in
`native-baseline-20260728.md`.

## Contract

The required proof crosses four independently checked boundaries:

1. the product's typed menu-section model;
2. the actual egui widgets exported through AccessKit and Windows UIA;
3. stable-ID physical interaction with File -> Open Media; and
4. exact receipt of the selected path by an attached deterministic player.

The detached baseline must expose `menu.open_media` as disabled. The separate
attached scenario must expose it as enabled, invoke it through stable menu IDs,
transition to the room surface, and record the exact path at
`PlayerAdapter::open_file`. A keyboard shortcut, a visible room transition, or
a free-form completion string cannot substitute for those observations.

The structured report requires these outcomes:

```json
[
  {
    "capability_id": "native.menu.inventory",
    "outcome": "required-pass",
    "source": "uia-accesskit"
  },
  {
    "capability_id": "native.menu.open-media.detached",
    "outcome": "required-pass",
    "source": "uia-accesskit"
  },
  {
    "capability_id": "native.menu.open-media.attached",
    "outcome": "required-pass",
    "source": "uia-accesskit+deterministic-test-player"
  }
]
```

The validator additionally requires each outcome's exact reviewed evidence
array and rejects missing, skipped, duplicate, forged-source, or unreviewed
capabilities.

## Fresh-binary required-pass

Command:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 `
  -Json -TimeoutMs 80000 `
  --scenario baseline --scenario menu-open-media
```

Provenance:

- artifact:
  `target/verification/gui-native-smoke/20260729T031013862Z-47644`;
- required scenarios: `baseline`, `menu-open-media`;
- binary provenance: `rebuilt-debug`;
- GUI SHA-256 before and after:
  `4d2195914472228541507c7ad4622adb3e622a231a4741f714179240d8394551`;
- producer exit: `0`;
- strict status: `required-pass`;
- reported native duration: 23,566 ms;
- wrapper runner duration: 23,603 ms;
- native stderr: 0 bytes;
- raw report SHA-256:
  `a73688f2f489c8a011a21fc6a12e1f1948ba431b9f533875af127c0165c258f3`;
- strict summary SHA-256:
  `b6458650ee229846ced5a737c370be10452aa6e9a2e329f545d63e344750433e`.

The report enumerated exactly:

```text
menu.section.file
menu.section.playback
menu.section.advanced
menu.section.window
menu.section.help
```

It also contained all three attached scenario markers and the detached marker:

```text
open-media-file-detached-disabled
menu-open-media-enabled
menu-open-media-invoked-by-automation-id
menu-open-media-runtime-observed
```

The final strengthened interaction sequence passed three consecutive combined
runs in 23,591, 23,339, and 23,566 ms. Their artifact directories are:

```text
target/verification/gui-native-smoke/20260729T030907561Z-15708
target/verification/gui-native-smoke/20260729T030944123Z-54784
target/verification/gui-native-smoke/20260729T031013862Z-47644
```

## Interaction experiments

Tightening the enabled-state probe produced two useful harness failures, both
with screenshot/UIA evidence:

- clicking an already-open egui menu section did not dismiss its popup; the
  screenshot at
  `target/verification/gui-native-smoke/20260729T030322805Z-55556`
  showed File and its leaf actions still open;
- Escape dismissed the popup, but left the File control focused in menu
  navigation mode, so the next physical click did not reopen it; evidence:
  `target/verification/gui-native-smoke/20260729T030516681Z-55324`.

These were native-driver assumptions, not product defects. The final probe
uses Escape, verifies the leaf is absent through UIA, then clicks the stable
`configuration-root` Setup surface to reset focus. The actual command path
still opens File once and clicks the exact `menu.open_media` leaf once.

## Adversarial failure-artifact experiment

The final harness was then run against an existing pre-change GUI binary by
supplying `-BinaryPath`. The strict producer rejected it immediately:

```text
accessibility menu inventory requires exactly one "menu.section.file" node;
observed 0
```

Artifact:
`target/verification/gui-native-smoke/20260729T024239593Z-11936`.

Before terminating the still-live process, the harness retained:

- `failure-primary.png`: 5,611,593 bytes, SHA-256
  `9ceb066a7eaa3c8461c76c06b2bbd86db3072f166ba030ec182f4cf2e87ad52e`;
- `failure-primary-accessibility.json`: 35,188 bytes, SHA-256
  `720f79f02e57416cf7ac1d3a210f5969f330ed588fa16c08591a45c11bc7395e`;
- `native-stderr.log`: 0 bytes.

The screenshot visibly contained the old menu, while the UIA tree confirmed
that the required typed identities were absent. This proves both sides of the
contract: current code passes for the intended reason, and a visually similar
pre-change implementation fails with reviewable native evidence.

## Broader default-inventory experiment

TC-NATIVE-001 closes the menu/Open Media slice, not the complete native
inventory. The hardened wrapper's default ten-scenario matrix was run twice.
Both attempts correctly remained red and surfaced additional findings without
changing product behavior.

The first run failed after 57,931 ms in `live-python`:

```text
artifact:
  target/verification/gui-native-smoke/20260729T032222583Z-54624
error:
  timed out waiting for accessibility name "interop-py-peer"
native stderr: 2,615 bytes
```

That run occurred before secondary-scenario capture was wired into every
cleanup path. After adding capture-before-termination to transport, loopback,
both missing-media paths, both drag/drop paths, both Python-peer paths, and all
relaunch windows, the full inventory was run again. It advanced through
`live-python` and failed after 59,418 ms on the same missing peer in
`controlled-room`:

```text
artifact:
  target/verification/gui-native-smoke/20260729T032952983Z-53868
screenshot:
  failure-controlled-room.png
  5,611,593 bytes
  sha256 962c73b222aa0f2e175024a62951643b31f5a04de6f182b21d94fa00a22acb43
redacted UIA:
  failure-controlled-room-accessibility.json
  31,394 bytes / 107 nodes
  sha256 0309d9f9cd93f038ae7899e0daec0a891338a038d5b7627bebc7b6a47f0a529c
```

The screenshot and UIA tree show `interop-gui-user` alone in the test-owned
room; `interop-py-peer` is absent. Both full runs also contain repeated
placeholder DNS resolution and negative TLS diagnostics, which the strict
wrapper rejects. These failures were assigned TC-HARNESS-007 and
TC-HARNESS-009.

An isolated `controlled-room` diagnostic then surfaced two additional
intermittent failures in the mandatory primary baseline:

- `20260729T033220059Z-4172`: File was focused, but its popup and `menu.exit`
  never appeared; the failure occurred after 5,850 ms with empty stderr
  (TC-HARNESS-008);
- `20260729T033324498Z-53816`: the runner found and clicked `menu.exit`, but
  the GUI process remained alive for the 80-second timeout. Its retained
  screenshot shows the still-present window in a disabled/closing-looking
  state, and stderr is empty (TC-NATIVE-002).

These later failures do not invalidate the three consecutive required-pass
menu/Open Media trials. At that point they proved the complete native inventory
was not yet a reliable required lane, and that the strict harness was correctly
retaining rather than concealing the fact.

## Full-inventory resolution experiment

The four follow-up fixes deliberately changed the contracts rather than
allowlisting their symptoms:

1. the Python probe gained `wait_for_user_presence`; initial, reconnect, and
   controlled-room setup require Python login, Python observation of the GUI
   roster entry, then UIA observation of the Python roster entry under one
   bounded deadline;
2. At this stage, Windows physical input required exact foreground ownership and a UIA
   hit test at the click coordinate, sends down/up separately, and requires the
   requested popup leaf. The baseline performs 25 open/dismiss cycles and fails
   if even its narrowly guarded redelivery path is used;
3. every native launch declares a typed detached, in-process loopback, or TCP
   loopback mode. TCP bootstrap rejects non-loopback hosts before spawn and
   ordinary test fixtures use plaintext rather than manufacturing unrelated
   TLS warnings;
4. File -> Exit requests explicit runtime cancellation and requires a bounded
   five-event lifecycle trace before the process-exit proof is accepted.

Three consecutive fresh-binary baseline trials passed:

```text
artifact                                      report sha256
20260729T043337574Z-52092  e1170e578be98d16733e811b86797764b7eecc4684d0f45ff39f1c9d44da0db0
20260729T043722955Z-17640  363a4e03eff8eaa4b83ad9924d331f64bd1236a4a72f96897e09c3477572f848
20260729T043822287Z-47496  f42753fbaeb2d6a76954118139f78cc46077f50e16fc7d5b2c41fc615b6c9e04
```

Each report contained:

```text
menu-input-stress-25
menu-input-recoveries=0
file-exit
file-exit-lifecycle-observed
native.menu.physical-input = required-pass
native.shutdown.file-exit = required-pass
native stderr = 0 bytes
```

This is 75 consecutive acknowledged physical menu transactions, with no
scenario retry and no recovery hidden by the producer.

The first combined post-fix run was intentionally retained when the strict
stderr oracle found a second-order fixture defect:

```text
artifact:
  target/verification/gui-native-smoke/20260729T043944366Z-52548
behavioral result:
  ok; all ten scenario assertions completed
strict result:
  failure
native stderr:
  330 bytes
raw report sha256:
  ef65d7e6fe5d2409552a87ca7ead6aad2fa24b4e39593140a8697ecc62453dac
```

The missing-media continuation took longer than 15 seconds, while its mock
session server silently expired after 10 seconds. The GUI therefore received a
forced close and attempted to reconnect after the UI assertion had passed.
Replacing the arbitrary fixture lifetime with explicit scenario release made
the ownership causal: the server remains live until the GUI closes and its
process is joined.

Both affected connectivity classes then passed independently with empty
stderr:

```text
transport:
  target/verification/gui-native-smoke/20260729T044524472Z-45404
  report sha256 82813145e4f2dd4386029b1b396af0703cda38866c42e003a9cdb841c59c3098

missing-media-continue:
  target/verification/gui-native-smoke/20260729T044615793Z-50012
  native duration 15,855 ms
  report sha256 e0d4365415a7f4ed4291fd31c912af47c979040a6b1088d445796337dd73b944
```

The final default inventory crossed all ten scenarios in one producer:

```text
artifact:
  target/verification/gui-native-smoke/20260729T044650510Z-42024
required scenarios:
  baseline, relaunch, drag-drop, loopback, menu-open-media, live-python,
  controlled-room, detached-missing-media, missing-media-continue, transport
producer exit:
  0
strict status:
  required-pass
native duration:
  111,871 ms
native stderr:
  0 bytes
raw report:
  3,856 bytes
raw report sha256:
  ba33aa0991001ebd83507a3ca0c23888ad62bf0f0811d7d0566c62ff8a9eb62e
```

That result resolves TC-HARNESS-007 through TC-HARNESS-009 and TC-NATIVE-002
as local native contracts. Promotion to a required interactive Windows CI lane
remains a deployment decision; it is no longer blocked by a known red
inventory.

A second consecutive default-inventory run independently confirmed the result:

```text
artifact:
  target/verification/gui-native-smoke/20260729T045502691Z-56304
strict status:
  required-pass
native duration:
  114,482 ms
native stderr:
  0 bytes
raw report sha256:
  8fc85391633226dfdebe3df1bf51f9c90e2a9a22975e251b0acc7f66212edaf3
strict summary sha256:
  b86102fa5e8d2004db0ba78b320debbc1f60296e7f0b67c04ffcb48c300596fe
```

## Subsequent supersession

A later current-source inventory reopened TC-HARNESS-008 and proved that the
guarded toggle redelivery and zero-coordinate button events were insufficient.
The historical artifacts above remain valid evidence for that implementation;
they are not the final input design. The retained causal experiments and final
atomic absolute-coordinate, single-delivery proof are in
[`native-input-ownership-20260729.md`](native-input-ownership-20260729.md).
