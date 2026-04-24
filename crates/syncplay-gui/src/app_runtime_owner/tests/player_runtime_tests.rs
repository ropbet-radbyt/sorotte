use super::*;
use crate::app::runtime_owner::GuiPendingStreamLoadContext;
use crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome;
use crate::app::{GuiStreamHelperHealth, GuiStreamHelperRuntimeSnapshot};
use syncplay_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible;

fn write_persisted_media_search_root_index(
    gui_root: &std::path::Path,
    media_root: &std::path::Path,
    built_at_unix_ms: u64,
    candidates_by_name: &[(&str, &[&str])],
) {
    let persisted = crate::app::media_search_cache::PersistedMediaSearchRootIndexV1 {
        version: 1,
        root_key: crate::app::media_search_cache::normalized_media_search_root_key(media_root),
        root_path: media_root.to_string_lossy().into_owned(),
        built_at_unix_ms,
        candidates_by_name: candidates_by_name
            .iter()
            .map(|(name, candidates)| {
                (
                    (*name).to_owned(),
                    candidates
                        .iter()
                        .map(|candidate| (*candidate).to_owned())
                        .collect(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>(),
    };
    crate::app::media_search_cache::persist_media_search_root_index_at_root(gui_root, &persisted)
        .expect("persisted media-search cache fixture should be written");
}

#[test]
fn gui_persisted_config_runtime_owner_syncs_attached_player_runtime_state() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: Vec<syncplay_player_api::LocalFileUpdate>,
        playback_updates: Vec<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_transport_reconnect_due_at: None,
        session_transport_reconnect_failures: 0,
        session_transport_disconnect_pending_cleanup: false,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_local_file_placeholder: false,
        last_published_local_file: None,
        attached_media_search_index: None,
        attached_media_search_next_retry_at: None,
        pending_attached_media_resolution: None,
        attached_media_search_progress: None,
        attached_media_search_progress_updated_at: None,
        attached_media_search_build_state: GuiAttachedMediaSearchBuildState::Idle,
        attached_media_search_build_roots: Vec::new(),
        attached_media_search_job_sequence: 0,
        attached_media_search_index_revision: 0,
        unresolved_attached_media_target: None,
        last_attached_media_resolution_trigger: None,
        last_applied_attached_room_playstate: None,
        suppressed_attached_room_playstate_after_playlist_reset: None,
        pending_local_attached_pause_override: None,
        player_position_seconds: None,
        player_paused: None,
        active_shared_playlist_index: None,
        playlist_auto_advance_eof_latched: false,
        user_offset_seconds: 0.0,
        stream_helper_runtime_snapshot: Default::default(),
        stream_helper_remediation_runtime_snapshot: Default::default(),
        pending_stream_retry_target: None,
        managed_stream_helper_refresh_required: false,
        pending_stream_feedback: std::collections::VecDeque::new(),
        pending_stream_load_context: None,
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let bootstrap_actions = handle.drain_actions();
    assert_eq!(
        bootstrap_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Play",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Toggle Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Seek",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Set Offset",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: false,
                update_notice_expected: false,
                about_dialog_available: true,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: false,
                    can_search_missing_media: false,
                    can_toggle_pause: true,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in bootstrap_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.commands.can_toggle_pause);

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));
    assert!(
        state.commands.can_send_chat_message,
        "config-driven chat availability should update immediately when no runtime field override is active"
    );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let refreshed_command_actions = handle.drain_actions();
    assert!(refreshed_command_actions.is_empty());
    for action in refreshed_command_actions {
        assert!(state.apply(action));
    }
    assert!(state.commands.can_send_chat_message);
    assert!(state.commands.can_reset_configuration);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_file_updates
        .push(
            syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_duration_seconds(93.5)
                .with_size_bytes(734003200),
        );
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let local_file_actions = handle.drain_actions();
    assert_eq!(
        local_file_actions,
        vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv [93.500s, 734003200 bytes]".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            },
        )]
    );
    for action in local_file_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv [93.500s, 734003200 bytes]"]
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let paused_actions = handle.drain_actions();
    assert_eq!(
        paused_actions,
        vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv [93.500s, 734003200 bytes]".to_owned()],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: true,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            },
        )]
    );
    for action in paused_actions {
        assert!(state.apply(action));
    }

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(
        handle.drain_actions().is_empty(),
        "idle runtime pumps should not emit redundant player projection actions"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_clears_placeholder_after_media_load_failure() {
    #[derive(Default)]
    struct FailingLoadPlayerAdapter {
        outcomes: Vec<syncplay_player_api::PlayerMediaLoadOutcome>,
    }

    impl PlayerAdapter for FailingLoadPlayerAdapter {
        fn name(&self) -> &'static str {
            "failing-load"
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerMediaLoadOutcome> {
            self.outcomes.pop()
        }
    }

    let requested_target = "https://cdn.example.com/broken.m3u8".to_owned();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(FailingLoadPlayerAdapter {
        outcomes: vec![syncplay_player_api::PlayerMediaLoadOutcome::failure(
            requested_target.clone(),
            None,
            syncplay_player_api::PlayerMediaLoadFailureKind::Unknown,
            "network timeout",
        )],
    })));
    owner.player_local_file =
        Some(GuiPersistedConfigRuntimeOwner::placeholder_local_file_for_path(&requested_target));
    owner.player_local_file_placeholder = true;
    owner.pending_stream_retry_target = Some(requested_target.clone());
    owner.pending_stream_load_context = Some(GuiPendingStreamLoadContext {
        requested_target: requested_target.clone(),
        user_initiated: true,
    });

    owner.refresh_player_state_impl();

    assert_eq!(owner.player_local_file, None);
    assert!(!owner.player_local_file_placeholder);
    assert_eq!(owner.player_position_seconds, None);
    assert_eq!(
        owner.pending_stream_retry_target.as_deref(),
        Some(requested_target.as_str())
    );
    assert_eq!(owner.pending_stream_load_context, None);
    assert_eq!(owner.pending_stream_feedback.len(), 1);
    let actions = owner
        .pending_stream_feedback
        .front()
        .expect("media-load failure should queue GUI feedback");
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message.contains("network timeout")
    )));
}

