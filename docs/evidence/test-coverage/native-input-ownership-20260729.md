# Native input ownership and state acknowledgement — 2026-07-29

## Scope

This record closes TC-HARNESS-010, closes TC-HARNESS-011, and records the
causal reopening and final resolution of TC-HARNESS-008. It covers Windows UI
Automation identity, AccessKit focus, egui content state, physical Win32 input,
virtual-desktop coordinates, strict native evidence, and automatic Cargo test
enrollment.

## Retained configuration-tab failure

The final current-source inventory first failed at:

```text
artifact: target/verification/gui-native-smoke/20260729T053110335Z-34976
duration before failure: 29.1 seconds
error: timed out waiting for accessibility name "Show OSD"
native stderr: 0 bytes
```

Artifacts:

| File | Bytes | SHA-256 |
|---|---:|---|
| `failure-primary.png` | 5,611,593 | `eab687fcd634ee91074f3a38d63017e00a0a2835ef630f4d77f26808a66d4349` |
| `failure-primary-accessibility.json` | 53,779 | `e0ad80cfebc1349d8236b0da5f5f1ad99d8226ba1ac2937bdc1c62c5b29639ec` |
| `native-report.json` | 672 | `af7a182c702fb185b09805ec6fe7ac4ff62ddebcbcc37ff3598eb382bcdf40b9` |

The screenshot showed Playback & Search settings. The accessibility tree
showed:

```text
configuration:tab:playback-search
  enabled=true, focused=false, offscreen=false, bounds=[833,785,1111,833]

configuration:tab:interface-system
  enabled=true, focused=true, offscreen=false, bounds=[1414,785,1692,833]

Show OSD
  absent
```

Both an accessibility action and an exact physical click had returned
success. The artifact therefore proves that input-API completion and focus are
not sufficient postconditions for tab selection.

## Tab resolution

Top-tab activation now uses a bounded, state-acknowledged sequence:

1. return immediately if the expected content already exists;
2. attempt accessibility activation, then require expected content;
3. attempt exact physical input, then require expected content;
4. re-resolve the exact enabled and visible automation node, foreground its
   HWND, set and verify keyboard focus, send Enter down/up, then require the
   expected content;
5. on failure, report every strategy diagnostic and the final accessibility
   snapshot.

The baseline deliberately uses the focused-keyboard path to switch to
Interface & System and requires both `Show OSD` and `Language`. Successful
reports contain:

```text
config-tab-focused-keyboard-activation
```

Two unit tests prove escalation happens only after a missing state
acknowledgement and that all strategy failures remain in diagnostics.

## Retained physical-input experiments

The first post-tab-fix complete inventory reopened TC-HARNESS-008:

| Artifact | Experiment | Result |
|---|---|---|
| `20260729T055406467Z-55552` | historical foreground + UIA focus + exact hit test + one guarded redelivery | both deliveries left `menu.exit` absent |
| `20260729T055813632Z-45932` | remove UIA focus and redelivery; one pure physical click | `menu.exit` still absent |
| `20260729T060034291Z-42000` | add exact live-cursor acknowledgement before down | intended `(64,104)`, observed `(0,59)` |

Artifact hashes:

| Artifact | File | Bytes | SHA-256 |
|---|---|---:|---|
| `20260729T055406467Z-55552` | `failure-primary.png` | 5,611,593 | `5270dfac26ccd3f6571d8b698846de7800364fcabf34ddb1463a3887ae150c0e` |
|  | `failure-primary-accessibility.json` | 28,125 | `8f22b5368e240c602a3041bc9ebff3d1ad9f5cbf3cf95816f8798af56c413743` |
|  | `native-report.json` | 232 | `a21cf49457e4c5ee8d354dbbe43989519e5ae686043f14d8b33d054c2095364b` |
| `20260729T055813632Z-45932` | `failure-primary.png` | 5,611,593 | `9010d5aa8d10bab403f4360efd29f8aa4c28a6f867c8d7b77e70a4326755dc59` |
|  | `failure-primary-accessibility.json` | 53,316 | `ffb93fe65fb45d5f6c41310b57c91a721f5f1af226448753905281fe7243fe42` |
|  | `native-report.json` | 212 | `e1593f0323f9a7659c0902f2a2cd0dc618c07070e48f348b681a397e724a67ab` |
| `20260729T060034291Z-42000` | `failure-primary.png` | 5,611,593 | `990c3e573229b2a01908649cf929c652fbf07bd54ac8dacd41a833dfcc6db8b4` |
|  | `failure-primary-accessibility.json` | 53,366 | `0762b2ef5454055525be228cc829112c5ea0a75613f7e3a508932c50f699f7db` |
|  | `native-report.json` | 259 | `0d531a0d8fa11f0934fc259e0f4086b44f588e4c4ef0815f95176cbb1178068c` |

