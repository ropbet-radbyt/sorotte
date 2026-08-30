use sorotte_client_app::app_boundary::application::{
    ClientApplication, ClientCommand, ClientEvent,
};
use sorotte_client_app::app_boundary::commands::{
    PlannedLocalRuntimeAction, plan_local_runtime_dispatch_legacy_compatible,
};
use sorotte_player_api::PlayerPlaybackTelemetryUpdate;
use sorotte_player_mpv::{
    MpvAdapter, MpvNetworkMediaDiagnosticSnapshot, MpvNetworkMediaPolicyState,
    MpvNetworkOptionsHookHealth, MpvNetworkOptionsRuntimeHealthSnapshot,
};
use sorotte_protocol::DirectReadinessSurface;

use crate::client_config::ClientLoopConfig;
use crate::language_support::current_legacy_runtime_language_tag_legacy_compatible;

pub(super) const PLAYER_CHAT_INPUT_POLL_INTERVAL_MS: u64 = 100;

#[derive(Debug, Default)]
pub(super) struct CliNetworkOptionsHealthReporter {
    last_revision: Option<u64>,
    last_reported_hook_failure: Option<String>,
    last_reported_policy_failure: Option<String>,
    player_telemetry_diagnostics_enabled: bool,
    last_network_media_diagnostic_line: Option<String>,
}

impl CliNetworkOptionsHealthReporter {
    pub(crate) fn set_player_telemetry_diagnostics_enabled(&mut self, enabled: bool) {
        self.player_telemetry_diagnostics_enabled = enabled;
        self.last_network_media_diagnostic_line = None;
    }

    fn current_failure(&self) -> Option<&str> {
        self.last_reported_hook_failure
            .as_deref()
            .or(self.last_reported_policy_failure.as_deref())
    }

    fn lines_for_snapshot(
        &mut self,
        snapshot: MpvNetworkOptionsRuntimeHealthSnapshot,
    ) -> Vec<String> {
        if self
            .last_revision
            .is_some_and(|last_revision| snapshot.revision <= last_revision)
        {
            return Vec::new();
        }
        self.last_revision = Some(snapshot.revision);

        let mut lines = Vec::new();
        match snapshot.hook_health {
            MpvNetworkOptionsHookHealth::Ready => {
                if self.last_reported_hook_failure.take().is_some() {
                    lines.push("info: mpv streaming-options hook recovered".to_owned());
                }
            }
            MpvNetworkOptionsHookHealth::Degraded(reason) => {
                if self.last_reported_hook_failure.as_deref() != Some(reason.as_str()) {
                    lines.push(network_media_options_warning_message(&reason));
                    self.last_reported_hook_failure = Some(reason);
                }
            }
            MpvNetworkOptionsHookHealth::Pending => {}
        }
        match snapshot.media_policy {
            MpvNetworkMediaPolicyState::NoActiveMedia
            | MpvNetworkMediaPolicyState::LocalMediaUnchanged
            | MpvNetworkMediaPolicyState::NetworkMediaUpdated => {
                if self.last_reported_policy_failure.take().is_some() {
                    lines.push("info: mpv streaming options for active media recovered".to_owned());
                }
            }
            MpvNetworkMediaPolicyState::Failed(reason) => {
                if self.last_reported_policy_failure.as_deref() != Some(reason.as_str()) {
                    lines.push(network_media_options_warning_message(&reason));
                    self.last_reported_policy_failure = Some(reason);
                }
            }
            MpvNetworkMediaPolicyState::Unknown
            | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad => {}
        }
        lines
    }

    fn line_for_network_media_diagnostic_snapshot(
        &mut self,
        snapshot: &MpvNetworkMediaDiagnosticSnapshot,
    ) -> Option<String> {
        if !self.player_telemetry_diagnostics_enabled {
            return None;
        }
        let line = network_media_diagnostic_support_line(snapshot);
        if self.last_network_media_diagnostic_line.as_deref() == Some(line.as_str()) {
            return None;
        }
        self.last_network_media_diagnostic_line = Some(line.clone());
        Some(line)
    }
}

