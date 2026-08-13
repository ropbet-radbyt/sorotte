use super::super::DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD;
use super::super::shell_state::{
    GuiInteractionRuntimeSnapshot, MainWindowParticipantStatusFreshness,
    MainWindowParticipantStatusPresentation, MainWindowParticipantStatusReport,
    MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    MainWindowShellState, MenuActionId, MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot,
    SorotteGuiShellAppState, browser_format_duration_label, browser_format_size_label,
    browser_is_url, browser_uri_is_trusted,
};
use super::super::support::{
    nonempty_room_name_text, normalized_editable_text, system_time_seconds,
};
use super::GuiClientCoreChatSessionRuntimeAdapter;
use sorotte_client_app::app_boundary::readiness::{
    ParticipantReadinessPresentation, PendingReadinessIntentPresentation,
};
use sorotte_client_core::RoomPlaystateAuthority;
use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantStatusAvailability, ParticipantStatusCorrelation,
    PlaybackBarrierParticipantPhase, PlaybackBarrierPhase,
};
use std::collections::{BTreeMap, BTreeSet};

fn playback_barrier_participant_label(phase: PlaybackBarrierParticipantPhase) -> &'static str {
    match phase {
        PlaybackBarrierParticipantPhase::Pending => "pending",
        PlaybackBarrierParticipantPhase::Ready => "ready",
        PlaybackBarrierParticipantPhase::Started => "started",
        PlaybackBarrierParticipantPhase::Degraded => "degraded",
        PlaybackBarrierParticipantPhase::PrepareTimedOut => "prepare timed out",
        PlaybackBarrierParticipantPhase::StartedAckTimedOut => "start acknowledgement timed out",
    }
}

fn playback_barrier_phase_label(phase: PlaybackBarrierPhase) -> &'static str {
    match phase {
        PlaybackBarrierPhase::Preparing => "preparing; waiting for participant readiness",
        PlaybackBarrierPhase::Committed => "committed by server",
        PlaybackBarrierPhase::AwaitingDecision => "awaiting controller decision",
        PlaybackBarrierPhase::Complete => "complete",
        PlaybackBarrierPhase::Degraded => "degraded",
    }
}

fn room_playstate_authority_label(authority: RoomPlaystateAuthority) -> String {
    match authority {
        RoomPlaystateAuthority::LegacyRemoteUser => "remote user (legacy playstate)".to_owned(),
        RoomPlaystateAuthority::LegacyLocalEcho => "local echo (legacy playstate)".to_owned(),
        RoomPlaystateAuthority::ServerBarrier {
            media_generation,
            state_revision,
        } => state_revision.map_or_else(
            || format!("server start barrier, generation {media_generation}"),
            |revision| {
                format!("server start barrier, generation {media_generation}, revision {revision}")
            },
        ),
        RoomPlaystateAuthority::ServerBufferingPolicy { media_generation } => {
            format!("server buffering policy, generation {media_generation}")
        }
    }
}

impl GuiClientCoreChatSessionRuntimeAdapter {
    fn session_readiness_presentations(
        &self,
        users: &[MainWindowRuntimeUserSnapshot],
    ) -> BTreeMap<String, ParticipantReadinessPresentation> {
        let session = self.runtime.session();
        let local_username = session.username();
        if !session.server_readiness_v2_supported() {
            return BTreeMap::new();
        }
        let Some(room_snapshot) = session.readiness_snapshot() else {
            return BTreeMap::new();
        };
        users
            .iter()
            .map(|user| {
                let presentation =
                    if let Some(canonical) = room_snapshot.participants.get(&user.username) {
                        let pending = session
                            .pending_readiness_intent()
                            .filter(|pending| {
                                pending
                                    .target_username()
                                    .or(local_username)
                                    .is_some_and(|target| target == user.username)
                            })
                            .map(PendingReadinessIntentPresentation::from);
                        ParticipantReadinessPresentation::from_v2(canonical, pending)
                    } else {
                        ParticipantReadinessPresentation::from_legacy(
                            user.username.clone(),
                            user.is_ready,
                        )
                    }
                    .with_room_snapshot(room_snapshot);
                (user.username.clone(), presentation)
            })
            .collect()
    }

    fn room_control_status_for_runtime_snapshot(&self, controlled_room_active: bool) -> String {
        let session = self.runtime.session();
        if !session.is_active() {
            return MainWindowShellState::room_control_status_waiting_for_server();
        }
        if !controlled_room_active {
            return MainWindowShellState::room_control_status_uncontrolled_room();
        }
        if session.local_can_control().unwrap_or(false) {
            MainWindowShellState::room_control_status_granted()
        } else {
            MainWindowShellState::room_control_status_locked()
        }
    }

