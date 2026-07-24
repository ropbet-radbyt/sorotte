# Player lifecycle stabilization follow-ups

This file records defects discovered while implementing player lifecycle
stabilization that are outside the branch's ownership scope. They are not fixed
here unless they block lifecycle validation or present immediate data-loss or
security risk.

## Live Python peer readiness smoke timeout

- Severity: medium (test reliability/interoperability signal).
- Baseline: `fe80cc75f2c2933b75298f865e2d528bcf73adfb`, before lifecycle production
  changes.
- Reproduction:

  ```text
  cargo test -p sorotte-gui --all-features runtime_owner -- --test-threads=1
  ```

- Result: 438 passed, 1 failed, 1 ignored, 558 filtered in the selected group.
- Failing test:
  `app::smoke_tests::live_python_smoke::gui_persisted_config_runtime_owner_projects_live_python_peer_shared_playlist_open_interop`.
- Isolated retry: failed again.
- Trace: the smoke timed out waiting for peer readiness; the GUI self user was
  ready while the Python peer remained not ready.
- Likely owner: live Python interoperability/runtime smoke harness, not mpv
  attachment, physical load-attempt, or ordered player-event ownership.

