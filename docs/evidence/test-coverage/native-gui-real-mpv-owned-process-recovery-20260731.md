# Native GUI real-mpv owned-process recovery — 2026-07-31

## Scope and safety boundary

This slice extends the existing opt-in Windows native GUI-to-real-mpv
capability without changing the healthy vertical's default contract. It uses:

- the locally installed exact `C:\Program Files\mpv\mpv.exe`;
- one generated 12-second silent PCM WAV under a fresh ignored artifact root;
- an isolated Sorotte INI, APPDATA root, mpv Lua observer, and mpv log;
- one OS-assigned IPv4 loopback Sorotte session; and
- only GUI- and harness-owned processes.

There is no external media, remote network target, credential, persistence,
privilege, or product-behavior change. The injected fault calls
`TerminateProcess` only after the harness has attested the exact mpv PID as a
direct child of the launched GUI and matched its running image digest to the
preflight binary digest.

## Contract

The default invocation of `scripts/gui-real-mpv-vertical.ps1` remains the
healthy 13-assertion, 10-artifact vertical. Recovery is a separate opt-in
inventory selected by `-ExerciseOwnedMpvRecovery` and
`--exercise-owned-mpv-recovery`.

After the healthy vertical has loaded the generated WAV and proved real
Play/Pause, the recovery inventory requires:

1. terminate and reap only the already attested initial mpv PID;
2. continuously require that PID to remain absent while polling at 50 ms;
3. observe the active-session runtime's bounded automatic replacement through
   the replacement Lua `pause=true` startup observation;
4. require a different positive PID and product-generated IPC endpoint;
5. re-attest the same GUI parent, exact image path, exact SHA-256, supported
   version, and isolated arguments;
6. prove the native GUI remains on the active room without a manual Retry
   action;
7. physically invoke Open Media again and require the replacement PID to load
   the exact generated WAV;
8. physically invoke GUI Play and Pause and require ordered replacement-mpv
   observations;
9. reject every initial-PID or foreign-PID observation at and after the
   termination boundary; and
10. use the native Exit leaf and prove the GUI, initial mpv, replacement mpv,
    loopback server thread, and loopback socket are all released.

The recovery report has an exact 20-assertion inventory and 13-artifact
inventory. The Python validator rejects missing, extra, or reordered
assertions and artifacts; reused PID/IPC identities; binary drift; non-loopback
session endpoints; Hello drift; manual-retry substitution; stale or foreign
post-boundary observations; incomplete cleanup; and artifact hash drift.

## Preserved red oracle-assumption bundle

The first real run used an incorrect test oracle: it expected the manual
`ExitedAfterLaunch` modal before recovery. Production instead calls
`ensure_configured_player_attached_for_active_session()` in the same runtime
pump after detecting the child exit. The runtime automatically launched a
replacement before projecting a manual-retry issue.

The complete red bundle is preserved at:

```text
target/verification/gui-real-mpv-owned-process-recovery/20260730T222451895Z-64648
```

It proves the assumption failure rather than a product defect:

- initial attested mpv PID: `58260`;
- automatically launched replacement PID visible in the retained observation
  and mpv log: `41532`;
- failure: timed out waiting for the disproved
  `mpv closed unexpectedly` UI oracle;
- `real-mpv-state.json` SHA-256:
  `127eb29ae6bb791b8e7a1c3a3855db4941da0252764c6a314914e13bc41fd355`;
- full failure accessibility JSON SHA-256:
  `33d975b8acabb777d018fd730870e7866121a65340db3c6f0974a6ec2d8b8922`.

No red artifact was overwritten or deleted. The harness oracle was corrected;
production code was not changed.

## Canonical green capability

Command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts\gui-real-mpv-vertical.ps1 `
  -MpvPath "C:\Program Files\mpv\mpv.exe" `
  -TimeoutMs 30000 `
  -ExerciseOwnedMpvRecovery
```

Result: exit `0`, producer `0`, strict validator `0`.

Fresh bundle:

```text
target/verification/gui-real-mpv-owned-process-recovery/20260731T000220834Z-11868
```

Exact identities and transitions:

