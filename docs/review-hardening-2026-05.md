# Review Hardening 2026-05

## codex/review-hardening-01-protocol-feature-parity

- Changed files:
  - `crates/syncplay-server/src/messages.rs`
  - `crates/syncplay-server/src/tests/session_tests.rs`
  - `crates/syncplay-client-core/src/session/apply.rs`
  - `crates/syncplay-client-core/src/session/tests/readiness_autoplay_tests.rs`
- Behavior changed:
  - Server feature lists now advertise `sharedPlaylists: true`.
  - Server feature lists now advertise `setOthersReadiness` only when readiness is enabled.
  - Client `Set.ready` application now treats `ready.username` as the target, falling back only to the local username. `setBy` remains setter metadata.
- Tests added/updated:
  - `server_feature_list_includes_shared_playlists`
  - `server_feature_list_set_others_readiness_tracks_readiness_enabled`
  - `client_ready_setby_does_not_become_target_username`
  - `client_ready_missing_username_targets_local_user`
- Commands run:
  - `cargo test -p syncplay-server server_feature_list`
  - `cargo test -p syncplay-client-core readiness`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Python compatibility is preserved: shared playlists are implemented and should be advertised, and `setBy` is metadata rather than the readiness target.
