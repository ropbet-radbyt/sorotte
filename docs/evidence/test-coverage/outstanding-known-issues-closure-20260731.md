# Outstanding known-issue closure — 2026-07-31

## Scope and inventory

This was a bounded defensive QA pass over Sorotte's own local Rust and GUI
code. The implementation base was
`f726b2bfe9e37b750691c8a1b5cfd9c645449ed7` on
`codex/test-coverage-design`.

The inventory combined:

- `coverage/known-defects.toml` and its source characterization scan;
- every existing `TC-*` finding in `docs/TEST_COVERAGE_FINDINGS.md`;
- exact Rust `TODO`, `FIXME`, `XXX`, and `HACK` markers;
- a read-only live query of `ropbet-radbyt/sorotte` issues and pull requests;
  and
- the complete locked all-feature workspace validation.

The executable registry contained zero defects and zero characterizations.
The live tracker contained zero open issues and zero open pull requests. Every
existing finding was resolved. The only initial actionable marker was one
duplicated Media Match debug TODO. No credentials, public network target,
reconnaissance, privilege change, or persistent system modification was in
scope.

## TC-MEDIA-001: invalid affine-unity timeline maps

The GUI's first positive current-position diagnostic regression exposed a
production representation mismatch rather than merely missing plumbing.
`MediaTimelineAlignment.scale_ppm` stores drift from unity (`0` at ordinary
speed), while `AlignedSegmentV3.scale_ppm` is the absolute affine multiplier
used by the forward and reverse mappers (`1_000_000` at unity). Production
copied the drift value directly, making ordinary generated maps unmappable.

The implementation now:

- converts checked drift to an absolute positive affine scale at segment
  construction;
- documents and labels the two units explicitly;
- snapshots the active local playback position only with a resolved local
  path;
- carries that snapshot through exact and broad index-rebuild requests; and
- appends a mapped timestamp only to `last_evidence`.

Finite nonnegative timestamps inside the `u32`-millisecond domain are accepted.
Invalid, missing, and edit-gap positions remain omitted. Ranking, readiness,
autoplay, seeking, and synchronization are unchanged.

Positive regressions prove production forward/reverse round-trip, debug-only
summary behavior, persisted no-op rebuild plumbing, invalid timestamp
rejection, and edit-gap fail closure.

## TC-MEDIA-002: transient Windows manifest activation denial

The first complete workspace run then failed the existing 100-generation
retention regression at epoch 25. The alternate activation manifest replace
returned Windows access-denied error 5. The new generation was not activated;
both valid epoch-25 manifest replicas and the preceding two generations
remained intact. The exact pre-fix filesystem was inspected and preserved at:

```text
C:\Users\shaun\AppData\Local\Temp\sorotte-media-index-bounded-generations-live-59836-1785475960691087100
```

The Windows `MoveFileExW` activation boundary now retries only native
access-denied (5), sharing-violation (32), and lock-violation (33) results. The
eight-attempt schedule waits 5, 10, 20, 40, 80, 100, and 100 milliseconds, so
the total delay cannot exceed 355 milliseconds. The exact
`MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH` operation is retained.
Nontransient failures still return immediately; persistent denial returns the
final native error without weakening generation or cleanup fencing.

Three deterministic regressions inject recovery, exhaustion, and
nontransient-error paths. The original 100-generation case then passed 20
consecutive runs, covering 2,000 activation cycles.

## Final validation

| Gate | Result |
| --- | --- |
| `cargo fmt --all --check` | passed |
| `git diff --check` | passed; only checkout line-ending notices |
| exact Rust outstanding-marker scan | no matches |
| known-defect policy | 0 defects, 0 characterizations |
| focused Windows retry regressions | 3/3 passed |
| focused high-churn campaign | 20/20 runs, 2,000 activation cycles |
| `cargo test --locked -p sorotte-media-match --all-features` | 84/84 passed |
| `cargo test --locked -p sorotte-gui --all-features` | 1,131 passed, 2 registered ignores; 41 native harness, 14 startup benchmark, 33 updater binary, and 2 updater integration tests also passed |
| `python -m unittest discover -s scripts/tests -p "test_*.py" -v` | 504/504 passed in 24.241 seconds before evidence finalization and 504/504 passed in 25.139 seconds afterward |
| warning-denied locked all-target/all-feature workspace Clippy | passed |
| complete locked all-feature workspace | passed, exit 0 |

The complete workspace log is retained at
`target/verification/outstanding-known-issues-workspace-test-20260731.log`
(25,700 bytes,
SHA-256 `1caac4dd874035b58b2f944e87216cdb9093bb7fcb889fca2942ed1ff234cf94`).
It ran from `2026-07-31T05:51:38.7855238Z` through
`2026-07-31T05:55:51.4720009Z`. Its separate exit artifact contains `0` and
has SHA-256
`13bf7b3039c63bf5a50491fa3cfd8eb4e699d1ba1436315aef9cbe5711530354`.

## Final native regression

One rebuilt `target/debug/sorotte-gui.exe` was used without modification for
all three canonical local modes:

```text
GUI SHA-256: 63a2b994ff9edfbfff96582ae9d1b10d3a98a86628d8ffc23ff9125ff5143d7b
mpv SHA-256: 2ea23bc508acdf8489c26ba79b094a02f9f27a4cef9326daf9ddb5b711a05ef0
mpv version: mpv v0.41.0-877-ge5486b96d
```

| Mode | Fresh retained bundle | Contract |
| --- | --- | --- |
| Healthy | `target/verification/gui-real-mpv-vertical/20260731T055800358Z-53836` | passed; 13 assertions, 10 artifacts |
| Owned-process recovery | `target/verification/gui-real-mpv-owned-process-recovery/20260731T055854503Z-27120` | passed; 20 assertions, 13 artifacts |
| Faulting HTTP | `target/verification/gui-real-mpv-faulting-http-recovery/20260731T055931506Z-38528` | passed; 18 assertions, 11 artifacts |

Every invocation recorded identical GUI digests before and after execution.
The healthy and owned-process cases used generated local PCM plus an
OS-assigned IPv4-loopback session fixture. The faulting case added only an
OS-assigned IPv4-loopback HTTP fixture and generated media. Its first response
injected one malformed chunk after 720,000 bytes; the second response
transferred all 4,320,024 bytes. The same mpv PID and IPC endpoint recovered,
and the GUI, player, session listener, HTTP listener, and sockets were all
released.

The strict summary SHA-256 values are:

```text
287afb9d0e682034f36247ab767483fadf4ae2157a84dc3f4ece1152afcce9c2  healthy/contract-summary.json
e5f05ee211f72622d92ea8fcbc5e2714e1dfdd1298ddfc77db05955a39ace19a  owned/contract-summary.json
8bc2e5ddd9e7cee8d0ed289b04890b4c934c38454b0d9c98f34ae7adbb503a98  fault/contract-summary.json
```

At closure, the known-defect registry remains explicitly empty and the exact
Rust outstanding-marker scan remains empty.
