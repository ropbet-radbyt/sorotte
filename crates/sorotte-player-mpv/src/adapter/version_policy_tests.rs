use super::*;
use std::{collections::VecDeque, io};

#[derive(Debug)]
struct VersionResponseTransport {
    responses: VecDeque<String>,
    version_query_clock: Option<(std::sync::Arc<std::sync::atomic::AtomicU64>, u64)>,
}

impl VersionResponseTransport {
    fn new(response: &str) -> Self {
        Self {
            responses: VecDeque::from([format!("{response}\n")]),
            version_query_clock: None,
        }
    }

    fn new_many(responses: &[&str]) -> Self {
        Self {
            responses: responses
                .iter()
                .map(|response| format!("{response}\n"))
                .collect(),
            version_query_clock: None,
        }
    }
}

impl MpvJsonIpcTransport for VersionResponseTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        if line.contains("mpv-version")
            && let Some((clock, elapsed_millis)) = self.version_query_clock.as_ref()
        {
            clock.fetch_add(*elapsed_millis, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self.responses.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "test response queue was empty",
            )
        })?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

fn initialize_with_version_response(response: &str) -> (MpvAdapter, Result<(), PlayerError>) {
    let client = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new(response)));
    let mut adapter = MpvAdapter::default();
    let result = adapter.initialize_json_ipc_attachment(PathBuf::from("test-mpv-ipc"), client);
    (adapter, result)
}

fn operation_failure_message(result: Result<(), PlayerError>) -> String {
    let error = result.expect_err("version policy should reject this attachment");
    assert!(crate::is_unsupported_mpv_version_error(&error));
    match error {
        PlayerError::OperationFailed(message) => message,
        other => panic!("unexpected version-policy error: {other:?}"),
    }
}

