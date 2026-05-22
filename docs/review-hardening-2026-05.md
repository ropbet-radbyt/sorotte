# Review Hardening 2026-05

## codex/review-hardening-01-protocol-feature-parity

- Changed files:
  - `crates/sorotte-server/src/messages.rs`
  - `crates/sorotte-server/src/tests/session_tests.rs`
  - `crates/sorotte-client-core/src/session/apply.rs`
  - `crates/sorotte-client-core/src/session/tests/readiness_autoplay_tests.rs`
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
  - `cargo test -p sorotte-server server_feature_list`
  - `cargo test -p sorotte-client-core readiness`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Python compatibility is preserved: shared playlists are implemented and should be advertised, and `setBy` is metadata rather than the readiness target.

## codex/review-hardening-02-controlled-room-auth

- Changed files:
  - `crates/sorotte-server/src/auth.rs`
  - `crates/sorotte-server/src/runtime_handlers.rs`
  - `crates/sorotte-server/src/tests/controller_playlist_tests.rs`
  - `crates/sorotte-server/src/tests/runtime_config_tests.rs`
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
  - `cargo test -p sorotte-server controlled_room`
  - `cargo test -p sorotte-server controller_auth`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Current-room fallback is preserved for omitted `controllerAuth.room`.
  - Full-match password validation is intentional Rust deployment hardening; it rejects inputs that previously matched only by prefix.

## codex/review-hardening-03-server-transport-hardening

- Changed files:
  - `crates/sorotte-server/src/lib.rs`
  - `crates/sorotte-server/src/network.rs`
  - `crates/sorotte-server/src/tests.rs`
  - `crates/sorotte-server/src/tests/network_tests.rs`
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
  - `cargo test -p sorotte-server network`
  - `cargo test -p sorotte-server server_release_verify` (completed successfully; 0 tests matched this filter)
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Oversized-line rejection, pre-Hello idle close, and bounded outbound queues are Rust deployment hardening, not Python protocol parity changes.

## codex/review-hardening-04-server-clock-model

- Changed files:
  - `crates/sorotte-server/src/runtime_api.rs`
  - `crates/sorotte-server/src/runtime_maintenance.rs`
  - `crates/sorotte-server/src/network.rs`
  - `crates/sorotte-server/src/tests/state_tests.rs`
  - `crates/sorotte-server/src/tests/network_tests.rs`
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
  - `cargo test -p sorotte-server state`
  - `cargo test -p sorotte-server network`
  - `cargo test -p sorotte-compat state_fanout`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Python parity is preserved for outbound periodic `ping.latencyCalculation`: catch-up dispatches continue to use the current collection timestamp rather than each scheduled tick timestamp.

## codex/review-hardening-05-tls-and-persistence

- Changed files:
  - `crates/sorotte-server/src/tls.rs`
  - `crates/sorotte-server/src/runtime_maintenance.rs`
  - `crates/sorotte-server/src/lib.rs`
  - `crates/sorotte-server/src/persistence.rs`
  - `crates/sorotte-server/src/compat.rs`
  - `crates/sorotte-server/src/tests/runtime_config_tests.rs`
  - `crates/sorotte-server/src/tests/persistence_tests.rs`
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
  - `cargo test -p sorotte-server tls`
  - `cargo test -p sorotte-server persistence`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Existing TLS reload behavior when some bundle files are missing is preserved by considering readable required files.
  - Permanent-room blank/comment parsing is Rust deployment hardening; existing Python fixtures do not require blank room names.
  - Persistence remains synchronous and lightweight; blocking disk I/O was not moved to a worker in this branch.

## codex/review-hardening-06-client-transport-hardening

- Changed files:
  - `crates/sorotte-protocol/src/codec.rs`
  - `crates/sorotte-protocol/src/lib.rs`
  - `crates/sorotte-cli/src/protocol_io.rs`
  - `crates/sorotte-cli/src/session_runner.rs`
  - `crates/sorotte-cli/src/session_runner/connected_session.rs`
  - `crates/sorotte-gui/src/app/runtime_stack/transport/tcp.rs`
  - `crates/sorotte-gui/src/app/runtime_stack/transport/tests.rs`
- Behavior changed:
  - Added a shared default protocol line limit of 64 KiB.
  - GUI TCP transport disconnects with a clear error when an inbound protocol line exceeds the limit before parsing.
  - CLI connected-session reads, including StartTLS negotiation, now use a capped inbound protocol line reader instead of unbounded `BufReader::lines()`.