    pub(super) fn shared_playlist_control_available(&self) -> bool {
        self.shared_playlist_server_supported()
            && self.runtime.session().local_can_control().unwrap_or(false)
    }

    pub(super) fn session_runtime_rooms(
        &self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<MainWindowRuntimeRoomSnapshot> {
        let session = self.runtime.session();
        let mut rooms = session
            .room_names()
            .into_iter()
            .filter_map(|room_name| {
                nonempty_room_name_text(&room_name).map(|room_name| MainWindowRuntimeRoomSnapshot {
                    has_named_users: !session.usernames_in_room(&room_name).is_empty(),
                    is_controlled: room_name.starts_with('+'),
                    room_name,
                })
            })
            .collect::<Vec<_>>();
        if rooms.is_empty()
            && let Some(room_name) = nonempty_room_name_text(&state.main_window.room_name)
        {
            rooms.push(MainWindowRuntimeRoomSnapshot {
                has_named_users: false,
                is_controlled: room_name.starts_with('+'),
                room_name,
            });
        }
        rooms
    }

    pub(super) fn session_runtime_users(&self) -> Vec<MainWindowRuntimeUserSnapshot> {
        let session = self.runtime.session();
        let playback = &self.runtime_settings.config.playback;
        let trusted_domains = &playback.trusted_domains;
        let only_switch_to_trusted_domains = playback.only_switch_to_trusted_domains;
        let local_username = session.username();
        let current_room = session.room();
        let now_seconds = system_time_seconds();
        let server_participant_status_supported = session.server_participant_status_v1_supported();
        let playback_barrier_status = session.playback_barrier_status();
        let local_participant_status = local_username
            .and_then(|username| session.user_participant_status_at(username, now_seconds));
        let reference_scope = session.participant_status_authoritative_scope();
        let reference_timeline = reference_scope
            .map(|scope| (Some(scope.media_generation), scope.state_revision))
            .or_else(|| {
                playback_barrier_status
                    .map(|status| (Some(status.media_generation), status.state_revision))
            })
            .or_else(|| {
                local_participant_status
                    .as_ref()
                    .and_then(|status| status.status.playback_scope)
                    .map(|scope| (Some(scope.media_generation), scope.state_revision))
            });
        let mut users = Vec::new();
        for room_name in session.room_names() {
            for username in session.usernames_in_room(&room_name) {
                let is_self = local_username == Some(username.as_str());
                let file_name = session
                    .user_file_name(&username)
                    .and_then(normalized_editable_text);
                let file_is_url = file_name.as_deref().is_some_and(browser_is_url);
                let file_is_trusted = file_name.as_deref().is_none_or(|file_name| {
                    browser_uri_is_trusted(
                        file_name,
                        only_switch_to_trusted_domains,
                        trusted_domains,
                    )
                });
                let differences = session
                    .file_differences_for_user(&username)
                    .unwrap_or_default();
                let in_current_room = current_room == Some(room_name.as_str());
                let participant_status_view = in_current_room
                    .then(|| session.user_participant_status_at(&username, now_seconds))
                    .flatten();
                let participant_status = if let Some(status) = participant_status_view {
                    match status.status.availability {
                        ParticipantStatusAvailability::Unsupported => {
                            MainWindowParticipantStatusPresentation::LegacyClient
                        }
                        ParticipantStatusAvailability::AwaitingReport => {
                            MainWindowParticipantStatusPresentation::WaitingForFirstReport
                        }
                        ParticipantStatusAvailability::Unavailable => {
                            MainWindowParticipantStatusPresentation::Unavailable
                        }
                        ParticipantStatusAvailability::Fresh
                        | ParticipantStatusAvailability::Delayed
                        | ParticipantStatusAvailability::Stale => {
                            let correlation = status.status.correlation;
                            let explicitly_correlated = correlation
                                == Some(ParticipantStatusCorrelation::Exact)
                                || correlation == Some(ParticipantStatusCorrelation::Uncorrelated);
                            let timeline_mismatch = if explicitly_correlated {
                                false
                            } else if correlation.is_some() {
                                true
                            } else {
                                let reported_scope = status.status.playback_scope;
                                let authoritative_scope_mismatch = reference_scope
                                    .is_some_and(|scope| reported_scope != Some(scope));
                                let reference_timeline_mismatch = reference_timeline.is_some_and(
                                    |(room_generation, room_revision)| {
                                        let reported_generation =
                                            reported_scope.map(|scope| scope.media_generation);
                                        let reported_revision =
                                            reported_scope.and_then(|scope| scope.state_revision);
                                        room_generation
                                            .is_some_and(|room| reported_generation != Some(room))
                                            || room_revision
                                                .is_some_and(|room| reported_revision != Some(room))
                                    },
                                );
                                authoritative_scope_mismatch || reference_timeline_mismatch
                            };
                            MainWindowParticipantStatusPresentation::Report(
                                MainWindowParticipantStatusReport::from_client_view(
                                    status,
                                    timeline_mismatch,
                                ),
                            )
                        }
                        _ => MainWindowParticipantStatusPresentation::Unavailable,
                    }
                } else if !in_current_room || !server_participant_status_supported {
                    MainWindowParticipantStatusPresentation::Unavailable
                } else {
                    match session.user_participant_status_v1_supported(&username) {
                        Some(false) => MainWindowParticipantStatusPresentation::LegacyClient,
                        Some(true) => {
                            MainWindowParticipantStatusPresentation::WaitingForFirstReport
                        }
                        None => MainWindowParticipantStatusPresentation::Unavailable,
                    }
                };
                let start_barrier_status = playback_barrier_status.and_then(|status| {
                    status
                        .participants
                        .get(&username)
                        .map(|participant| {
                            playback_barrier_participant_label(participant.phase).to_owned()
                        })
                        .or_else(|| {
                            status
                                .excluded_legacy_clients
                                .contains(&username)
                                .then(|| "excluded legacy participant".to_owned())
                        })
                });
                users.push(MainWindowRuntimeUserSnapshot {
                    username: username.clone(),
                    room_name: room_name.clone(),
                    is_self,
                    is_ready: session.user_ready(&username).unwrap_or(false),
                    is_controller: session.user_controller(&username).unwrap_or(false),
                    has_file: session
                        .user_has_file(&username)
                        .unwrap_or(file_name.is_some()),
                    file_name,
                    file_size_label: browser_format_size_label(session.user_file_size(&username)),
                    file_duration_label: browser_format_duration_label(
                        session.user_file_duration(&username),
                    ),
                    file_is_url,
                    file_is_trusted,
                    filename_differs: differences.filename,
                    filesize_differs: differences.filesize,
                    fileduration_differs: differences.fileduration,
                    participant_status,
                    start_barrier_status,
                });
            }
        }
        users
    }

    pub(super) fn main_window_runtime_snapshot(
        &self,
        state: &SorotteGuiShellAppState,
    ) -> Option<MainWindowRuntimeSnapshot> {
        let baseline_main_window =
            MainWindowShellState::from_stored_settings(&self.runtime_settings.settings);
        let session = self.runtime.session();
        let shared_playlist_server_supported = self.shared_playlist_server_supported();
        let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        snapshot.room_name = baseline_main_window.room_name.clone();
        snapshot.room_control_status = baseline_main_window.room_control_status.clone();
        snapshot.shared_playlist_enabled =
            shared_playlist_server_supported && baseline_main_window.shared_playlist_enabled;
        snapshot.controlled_room_active = baseline_main_window.controlled_room_active;
        snapshot.hide_empty_rooms = state.main_window.hide_empty_rooms;
        snapshot.rooms = baseline_main_window
            .rooms
            .clone()
            .into_iter()
            .map(|room| MainWindowRuntimeRoomSnapshot {
                room_name: room.room_name,
                is_controlled: room.is_controlled,
                has_named_users: room.has_named_users,
            })
            .collect();
        snapshot.users = baseline_main_window
            .users
            .iter()
            .map(|user| MainWindowRuntimeUserSnapshot {
                username: user.username.clone(),
                room_name: user.room_name.clone(),
                is_self: user.is_self,
                is_ready: user.is_ready,
                is_controller: user.is_controller,
                has_file: user.has_file,
                file_name: user.file_name.clone(),
                file_size_label: user.file_size_label.clone(),
                file_duration_label: user.file_duration_label.clone(),
                file_is_url: user.file_is_url,
                file_is_trusted: user.file_is_trusted,
                filename_differs: user.filename_differs,
                filesize_differs: user.filesize_differs,
                fileduration_differs: user.fileduration_differs,
                participant_status: user.participant_status.clone(),
                start_barrier_status: user.start_barrier_status.clone(),
            })
            .collect();
        snapshot.room_playback_intent = baseline_main_window.room_playback_intent.clone();
        snapshot.playlist = baseline_main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect();
        snapshot.playlist_entry_ids.clear();
        snapshot.playlist_source_states.clear();
        snapshot.active_playlist_index = None;
        snapshot.can_set_ready = baseline_main_window.playback.can_set_ready;
        snapshot.can_set_others_ready = baseline_main_window.playback.can_set_others_ready;
        snapshot.playback_paused = baseline_main_window.playback_paused;
        snapshot.autoplay_active = state.main_window.autoplay_active;
        snapshot.autoplay_threshold = state.main_window.autoplay_threshold;
        snapshot.autoplay_countdown_seconds = state.main_window.autoplay_countdown_seconds;
        snapshot.user_offset_seconds = state.main_window.user_offset_seconds;
        snapshot.show_playback_buttons = state.main_window.show_playback_buttons;
        snapshot.show_autoplay_controls = state.main_window.show_autoplay_controls;
        if let Some(room_name) = session
            .room()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let controlled_room_active = room_name.starts_with('+');
            snapshot.room_name = room_name.to_owned();
            snapshot.controlled_room_active = controlled_room_active;
            snapshot.rooms = self.session_runtime_rooms(state);
            snapshot.users = self.session_runtime_users();
        }
        snapshot.room_control_status =
            self.room_control_status_for_runtime_snapshot(snapshot.controlled_room_active);
        if shared_playlist_server_supported
            && let Some(playlist) = self.projected_current_room_playlist()
        {
            snapshot.shared_playlist_enabled = true;
            snapshot.playlist = playlist.files.clone();
            snapshot.playlist_entry_ids.clear();
            snapshot.playlist_source_states.clear();
            snapshot.active_playlist_index = playlist
                .index
                .and_then(|index| usize::try_from(index).ok())
                .filter(|index| *index < snapshot.playlist.len());
        } else if !shared_playlist_server_supported {
            snapshot.playlist.clear();
            snapshot.playlist_entry_ids.clear();
            snapshot.playlist_source_states.clear();
            snapshot.active_playlist_index = None;
        }
        snapshot.can_manage_playlist =
            snapshot.shared_playlist_enabled && self.shared_playlist_control_available();
        snapshot.can_undo_seek = session.last_seek_position_before_manual_seek().is_some();
        snapshot.can_toggle_autoplay = true;
        snapshot.can_adjust_autoplay_threshold = true;
        snapshot.autoplay_active = session.autoplay_enabled();
        snapshot.autoplay_threshold = session
            .readiness_autoplay_config()
            .auto_play_threshold
            .unwrap_or(DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD);
        snapshot.autoplay_countdown_seconds = session
            .autoplay_timer_is_running()
            .then(|| session.autoplay_time_left_seconds().max(0.0).floor() as u32);
        let now_seconds = system_time_seconds();
        if let Some(playstate) = session.current_room_playstate_at(now_seconds) {
            snapshot.room_playback_intent.position_seconds = playstate.position;
            snapshot.room_playback_intent.paused = playstate.paused;
            snapshot.room_playback_intent.set_by = playstate.set_by;
            snapshot.room_playback_intent.authority = session
                .current_room_playstate_authority()
                .map(room_playstate_authority_label);
            if let Some(paused) = snapshot.room_playback_intent.paused {
                snapshot.playback_paused = paused;
            }
        }
        if let Some(paused) = session.local_paused() {
            snapshot.playback_paused = paused;
        }
        snapshot.can_set_ready = session.is_active() && session.server_readiness_supported();
        snapshot.can_set_others_ready = session.server_set_others_readiness_supported()
            && session.local_can_control().unwrap_or(false);
        snapshot.readiness = self.session_readiness_presentations(&snapshot.users);
        snapshot.room_playback_intent.start_gate = snapshot
            .readiness
            .values()
            .next()
            .map(ParticipantReadinessPresentation::start_gate_detail_label)
            .or_else(|| {
                session
                    .playback_barrier_status()
                    .map(|status| playback_barrier_phase_label(status.phase).to_owned())
            });
        let current_room = snapshot.room_name.as_str();
        snapshot.room_playback_intent.participant_count = snapshot
            .users
            .iter()
            .filter(|user| user.room_name == current_room)
            .count();
        snapshot.room_playback_intent.maximum_observed_drift_seconds = snapshot
            .users
            .iter()
            .filter(|user| user.room_name == current_room)
            .filter_map(|user| {
                let MainWindowParticipantStatusPresentation::Report(status) =
                    &user.participant_status
                else {
                    return None;
                };
                (status.freshness == MainWindowParticipantStatusFreshness::Fresh
                    && !status.timeline_mismatch
                    && status.status.correlation
                        == Some(sorotte_protocol::ParticipantStatusCorrelation::Exact))
                .then_some(status.status.room_offset_seconds)
                .flatten()
                .map(f64::abs)
            })
            .reduce(f64::max);
        let mut buffering_participants: BTreeSet<String> = session
            .playback_barrier_buffering_status()
            .map(|status| status.buffering_clients.clone())
            .unwrap_or_default();
        buffering_participants.extend(
            snapshot
                .users
                .iter()
                .filter(|user| user.room_name == current_room)
                .filter_map(|user| {
                    let MainWindowParticipantStatusPresentation::Report(status) =
                        &user.participant_status
                    else {
                        return None;
                    };
                    (status.freshness == MainWindowParticipantStatusFreshness::Fresh
                        && !status.timeline_mismatch
                        && status.status.player_connection
                            == Some(sorotte_protocol::ParticipantPlayerConnection::Connected)
                        && status.status.phase == Some(ParticipantPlaybackPhase::Rebuffering)
                        && status.status.paused_for_cache == Some(true))
                    .then(|| user.username.clone())
                }),
        );
        let current_room_usernames: BTreeSet<&str> = snapshot
            .users
            .iter()
            .filter(|user| user.room_name == current_room)
            .map(|user| user.username.as_str())
            .collect();
        buffering_participants
            .retain(|username| current_room_usernames.contains(username.as_str()));
        snapshot.room_playback_intent.buffering_participants =
            buffering_participants.into_iter().collect();
        (!snapshot.matches_shell_state_with_omitted_playlist_metadata(&state.main_window))
            .then_some(snapshot)
    }

