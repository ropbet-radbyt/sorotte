# Native display assurance

The selected Windows profiles are 96, 144, and 192 native DPI (100%, 150%,
200%). Configure each profile on an isolated interactive desktop, then run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gui-display-matrix.ps1 -ExpectedNativeDpi 144
```

The command builds the GUI and runs six cases for that measured native DPI:

| Case | Theme | App zoom | Content and assertions |
|---|---|---|---|
| Narrow settings | Light | 1.0 | 900x760 physical window; labeled essential field reachable |
| Wide settings | Dark | 1.0 | 1700x1100; labeled essential field reachable |
| Invalid port | Light | 1.5 | Error associated with edited field; invalid draft preserved |
| Data confirmation | Dark | 1.0 | Keyboard cancellation dismisses confirmation; reopen for capture |
| Large room | Light | 1.0 | 128 members, 256 long Unicode playlist labels; both scroll endpoints reachable |
| Large room with zoom | Dark | 1.5 | Same complete content; final playlist item receives labeled keyboard focus |

`GetDpiForWindow` must match the requested native DPI before a capture passes.
Application zoom is a separate explicit input; changing it does not simulate a
different Windows DPI. The application test inputs are `SOROTTE_GUI_TEST_THEME`
and `SOROTTE_GUI_TEST_UI_SCALE` (finite 1.0 through 3.0). Invalid zoom overrides
are ignored by the application and rejected by the test CLI. Window dimensions
are physical pixels. Default egui fonts remain part of the build/environment;
the suite does not substitute fonts or assert platform-independent pixels.

Each scenario retains its screenshot and UIA tree, source commit/dirty state,
theme, native DPI, app zoom, focus, assertions, and artifact hashes. The outer
report binds the GUI executable digest. Use the exact committed candidate for
release evidence. A successful single-DPI report does not attest the other two
profiles. Inspect screenshots for glyph fallback, clipping, and contrast;
geometry/semantics checks replace brittle whole-screen pixel equality.

The runner uses StrictPhysical input and disposable local settings, TCP peer,
and test player. Do not run it on the user's active desktop. UiaOnly development
checks remain separate and cannot satisfy this matrix. No actual screen-reader
interaction is attested by these checks; any screen-reader usability claim
requires a recorded reader/version/task interaction on the isolated runner.

Implementation tests validate real protocol fixture decoding, roster/playlist
counts, display arguments, mismatched DPI rejection, and existing semantic
contracts without controlling a desktop. Native matrix execution and visual
review are separate proof steps, recorded in the release implementation ledger.

The 0.2.9 continuation passed all six cases on a measured 144-DPI desktop with
the user's explicit exception to the isolated-desktop rule. The report is
`target/verification/display-matrix-closure/dpi-144/display-matrix.json`.
Bidirectional scrolling requires complete bounds inside the content viewport;
the final selected row names the real shared keyboard-focus owner. Regression
tests also check full accessible participant names under truncation and visible
playlist title text beside compact source/removal controls. Theme overrides
select egui's matching global palette, so images are not mixed light/dark styles.

Visual review confirmed the six layouts and recorded a font limitation: the
default font renders the fixture's CJK character as a placeholder glyph. The
Unicode string remains intact in UIA; this is not a claim of complete CJK font
coverage or screen-reader usability. Native 96/192-DPI execution and an actual
screen-reader interaction remain unavailable on this desktop.