#[test]
fn gui_persisted_config_runtime_owner_resets_stale_position_when_the_player_reports_a_new_file() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque<syncplay_player_api::LocalFileUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop_front()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState {
        local_file_updates: std::collections::VecDeque::from([
            syncplay_player_api::LocalFileUpdate::new("episode2.mkv")
                .with_path("C:/Media/episode2.mkv"),
        ]),
    }));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TelemetryPlayerAdapter {
        state: player_state,
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv"),
    );
    owner.player_position_seconds = Some(42.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .ensure_detached_client_core_chat_session(&state)
        .expect("detached client-core session should bootstrap");

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        owner.player_position_seconds,
        Some(0.0),
        "a newly reported file should reset the stored global playback position"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(0.0),
        "detached-session telemetry should publish the new file from the start instead of reusing the old timestamp"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_uses_attached_player_for_media_open_and_seek() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        local_file_updates: Vec<syncplay_player_api::LocalFileUpdate>,
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.opened_paths.push(path.to_owned());
            state.local_file_updates.push(
                syncplay_player_api::LocalFileUpdate::new(
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path),
                )
                .with_path(path.to_owned()),
            );
            Ok(())
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_transport_reconnect_due_at: None,
        session_transport_reconnect_failures: 0,
        session_transport_disconnect_pending_cleanup: false,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_local_file_placeholder: false,
        last_published_local_file: None,
        attached_media_search_index: None,
        attached_media_search_next_retry_at: None,
        pending_attached_media_resolution: None,
        attached_media_search_progress: None,
        attached_media_search_progress_updated_at: None,
        attached_media_search_build_state: GuiAttachedMediaSearchBuildState::Idle,
        attached_media_search_build_roots: Vec::new(),
        attached_media_search_job_sequence: 0,
        attached_media_search_index_revision: 0,
        unresolved_attached_media_target: None,
        last_attached_media_resolution_trigger: None,
        last_applied_attached_room_playstate: None,
        suppressed_attached_room_playstate_after_playlist_reset: None,
        pending_local_attached_pause_override: None,
        player_position_seconds: None,
        player_paused: None,
        active_shared_playlist_index: None,
        playlist_auto_advance_eof_latched: false,
        user_offset_seconds: 0.0,
        stream_helper_runtime_snapshot: Default::default(),
        stream_helper_remediation_runtime_snapshot: Default::default(),
        pending_stream_retry_target: None,
        managed_stream_helper_refresh_required: false,
        pending_stream_feedback: std::collections::VecDeque::new(),
        pending_stream_load_context: None,
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let open_actions = handle.drain_actions();
    assert_eq!(
        open_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: true,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                active_playlist_index: Some(0),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_offset: false,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Loaded 2 selected media entries into the shared playlist."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Loaded 2 selected media entries into the shared playlist."
                    .to_owned(),
            ),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
                    .to_owned(),
            ),
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                room_control_status: "Unavailable: no active server session.".to_owned(),
                shared_playlist_enabled: true,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                active_playlist_index: Some(0),
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: true,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                GuiStreamHelperRuntimeSnapshot {
                    health: GuiStreamHelperHealth::Healthy,
                    message: None,
                    target: None,
                    install_supported: false,
                    integration_supported: false,
                    retry_available: false,
                    install_location: None,
                    downloader_status: Some(
                        "Missing from Syncplay's managed install and PATH for yt-dlp."
                            .to_owned(),
                    ),
                    js_runtime_status: Some(
                        "Missing from Syncplay's managed install and PATH for Deno."
                            .to_owned(),
                    ),
                    open_install_location_available: false,
                },
            ),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Play",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Toggle Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Seek",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Shared Playlist",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Set Offset",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_saved_server: false,
                    can_disconnect_session: false,
                    can_connect_public_server: false,
                    can_refresh_public_servers: true,
                    can_search_missing_media: false,
                    can_toggle_pause: true,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in open_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv", "episode2.mkv"]
    );

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::TogglePlaybackPause,
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let toggle_actions = handle.drain_actions();
    assert!(
        toggle_actions.contains(&GuiShellAction::CompletePlaybackPauseToggle),
        "pending pause-toggle completion should still emit the completion action",
    );
    for action in toggle_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback_paused);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec!["C:/Media/episode1.mkv".to_owned()]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true]
    );

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let direct_toggle_actions = handle.drain_actions();
    assert!(
        direct_toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "direct pause toggles should still resume playback",
    );
    for action in direct_toggle_actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.playback_paused);

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Applied a 12.5 second seek via the attached recording player (target 12.500 seconds)."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Applied a 12.5 second seek via the attached recording player (target 12.500 seconds)."
                    .to_owned(),
            ),
        ]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(-2.5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Applied a -2.5 second seek via the attached recording player (target 10.000 seconds)."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Applied a -2.5 second seek via the attached recording player (target 10.000 seconds)."
                    .to_owned(),
            ),
        ]
    );

    handle.push_request(GuiRuntimeRequest::SeekToPosition(42.0));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: "Applied an absolute seek via the attached recording player (target 42.000 seconds)."
                    .to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Applied an absolute seek via the attached recording player (target 42.000 seconds)."
                    .to_owned(),
            ),
        ]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![12.5, 10.0, 42.0]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true, false]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_commit_undo_seek_when_player_seek_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_position_attempts: usize,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            _position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_position_attempts += 1;
            Err(syncplay_player_api::PlayerError::OperationFailed(
                "seek failed".to_owned(),
            ))
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(20.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    owner
        .ensure_detached_client_core_chat_session(&state)
        .expect("detached client-core session should bootstrap");

    {
        let session = owner.session.as_mut().expect("session should exist");
        session
            .sync_local_playback_telemetry(Some(false), Some(10.0))
            .expect("initial local telemetry should sync");
        let _ = session
            .record_manual_seek_to_position(20.0)
            .expect("manual seek should record undo state");
        session
            .sync_local_playback_telemetry(Some(false), Some(20.0))
            .expect("post-seek local telemetry should sync");
    }

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();

    handle.push_request(GuiRuntimeRequest::UndoSeek);
    let undo_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        undo_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Error
                    && message.contains("Playback undo seek through the attached recording player failed")
        )),
        "failed undo seek should surface the player seek error"
    );
    assert_eq!(owner.player_position_seconds, Some(20.0));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(20.0),
        "the detached runtime should keep the pre-undo local position when the player seek fails"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.pending_undo_seek_target_position()),
        Some(10.0),
        "the undo target should remain available after a failed player seek"
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_position_attempts,
        1
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_offset_commands_on_global_timeline() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_transport_reconnect_due_at: None,
        session_transport_reconnect_failures: 0,
        session_transport_disconnect_pending_cleanup: false,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
            state: player_state.clone(),
        }))),
        player_launch_state: GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_local_file_placeholder: false,
        last_published_local_file: None,
        attached_media_search_index: None,
        attached_media_search_next_retry_at: None,
        pending_attached_media_resolution: None,
        attached_media_search_progress: None,
        attached_media_search_progress_updated_at: None,
        attached_media_search_build_state: GuiAttachedMediaSearchBuildState::Idle,
        attached_media_search_build_roots: Vec::new(),
        attached_media_search_job_sequence: 0,
        attached_media_search_index_revision: 0,
        unresolved_attached_media_target: None,
        last_attached_media_resolution_trigger: None,
        last_applied_attached_room_playstate: None,
        suppressed_attached_room_playstate_after_playlist_reset: None,
        pending_local_attached_pause_override: None,
        player_position_seconds: Some(100.0),
        player_paused: Some(false),
        active_shared_playlist_index: None,
        playlist_auto_advance_eof_latched: false,
        user_offset_seconds: 0.0,
        stream_helper_runtime_snapshot: Default::default(),
        stream_helper_remediation_runtime_snapshot: Default::default(),
        pending_stream_retry_target: None,
        managed_stream_helper_refresh_required: false,
        pending_stream_feedback: std::collections::VecDeque::new(),
        pending_stream_load_context: None,
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    handle.push_request(GuiRuntimeRequest::SetOffset(
        syncplay_client_app::app_boundary::commands::LocalOffsetCommand::Absolute(5.0),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();
    assert_eq!(owner.user_offset_seconds, 5.0);
    assert_eq!(
        owner.player_position_seconds,
        Some(100.0),
        "changing offset should not rewrite the stored global position"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(100.0),
        "offset changes should keep detached-session telemetry on the global timeline"
    );

    handle.push_request(GuiRuntimeRequest::SetOffset(
        syncplay_client_app::app_boundary::commands::LocalOffsetCommand::Absolute(7.0),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();
    assert_eq!(owner.user_offset_seconds, 7.0);
    assert_eq!(owner.player_position_seconds, Some(100.0));

    handle.push_request(GuiRuntimeRequest::SeekToPosition(42.0));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let _ = handle.drain_actions();

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![105.0, 107.0, 49.0],
        "offset commands should target player-local time, while global seeks add the active offset only once"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(42.0),
        "global seek state should remain offset-free after attached-player requests"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(42.0),
        "detached-session seek history should record the global seek target rather than the shifted player position"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_allows_offset_changes_without_a_player() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_position_seconds = Some(100.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        state.main_window.playback.can_set_offset,
        "offset controls should stay available even without an attached player"
    );

    handle.push_request(GuiRuntimeRequest::SetOffset(
        syncplay_client_app::app_boundary::commands::LocalOffsetCommand::Absolute(5.0),
    ));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.user_offset_seconds, 5.0);
    assert_eq!(
        owner.player_position_seconds,
        Some(100.0),
        "offset changes without a player should preserve the stored global position"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(100.0),
        "offset changes without a player should still keep detached-session telemetry on the global timeline"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message.contains("offset")
        )),
        "offset changes without a player should still report success"
    );
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Error
                    && message.contains("offset")
        )),
        "offset changes without a player should not surface a runtime-unavailable error"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_suppresses_attached_seeks_after_recent_rewind() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(2.0);
    owner.player_paused = Some(false);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    {
        let session = owner.session.as_mut().expect("session should exist");
        session
            .sync_local_playback_telemetry(Some(false), Some(2.0))
            .expect("initial local telemetry should sync");
        session.note_local_playlist_index_reset_intent(true);
    }

    handle.push_request(GuiRuntimeRequest::SeekToPosition(10.0));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if (*level == GuiTransientNotificationLevel::Success
                    || *level == GuiTransientNotificationLevel::Error)
                    && message.contains("seek")
        )),
        "recent-rewind seek suppression should not emit a seek success or error notification"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .is_empty(),
        "recent-rewind seek suppression should prevent the attached player seek"
    );
    assert_eq!(
        owner.player_position_seconds,
        Some(2.0),
        "suppressed attached seeks should leave the stored global position unchanged"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_position_seconds()),
        Some(2.0),
        "suppressed attached seeks should not advance detached-session telemetry"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_resets_inbound_shared_playlist_switches_before_applying_fresh_room_playstate()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        local_file_updates: Vec<syncplay_player_api::LocalFileUpdate>,
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.opened_paths.push(path.to_owned());
            state.local_file_updates.push(
                syncplay_player_api::LocalFileUpdate::new(
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path),
                )
                .with_path(path.to_owned()),
            );
            Ok(())
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let root = test_temp_root("shared-playlist-background-search");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("background shared-playlist search fixture directory should be created");
    let current_media_path = root.join("episode1.mkv");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("background shared-playlist current media fixture should be written");
    std::fs::write(&selected_media_path, b"test")
        .expect("background shared-playlist search fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_paused_values
        .clear();

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        let recorded_state = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let opened_selected_media = recorded_state
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref());
        let applied_reset_rewind = recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 0.0).abs() < f64::EPSILON);
        drop(recorded_state);
        if opened_selected_media && applied_reset_rewind {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref()),
        "background shared-playlist search should eventually open the selected media"
    );
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 0.0).abs() < f64::EPSILON),
        "playlist index changes should rewind a newly opened item before any fresh room sync arrives; recorded_state={recorded_state:?}, pending_reset={}, placeholder={}, player_local_file={:?}",
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        owner.player_local_file_placeholder,
        owner.player_local_file,
    );
    assert!(
        !recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < f64::EPSILON),
        "stale room playstate from the previous file should not be replayed onto the newly opened item"
    );
    drop(recorded_state);

    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":7.5,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions
            .iter()
            .any(|position| *position > 7.4)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| *position > 7.4),
        "once the room playstate changes for the new file, the attached player should follow it"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_applies_user_offset_only_at_player_sync_boundary() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.user_offset_seconds = 5.0;
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .set_positions
            .iter()
            .all(|position| *position > 14.9 && *position < 15.5),
        "attached-player room sync should add the active user offset when seeking the player"
    );
    assert_eq!(
        owner
            .player_position_seconds
            .map(|position| position.round()),
        Some(10.0),
        "stored runtime playback position should stay on the global timeline instead of the shifted player time"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_seeks_before_pausing_attached_player_for_room_pause() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_position_seconds = Some(5.0);
    owner.player_paused = Some(false);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < f64::EPSILON),
        "attached-player pause sync should seek to the room position before pausing"
    );
    assert!(
        recorded_state.set_paused_values.contains(&true),
        "attached-player pause sync should still pause once the position is corrected"
    );
    drop(recorded_state);
    assert_eq!(owner.player_position_seconds, Some(10.0));
    assert_eq!(owner.player_paused, Some(true));
}

