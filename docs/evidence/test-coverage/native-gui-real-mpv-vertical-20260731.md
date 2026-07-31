# Native GUI to real mpv vertical evidence (2026-07-31)

## Scope and safety boundary

This slice adds an explicit local Windows native-GUI vertical for Sorotte's supported mpv integration. It does not target an external service. The only session transport is an existing native-smoke fixture bound to an OS-assigned `127.0.0.1` port; both the listener and connected peer are attested as nonzero IPv4-loopback endpoints. Media is a deterministically generated 12-second PCM WAV. The isolated Sorotte config, APPDATA root, generated media, mpv Lua observer, logs, screenshots, state, and manifests all live below one fresh `target/verification/gui-real-mpv-vertical/<run>` root.

The mpv IPC endpoint is a Windows named-pipe namespace path rather than a filesystem path. The lane requires the product-generated `\\.\pipe\sorotte-gui-mpv-<gui-pid>-...` prefix and binds it to the attested GUI PID in the report.

The lane is opt-in through `scripts/gui-real-mpv-vertical.ps1`. A missing explicit mpv fails before build or GUI launch, retains a structured bundle, and returns exit code 125.

## Executable contract

The wrapper rebuilds the GUI and native harness unless an explicit GUI binary is supplied. It records the GUI digest before and after the run and requires equality. The native harness then proves:

1. The exact mpv binary is supported and digest-bound.
2. The GUI is a real native window and owns foreground before physical input.
3. The mock listener, connected peer, exact client/server Hello frames, advertised `chat`, `readiness`, and `sharedPlaylists` capabilities, and release state are retained.
4. The exact File -> Open Media leaf is delivered once by a physical click.
5. The GUI owns the exact mpv process; its parent PID, process image, digest, managed IPC endpoint, generated-media path, filename, and 12-second duration match.
6. The GUI exposes enabled transport controls and a paused state.
7. GUI Play produces an ordered real-mpv `pause=false` observation and GUI `playing` projection.
8. GUI Pause produces a later real-mpv `pause=true` observation and GUI `paused` projection.
9. A success screenshot is retained.
10. The exact File -> Exit leaf is delivered once by a physical click; the complete lifecycle sequence is observed; the GUI and owned mpv terminate naturally; and the server thread/socket are joined and released.

Menu-section opening remains fail-closed. The normal path is a physical section click. If that click returns without exposing its leaf, the dedicated vertical retains two separate snapshots proving the leaf is still hidden before one UI Automation section-open fallback. The leaf itself is always one exact physical click and is never retried after delivery.

The strict Python validator rejects nonzero producer exits, any assertion inventory or order other than the canonical 13 entries, missing or extra artifacts, digest or ownership drift, non-loopback endpoints, any client/server Hello object other than the complete canonical identity, version, and feature maps, altered advertised capabilities, unreleased threads/sockets, unordered mpv state, incomplete lifecycle state, relative or different binary paths, and incomplete menu fallback evidence. Windows `\\?\X:\...` and `\\?\UNC\...` spellings are accepted only when they resolve to the same absolute path under Windows path semantics.

## Canonical local pass

