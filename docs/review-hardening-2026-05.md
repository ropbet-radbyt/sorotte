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

## codex/review-hardening-02-controlled-room-auth

- Changed files:
  - `crates/syncplay-server/src/auth.rs`
  - `crates/syncplay-server/src/runtime_handlers.rs`
  - `crates/syncplay-server/src/tests/controller_playlist_tests.rs`
  - `crates/syncplay-server/src/tests/runtime_config_tests.rs`
- Behavior changed:
  - `controllerAuth.room` now controls the room used for password validation, controller grants, status payloads, and peer fanout.
  - Omitting `controllerAuth.room` still authenticates against the sender's current room.
  - Controlled-room passwords must fully match the `AA-123-456` legacy shape; trailing characters are rejected.
- Tests added/updated:
  - `controller_auth_grants_requested_room_when_current_room_differs`
  - `controller_auth_omitted_room_uses_current_room`
  - `controller_auth_status_reports_requested_room`
  - `controlled_room_password_rejects_trailing_characters`
  - `controlled_room_password_accepts_exact_legacy_format`
- Commands run:
  - `cargo test -p syncplay-server controlled_room`
  - `cargo test -p syncplay-server controller_auth`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Current-room fallback is preserved for omitted `controllerAuth.room`.
  - Full-match password validation is intentional Rust deployment hardening; it rejects inputs that previously matched only by prefix.