    pub(super) fn session_playlist_selection_index(&self, playlist_len: usize) -> Option<usize> {
        self.projected_current_room_playlist()
            .and_then(|playlist| playlist.index)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|&index| index < playlist_len)
    }

    pub(super) fn interaction_runtime_snapshot(
        &self,
        _state: &SorotteGuiShellAppState,
        interaction_state: &SorotteGuiShellAppState,
        playlist_len: usize,
    ) -> Option<GuiInteractionRuntimeSnapshot> {
        if interaction_state.main_window_playlist_selection_is_local {
            return None;
        }

        let selected_main_window_playlist = self.session_playlist_selection_index(playlist_len);
        if interaction_state.selection.selected_main_window_playlist
            == selected_main_window_playlist
        {
            return None;
        }

        let mut snapshot = GuiInteractionRuntimeSnapshot::from_shell_state(interaction_state);
        snapshot.selection.selected_main_window_playlist = selected_main_window_playlist;
        Some(snapshot)
    }

    pub(super) fn menu_dialog_runtime_snapshot(
        &self,
        state: &SorotteGuiShellAppState,
        shared_playlist_enabled: bool,
    ) -> Option<MenuDialogRuntimeSnapshot> {
        let mut action_overrides = Vec::new();
        let session_room_name = self
            .runtime
            .session()
            .room()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let managed_rooms_server_supported = self.managed_rooms_server_supported();
        let create_controlled_room_enabled =
            managed_rooms_server_supported && session_room_name.is_some();
        let identify_as_controller_enabled = managed_rooms_server_supported
            && session_room_name.is_some_and(|room_name| room_name.starts_with('+'));
        let current_playlist_actions_enabled = state
            .menus
            .action(MenuActionId::SharedPlaylist)
            .map(|action| action.enabled);
        let desired_playlist_actions_enabled =
            shared_playlist_enabled && self.shared_playlist_control_available();
        if current_playlist_actions_enabled
            .is_some_and(|current_enabled| current_enabled != desired_playlist_actions_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                id: MenuActionId::SharedPlaylist,
                enabled: desired_playlist_actions_enabled,
            });
        }

        for (id, enabled) in [
            (
                MenuActionId::CreateControlledRoom,
                create_controlled_room_enabled,
            ),
            (
                MenuActionId::IdentifyAsController,
                identify_as_controller_enabled,
            ),
        ] {
            let current_enabled = state.menus.action(id).map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride { id, enabled });
            }
        }

        if action_overrides.is_empty() {
            return None;
        }

        Some(MenuDialogRuntimeSnapshot {
            action_overrides,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        })
    }
}