```text
implementation:    b9e8c2bbee4aca55beb07f4a2ebaacb3d67ffb46
code HEAD:         3cd64ce2e2f0a51a7e31b9862a6bde9cd40c6f16
GUI SHA-256:       b805d7745e43245fd0941aa33170df710cfa048d13810d5d40f7d34e6ce0e279
mpv SHA-256:       2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
mpv version:       mpv v0.41.0-877-ge5486b96d
GUI PID:           62952
initial mpv PID:   23660
replacement PID:  45732
initial IPC:       \\.\pipe\sorotte-gui-mpv-62952-1785456156128
replacement IPC:   \\.\pipe\sorotte-gui-mpv-62952-1785456165943
session listener:  127.0.0.1:57250
session peer:      127.0.0.1:57251
relaunch bound:    12000 ms
runner duration:   27886 ms
report duration:   27453 ms
assertions:        20
artifacts:         13
```

The exact post-termination observation suffix was:

```text
index 4  PID 45732  pause=true    automatic replacement start
index 5  PID 45732  file-loaded   exact generated-silence.wav
index 6  PID 45732  pause=false   GUI Play
index 7  PID 45732  pause=true    GUI Pause
```

There was no initial-PID or foreign-PID observation in that suffix. After the
native Exit path completed, PIDs `23660`, `45732`, and `62952` were each
independently confirmed absent.

Bundle binding:

```text
268fa8cb5e81b79f6dcdd1d1e0d7ac7a3504356dc091d5eb1b67bae122ed54bc  harness-report.json
ab15719f303ce33e427e25bd6b672972b11a38ca175099c98c2f93350b57ec94  contract-summary.json
ecd6c9099fbbb9a832353433047ab8daa2b5ae7ddc4096bf58f0e1c87cafc8ef  invocation.json
```

The strict summary records `result=passed`, `recovery_exercised=true`,
`assertion_count=20`, and `artifact_count=13`. The invocation binds identical
GUI digests before and after execution. This final bundle was created only
after repository formatting, actionlint, all 496 Python infrastructure tests,
mutation/known-defect policy, warning-denied workspace Clippy, and the complete
locked all-feature workspace test suite had passed.

## Healthy default regression

The unchanged default inventory was rerun after the recovery-only validator
path was finalized:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts\gui-real-mpv-vertical.ps1 `
  -MpvPath "C:\Program Files\mpv\mpv.exe" `
  -BinaryPath target\debug\sorotte-gui.exe `
  -TimeoutMs 30000
```

Result: exit `0`, producer `0`, strict validator `0`.

```text
target/verification/gui-real-mpv-vertical/20260731T000311349Z-65428
```

The strict summary remained the original `13` assertions and `10` artifacts
and contained no recovery-only field. The native Exit leaf reaped GUI PID
`64584` and its owned mpv PID `54360`; both were independently confirmed
absent after the wrapper returned.

```text
b0663f0856c19d8a74a31498ae9722ed4feb85e3c1a6a00bd04ec5b51420aa90  harness-report.json
6fc9b6fae64fcd7027e5d2b1b811b198cf4b692129aa15c924646e2715f8bda6  contract-summary.json
e73138b1b5600a67d640cc01756440523d0e54db38d0d1a03420fb047edd05fb  invocation.json
```

The healthy invocation used the caller-supplied final GUI binary, recorded the
same `b805d7745e43245fd0941aa33170df710cfa048d13810d5d40f7d34e6ce0e279`
digest before and after execution, and completed its runner in 16,503 ms
(16,480 ms inside the report).

## Final post-gate confirmation

After the later generated-compatibility, Unix-kernel, and faulting-HTTP
coverage slices completed every build-producing validation gate, the
owned-process inventory was executed again:

```text
target/verification/gui-real-mpv-owned-process-recovery/20260731T045019794Z-49868
```

It passed all 20 assertions with all 13 exact artifacts in 25,435 ms. GUI PID
`22372` initially owned exact mpv PID `61396` through
`\\.\pipe\sorotte-gui-mpv-22372-1785473424694`. The harness terminated only
that attested child. Sorotte automatically launched exact mpv PID `48892`
through the distinct endpoint
`\\.\pipe\sorotte-gui-mpv-22372-1785473433357`, without a manual retry.
Replacement observations proved file load, Play, and Pause; the old process
remained terminated. Native Exit reaped the replacement and GUI and released
the loopback session fixture.