#[test]
fn json_ipc_initialization_accepts_minimum_and_newer_mpv_versions() {
    for reported in ["0.41.0", "mpv 0.41.1-UNKNOWN", "1.0.0"] {
        let response = format!(r#"{{"request_id":1,"error":"success","data":"{reported}"}}"#);
        let (adapter, result) = initialize_with_version_response(&response);

        result.unwrap_or_else(|error| panic!("{reported} should be supported: {error}"));
        assert!(adapter.ipc_client.is_some());
        assert_eq!(adapter.ipc_endpoint, Some(PathBuf::from("test-mpv-ipc")));
    }
}

#[test]
fn explicit_json_ipc_retry_retains_endpoint_backs_off_and_reattaches() {
    let endpoint = PathBuf::from("late-mpv-ipc");
    let mut adapter = MpvAdapter::disconnected_with_json_ipc_retry(&endpoint);
    let first_attempt_at = adapter
        .ipc_reconnect_not_before
        .expect("the constructor should make the first retry immediately due");
    let first_attempt_completed_at = first_attempt_at + Duration::from_secs(2);
    let mut failed_attempts = 0;
    adapter.maintain_json_ipc_reconnection_using_clock(
        first_attempt_at,
        |observed_endpoint| {
            failed_attempts += 1;
            assert_eq!(observed_endpoint, endpoint);
            Err("endpoint absent".to_owned())
        },
        || first_attempt_completed_at,
    );
    assert_eq!(failed_attempts, 1);
    assert_eq!(adapter.ipc_endpoint.as_deref(), Some(endpoint.as_path()));
    assert!(adapter.ipc_client.is_none());
    let retry_at = first_attempt_completed_at + IPC_RECONNECT_INTERVAL;
    assert_eq!(adapter.ipc_reconnect_not_before, Some(retry_at));
    assert!(
        retry_at > first_attempt_at + IPC_RECONNECT_INTERVAL,
        "a slow failed connect must not consume its retry backoff while blocked",
    );

    let mut premature_attempts = 0;
    adapter.maintain_json_ipc_reconnection_using(retry_at - Duration::from_millis(1), |_| {
        premature_attempts += 1;
        Err("retry should still be backed off".to_owned())
    });
    assert_eq!(premature_attempts, 0);

    let response = format!(
        r#"{{"request_id":1,"error":"success","data":"{}"}}"#,
        crate::MINIMUM_SUPPORTED_MPV_VERSION
    );
    let mut successful_attempts = 0;
    adapter.maintain_json_ipc_reconnection_using_clock(
        retry_at,
        |observed_endpoint| {
            successful_attempts += 1;
            assert_eq!(observed_endpoint, endpoint);
            Ok(MpvJsonIpcClient::new(Box::new(
                VersionResponseTransport::new(&response),
            )))
        },
        || retry_at,
    );

    assert_eq!(successful_attempts, 1);
    assert!(adapter.ipc_client.is_some());
    assert_eq!(adapter.ipc_endpoint.as_deref(), Some(endpoint.as_path()));
    assert_eq!(adapter.ipc_reconnect_not_before, None);
}

#[test]
fn explicit_json_ipc_retry_is_disabled_for_simulation_and_live_connections() {
    let endpoint = PathBuf::from("late-mpv-ipc");
    let now = Instant::now();
    let mut simulated = MpvAdapter::disconnected_with_json_ipc_retry(&endpoint);
    simulated.simulation_mode = true;
    simulated.ipc_reconnect_not_before = None;
    let mut simulated_attempts = 0;
    simulated.maintain_json_ipc_reconnection_using(now, |_| {
        simulated_attempts += 1;
        Err("simulation must not connect".to_owned())
    });
    assert_eq!(simulated_attempts, 0);
    assert_eq!(simulated.ipc_reconnect_not_before, None);

    let (mut connected, result) =
        initialize_with_version_response(r#"{"request_id":1,"error":"success","data":"0.41.0"}"#);
    result.expect("the supported attachment should connect");
    connected.ipc_reconnect_not_before = Some(now);
    let mut connected_attempts = 0;
    connected.maintain_json_ipc_reconnection_using(now, |_| {
        connected_attempts += 1;
        Err("an attached adapter must not reconnect".to_owned())
    });
    assert_eq!(connected_attempts, 0);
    assert_eq!(connected.ipc_reconnect_not_before, None);
}

#[test]
fn explicit_json_ipc_retry_backs_off_after_attachment_initialization_failure() {
    let endpoint = PathBuf::from("late-unsupported-mpv-ipc");
    let mut adapter = MpvAdapter::disconnected_with_json_ipc_retry(&endpoint);
    let attempt_at = adapter
        .ipc_reconnect_not_before
        .expect("the constructor should make the first retry immediately due");
    let completed_at = attempt_at + Duration::from_secs(2);
    let unsupported = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new(
        r#"{"request_id":1,"error":"success","data":"0.40.0"}"#,
    )));

    adapter.maintain_json_ipc_reconnection_using_clock(
        attempt_at,
        |_| Ok(unsupported),
        || completed_at,
    );

    assert!(adapter.ipc_client.is_none());
    assert_eq!(
        adapter.ipc_reconnect_not_before,
        Some(completed_at + IPC_RECONNECT_INTERVAL)
    );
}