#[test]
fn gui_persisted_config_runtime_owner_marks_local_user_ready_when_attached_player_unpauses() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(false));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "attached-player unpause should queue a local ready update"
    );
    assert_eq!(owner.player_paused, Some(false));
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "python-compatible default unpause handling should not re-pause when no other users block playback"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_marks_local_user_not_ready_when_attached_player_pauses() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        playback_updates: Vec<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":false")),
        "attached-player pause should queue a local not-ready update"
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "local attached-player pause should survive until the room playstate catches up"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "stale room pause snapshots should not immediately resume the attached player before the server echo arrives"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_blocks_gui_unpause_when_readiness_gate_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5},"isReady":false}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    let toggle_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        !toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "blocked GUI unpause should not announce a local resume"
    );
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        state.main_window.playback_paused,
        "shell state should stay paused when readiness blocks the unpause"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values
            .is_empty(),
        "blocked GUI unpause should not momentarily resume the attached player"
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "blocked GUI unpause should still mark the local user ready"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_emits_immediate_state_update_when_gui_unpause_is_allowed() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5},"isReady":true}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    let toggle_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "allowed GUI unpause should still announce the local resume"
    );
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.pending_local_attached_pause_override, Some(false));
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "allowed GUI unpause should resume the attached player exactly once"
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "allowed GUI unpause should still mark the local user ready"
    );
    assert!(
        outbound_protocol_lines.iter().any(|line| {
            line.contains("\"State\"")
                && line.contains("\"paused\":false")
                && line.contains("\"position\":10.0")
        }),
        "allowed GUI unpause should emit an immediate paused=false state update"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_commit_runtime_unpause_when_player_resume_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        resume_attempts: usize,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            if !paused {
                self.state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .resume_attempts += 1;
                return Err(syncplay_player_api::PlayerError::OperationFailed(
                    "resume failed".to_owned(),
                ));
            }
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(true);
    owner.player_position_seconds = Some(10.0);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","duration":95.5},"isReady":true}}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":10.0,"paused":true,"setBy":"bob"}}}"#.to_owned(),
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    handle.push_request(GuiRuntimeRequest::TogglePlaybackPause);
    let toggle_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        !toggle_actions.contains(&GuiShellAction::AnnouncePlaybackResumed),
        "failed GUI unpause should not announce a local resume"
    );
    assert_eq!(owner.player_paused, Some(true));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state()),
        Some(true),
        "the detached runtime should stay paused when the physical resume fails"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_username()),
        Some("alice")
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .resume_attempts,
        1,
        "the attached player should still receive one resume attempt"
    );

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(
        !outbound_protocol_lines
            .iter()
            .any(|line| line.contains("\"ready\"") && line.contains("\"isReady\":true")),
        "a failed player resume must not optimistically mark the local user ready"
    );
    assert!(
        !outbound_protocol_lines.iter().any(|line| {
            line.contains("\"State\"")
                && line.contains("\"paused\":false")
                && line.contains("\"position\":10.0")
        }),
        "a failed player resume must not emit a paused=false heartbeat"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_resets_same_file_playlist_index_switches() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let root = test_temp_root("shared-playlist-same-file-reset");
    let current_media_path = root.join("episode1.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("same-file playlist reset fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode1.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state.opened_paths.is_empty(),
        "same-file playlist index changes should not reopen the attached media"
    );
    assert!(
        recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 0.0).abs() < f64::EPSILON),
        "same-file playlist index changes should still consume the reset handoff and rewind"
    );
    assert!(
        !recorded_state
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < f64::EPSILON),
        "same-file playlist index changes should not replay the stale room timeline"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reuses_media_search_index_for_later_playlist_selection() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("shared-playlist-search-cache");
    let season_directory = root.join("season-1");
    std::fs::create_dir_all(&season_directory)
        .expect("shared-playlist cache fixture directory should be created");
    let episode_two_path = season_directory.join("episode2.mkv");
    let episode_three_path = season_directory.join("episode3.mkv");
    std::fs::write(&episode_two_path, b"test")
        .expect("shared-playlist cache fixture episode two should be written");
    std::fs::write(&episode_three_path, b"test")
        .expect("shared-playlist cache fixture episode three should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == episode_two_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner.attached_media_search_index.is_some(),
        "first background search should populate the reusable media index"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .opened_paths
        .clear();

    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"bob"}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#.to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == episode_three_path.to_string_lossy().as_ref()),
        "later playlist selections should resolve immediately from the cached media index"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_skips_self_origin_room_position_sync_for_attached_player() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("self-origin room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "force-sync should not replay the local user's own room position back into the attached player"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "force-sync should not replay the local user's own room pause state back into the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_ignores_unattributed_room_playstate_when_no_remote_users_are_known()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);
    owner.player_paused = Some(false);

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(r#"{"State":{"playstate":{"position":0.0,"paused":true}}}"#)
        .expect("unattributed room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "room playstate without remote authority should not rewind the attached player while alone"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "room playstate without remote authority should not pause the attached player while alone"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_local_file_before_applying_room_playstate() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(recorded.set_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "room playstate should seek once the attached player reports a local file"
    );
    assert_eq!(recorded.set_paused_values, vec![true]);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_force_room_sync_for_matched_playlist_target_without_reset_intent()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let root = test_temp_root("matched-playlist-target-no-reset");
    let media_path = root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("playlist target fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(media_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(0.0);
    owner.player_paused = Some(false);

    let stored_settings = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        rewind_on_desync: Some(false),
        fastforward_on_desync: Some(false),
        slow_on_desync: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&stored_settings);
    state.apply_shared_playlist_entries(vec!["episode1.mkv".to_owned()], Some(0), false);
    owner.active_shared_playlist_index = Some(0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_runtime_settings(&stored_client_settings_runtime_snapshot_legacy_compatible(
            &stored_settings,
        ))
        .expect("runtime settings should sync");

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":41.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded.set_positions.clear();
        recorded.set_paused_values.clear();
    }
    owner.player_position_seconds = Some(42.0);

    let selected_media_sync =
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        selected_media_sync,
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
    );

    let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
    );
    assert!(
        !selection_handoff_ready,
        "matched playlist targets without a pending reset should not force a room playstate handoff"
    );

    owner.apply_pending_playlist_index_reset_to_attached_player_impl(
        &state,
        selection_handoff_ready,
    );
    owner.sync_session_playstate_to_attached_player_impl(&state, selection_handoff_ready);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "playlist updates that keep the current target selected should not rewind the attached player; recorded={recorded:?}"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "playlist updates that keep the current target selected should not toggle pause state; recorded={recorded:?}"
    );
    assert_eq!(owner.player_position_seconds, Some(42.0));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_initially_syncs_live_room_position_to_attached_player() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(0.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("live room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 42.0).abs() < 1.0),
        "the first live room playstate should seek the attached player onto the active timeline"
    );
    assert!(
        owner
            .player_position_seconds
            .is_some_and(|position| (position - 42.0).abs() < 1.0)
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_matching_local_file_before_applying_playlist_reset()
{
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.shared_playlist_enabled = true;
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .note_local_playlist_index_reset_intent(true);

    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path("C:/Media/episode2.mkv".to_owned()),
    );
    owner.player_local_file_placeholder = true;
    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(recorded.set_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert!(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "playlist reset intent should remain pending until the attached player reports a real local file update"
    );

    owner.player_local_file_placeholder = false;
    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded.set_positions, vec![0.0]);
    assert_eq!(recorded.set_paused_values, vec![true]);
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "playlist reset intent should clear after the rewind is applied"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retries_playlist_reset_after_transient_attached_player_rewind_failure()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_position_attempts: usize,
        successful_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.set_position_attempts += 1;
            if state.set_position_attempts == 1 {
                return Err(syncplay_player_api::PlayerError::OperationFailed(
                    "property unavailable".to_owned(),
                ));
            }
            state.successful_positions.push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.shared_playlist_enabled = true;
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .note_local_playlist_index_reset_intent(true);

    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path("C:/Media/episode2.mkv".to_owned()),
    );
    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.set_position_attempts, 1);
        assert!(recorded.successful_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert!(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "transient rewind failures should leave the playlist reset intent pending for a later retry"
    );

    owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded.set_position_attempts, 2);
    assert_eq!(recorded.successful_positions, vec![0.0]);
    assert_eq!(recorded.set_paused_values, vec![true]);
    assert_eq!(owner.player_position_seconds, Some(0.0));
    assert_eq!(owner.player_paused, Some(true));
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
        "playlist reset intent should clear after a later retry succeeds"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_applies_desync_seek_when_room_playstate_is_unchanged() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(10.0))
        .expect("initial local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_positions
        .clear();

    owner.player_position_seconds = Some(20.0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(20.0))
        .expect("desynced local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "steady-state attached-player sync should still rewind desynced playback even when the room playstate snapshot is unchanged"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retries_attached_player_seek_after_transient_failure() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_position_attempts: usize,
        successful_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.set_position_attempts += 1;
            if state.set_position_attempts == 1 {
                return Err(syncplay_player_api::PlayerError::OperationFailed(
                    "transient failure".to_owned(),
                ));
            }
            state.successful_positions.push(position_seconds);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(0.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.set_position_attempts, 1);
        assert!(recorded.successful_positions.is_empty());
    }
    assert_eq!(owner.player_position_seconds, Some(0.0));

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(recorded.set_position_attempts, 2);
    assert!(
        recorded
            .successful_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "retrying the room playstate sync should seek close to the requested room position"
    );
    assert!(
        owner
            .player_position_seconds
            .is_some_and(|position| (position - 10.0).abs() < 1.0)
    );
}

