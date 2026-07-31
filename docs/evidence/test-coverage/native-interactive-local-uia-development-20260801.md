# Native interactive local UIA development mode — 2026-08-01

## Outcome and authority boundary

Sorotte now has a local Windows native-smoke mode that does not inject mouse,
keyboard, wheel, or cursor movement into the active desktop:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-native-smoke.ps1 `
  -Json `
  -TimeoutMs 80000 `
  -InputMode UiaOnly
```

The committed implementation is:

```text
374b6a6f7edefef6f7db44422d16e9c11dd6f8bf
```

This is deliberately local, bounded, non-authoritative development evidence.
It does not replace the ten-scenario physical native contract. Strict physical
input remains the default, and the dispatch-only interactive workflow now
passes `-InputMode StrictPhysical` explicitly.

## What the local mode executes

The fixed UIA-only inventory:

1. launches the freshly built native Sorotte GUI with isolated configuration;
2. reads the real AccessKit tree through Windows UI Automation;
3. requires the exact File, Playback, Advanced, Window, and Help automation
   identities;
4. opens File through UI Automation patterns;
5. invokes Exit through UI Automation patterns;
6. requires natural process exit and the complete ordered lifecycle trace; and
7. requires zero desktop-input attempts.

It does not accept `--scenario` or `--keep-open`. Physical menu stress,
focused-keyboard activation, editing, scrolling, drag input, and every other
strict scenario remain outside this local inventory.

## Fail-closed desktop-input boundary

`PlatformNativeGuiDriver` owns an explicit `NativeInputMode`. Every native
`SendInput` dispatch passes through one central guard. The only direct
`SetCursorPos` path is guarded before the first cursor move. In `uia-only`, the
guard records the attempt and returns an error before either API can change the
desktop. A successful report additionally requires the recorded attempt count
to remain zero.

Source policy scans the complete native-smoke Rust tree and requires exactly
one `SendInput(...)` call site in `windows_input.rs`, with the guard before the
dispatch. It also requires the cursor guard to precede `SetCursorPos`.

The report records:

```text
input_mode = uia-only
interaction_contract = local-uia-only-non-authoritative
native.menu.physical-input = optional-skip
native.input.focused-keyboard = optional-skip
reason = local-uia-mode
win32-sendinput = disabled
desktop-input-attempt-count = 0
```

The validator emits `status=local-pass` and `authoritative=false`. Conversely,
strict validation requires `input_mode=strict-physical`, exact required
scenarios, and the existing physical capability sources. A UIA-only report
therefore cannot satisfy strict CI even if copied into a strict artifact root.

## Committed-source live result

Canonical local bundle:

```text
target/verification/gui-native-smoke/20260731T220008415Z-13100
```

The UTC bundle timestamp is 2026-07-31 because the local date was already
2026-08-01 in Australia/Sydney.

Observed result:

```text
result:                         ok
input mode:                     uia-only
window title:                   Sorotte GUI
accessible names:               100
menu identities:                5/5 exact
UIA File -> Exit:               passed
lifecycle trace:                complete
desktop-input attempts:         0
native stderr bytes:            0
producer exit:                  0
validator status:               local-pass
validator authoritative:        false
remaining Sorotte GUI processes: 0
GUI SHA-256 before/after:        1b7efb4867490143a7b0da67f1939a6711c86809f9fca47aef097213c5c65a21
runner duration:                 1,762 ms
reported interaction duration:  1,296 ms
```

Artifact inventory:

| File | Bytes | SHA-256 |
|---|---:|---|
| `native-report.json` | 1,607 | `c3d25ddb16fadaaab4ef18f66b8fd88d13f0dab21a858ce29b87fcd220626d42` |
| `contract-summary.json` | 514 | `3dec5bba6fc3e42a4faf9bb15828fe38bac431ebe1f707fb12d0b9081cf59d72` |
| `invocation.json` | 1,880 | `092855b0c227b5138a5606111b9a4581ebaffadfd8002a4ab2d767c0920b270b` |
| `native-stderr.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `build-stdout.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `build-stderr.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `harness-build-stdout.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `harness-build-stderr.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

Three earlier green development bundles are intentionally retained:

```text
target/verification/gui-native-smoke/20260731T213610609Z-43232
target/verification/gui-native-smoke/20260731T214102021Z-24724
target/verification/gui-native-smoke/20260731T214657652Z-48968
```

The first preceded the additional cursor-movement guard. The second used the
final guard but preceded the implementation commit. The third was bound to the
initial implementation commit, before the Windows-only guard members were
correctly cfg-gated for warning-denied Linux builds. None is canonical.

## Validation

The completed gates were:

```text
cargo fmt --all --check
cargo +1.97.1 clippy --locked -p sorotte-gui --bin sorotte-gui-native-smoke --all-features -- -D warnings (Ubuntu/WSL)
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python -m unittest discover -s scripts/tests -p "test_*.py" -v
C:\Users\shaun\go\bin\actionlint.exe .github\workflows\gui-native-interactive.yml
PowerShell AST parse of scripts/gui-native-smoke.ps1
git diff --check
```

Results:

```text
focused native-smoke Rust tests:       50/50
focused native/workflow Python tests:  46/46
complete Python policy suite:          536/536
workspace tests and doctests:          passed
workspace warning-denied Clippy:        passed
Linux native-smoke warning-denied lint: passed
formatting/actionlint/PowerShell parse: passed
```

The first exact-head hosted Linux lint exposed that the input-mode field and
desktop-input guard method are Windows-only outside unit tests. Commit
`374b6a6f7edefef6f7db44422d16e9c11dd6f8bf` cfg-gates only those members. The
exact affected all-feature target then passed warning-denied Clippy on both
Ubuntu/WSL and Windows before this canonical campaign was recorded.

One focused parallel native-smoke test run transiently received Windows socket
abort 10053 while a negative real-mpv fixture wrote after its server had
already rejected the connection. Its exact retry passed 1/1, and the subsequent
all-feature workspace run passed the complete native-smoke binary suite. No
input-mode source change was made for that unrelated timing observation.

## Limits and remaining system proof

The native harness does not install global keyboard/mouse hooks or capture
whole-PC input. This mode additionally prevents its desktop-wide input
injection paths. It may still display, resize, or foreground the Sorotte window
for UI Automation and failure capture, so it is convenient rather than fully
background/headless.

UIA-only does not prove hit testing, real keyboard focus, pointer routing,
physical menu delivery, DPI-specific physical coordinates, or resistance to
other desktop actors. Those remain properties of `strict-physical`. The
outstanding test-plan item is still operational: provision and independently
attest a one-job ephemeral interactive Windows runner, execute the strict
zero-stderr inventory repeatedly, retain its artifacts, verify destruction,
and only then consider gate promotion.
