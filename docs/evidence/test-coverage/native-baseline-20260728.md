# Native GUI baseline evidence — 2026-07-28

This is a reviewable record of the real interactive-Windows experiment
described by `docs/TEST_COVERAGE_FINDINGS.md`. The application was not changed
after the strict validator rejected the legacy runner's green result.

## Provenance

- Branch base: `a08a06ea7c6cada5413b0dba73b16f940cfd46e1`
- Scenario: `baseline`
- Legacy runner exit/result: `0` / `"ok"`
- Main GUI build: 25,858 ms
- Native harness build: 4,755 ms
- Direct runner duration: 54,373 ms (53,949 ms reported in-band)
- Watchdog bound: 190,000 ms
- GUI SHA-256 before and after:
  `e923e92ec096b3ddf1e8e527fed4ddf0475d1f3a5e99080511e9cd194bddf6e2`
- Raw report SHA-256:
  `a102c5dcbd8a653cd32b0c01675a332ecf677e8df7097a6bd7f12c8aa8f0aabe`
- Original local evidence directory:
  `target/verification/gui-native-smoke/20260728T054736251Z-64192`

The raw report identified the launched executable and PID, reported an empty
menu inventory, and nevertheless marked both the interaction and accessibility
contracts verified:

```json
{
  "result": "ok",
  "binary": "\\\\?\\C:\\tmp\\sorotte-test-coverage-design\\target\\debug\\sorotte-gui.exe",
  "pid": 55564,
  "window_title": "Sorotte GUI",
  "menu_labels": [],
  "menu_contract": "skipped-no-native-menu",
  "accessible_name_count": 110,
  "accessibility_contract": "verified",
  "interaction_contract": "verified",
  "closed": true,
  "duration_ms": 53949
}
```

Its interaction steps contained no `open-media-file` completion marker. The
runner instead emitted one `open-media-file-skipped:` step recording that menu
item, fallback control, and quick-open button discovery all timed out. The last
accessibility snapshot remained on the setup/configuration surface.

## Strict replay result

The strict wrapper bound the report to the expected binary path, verified the
same SHA-256 before and after execution, recorded producer exit `0`, and then
returned exit `1` with five independent contract errors:

1. required native menu labels are absent;
2. `menu_contract` is a skip rather than `verified`;
3. a required Open Media capability was skipped;
4. the `baseline` scenario lacks its `open-media-file` completion marker; and
5. native stderr is nonempty.

The stderr evidence contains 20 repeated attempts to resolve the placeholder
endpoint `syncplay.example:8999`, each failing with Windows error 11001. The
strict wrapper now rejects that diagnostic output instead of treating it as a
successful isolated smoke run.

This record intentionally retains only non-secret, decision-relevant fields.
The raw report digest above binds the complete preserved local report used by
the replay.