fn cache_options_support_text(options: &std::collections::BTreeMap<String, String>) -> String {
    options
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn network_option_results_support_text(snapshot: &MpvNetworkMediaDiagnosticSnapshot) -> String {
    let mut results = snapshot.option_results.iter().collect::<Vec<_>>();
    results.sort_by(|left, right| left.name.cmp(&right.name));
    results
        .into_iter()
        .map(|result| format!("{}={:?}", result.name, result.status))
        .collect::<Vec<_>>()
        .join(",")
}

fn network_media_diagnostic_support_line(snapshot: &MpvNetworkMediaDiagnosticSnapshot) -> String {
    let media_generation = snapshot
        .media_generation
        .map(|generation| generation.get().to_string())
        .unwrap_or_else(|| "none".to_owned());
    let load_sequence = snapshot
        .load_sequence
        .map(|sequence| sequence.to_string())
        .unwrap_or_else(|| "none".to_owned());
    format!(
        "diagnostic: mpv-network-media media-generation={media_generation} policy-generation={} load-sequence={load_sequence} application={:?} verification-complete={} option-results=[{}] desired-cache=[{}] effective-cache=[{}] transport={:?} cache-pause={:?} demuxer-idle={:?} cache-duration-seconds={:?} forward-bytes={:?} input-rate-bytes-per-second={:?} reader-position-seconds={:?} cache-end-seconds={:?} cache-eof={:?} cache-underrun={:?}",
        snapshot.network_policy_generation,
        snapshot.application_state,
        snapshot.verification_complete,
        network_option_results_support_text(snapshot),
        cache_options_support_text(&snapshot.desired_cache_options),
        cache_options_support_text(&snapshot.effective_cache_options),
        snapshot.transport_phase,
        snapshot.paused_for_cache,
        snapshot.demuxer_cache_idle,
        snapshot.cache_duration_seconds,
        snapshot.forward_bytes,
        snapshot.raw_input_rate_bytes_per_second,
        snapshot.reader_position_seconds,
        snapshot.cache_end_seconds,
        snapshot.cache_eof,
        snapshot.cache_underrun,
    )
}

pub(super) fn publish_pending_local_file_updates(
    application: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
    network_options_health_reporter: &mut CliNetworkOptionsHealthReporter,
    now_seconds: f64,
) -> anyhow::Result<()> {
    loop {
        let published = application.publish_pending_local_file_update_legacy_compatible_at(
            config.filename_privacy_mode,
            config.filesize_privacy_mode,
            now_seconds,
        );
        surface_network_media_options_transition_outcomes(
            application,
            network_options_health_reporter,
        )?;
        let published = published?;
        if !published {
            break;
        }
    }
    Ok(())
}

fn surface_network_media_options_transition_outcomes(
    application: &mut ClientApplication<MpvAdapter>,
    reporter: &mut CliNetworkOptionsHealthReporter,
) -> anyhow::Result<()> {
    surface_network_media_options_transition_outcomes_to_sink(application, reporter, |line| {
        eprintln!("{line}");
    })
}

fn surface_network_media_options_transition_outcomes_to_sink(
    application: &mut ClientApplication<MpvAdapter>,
    reporter: &mut CliNetworkOptionsHealthReporter,
    mut emit: impl FnMut(String),
) -> anyhow::Result<()> {
    while application
        .with_player_io(MpvAdapter::take_network_options_hook_health_transition_nonblocking)
        .is_some()
    {}
    while application
        .with_player_io(MpvAdapter::take_network_media_policy_outcome_nonblocking)
        .is_some()
    {}
    let (snapshot, diagnostic_snapshot, player_connected) = application.with_player_io(|player| {
        (
            player.network_options_runtime_health_snapshot(),
            player.network_media_diagnostic_snapshot(),
            player.is_connected(),
        )
    });
    let mut lines = reporter.lines_for_snapshot(snapshot);
    if let Some(line) = reporter.line_for_network_media_diagnostic_snapshot(&diagnostic_snapshot) {
        lines.push(line);
    }
    if !player_connected && let Some(error) = reporter.current_failure() {
        return Err(network_media_options_fatal_error(error));
    }
    for line in lines {
        emit(line);
    }
    Ok(())
}

fn network_media_options_warning_message(error: &str) -> String {
    format!(
        "warning: mpv playback remains available, but streaming options need attention: {error}; desired options remain configured for later network transitions"
    )
}

fn network_media_options_fatal_error(error: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "mpv JSON IPC became unavailable while applying streaming options to externally activated network media: {error}"
    )
}