```text
673dda5226c433950d3074cb4f1b2b6d222802eda6e30cc8a9b5d6e0ef12271c  final GUI before/after
2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0  initial/replacement mpv
2791765b6d120cb3f7c3dc640e7125b67726d398899478461e54d2d44262ba41  harness-report.json
b9e038fed7004237e8e9faef74bf911ab7e51c686d7ddc0fccb4b280439760df  contract-summary.json
198746e33f13666cd0a8d280afae437a963701751190d44bc57dec990baf47df  invocation.json
258ff056a100baf347b00a6a4db96b01ce604cde4a5e1d6482757369fe3bb18  owned-mpv-recovery.json
```

## Focused validation

Environment:

```text
Microsoft Windows 11 Pro 10.0.26200 build 26200
rustc 1.97.1 (8bab26f4f 2026-07-14), x86_64-pc-windows-msvc, LLVM 22.1.6
cargo 1.97.1 (c980f4866 2026-06-30)
Python 3.13.5
```

Commands and results:

```powershell
python -m py_compile `
  scripts\gui_real_mpv_vertical_contract.py `
  scripts\tests\test_gui_real_mpv_vertical_contract.py
# passed

python -m unittest scripts.tests.test_gui_real_mpv_vertical_contract -v
# 12/12 passed

cargo test --locked -p sorotte-gui --features gui-native-smoke `
  --bin sorotte-gui-native-smoke real_mpv_ -- --nocapture
# 6/6 passed

cargo check --locked -p sorotte-gui --features gui-native-smoke `
  --bin sorotte-gui-native-smoke
# passed

cargo clippy --locked -p sorotte-gui --all-targets `
  --features gui-native-smoke -- -D warnings
# passed

rustfmt +1.97.1 --edition 2024 --check `
  crates\sorotte-gui\src\bin\sorotte-gui-native-smoke\`
native_smoke_runner\real_mpv_vertical.rs
# passed
```

Final integrated validation before the two canonical native campaigns:

```text
cargo fmt --all -- --check
# passed

actionlint -config-file .github/actionlint.yaml \
  .github/workflows/rust-ci.yml .github/workflows/rust-fuzz.yml
# passed

python -m unittest discover -s scripts/tests -p "test_*.py" -v
# 496/496 passed in 22.380 seconds

python scripts/mutation_ci.py validate --repo-root . \
  --policy coverage/mutation-policy.toml
# 10 shards, 17 exact accepted unviables

python scripts/known_defect_policy.py validate \
  --registry coverage/known-defects.toml --repo-root . \
  --catalog coverage/behaviors.toml
# 0 defects, 0 characterizations

cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
# passed in 15.8 seconds

cargo test --locked --workspace --all-features
# passed on the first attempt in 257.5 seconds
```

Final implementation bindings:

```text
8e0d93ea3e18fa28fbe40780822238190923ea34cd6e069b1eee2faf9f9eeb5f  real_mpv_vertical.rs
c265588d54fcc83f13b991d243375db62f33f0fccd1524b85f2f20de924465cc  gui-real-mpv-vertical.ps1
cc6d7fc009bf43ddde003f877f874622878c32ccf4b6cea493c73c516bcd7a5f  gui_real_mpv_vertical_contract.py
e5c1bbf27d7a51509973d04456c6d874c80610e848e6309f5f2c5bed072b1ebf  test_gui_real_mpv_vertical_contract.py
```

## Limitations

- This proves one bounded automatic-relaunch cycle on Windows with this exact
  local mpv build. It does not claim Unix native GUI behavior.
- It tests active-session automatic managed-player replacement. It does not
  claim the detached/manual Retry modal path, application restart recovery, or
  repeated crash-loop policy.
- PID absence is polled during the automatic-relaunch wait and checked again
  after replacement activity and final GUI exit. It cannot prove properties
  between OS scheduling instants beyond those observations.
- The session is a strict local mock over IPv4 loopback. No public Sorotte
  server or external media service is contacted.
- Generated output remains ignored under `target/`; the committed evidence
  binds the retained bundle by exact paths and hashes.