#[test]
fn explicit_json_ipc_retry_waits_a_full_interval_after_slow_version_failure() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    for initialization_millis in [500, 1_000, 2_500] {
        let mut adapter = MpvAdapter::disconnected_with_json_ipc_retry("slow-version-ipc");
        let attempt_at = adapter.ipc_reconnect_not_before.unwrap();
        let elapsed = Arc::new(AtomicU64::new(0));
        let mut transport =
            VersionResponseTransport::new(r#"{"request_id":1,"error":"success","data":"0.40.0"}"#);
        transport.version_query_clock = Some((Arc::clone(&elapsed), initialization_millis));
        adapter.maintain_json_ipc_reconnection_using_clock(
            attempt_at,
            |_| Ok(MpvJsonIpcClient::new(Box::new(transport))),
            || attempt_at + Duration::from_millis(elapsed.load(Ordering::SeqCst)),
        );
        assert_eq!(elapsed.load(Ordering::SeqCst), initialization_millis);
        let retry_at =
            attempt_at + Duration::from_millis(initialization_millis) + IPC_RECONNECT_INTERVAL;
        assert_eq!(adapter.ipc_reconnect_not_before, Some(retry_at));
        adapter.maintain_json_ipc_reconnection_using(retry_at - Duration::from_millis(1), |_| {
            panic!("initialization must not consume the retry interval");
        });
        let mut attempts = 0;
        adapter.maintain_json_ipc_reconnection_using_clock(
            retry_at,
            |_| {
                attempts += 1;
                Err("still unavailable".to_owned())
            },
            || retry_at,
        );
        assert_eq!(attempts, 1);
    }
}

#[test]
fn rejected_replacement_preserves_the_existing_supported_attachment() {
    let (mut adapter, result) =
        initialize_with_version_response(r#"{"request_id":1,"error":"success","data":"0.41.0"}"#);
    result.expect("the initial supported attachment should succeed");
    let replacement = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new(
        r#"{"request_id":1,"error":"success","data":"0.40.0"}"#,
    )));

    let error = adapter
        .initialize_json_ipc_attachment(PathBuf::from("unsupported-replacement"), replacement)
        .expect_err("an unsupported replacement must be rejected");

    assert!(crate::is_unsupported_mpv_version_error(&error));
    assert!(adapter.ipc_client.is_some());
    assert_eq!(adapter.ipc_endpoint, Some(PathBuf::from("test-mpv-ipc")));
}