#[test]
fn gui_persisted_config_runtime_owner_applies_desync_slowdown_when_room_playstate_is_unchanged() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_playback_rates: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_playback_rate(&mut self, rate: f64) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_playback_rates
                .push(rate);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_paused = Some(false);
    owner.player_position_seconds = Some(10.0);
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(10.0))
        .expect("initial local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);
    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_playback_rates
        .clear();

    owner.player_position_seconds = Some(12.0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_local_playback_telemetry(Some(false), Some(12.0))
        .expect("desynced local telemetry should sync");

    owner.sync_session_playstate_to_attached_player_impl(&state, false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        recorded.set_playback_rates,
        vec![0.95],
        "steady-state attached-player sync should still apply slowdown corrections while playback continues"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_warm_starts_shared_playlist_resolution_from_persisted_cache()
{
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-warm-start");
    let config_path = root.join("syncplay.ini");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("warm-start shared-playlist fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("warm-start shared-playlist fixture should be written");
    write_persisted_media_search_root_index(
        &root,
        &root,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_millis() as u64,
        &[("episode2.mkv", &["nested\\episode2.mkv"])],
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![selected_media_path.to_string_lossy().into_owned()]
    );
    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| {
                index.root_indexes_by_key.contains_key(
                    &crate::app::media_search_cache::normalized_media_search_root_key(&root),
                )
            }),
        "warm-start media resolution should load the initial root segment from the persisted cache before any later root warming occurs"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_resolves_from_stale_persisted_cache_and_refreshes_in_background()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-stale-cache");
    let config_path = root.join("syncplay.ini");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("stale shared-playlist fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("stale shared-playlist fixture should be written");
    let stale_built_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
        .saturating_sub(120_000) as u64;
    write_persisted_media_search_root_index(
        &root,
        &root,
        stale_built_at,
        &[("episode2.mkv", &["nested\\episode2.mkv"])],
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![selected_media_path.to_string_lossy().into_owned()]
    );

    let refresh_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < refresh_deadline {
        if owner.pending_attached_media_resolution.is_none()
            && owner
                .attached_media_search_index
                .as_ref()
                .and_then(|index| {
                    index
                        .root_indexes_by_key
                        .get(
                            &crate::app::media_search_cache::normalized_media_search_root_key(
                                &root,
                            ),
                        )
                        .map(|root_index| root_index.built_at_unix_ms)
                })
                .is_some_and(|built_at| built_at > stale_built_at)
        {
            break;
        }
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .and_then(|index| {
                index
                    .root_indexes_by_key
                    .get(&crate::app::media_search_cache::normalized_media_search_root_key(&root))
                    .map(|root_index| root_index.built_at_unix_ms)
            })
            .is_some_and(|built_at| built_at > stale_built_at),
        "stale persisted cache entries should refresh in the background after the immediate warm-start hit"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_current_player_locality_for_duplicate_cached_names() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-duplicate-ranking");
    let config_path = root.join("syncplay.ini");
    let preferred_root = root.join("preferred");
    let preferred_season = preferred_root.join("season-1");
    let fallback_root = root.join("fallback");
    std::fs::create_dir_all(&preferred_season)
        .expect("preferred duplicate-ranking fixture directory should be created");
    std::fs::create_dir_all(&fallback_root)
        .expect("fallback duplicate-ranking fixture directory should be created");
    let preferred_current_path = preferred_season.join("episode1.mkv");
    let preferred_target_path = preferred_season.join("episode2.mkv");
    let fallback_target_path = fallback_root.join("episode2.mkv");
    std::fs::write(&preferred_current_path, b"test")
        .expect("preferred duplicate-ranking current fixture should be written");
    std::fs::write(&preferred_target_path, b"test")
        .expect("preferred duplicate-ranking target fixture should be written");
    std::fs::write(&fallback_target_path, b"test")
        .expect("fallback duplicate-ranking target fixture should be written");

    let built_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64;
    write_persisted_media_search_root_index(
        &root,
        &preferred_root,
        built_at_unix_ms,
        &[("episode2.mkv", &["season-1\\episode2.mkv"])],
    );
    write_persisted_media_search_root_index(
        &root,
        &fallback_root,
        built_at_unix_ms,
        &[("episode2.mkv", &["episode2.mkv"])],
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(preferred_current_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![
            fallback_root.to_string_lossy().into_owned(),
            preferred_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![preferred_target_path.to_string_lossy().into_owned()]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_cached_roots_when_one_refresh_result_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-partial-refresh");
    let config_path = root.join("syncplay.ini");
    let good_root = root.join("good");
    let bad_root = root.join("bad");
    std::fs::create_dir_all(&good_root)
        .expect("partial-refresh good fixture directory should be created");
    std::fs::create_dir_all(&bad_root)
        .expect("partial-refresh bad fixture directory should be created");
    let selected_media_path = good_root.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("partial-refresh good fixture should be written");

    let good_key = crate::app::media_search_cache::normalized_media_search_root_key(&good_root);
    let bad_key = crate::app::media_search_cache::normalized_media_search_root_key(&bad_root);
    let stale_built_at = 1;

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![
            good_root.to_string_lossy().into_owned(),
            bad_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![good_key.clone(), bad_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([
            (
                good_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: good_key.clone(),
                    root_path: good_root.clone(),
                    built_at_unix_ms: stale_built_at,
                    candidates_by_name: std::collections::HashMap::from([(
                        "episode2.mkv".to_owned(),
                        vec!["episode2.mkv".to_owned()],
                    )]),
                },
            ),
            (
                bad_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: bad_key.clone(),
                    root_path: bad_root.clone(),
                    built_at_unix_ms: stale_built_at,
                    candidates_by_name: std::collections::HashMap::new(),
                },
            ),
        ]),
        roots_requiring_refresh: [good_key.clone(), bad_key.clone()].into_iter().collect(),
    });
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(vec![
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: good_key.clone(),
                index: Some(GuiAttachedMediaSearchRootIndex {
                    root_key: good_key.clone(),
                    root_path: good_root.clone(),
                    built_at_unix_ms: stale_built_at + 1,
                    candidates_by_name: std::collections::HashMap::from([(
                        "episode2.mkv".to_owned(),
                        vec!["episode2.mkv".to_owned()],
                    )]),
                }),
                error: None,
            },
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: bad_key.clone(),
                index: None,
                error: Some("simulated refresh failure".to_owned()),
            },
        ]))
        .expect("partial-refresh result fixture should be queued");
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        job_id: GuiMediaIndexJobId(1),
        roots: vec![good_key.clone(), bad_key.clone()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx,
    });
    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![selected_media_path.to_string_lossy().into_owned()]
    );
    assert!(
        owner.attached_media_search_next_retry_at.is_some(),
        "a failed per-root refresh should schedule a retry without dropping successful roots"
    );
    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| index.root_indexes_by_key.contains_key(&bad_key)),
        "failed roots should keep their prior cached segment until a later refresh succeeds"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_projects_media_index_progress_into_shell_state() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.shared_playlist_enabled = true;
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let latest_progress = std::sync::Arc::new(std::sync::Mutex::new(None));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        job_id: GuiMediaIndexJobId(2),
        roots: vec!["c:/media/anime".to_owned(), "d:/archive".to_owned()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: latest_progress.clone(),
        result_rx,
    });
    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 0,
            current_root_key: "c:/media/anime".to_owned(),
            current_root_path: PathBuf::from("C:/Media/Anime"),
            scanned_directories: 14,
            indexed_files: 2048,
        });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(state.media_index_status.active);
    assert_eq!(
        state.media_index_status.message.as_deref(),
        Some("Indexing media 1/2: 14 folders, 2048 files (Anime)")
    );

    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Cancelled)
        .expect("media-index cancel fixture should be queued");
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(!state.media_index_status.active);
    assert_eq!(state.media_index_status.message, None);
}

