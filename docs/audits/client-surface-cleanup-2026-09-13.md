# Client surface and notification cleanup

This batch follows the runtime-planning cleanup merged in PR #72. It removes
unused internal surfaces and duplicated delivery paths while retaining Syncplay
protocol and settings compatibility. Rust crates have no consumers outside this
workspace; removed entrypoints have no compatibility aliases.

## Removed surfaces

- **TLS certificate GUI:** the Advanced menu opened a certificate prompt whose
  Trust and Reject actions both cleared shell state. Neither action affected the
  transport. Its flag was also derived from `only_switch_to_trusted_domains`,
  which governs automatic media URL switching. The menu, modal, actions, flag,
  snapshot/rebase plumbing and native-harness dismissal routines are removed.
  Menu defaults now have no settings argument. Media trust controls, rustls
  certificate verification, STARTTLS policy and credential guards are retained.
- **Update notice modal:** the renderer explicitly skipped this modal. Its
  announcement and dismissal actions were test-only, while real update checks
  already used the update indicator. The variant, flags, unused modal body and
  obsolete translations are removed. The GitHub updater, download/staging/apply
  paths, indicator and active public-server directory service remain.
- **Fingerprint entrypoints:** the empty extraction-options type and four
  forwarding entrypoints collapse into `fingerprint_media_file_with_report`,
  which accepts optional cancellation. GUI and diagnostic callers share it.
  The overall extraction deadline, output bounds, subprocess containment and
  diagnostic report remain. Two unused timeline classification/presence helpers
  are removed; forward and inverse position mapping remain.
- **Storage resolver wrappers:** two unused public wrappers converted locator
  read failures into `None`. CLI and GUI already used checked resolution. Seven
  successful precedence tests now call the existing fallible reader seam; the
  wrapper-only failure test is retired. Checked unreadable, busy, invalid-UTF-8,
  missing-file and transactional locator tests remain.

## Notification delivery

Six CLI test-only flush functions duplicated or bypassed production delivery.
Tests now call the production autoplay, chat, controller-auth, file-difference,
reconnect and user-change flush functions. The existing console emitters are
passed as callbacks by the session runner. Player delivery still precedes output;
queued notifications are acknowledged only after output succeeds. Player display
failure remains best effort, and hidden controller/user notifications retain
their existing display policy.

This exposed a file-difference retry defect. Before the fix, a callback failure
left `last_summary = Some("duration")`, causing the next flush to suppress the
notification. The regression first failed on that state transition. The flush
now calculates the next deduplication state separately and commits it after
successful output. The same regression then passed, including a successful retry,
deduplication and duration-policy changes.

Tests also exercise retained queued items on output failure, simulated mpv OSD,
hidden user notifications, and ordered chat retry after an IPC delivery failure.
The chat fixture records commands and returns EOF while awaiting a reply; it
proves delivery attempts and acknowledgement behavior, not real player rendering.

## Validation boundaries

Local implementation checks passed:

- `cargo fmt --all -- --check` and workspace Clippy with all targets/features and
  warnings denied.
- CLI library: 416 passed, 8 ignored under the existing ignored-test policy.
- Client-app library with all features: 217 passed, 2 ignored.
- GUI library with all features: 1,221 passed, 7 ignored, including the existing
  invalid-certificate/STARTTLS credential guards and update workflows.
- Media-match library with all features: 98 passed, 1 ignored, including
  cancellation, bounded output and process cleanup.
- Headless semantic suite: all 14 scenarios passed. The core shell scenario now
  toggles media trust and verifies that no modal opens. Its first revision needed
  an explicit return to the Connection tab before editing connection settings.
- Static preflight passed with ordinary process permissions. The sandboxed
  attempt failed its owned-process cleanup probe because `taskkill` was denied;
  the failed receipt is retained separately.

These implementation checks do not replace the exact-commit workspace run,
required hosted checks, authorized native GUI/playback run or package lifecycle
qualification. Their source-bound results are recorded on the PR. No release or
publication is part of this batch. The larger session interface refactor and the
optional qualification-tooling proposals remain separate work.