#[test]
fn supported_replacement_fences_old_commands_and_reuses_no_playlist_ownership() {
    let (mut adapter, result) =
        initialize_with_version_response(r#"{"request_id":1,"error":"success","data":"0.41.0"}"#);
    result.expect("the initial supported attachment should succeed");
    let old_attachment = adapter.lifecycle_epoch();
    let old_generation = adapter.allocate_media_generation();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch: old_attachment,
        media_generation: old_generation,
        playlist_entry_id: 1,
        observed_target: "C:/old-core.mkv".to_owned(),
        file_loaded: true,
    });
    let old_attempt = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("old core should have active reducer ownership");
    adapter.active_playlist_entry_id = Some(1);
    adapter.active_media_generation = Some(old_generation);
    adapter.current_path = Some("C:/old-core.mkv".to_owned());
    adapter.observed_state.path = adapter.current_path.clone();
    let old_command = adapter.register_tracked_command(
        Some(old_generation),
        TrackedCommandKind::Seek {
            target_seconds: 30.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(old_command);
    adapter.pending_command_progress_updates.clear();
    adapter.pending_ordered_player_events.clear();

    let replacement = MpvJsonIpcClient::new(Box::new(VersionResponseTransport::new_many(&[
        r#"{"request_id":1,"error":"success","data":"0.41.1"}"#,
        r#"{"request_id":2,"error":"success","data":[{"id":1,"filename":"C:/new-core.mkv","current":true,"playing":true}]}"#,
        r#"{"request_id":3,"error":"success","data":"C:/new-core.mkv"}"#,
        r#"{"request_id":4,"error":"success","data":false}"#,
        r#"{"request_id":5,"error":"success","data":0.0}"#,
        r#"{"request_id":6,"error":"success","data":1.0}"#,
        r#"{"request_id":7,"error":"success","data":false}"#,
        r#"{"request_id":8,"error":"success","data":100.0}"#,
        r#"{"request_id":9,"error":"success","data":false}"#,
        r#"{"request_id":10,"error":"success","data":true}"#,
        r#"{"request_id":11,"error":"success","data":false}"#,
        r#"{"request_id":12,"error":"success","data":false}"#,
        r#"{"request_id":13,"error":"success","data":false}"#,
    ])));
    adapter
        .initialize_json_ipc_attachment(PathBuf::from("supported-replacement"), replacement)
        .expect("supported replacement should attach");
    adapter.reconcile_lifecycle_from_authority();

    assert_ne!(adapter.lifecycle_epoch(), old_attachment);
    let new_attempt = adapter
        .player_lifecycle
        .playlist_entry_attempts
        .get(&1)
        .copied()
        .expect("new core entry should receive new attachment ownership");
    assert_ne!(new_attempt, old_attempt);
    assert_eq!(
        adapter.player_lifecycle.active_load_attempt,
        Some(new_attempt)
    );
    assert_ne!(adapter.active_media_generation, Some(old_generation));
    assert_eq!(adapter.current_path.as_deref(), Some("C:/new-core.mkv"));
    assert!(
        adapter
            .pending_command_progress_updates
            .iter()
            .any(|progress| {
                progress.command_id == old_command
                    && progress.state
                        == sorotte_player_api::PlayerCommandProgressState::Finished(
                            PlayerCommandResult::Failed(
                                PlayerCommandFailureKind::TransportDisconnected,
                            ),
                        )
            })
    );
    assert!(
        adapter
            .pending_media_load_outcomes
            .iter()
            .all(|outcome| outcome.outcome.loaded_target.as_deref() != Some("C:/old-core.mkv"))
    );
}

#[test]
fn json_ipc_initialization_rejects_mpv_older_than_0_41_0() {
    for reported in ["0.34.1", "0.40.99"] {
        let response = format!(r#"{{"request_id":1,"error":"success","data":"{reported}"}}"#);
        let (adapter, result) = initialize_with_version_response(&response);
        let message = operation_failure_message(result);

        assert!(message.contains("requires mpv 0.41.0 or newer"));
        assert!(message.contains(&format!("reports mpv {reported}")));
        assert!(message.contains("upgrade mpv"));
        assert!(adapter.ipc_client.is_none());
        assert!(adapter.ipc_endpoint.is_none());
    }
}

#[test]
fn json_ipc_initialization_rejects_missing_or_unrecognized_versions() {
    let cases = [
        (
            r#"{"request_id":1,"error":"property unavailable"}"#,
            "does not expose the mpv-version property",
        ),
        (
            r#"{"request_id":1,"error":"success","data":null}"#,
            "did not report an mpv-version",
        ),
        (
            r#"{"request_id":1,"error":"success","data":"custom-build"}"#,
            "reported an unrecognized mpv-version",
        ),
        (
            r#"{"request_id":1,"error":"success","data":"0.41"}"#,
            "reported an unrecognized mpv-version",
        ),
    ];

    for (response, expected_reason) in cases {
        let (adapter, result) = initialize_with_version_response(response);
        let message = operation_failure_message(result);

        assert!(message.contains("requires mpv 0.41.0 or newer"));
        assert!(
            message.contains(expected_reason),
            "unexpected error: {message}"
        );
        assert!(adapter.ipc_client.is_none());
        assert!(adapter.ipc_endpoint.is_none());
    }
}

#[test]
fn unsupported_version_predicate_does_not_match_unrelated_operation_failures() {
    assert!(!crate::is_unsupported_mpv_version_error(
        &PlayerError::OperationFailed("mpv IPC connection timed out".to_owned())
    ));
    assert_eq!(crate::MINIMUM_SUPPORTED_MPV_VERSION, "0.41.0");
}

#[test]
fn protocol_failures_are_not_misclassified_as_version_rejections() {
    let (_adapter, result) = initialize_with_version_response("not-json");
    let error = result.expect_err("invalid IPC JSON must fail initialization");

    assert!(!crate::is_unsupported_mpv_version_error(&error));
    assert!(
        matches!(error, PlayerError::OperationFailed(message) if message.contains("invalid mpv IPC JSON"))
    );
}