#[test]
fn gui_persisted_config_runtime_owner_coalesces_latest_media_index_progress_per_pump() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let latest_progress = std::sync::Arc::new(std::sync::Mutex::new(None));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        job_id: GuiMediaIndexJobId(3),
        roots: vec!["c:/media/anime".to_owned(), "d:/archive".to_owned()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: latest_progress.clone(),
        result_rx,
    });

    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 0,
            current_root_key: "c:/media/anime".to_owned(),
            current_root_path: PathBuf::from("C:/Media/Anime"),
            scanned_directories: 32,
            indexed_files: 4096,
        });
    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 0,
            current_root_key: "c:/media/anime".to_owned(),
            current_root_path: PathBuf::from("C:/Media/Anime"),
            scanned_directories: 64,
            indexed_files: 8192,
        });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        state.media_index_status.message.as_deref(),
        Some("Indexing media 1/2: 64 folders, 8192 files (Anime)")
    );

    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 1,
            current_root_key: "d:/archive".to_owned(),
            current_root_path: PathBuf::from("D:/Archive"),
            scanned_directories: 8,
            indexed_files: 512,
        });
    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 1,
            current_root_key: "d:/archive".to_owned(),
            current_root_path: PathBuf::from("D:/Archive"),
            scanned_directories: 12,
            indexed_files: 768,
        });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        state.media_index_status.message.as_deref(),
        Some("Indexing media 2/2: 12 folders, 768 files (Archive)")
    );

    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Cancelled)
        .expect("media-index cancel fixture should be queued");
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(!state.media_index_status.active);
    assert_eq!(state.media_index_status.message, None);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_media_index_status_when_pending_search_build_completes_before_projection()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let latest_progress = std::sync::Arc::new(std::sync::Mutex::new(Some(
        GuiAttachedMediaSearchBuildProgress {
            total_roots: 1,
            completed_roots: 0,
            current_root_key: "c:/media".to_owned(),
            current_root_path: PathBuf::from("C:/Media"),
            scanned_directories: 4,
            indexed_files: 32,
        },
    )));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        job_id: GuiMediaIndexJobId(4),
        roots: vec!["c:/media".to_owned()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress,
        result_rx,
    });
    state.pending_operation = Some(crate::app::GuiPendingOperationState {
        kind: GuiPendingOperationKind::SearchMissingMedia,
    });
    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(Vec::new()))
        .expect("media-index completion fixture should be queued");

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(snapshot)
                if snapshot.active
                    && snapshot.message.as_deref()
                        == Some("Indexing media 1/1: 4 folders, 32 files (Media)")
        )),
        "pending missing-media searches should still surface the last background index status even when the build completes before projection"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_apply_room_playstate_while_selected_playlist_target_is_unresolved()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let root = test_temp_root("shared-playlist-unresolved-playstate");
    let current_media_path = root.join("episode1.mkv");
    std::fs::write(&current_media_path, b"test")
        .expect("existing attached-player media fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"bob"},"ping":{"latencyCalculation":123.0}}}"#
            .to_owned(),
    );

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let recorded_state = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded_state.opened_paths.is_empty(),
        "unresolved shared-playlist targets should not open a replacement file until resolution succeeds"
    );
    assert!(
        recorded_state.set_positions.is_empty(),
        "room seek state should not be applied to the previously-open file while the new playlist target is unresolved"
    );
    assert!(
        recorded_state.set_paused_values.is_empty(),
        "room pause state should not be applied to the previously-open file while the new playlist target is unresolved"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_unresolved_shared_playlist_media_after_double_check_interval()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("shared-playlist-double-check-retry");
    let delayed_directory = root.join("delayed");
    let selected_media_path = delayed_directory.join("episode2.mkv");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        folder_search_timeout_seconds: Some(1.0),
        folder_search_double_check_interval_seconds: Some(0.05),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
    );
    session_transport.push_inbound_protocol_line(
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    );

    let first_scan_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < first_scan_deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner.attached_media_search_index.is_some()
            && owner.pending_attached_media_resolution.is_none()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner.attached_media_search_index.is_some(),
        "first missing-media scan should populate the reusable index even when the target is still missing"
    );
    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .is_empty(),
        "no file should open before the missing target appears on disk"
    );

    std::fs::create_dir_all(&delayed_directory)
        .expect("delayed shared-playlist search fixture directory should be created");
    std::fs::write(&selected_media_path, b"test")
        .expect("delayed shared-playlist search fixture should be written");

    let retry_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < retry_deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .iter()
            .any(|path| path == selected_media_path.to_string_lossy().as_ref()),
        "automatic missing-media resolution should retry after the configured double-check interval and open files that appear later"
    );

    let _ = std::fs::remove_dir_all(&root);
}