pub(super) fn drain_player_chat_input_legacy_compatible(
    application: &mut ClientApplication<MpvAdapter>,
) -> anyhow::Result<bool> {
    let mut sent = 0usize;
    while let Some(message) =
        application.with_player_io(MpvAdapter::take_pending_chat_request_nonblocking)
    {
        if application.run_send_chat_message(message)? {
            sent += 1;
        }
    }
    Ok(sent > 0)
}

fn command_result(events: Vec<ClientEvent>) -> anyhow::Result<bool> {
    if let Some(ClientEvent::OperationFailed { message, .. }) = events
        .iter()
        .find(|event| matches!(event, ClientEvent::OperationFailed { .. }))
    {
        return Err(anyhow::anyhow!(message.clone()));
    }
    Ok(events
        .iter()
        .find_map(ClientEvent::command_changed)
        .unwrap_or(false))
}

pub(super) fn run_planned_local_runtime_action_legacy_compatible(
    application: &mut ClientApplication<MpvAdapter>,
    user_offset_seconds: &mut f64,
    now_seconds: f64,
    action: PlannedLocalRuntimeAction,
) -> anyhow::Result<bool> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let dispatch = plan_local_runtime_dispatch_legacy_compatible(
        application.session(),
        *user_offset_seconds,
        action,
        language.as_deref(),
    );
    if let Some(updated_user_offset_seconds) = dispatch.updated_user_offset_seconds {
        *user_offset_seconds = updated_user_offset_seconds;
    }
    if let Some(line_to_emit) = dispatch.line_to_emit {
        println!("{line_to_emit}");
    }

    let command = match dispatch.action {
        Some(PlannedLocalRuntimeAction::SendChat(message)) => {
            Some(ClientCommand::SendChat(message))
        }
        Some(PlannedLocalRuntimeAction::RequestUserList) => Some(ClientCommand::RequestUserList),
        Some(PlannedLocalRuntimeAction::SetPlaylistIndex(index)) => {
            Some(ClientCommand::SetPlaylistIndex(index))
        }
        Some(PlannedLocalRuntimeAction::AdvancePlaylistIndex) => {
            Some(ClientCommand::AdvancePlaylistIndex)
        }
        Some(PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }) => Some(ClientCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }),
        Some(PlannedLocalRuntimeAction::DeletePlaylistIndex(index)) => {
            Some(ClientCommand::DeletePlaylistIndex(index))
        }
        Some(PlannedLocalRuntimeAction::UndoPlaylistChange) => {
            Some(ClientCommand::UndoPlaylistChange)
        }
        Some(PlannedLocalRuntimeAction::ShuffleRemainingPlaylist) => {
            Some(ClientCommand::ShuffleRemainingPlaylist)
        }
        Some(PlannedLocalRuntimeAction::ShuffleEntirePlaylist) => {
            Some(ClientCommand::ShuffleEntirePlaylist)
        }
        Some(PlannedLocalRuntimeAction::UndoSeek) => Some(ClientCommand::UndoSeek),
        Some(PlannedLocalRuntimeAction::KeepWaitingForSeekPreparation) => {
            return Ok(application.run_keep_waiting_for_seek_preparation(now_seconds)?);
        }
        Some(PlannedLocalRuntimeAction::JoinNearestBufferedSeekPreparation) => {
            return Ok(application.run_join_nearest_buffered_seek_preparation(now_seconds)?);
        }
        Some(PlannedLocalRuntimeAction::CancelSeekPreparation) => {
            return Ok(application.run_cancel_seek_preparation(now_seconds)?);
        }
        Some(PlannedLocalRuntimeAction::SetUserOffset(_)) => None,
        Some(PlannedLocalRuntimeAction::SeekToPosition(position_seconds)) => {
            Some(ClientCommand::SeekToPosition(position_seconds))
        }
        Some(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds)) => {
            Some(ClientCommand::SeekByOffset(offset_seconds))
        }
        Some(PlannedLocalRuntimeAction::Play) => Some(ClientCommand::SetPaused(false)),
        Some(PlannedLocalRuntimeAction::Pause) => Some(ClientCommand::SetPaused(true)),
        Some(PlannedLocalRuntimeAction::TogglePause) => {
            let paused = application.player().paused();
            let position_seconds = application.player().position_seconds();
            let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(paused)
                    .with_position_seconds(position_seconds),
            ));
            Some(ClientCommand::TogglePause)
        }
        Some(PlannedLocalRuntimeAction::ToggleReady) => Some(ClientCommand::SetReadyFrom {
            username: None,
            ready: None,
            manually_initiated: true,
            surface: DirectReadinessSurface::CliCommand,
        }),
        Some(PlannedLocalRuntimeAction::SetUserReady { username, ready }) => {
            Some(ClientCommand::SetReadyFrom {
                username: Some(username),
                ready: Some(ready),
                manually_initiated: true,
                surface: DirectReadinessSurface::CliCommand,
            })
        }
        Some(PlannedLocalRuntimeAction::RequestControllerAuth { room, password }) => {
            Some(ClientCommand::RequestControllerAuth { room, password })
        }
        Some(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(room)) => {
            Some(ClientCommand::SetRoom {
                room,
                legacy_fallback: true,
            })
        }
        Some(PlannedLocalRuntimeAction::SetRoom(room)) => Some(ClientCommand::SetRoom {
            room,
            legacy_fallback: false,
        }),
        None => None,
    };

    command.map_or(Ok(false), |command| {
        command_result(application.dispatch(command))
    })
}