Command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gui-real-mpv-vertical.ps1 -MpvPath "C:\Program Files\mpv\mpv.exe" -TimeoutMs 30000
```

Result: exit 0, producer `passed`, strict contract `passed`, 13 assertions, and 10 hashed contract artifacts.

Canonical bundle:

```text
target\verification\gui-real-mpv-vertical\20260730T220441781Z-41332
```

Bound identities:

- GUI SHA-256 before/after: `247df3f60f2dda546317ab2a8dd209a130bb887ac5ca1c5d6d996f1f9035037a`
- mpv: `mpv v0.41.0-877-ge5486b96d`
- mpv bytes: `121090048`
- mpv SHA-256: `2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0`
- Session listener/peer: `127.0.0.1:63930` / `127.0.0.1:63931`
- GUI PID / owned mpv PID: `1140` / `66728`
- Managed IPC: `\\.\pipe\sorotte-gui-mpv-1140-1785449096096`

Both menu sections used the normal `physical-section-open` strategy; both exact leaves record `single-exact-physical-click-no-retry`. The lifecycle contains, in order, `exit-action-applied`, `viewport-close-requested`, `runtime-stop-requested`, `runtime-worker-stopped`, and `app-drop-complete`. No Sorotte GUI or mpv process remained afterward.

This terminal pass ran after the complete all-feature workspace gate rebuilt the
GUI, so the evidence is bound to the final validated binary rather than the
earlier pre-gate build.

The earlier producer-complete bundle at `target\verification\gui-real-mpv-vertical\20260730T213424802Z-42412` was preserved unchanged. Its producer passed the same substantive vertical; the original validator rejected only equivalent Windows extended-length versus ordinary path spelling. After the documented-path normalization regression fix, that unchanged bundle revalidated with 13 assertions and 10 artifacts; the revalidation summary is outside the bundle at `target\verification\gui-real-mpv-vertical-revalidation-20260731.json`.

## Prior three-mode post-gate confirmation

At the later generated-compatibility/Unix-kernel/faulting-HTTP three-mode
checkpoint, after every then-current build-producing validation gate, the
unchanged healthy inventory was executed again:

```text
target/verification/gui-real-mpv-vertical/20260731T044916649Z-67112
```

It passed all 13 assertions with all 10 exact artifacts in 16,713 ms. GUI PID
`57104` owned mpv PID `64968` through
`\\.\pipe\sorotte-gui-mpv-57104-1785473377045`; the loopback session endpoints
were `127.0.0.1:58072` and `127.0.0.1:58073`. Native Exit reaped the owned mpv
and GUI and released the session fixture.

```text
673dda5226c433950d3074cb4f1b2b6d222802eda6e30cc8a9b5d6e0ef12271c  final GUI before/after
2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0  mpv v0.41.0-877-ge5486b96d
2735324df56ca5476780341a2d08237ba2a57050b8e218624e265fcd32528ab6  harness-report.json
6de31c5042d2d76ccb81cbb9227dd30c24752c73200daeb7463df300d5056c96  contract-summary.json
9cb240c153b201db3a713f7285c33ff4941e7a4be5e7703e25e37c26b74366c9  invocation.json
```

## Four-mode tranche post-gate reconfirmation

The later client-timing/generated-media/CLI/stalled-HTTP tranche reran the
healthy mode after its final workspace, semantic, native-smoke, and GUI build
gates:

```text
target/verification/gui-real-mpv-vertical/20260731T081916515Z-57224
```

It passed 13 assertions and 10 artifacts in 16,483 ms. GUI PID `63552` owned
mpv PID `55740` through
`\\.\pipe\sorotte-gui-mpv-63552-1785485961613`; the session endpoints were
`127.0.0.1:64048` and `127.0.0.1:64049`. The same GUI and mpv digests then
passed owned-process, malformed-HTTP, and valid byte-silent stalled-HTTP
campaigns, with stalled HTTP deliberately last.

```text
a680ec8323011e4083c51b2de64473f8a4b9ef1aef8507131d03eb721e22bab3  final GUI before/after
2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0  mpv v0.41.0-877-ge5486b96d
a6336398ea5dec2f41cf2a05f50f2c950a0706c55e29318fc2b747af393ff526  harness-report.json
ae885c0ecc3b7a720070b231404fc3e5a01255fd6770af748c871ac9661703d4  contract-summary.json
41771e8386b082a9029cecc81a1de81c471c79c3cdeeae4c258895196b8f40cc  invocation.json
```

## Final implementation-source four-mode campaign

After the final source-bound compatibility/fuzz campaigns, all 519 Python
self-tests, warning-denied workspace Clippy, the complete locked all-feature
workspace, a fresh GUI build, 14/14 semantic scenarios, and native smoke, the
healthy mode started the final four-mode sequence:

```text
target/verification/gui-real-mpv-vertical/20260731T115509993Z-33888
```

It passed 13 assertions and 10 artifacts in 15,566 ms. GUI PID `50072` owned
mpv PID `7832` through
`\\.\pipe\sorotte-gui-mpv-50072-1785498914884`; the loopback session used
`127.0.0.1:59405` / `127.0.0.1:59406`. Owned-process, malformed-HTTP, and
valid byte-silent stalled-HTTP modes followed, with stalled HTTP last.

```text
439174541d461db90fc66be088152024814e3ba4fe0d0d6b3add464103205d9e  final GUI before/after
2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0  mpv v0.41.0-877-ge5486b96d
4dc04a0e3c266bb6eefb00e17fdd09f39e1a742fab7d9a06044274c6831ca350  harness-report.json
6c0379af88ba593a0b9e0f7589ccca90c04d575b7d58203184559919090e50b9  contract-summary.json
8c322d3f70c189c7a828f6f8c78f110315405a587c3abf397cf636e2cf4e0254  invocation.json
```

## Fail-closed prerequisite evidence

Command:

```powershell
& powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gui-real-mpv-vertical.ps1 -MpvPath "C:\tmp\sorotte-test-coverage-design\target\verification\missing-real-mpv.exe" -TimeoutMs 30000
$LASTEXITCODE
```

Result: `125`; capability `missing-prerequisite`; stage `mpv-preflight`; GUI build, harness build, and runner were not started.

Bundle:

```text
target\verification\gui-real-mpv-vertical\20260730T213718625Z-61044
```

## Preserved diagnostic bundles

All unsuccessful exploratory bundles remain intact:

- `20260730T211217393Z-43104`: foreground ownership failure before menu input.
- `20260730T211348973Z-22180`: unchanged replay of the same foreground failure.
- `20260730T211745248Z-67240`: owned mpv file-loaded proof; disconnected UI has no filename accessibility contract.
- `20260730T212054568Z-46280`: generic in-process chat loopback correctly lacked shared-playlist control.
- `20260730T212807426Z-57304`: full media/play/pause proof; final Exit section did not open.
- `20260730T213011151Z-64300`: Open Media section did not open.

These bundles retain progressive state, failure screenshot/accessibility tree when a window existed, isolated config/media/Lua, mpv observation/log, lifecycle, wrapper metadata, producer report, and strict-contract summary.

## Focused validation

The slice was validated with:

```powershell
cargo fmt -p sorotte-gui -- --check
cargo test --locked -p sorotte-gui --features gui-native-smoke --bin sorotte-gui-native-smoke real_mpv -- --nocapture
cargo clippy --locked -p sorotte-gui --features gui-native-smoke --bin sorotte-gui-native-smoke -- -D warnings
python -m py_compile scripts\gui_real_mpv_vertical_contract.py scripts\tests\test_gui_real_mpv_vertical_contract.py
python -m unittest scripts.tests.test_gui_real_mpv_vertical_contract -v
```

Results at evidence capture: 6 Rust tests passed; focused Clippy passed with warnings denied; 9 Python tests passed.
