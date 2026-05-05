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

## codex/review-hardening-03-server-transport-hardening

- Changed files:
  - `crates/syncplay-server/src/lib.rs`
  - `crates/syncplay-server/src/network.rs`
  - `crates/syncplay-server/src/tests.rs`
  - `crates/syncplay-server/src/tests/network_tests.rs`
- Behavior changed:
  - Server protocol line reads are capped at 64 KiB and oversized lines receive `Error: Protocol line too long` before the connection closes.
  - Clients that do not send a complete pre-Hello protocol line before `PROTOCOL_TIMEOUT_SECONDS` are closed without creating a runtime session.
  - Per-client outbound event queues are bounded at 256 entries; a full queue removes the sender so the slow client session closes instead of accumulating unbounded messages.
  - The network loop prunes finished session task handles and reports session/task errors via lightweight stderr diagnostics.
- Tests added/updated:
  - `server_network_rejects_line_over_max_bytes`
  - `server_network_closes_pre_hello_idle_client`
  - `server_network_does_not_create_session_for_pre_hello_idle_client`
  - `server_network_prunes_finished_session_tasks`
  - `server_network_closes_or_drops_slow_client_when_outbound_queue_full`
- Commands run:
  - `cargo test -p syncplay-server network`
  - `cargo test -p syncplay-server server_release_verify` (completed successfully; 0 tests matched this filter)
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Oversized-line rejection, pre-Hello idle close, and bounded outbound queues are Rust deployment hardening, not Python protocol parity changes.

## codex/review-hardening-04-server-clock-model

- Changed files:
  - `crates/syncplay-server/src/runtime_api.rs`
  - `crates/syncplay-server/src/runtime_maintenance.rs`
  - `crates/syncplay-server/src/network.rs`
  - `crates/syncplay-server/src/tests/state_tests.rs`
  - `crates/syncplay-server/src/tests/network_tests.rs`
- Behavior changed:
  - Added an absolute-time dispatch path for runtime maintenance.
  - Production network ticks now collect dispatch at the current Unix wall-clock time instead of advancing the runtime's deterministic test override by a fixed interval.
  - Deterministic delta-based time advancement remains available for tests.
  - Periodic playback aging and timeout checks use supplied absolute elapsed time while outbound ping timestamps remain current collection-time values for Python compatibility.
- Tests added/updated:
  - `server_runtime_tick_uses_supplied_absolute_time`
  - `server_runtime_delta_helper_remains_deterministic_for_tests`
  - `server_network_tick_does_not_accumulate_simulated_time`
  - `room_playback_position_ages_by_actual_elapsed_seconds`
  - `state_timeout_uses_actual_elapsed_seconds`
- Commands run:
  - `cargo test -p syncplay-server state`
  - `cargo test -p syncplay-server network`
  - `cargo test -p syncplay-compat state_fanout`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Python parity is preserved for outbound periodic `ping.latencyCalculation`: catch-up dispatches continue to use the current collection timestamp rather than each scheduled tick timestamp.

## codex/review-hardening-05-tls-and-persistence

- Changed files:
  - `crates/syncplay-server/src/tls.rs`
  - `crates/syncplay-server/src/runtime_maintenance.rs`
  - `crates/syncplay-server/src/lib.rs`
  - `crates/syncplay-server/src/persistence.rs`
  - `crates/syncplay-server/src/compat.rs`
  - `crates/syncplay-server/src/tests/runtime_config_tests.rs`
  - `crates/syncplay-server/src/tests/persistence_tests.rs`
- Behavior changed:
  - TLS rotation now tracks the max modified time across readable required bundle files: `privkey.pem`, `cert.pem`, and `chain.pem`.
  - Room persistence SQLite connections set a busy timeout and initialize WAL journal mode.
  - Permanent rooms files are trimmed and ignore blank/comment lines.
- Tests added/updated:
  - `tls_rotation_detects_cert_change`
  - `tls_rotation_detects_chain_change`
  - `tls_rotation_detects_privkey_change`
  - `room_persistence_sets_busy_timeout_or_wal`
  - `permanent_rooms_file_ignores_blank_lines`
  - `permanent_rooms_file_ignores_comment_lines`
- Commands run:
  - `cargo test -p syncplay-server tls`
  - `cargo test -p syncplay-server persistence`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Existing TLS reload behavior when some bundle files are missing is preserved by considering readable required files.
  - Permanent-room blank/comment parsing is Rust deployment hardening; existing Python fixtures do not require blank room names.
  - Persistence remains synchronous and lightweight; blocking disk I/O was not moved to a worker in this branch.