#[cfg(test)]
mod network_media_options_transition_outcome_tests {
    use super::*;
    use sorotte_player_mpv::{
        LegacySyncplayUiSettings, MpvNetworkOptionApplyResult, MpvNetworkOptionApplyStatus,
    };

    fn snapshot(
        revision: u64,
        hook_health: MpvNetworkOptionsHookHealth,
        media_policy: MpvNetworkMediaPolicyState,
    ) -> MpvNetworkOptionsRuntimeHealthSnapshot {
        MpvNetworkOptionsRuntimeHealthSnapshot {
            revision,
            hook_health,
            media_policy,
        }
    }

    #[test]
    fn reporter_emits_each_authoritative_failure_and_recovery_once() {
        let mut reporter = CliNetworkOptionsHealthReporter::default();
        let hook_failure = "hook lease expired";
        let policy_failure = "active network option rejected";

        let hook_degraded = snapshot(
            1,
            MpvNetworkOptionsHookHealth::Degraded(hook_failure.to_owned()),
            MpvNetworkMediaPolicyState::Unknown,
        );
        assert_eq!(reporter.lines_for_snapshot(hook_degraded.clone()).len(), 1);
        assert!(reporter.lines_for_snapshot(hook_degraded).is_empty());
        assert!(
            reporter
                .lines_for_snapshot(snapshot(
                    2,
                    MpvNetworkOptionsHookHealth::Degraded(hook_failure.to_owned()),
                    MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad,
                ))
                .is_empty(),
            "an unrelated newer revision must not repeat an unchanged hook warning"
        );

        let policy_degraded = reporter.lines_for_snapshot(snapshot(
            3,
            MpvNetworkOptionsHookHealth::Degraded(hook_failure.to_owned()),
            MpvNetworkMediaPolicyState::Failed(policy_failure.to_owned()),
        ));
        assert_eq!(policy_degraded.len(), 1);
        assert!(policy_degraded[0].contains(policy_failure));

        assert_eq!(
            reporter.lines_for_snapshot(snapshot(
                4,
                MpvNetworkOptionsHookHealth::Ready,
                MpvNetworkMediaPolicyState::Failed(policy_failure.to_owned()),
            )),
            vec!["info: mpv streaming-options hook recovered"]
        );
        assert!(
            reporter
                .lines_for_snapshot(snapshot(
                    4,
                    MpvNetworkOptionsHookHealth::Ready,
                    MpvNetworkMediaPolicyState::Failed(policy_failure.to_owned()),
                ))
                .is_empty()
        );
        assert_eq!(
            reporter.lines_for_snapshot(snapshot(
                5,
                MpvNetworkOptionsHookHealth::Ready,
                MpvNetworkMediaPolicyState::NetworkMediaUpdated,
            )),
            vec!["info: mpv streaming options for active media recovered"]
        );
        assert!(
            reporter
                .lines_for_snapshot(snapshot(
                    6,
                    MpvNetworkOptionsHookHealth::Ready,
                    MpvNetworkMediaPolicyState::NetworkMediaUpdated,
                ))
                .is_empty(),
            "a newer healthy revision must not repeat either recovery"
        );
    }