- Tests added/updated:
  - `gui_tcp_rejects_inbound_line_over_max_bytes`
  - `gui_tcp_accepts_line_at_or_under_max_bytes`
  - `cli_connected_session_rejects_inbound_line_over_max_bytes`
  - `cli_connected_session_accepts_batched_valid_line`
- Commands run:
  - `cargo test -p sorotte-gui transport`
  - `cargo test -p sorotte-cli connected_session`
  - `cargo fmt --all`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - CRLF, LF, and batched multi-command JSON lines remain supported.
  - Oversized inbound-line rejection is Rust deployment hardening, not a Python parity behavior change.

## codex/review-hardening-07-mpv-ipc-timeouts

- Changed files:
  - `crates/sorotte-player-mpv/src/ipc.rs`
  - `crates/sorotte-player-mpv/src/adapter.rs`
  - `crates/sorotte-player-mpv/src/tests/ipc_tests.rs`
- Behavior changed:
  - mpv IPC command execution now runs through a worker thread and returns a timeout error after 5 seconds without a matching response.
  - mpv IPC line reads are capped at 1 MiB in both the buffered pipe reader and IPC client response handling.
  - Unrelated mpv events observed while waiting for a command response remain buffered for the adapter.
- Tests added/updated:
  - `mpv_ipc_rejects_line_over_max_bytes`
  - `mpv_ipc_command_times_out_without_matching_response`
  - `mpv_ipc_preserves_unrelated_events_while_waiting`
  - `mpv_ipc_ignores_response_for_other_request_id`
  - `mpv_adapter_surfaces_timeout_as_player_error`
- Commands run:
  - `cargo test -p sorotte-player-mpv ipc`
  - `cargo test -p sorotte-player-mpv adapter`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Matching-response semantics and buffering of unrelated mpv events are preserved.
  - Timeout and oversized-line failures are Rust deployment hardening; the client runtime can continue after the operation returns an error.

## codex/review-hardening-final-integration

- Changed files:
  - `docs/review-hardening-2026-05.md`
- Behavior changed:
  - None beyond the integrated topic branches.
- Tests added/updated:
  - None beyond the integrated topic branches.
- Commands run:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `python -m pip install -r requirements/legacy-python-interop.txt`
  - `powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Final integration preserved the Python interop fixtures and compatibility tests.
  - The server release verification script passed, including the strict ignored server release matrix.

## codex/review-hardening-08-server-io-timeouts

- Changed files:
  - `crates/sorotte-server/src/lib.rs`
  - `crates/sorotte-server/src/network.rs`
  - `crates/sorotte-server/src/tests.rs`
  - `crates/sorotte-server/src/tests/network_tests.rs`
- Behavior changed:
  - Server StartTLS handshakes now time out after `TLS_HANDSHAKE_TIMEOUT_SECONDS` and close the session on timeout.
  - Server protocol writes now time out after `SERVER_WRITE_TIMEOUT_SECONDS` for direct responses, queued outbound events, protocol error responses, and TLS negotiation responses.
  - The accepted-client queue is bounded at `ACCEPTED_CLIENT_QUEUE_CAPACITY`.
- Tests added/updated:
  - `server_network_closes_starttls_client_that_never_handshakes`
  - `server_network_starttls_handshake_timeout_does_not_create_session`
  - `server_network_starttls_success_still_allows_hello`
  - `server_network_write_timeout_closes_stalled_client`
  - `server_network_error_response_write_timeout_does_not_hang_session`
  - `server_network_direct_response_write_timeout_does_not_block_loop`
  - `server_network_accept_queue_is_bounded`
- Commands run:
  - `cargo test -p sorotte-server network`
  - `cargo test -p sorotte-server tls`
  - `cargo fmt --all`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Commands not run and why:
  - None.
- Compatibility notes:
  - Protocol serialization and CRLF framing are unchanged.
  - StartTLS success and plain fallback behavior are preserved.
  - TLS handshake timeouts, write timeouts, and accepted-client queue bounding are Rust deployment hardening.

## codex/review-hardening-final-integration branch 8 merge refresh

- Changed files:
  - `docs/review-hardening-2026-05.md`
- Behavior changed:
  - None beyond merging `codex/review-hardening-08-server-io-timeouts`.
- Tests added/updated:
  - None beyond the merged topic branch.
- Commands run:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo test -p sorotte-server network`
  - `cargo test -p sorotte-server tls`
  - `python -m pip install -r requirements/legacy-python-interop.txt`
  - `powershell -ExecutionPolicy Bypass -File scripts/server-release-verify.ps1`
- Commands not run and why:
  - None.
- Compatibility notes:
  - The merge preserves the topic-branch commit structure.
  - The server release verification script passed after merging branch 8 into the final integration branch.