The decisive observation explains why the historical exact hit test was
unsound. `ElementFromPoint` validated the intended coordinate, while a
zero-coordinate mouse button `SendInput` used the desktop's shared live
cursor. Another desktop actor moved that cursor before button delivery.

The earlier guarded retry was also unsafe: sampled absence of a toggle's popup
does not prove its first click is no longer queued. A delayed first open can be
closed by the second click. This is why the final design contains no menu
redelivery.

## Physical-input resolution

The Windows driver now:

1. foregrounds and acknowledges the exact owning HWND;
2. resolves an enabled and visible target by name or stable automation ID;
3. primes a pointer transition outside the target;
4. proves the target at its center with UIA `ElementFromPoint` and ancestor
   identity matching;
5. normalizes the target center against the full virtual desktop, including
   negative monitor origins;
6. atomically submits absolute move-to-center plus mouse-down;
7. after the inter-frame hold, atomically submits absolute move-to-center plus
   mouse-up;
8. waits for the requested UIA state under the original deadline.

UIA `SetFocus` is not mixed into physical clicks. Each menu toggle is delivered
once. The strict capability is:

```text
capability: native.menu.physical-input
source: uia-hit-test+win32-sendinput
evidence:
  - menu-input-stress-25
  - menu-input-single-delivery
```

Two unit tests cover normalization endpoints, a negative-origin
multi-monitor desktop, invalid spans, and out-of-range coordinates.

## Automatic contract-test enrollment

The native smoke binary contained unit contracts but declared `test = false`.
They ran only when the binary was explicitly named. The target now declares
`test = true`, so `cargo test --workspace --all-features` must discover the
native harness. The focused target passes all 25 tests.

## Repeated real-Windows proof

Three consecutive focused baselines passed the final source:

| Artifact | Scenarios | Duration | Native stderr | Report SHA-256 |
|---|---:|---:|---:|---|
| `20260729T060444829Z-53772` | 1 | 46,695 ms | 0 bytes | `bcb308701498a44ec252f7453099379ea59322f77bb5f91f676f7944dc9c7f3c` |
| `20260729T060546798Z-8848` | 1 | 46,922 ms | 0 bytes | `f33b6cdb6e4faf9829cefe64a4eac1834104f20e46bd19a7be7b1a65d2043290` |
| `20260729T060643945Z-32828` | 1 | 46,548 ms | 0 bytes | `8bfba2981b050ea54e0e8e1f2add2ed295011ac7fa890b3699c656bb97a2a5a9` |

That is 75 consecutive single-delivery File-menu transactions. Each run also
passed the deliberate keyboard-tab switch and exact five-event shutdown trace.

Two consecutive complete inventories then passed all ten required scenarios:

| Artifact | Duration | Native stderr | Report SHA-256 |
|---|---:|---:|---|
| `20260729T060756380Z-55276` | 110,173 ms | 0 bytes | `9068e7f32787c84a8782017dcb3a2517adf56feeb47ba86fabb0762f73767cbe` |
| `20260729T061005422Z-54068` | 109,614 ms | 0 bytes | `01e61530d011f38a4fd274520c8445a95edde352dd131f4a2ad414c1243fe7aa` |

Both complete reports include baseline, relaunch, drag-drop, loopback,
menu-open-media, live-python, controlled-room, detached-missing-media,
missing-media-continue, and transport; all strict capability outcomes are
`required-pass`, the primary GUI is closed, and no test-owned GUI or Python
process remains.

## Final broad gates

The exact documented source then passed:

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `git diff --check` | pass; only existing LF/CRLF conversion notices |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --workspace --all-features` | pass in 185.7 seconds; output includes `sorotte-gui-native-smoke` and all six real server release verifiers |
| `python -m unittest discover -s scripts/tests -p "test_*.py"` | 252 passed in 11.7 seconds |
| `scripts/gui-semantic-suite.ps1 -Json` | 14/14 passed in 21.9 seconds |

The final semantic result includes `live-python-peer-connect-flow` and
`live-python-peer-controlled-room-flow`; the native result includes the same
live interoperability, controlled-room, reconnect, missing-media, and transport
boundaries through real Windows UIA.