    #[test]
    fn pending_states_preserve_independent_reported_failures() {
        let mut reporter = CliNetworkOptionsHealthReporter::default();
        assert_eq!(
            reporter
                .lines_for_snapshot(snapshot(
                    1,
                    MpvNetworkOptionsHookHealth::Degraded("hook failure".to_owned()),
                    MpvNetworkMediaPolicyState::Failed("policy failure".to_owned()),
                ))
                .len(),
            2
        );
        assert!(
            reporter
                .lines_for_snapshot(snapshot(
                    2,
                    MpvNetworkOptionsHookHealth::Pending,
                    MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad,
                ))
                .is_empty()
        );
        assert_eq!(reporter.current_failure(), Some("hook failure"));
        assert_eq!(
            reporter.lines_for_snapshot(snapshot(
                3,
                MpvNetworkOptionsHookHealth::Ready,
                MpvNetworkMediaPolicyState::LocalMediaUnchanged,
            )),
            vec![
                "info: mpv streaming-options hook recovered",
                "info: mpv streaming options for active media recovered",
            ]
        );
    }

    #[test]
    fn repeated_runtime_drains_do_not_repeat_streaming_health_lines() {
        let (mut adapter, _commands, _transition_trigger) =
            MpvAdapter::with_external_network_media_transition_test_ipc(
                LegacySyncplayUiSettings::default(),
            );
        adapter.inject_test_network_media_options_hook_degradation("test hook loss");
        let mut application = ClientApplication::with_default_session(adapter);
        let mut reporter = CliNetworkOptionsHealthReporter::default();
        let mut lines = Vec::new();

        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("an attached hook failure should remain a warning");
        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("a repeated drain should remain healthy");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("test hook loss"));

        application.with_player_io(MpvAdapter::inject_test_network_media_options_hook_recovery);
        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("hook recovery should remain healthy");
        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("a repeated recovery drain should remain healthy");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("test hook loss"));
        assert_eq!(lines[1], "info: mpv streaming-options hook recovered");
    }

    #[test]
    fn credential_bearing_targets_never_reach_cli_warning_or_fatal_text() {
        const CANARY: &str = "SOROTTE-CLI-RAW-TARGET-CANARY";
        let cases = [
            (
                format!("https://alice:{CANARY}@example.test/media"),
                "https://cdn.example.test/media".to_owned(),
            ),
            (
                format!("https://example.test/media?sig={CANARY}"),
                format!("https://example.test/media?sig={CANARY}"),
            ),
            (
                format!("https://example.test/media?auth={CANARY}"),
                format!("https://example.test/media?auth={CANARY}"),
            ),
            (
                format!("https://example.test/media?X-Amz-Signature={CANARY}"),
                format!("https://example.test/media?X-Amz-Signature={CANARY}"),
            ),
            (
                "https://example.test/watch/1".to_owned(),
                format!("edl://nested.example.test/video?token={CANARY}"),
            ),
            (
                format!("C:/Users/{CANARY}/private/movie.mkv"),
                format!("https://cdn.example.test/video?opaque={CANARY}"),
            ),
        ];

        for (source, resolved) in cases {
            let mut adapter = MpvAdapter::default();
            adapter.inject_test_network_media_options_policy_failure(42, &source, &resolved);
            let mut reporter = CliNetworkOptionsHealthReporter::default();
            let lines =
                reporter.lines_for_snapshot(adapter.network_options_runtime_health_snapshot());
            let warning = lines
                .first()
                .expect("the scoped warning should remain active");
            assert!(!warning.contains(CANARY));
            assert!(!warning.contains(&source));
            assert!(!warning.contains(&resolved));
            assert!(warning.contains("hook load 42"));

            let fatal = network_media_options_fatal_error(
                reporter
                    .current_failure()
                    .expect("the failure should remain available for fatal surfacing"),
            )
            .to_string();
            assert!(!fatal.contains(CANARY));
            assert!(!fatal.contains(&source));
            assert!(!fatal.contains(&resolved));
        }
    }

    #[test]
    fn production_support_projection_is_change_gated_and_excludes_advanced_secrets() {
        const CANARY: &str = "SOROTTE-NETWORK-DIAGNOSTIC-SECRET-CANARY";
        let mut adapter = MpvAdapter::default();
        adapter.configure_network_media_options([
            ("cache-secs".to_owned(), "60".to_owned()),
            ("demuxer-max-bytes".to_owned(), "524288".to_owned()),
            (
                "http-header-fields".to_owned(),
                format!("Authorization: Bearer {CANARY}"),
            ),
        ]);
        adapter.inject_test_verified_network_media_diagnostic_snapshot(42);
        let expected_policy_generation = adapter
            .network_media_diagnostic_snapshot()
            .network_policy_generation;
        let mut application = ClientApplication::with_default_session(adapter);
        let mut reporter = CliNetworkOptionsHealthReporter::default();
        reporter.set_player_telemetry_diagnostics_enabled(true);
        let mut lines = Vec::new();

        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("a successful diagnostic projection should remain nonfatal");
        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("an unchanged diagnostic projection should remain nonfatal");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("diagnostic: mpv-network-media"))
                .count(),
            1,
        );

        reporter.set_player_telemetry_diagnostics_enabled(true);
        surface_network_media_options_transition_outcomes_to_sink(
            &mut application,
            &mut reporter,
            |line| lines.push(line),
        )
        .expect("a new connected-session boundary should emit an initial projection");

        let diagnostic_lines = lines
            .iter()
            .filter(|line| line.starts_with("diagnostic: mpv-network-media"))
            .collect::<Vec<_>>();
        assert_eq!(diagnostic_lines.len(), 2);
        assert_eq!(diagnostic_lines[0], diagnostic_lines[1]);
        let line = diagnostic_lines[0];
        assert!(line.contains("media-generation=1"));
        assert!(line.contains(&format!("policy-generation={expected_policy_generation}")));
        assert!(line.contains("load-sequence=42"));
        assert!(line.contains("application=Some(Applied)"));
        assert!(line.contains("verification-complete=true"));
        assert!(line.contains("cache-secs=Applied"));
        assert!(line.contains("demuxer-max-bytes=Applied"));
        assert!(line.contains("http-header-fields=Applied"));
        assert!(line.contains("cache-secs=60"));
        assert!(line.contains("demuxer-max-bytes=524288"));
        assert!(line.contains("forward-bytes=Some(524288)"));
        assert!(!line.contains(CANARY));
    }

    #[test]
    fn support_projection_orders_and_labels_every_option_result_status() {
        let snapshot = MpvNetworkMediaDiagnosticSnapshot {
            option_results: vec![
                MpvNetworkOptionApplyResult {
                    name: "zeta-option".to_owned(),
                    status: MpvNetworkOptionApplyStatus::Rejected,
                },
                MpvNetworkOptionApplyResult {
                    name: "alpha-option".to_owned(),
                    status: MpvNetworkOptionApplyStatus::Mismatched,
                },
                MpvNetworkOptionApplyResult {
                    name: "middle-option".to_owned(),
                    status: MpvNetworkOptionApplyStatus::Applied,
                },
            ],
            ..MpvNetworkMediaDiagnosticSnapshot::default()
        };

        assert!(network_media_diagnostic_support_line(&snapshot).contains(
            "option-results=[alpha-option=Mismatched,middle-option=Applied,zeta-option=Rejected]"
        ));
    }
}
