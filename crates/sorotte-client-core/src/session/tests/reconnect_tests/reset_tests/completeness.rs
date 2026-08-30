use std::collections::{BTreeMap, BTreeSet};

use sorotte_protocol::{
    CommitStartPayload, DirectReadinessSurface, ParticipantPlaybackScope,
    ParticipantReadinessUpdate, ParticipantStatusAvailability, ParticipantStatusSnapshotMode,
    ParticipantStatusView, PlaybackBarrierParticipantStatus, PlaybackBarrierPhase,
    PlaybackBarrierPolicy, PlaybackBarrierSetExtension, PlaybackBarrierStatusPayload,
    PrepareMediaPayload, ProtocolMessage, ReadinessMutationSource, RoomBufferingPhase,
    RoomBufferingPolicy, RoomBufferingPolicyPayload, RoomBufferingStatusPayload, RoomPauseOwner,
    RoomReadinessSnapshot, RoomStartGatePhase, SetPayload, StartParticipationRole,
    TechnicalPlayabilityPhase, TechnicalPlayabilitySummary, UserReadinessIntent,
    UserReadinessMutationSource,
};

use super::*;
use crate::{ClientEvent, ReconnectPlaylistRestoreIntent};

const RESET_DEFECT_ID: &str =
    "TC-CLIENT-002: reconnect reset retains in-flight reducer transactions";

#[derive(Debug, Clone, PartialEq)]
struct PlaybackResetProjection {
    desync_config: crate::DesyncCorrectionConfig,
    speed_changed: bool,
    speed_correction_rate: Option<f64>,
    behind_first_detected_at_seconds: Option<f64>,
    last_paused_on_leave_at_seconds: Option<f64>,
    last_advanced_at_seconds: Option<f64>,
    last_rewound_at_seconds: Option<f64>,
    local_position: Option<f64>,
    local_paused: Option<bool>,
    local_playback_rate: Option<f64>,
    local_paused_for_cache: Option<bool>,
    local_cache_buffering_percent: Option<f64>,
    pending_cache_room_playstate_resync: bool,
    cache_recovery_observation_position: Option<f64>,
    cache_recovery_waiting_for_post_cache_position: bool,
    client_ignoring_on_the_fly: u32,
    server_ignoring_on_the_fly: u32,
    pending_room_pause_sync: bool,
    pending_local_pause_change: bool,
    local_pause_change_health: crate::LocalPauseChangeHealth,
}

impl PlaybackResetProjection {
    fn from_session(session: &ClientSession) -> Self {
        let playback = &session.model.playback;
        Self {
            desync_config: playback.desync_config.clone(),
            speed_changed: playback.speed_changed,
            speed_correction_rate: playback.speed_correction_rate,
            behind_first_detected_at_seconds: playback.behind_first_detected_at_seconds,
            last_paused_on_leave_at_seconds: playback.last_paused_on_leave_at_seconds,
            last_advanced_at_seconds: playback.last_advanced_at_seconds,
            last_rewound_at_seconds: playback.last_rewound_at_seconds,
            local_position: playback.local_position,
            local_paused: playback.local_paused,
            local_playback_rate: playback.local_playback_rate,
            local_paused_for_cache: playback.local_paused_for_cache,
            local_cache_buffering_percent: playback.local_cache_buffering_percent,
            pending_cache_room_playstate_resync: playback.pending_cache_room_playstate_resync,
            cache_recovery_observation_position: playback.cache_recovery_observation_position,
            cache_recovery_waiting_for_post_cache_position: playback
                .cache_recovery_waiting_for_post_cache_position,
            client_ignoring_on_the_fly: playback.client_ignoring_on_the_fly,
            server_ignoring_on_the_fly: playback.server_ignoring_on_the_fly,
            pending_room_pause_sync: room_pause_sync_in_flight(session),
            pending_local_pause_change: session.model.local_pause_change_in_flight(),
            local_pause_change_health: playback.local_pause_change_health,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlaylistEchoTrackerProjection {
    room: String,
    pending_revisions: Vec<u64>,
    invalidated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlaylistIndexEchoTrackerProjection {
    room: String,
    pending: Vec<(u64, i64)>,
    invalidated: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct PlaylistResetProjection {
    rooms: BTreeMap<String, crate::RoomPlaylistView>,
    pending: Option<crate::RoomPlaylistView>,
    pending_remote_revision: u64,
    selection_revisions: BTreeMap<String, u64>,
    pending_selection_revision: u64,
    pending_local_change_echoes: Vec<PlaylistEchoTrackerProjection>,
    pending_local_index_echoes: Vec<PlaylistIndexEchoTrackerProjection>,
    remote_revisions: BTreeMap<String, u64>,
    active_targets_before_index_update: BTreeMap<String, String>,
    undo_snapshots: BTreeMap<String, Vec<String>>,
    shuffle_nonce: u64,
    received_first_index: bool,
    pending_index_reset_pause_before_sync: Option<bool>,
    pending_index_reset_refresh_recently_advanced: bool,
    suppress_next_self_index_reset: bool,
    last_seek_position_before_manual_seek: Option<f64>,
}

impl PlaylistResetProjection {
    fn from_session(session: &ClientSession) -> Self {
        let playlist = &session.model.playlist;
        Self {
            rooms: playlist.rooms.clone(),
            pending: playlist.pending.clone(),
            pending_remote_revision: playlist.pending_remote_revision,
            selection_revisions: playlist.selection_revisions.clone(),
            pending_selection_revision: playlist.pending_selection_revision,
            pending_local_change_echoes: playlist
                .pending_local_change_echoes
                .iter()
                .map(|(room, tracker)| PlaylistEchoTrackerProjection {
                    room: room.clone(),
                    pending_revisions: tracker
                        .pending
                        .iter()
                        .map(|pending| pending.revision)
                        .collect(),
                    invalidated: tracker.invalidated,
                })
                .collect(),
            pending_local_index_echoes: playlist
                .pending_local_index_echoes
                .iter()
                .map(|(room, tracker)| PlaylistIndexEchoTrackerProjection {
                    room: room.clone(),
                    pending: tracker
                        .pending
                        .iter()
                        .map(|pending| (pending.playlist_revision, pending.index))
                        .collect(),
                    invalidated: tracker.invalidated,
                })
                .collect(),
            remote_revisions: playlist.remote_revisions.clone(),
            active_targets_before_index_update: playlist.active_targets_before_index_update.clone(),
            undo_snapshots: playlist.undo_snapshots.clone(),
            shuffle_nonce: playlist.shuffle_nonce,
            received_first_index: playlist.received_first_index,
            pending_index_reset_pause_before_sync: playlist.pending_index_reset_pause_before_sync,
            pending_index_reset_refresh_recently_advanced: playlist
                .pending_index_reset_refresh_recently_advanced,
            suppress_next_self_index_reset: playlist.suppress_next_self_index_reset,
            last_seek_position_before_manual_seek: playlist.last_seek_position_before_manual_seek,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ReadinessResetProjection {
    config: ReadinessAutoplayConfig,
    autoplay_enabled: bool,
    autoplay_timer_running: bool,
    autoplay_time_left_seconds: f64,
    canonical_snapshot: Option<RoomReadinessSnapshot>,
    canonical_room: Option<String>,
    awaiting_readiness_reconciliation_snapshot: bool,
    pending_intent: Option<crate::PendingReadinessIntent>,
    next_request_nonce: u64,
    reconnect_token: Option<String>,
}

impl ReadinessResetProjection {
    fn from_session(session: &ClientSession) -> Self {
        let readiness = &session.model.readiness;
        Self {
            config: readiness.config.clone(),
            autoplay_enabled: readiness.autoplay_enabled,
            autoplay_timer_running: readiness.autoplay_timer_running,
            autoplay_time_left_seconds: readiness.autoplay_time_left_seconds,
            canonical_snapshot: readiness.canonical_snapshot.clone(),
            canonical_room: readiness.canonical_room.clone(),
            awaiting_readiness_reconciliation_snapshot: readiness
                .awaiting_readiness_reconciliation_snapshot,
            pending_intent: readiness.pending_intent.clone(),
            next_request_nonce: readiness.next_request_nonce,
            reconnect_token: readiness
                .reconnect_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ReconnectResetProjection {
    policy: crate::ReconnectPolicyConfig,
    ready_restore_snapshot: Option<bool>,
    ready_restore_intent: Option<bool>,
    file_restore_snapshot: Option<crate::SharedFile>,
    file_restore_intent: Option<crate::SharedFile>,
    controller_restore_snapshot: Option<bool>,
    playlist_restore_snapshot: Option<ReconnectPlaylistRestoreIntent>,
    playlist_restore_intent: Option<ReconnectPlaylistRestoreIntent>,
    playlist_restore_pending_ack: Option<ReconnectPlaylistRestoreIntent>,
    state_restore_validation_pending: bool,
    state_restore_validation_retry_attempts: u32,
    state_restore_validation_retry_cooldown_ticks: u32,
    state_restore_validation_mismatch_notified: bool,
    state_restore_validation_mismatch_seen_in_cycle: bool,
    state_restore_correction_consecutive_mismatch_cycles: u32,
    state_restore_correction_consecutive_retry_exhaustions: u32,
    state_restore_correction_recovery_cooldown_reconnect_cycles_remaining: u32,
    state_restore_correction_recovery_suppressed_this_cycle: bool,
    state_restore_correction_recovery_reenable_notification_pending: bool,
    state_restore_correction_recovery_reenabled_this_cycle: bool,
    state_restore_correction_metrics: ReconnectStateRestoreCorrectionMetrics,
    in_progress: bool,
    connected_intent: bool,
}

impl ReconnectResetProjection {
    fn from_session(session: &ClientSession) -> Self {
        let reconnect = &session.model.reconnect;
        Self {
            policy: reconnect.policy.clone(),
            ready_restore_snapshot: reconnect.ready_restore_snapshot,
            ready_restore_intent: reconnect.ready_restore_intent,
            file_restore_snapshot: reconnect.file_restore_snapshot.clone(),
            file_restore_intent: reconnect.file_restore_intent.clone(),
            controller_restore_snapshot: reconnect.controller_restore_snapshot,
            playlist_restore_snapshot: reconnect.playlist_restore_snapshot.clone(),
            playlist_restore_intent: reconnect.playlist_restore_intent.clone(),
            playlist_restore_pending_ack: reconnect.playlist_restore_pending_ack.clone(),
            state_restore_validation_pending: reconnect.state_restore_validation_pending,
            state_restore_validation_retry_attempts: reconnect
                .state_restore_validation_retry_attempts,
            state_restore_validation_retry_cooldown_ticks: reconnect
                .state_restore_validation_retry_cooldown_ticks,
            state_restore_validation_mismatch_notified: reconnect
                .state_restore_validation_mismatch_notified,
            state_restore_validation_mismatch_seen_in_cycle: reconnect
                .state_restore_validation_mismatch_seen_in_cycle,
            state_restore_correction_consecutive_mismatch_cycles: reconnect
                .state_restore_correction_consecutive_mismatch_cycles,
            state_restore_correction_consecutive_retry_exhaustions: reconnect
                .state_restore_correction_consecutive_retry_exhaustions,
            state_restore_correction_recovery_cooldown_reconnect_cycles_remaining: reconnect
                .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            state_restore_correction_recovery_suppressed_this_cycle: reconnect
                .state_restore_correction_recovery_suppressed_this_cycle,
            state_restore_correction_recovery_reenable_notification_pending: reconnect
                .state_restore_correction_recovery_reenable_notification_pending,
            state_restore_correction_recovery_reenabled_this_cycle: reconnect
                .state_restore_correction_recovery_reenabled_this_cycle,
            state_restore_correction_metrics: reconnect.state_restore_correction_metrics,
            in_progress: reconnect.in_progress,
            connected_intent: reconnect.connected_intent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControllerResetProjection {
    controlled_room_switch_intent: Option<String>,
    pending_local_room_switch_target: Option<String>,
    reidentify_intent: Option<(String, String)>,
    last_auth_password_attempt: Option<String>,
    room_passwords: BTreeMap<String, String>,
}

impl ControllerResetProjection {
    fn from_session(session: &ClientSession) -> Self {
        let controller = &session.model.controller;
        Self {
            controlled_room_switch_intent: controller.controlled_room_switch_intent.clone(),
            pending_local_room_switch_target: controller.pending_local_room_switch_target.clone(),
            reidentify_intent: controller
                .reidentify_intent
                .as_ref()
                .map(|(room, password)| (room.clone(), password.expose_secret().to_owned())),
            last_auth_password_attempt: controller
                .last_auth_password_attempt
                .as_ref()
                .map(|password| password.expose_secret().to_owned()),
            room_passwords: controller
                .room_passwords
                .iter()
                .map(|(room, password)| (room.clone(), password.expose_secret().to_owned()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SessionResetProjection {
    connection_username: Option<String>,
    connection_phase: ConnectionPhase,
    connection_participant_status_v1: bool,
    room_name: Option<String>,
    room_domain: String,
    room_users: BTreeMap<String, crate::ClientUserView>,
    room_participant_status_capabilities: BTreeMap<String, bool>,
    room_legacy_list_position_snapshots: BTreeMap<String, f64>,
    room_participant_statuses: BTreeMap<String, crate::ClientParticipantStatusView>,
    room_participant_status_receipts: BTreeMap<String, (u64, bool)>,
    room_participant_status_snapshot_revision: Option<u64>,
    room_participant_status_snapshot_mode: ParticipantStatusSnapshotMode,
    room_participant_status_authoritative_scope: Option<ParticipantPlaybackScope>,
    room_media_match_peer_tiers: BTreeMap<String, MediaMatchTier>,
    room_known_rooms: BTreeSet<String>,
    room_playstates: BTreeMap<String, RoomPlaystateView>,
    room_playstate_updated_at_seconds: BTreeMap<String, f64>,
    room_playstate_authority_changed_at_seconds: BTreeMap<String, f64>,
    playback: PlaybackResetProjection,
    playlist: PlaylistResetProjection,
    readiness: ReadinessResetProjection,
    reconnect: ReconnectResetProjection,
    controller: ControllerResetProjection,
    behavior_config: crate::SessionBehaviorConfig,
    chat_config: ChatConfig,
    pending_chat_notifications: Vec<ChatNotification>,
    pending_controlled_room_creation_notifications: Vec<ControlledRoomCreationNotification>,
    pending_controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
    pending_user_change_notifications: Vec<UserChangeNotification>,
    pending_compatibility_fallbacks: Vec<crate::ClientCompatibilityFallback>,
    playback_barrier: String,
}

impl SessionResetProjection {
    fn from_session(session: &ClientSession) -> Self {
        // Keep this exhaustive: a new top-level session/model aggregate must
        // be classified as durable or reset-scoped before this compiles.
        let ClientSession {
            model,
            behavior_config,
            chat_config,
            pending_chat_notifications,
            pending_controlled_room_creation_notifications,
            pending_controller_auth_notifications,
            pending_user_change_notifications,
            pending_compatibility_fallbacks,
            playback_barrier,
        } = session;
        let crate::ClientModel {
            connection,
            room,
            playback: _,
            playlist: _,
            readiness: _,
            reconnect: _,
            controller: _,
        } = model;

        Self {
            connection_username: connection.username.clone(),
            connection_phase: connection.phase.clone(),
            connection_participant_status_v1: connection.participant_status_v1,
            room_name: room.name.clone(),
            room_domain: format!("{:#?}", room.domain),
            room_users: room.users.clone(),
            room_participant_status_capabilities: room.participant_status_capabilities.clone(),
            room_legacy_list_position_snapshots: room.legacy_list_position_snapshots.clone(),
            room_participant_statuses: room.participant_statuses.clone(),
            room_participant_status_receipts: room
                .participant_status_receipts
                .iter()
                .map(|(username, receipt)| {
                    (
                        username.clone(),
                        (
                            receipt.received_at_seconds.to_bits(),
                            receipt
                                .clock_invalidated
                                .load(std::sync::atomic::Ordering::Relaxed),
                        ),
                    )
                })
                .collect(),
            room_participant_status_snapshot_revision: room.participant_status_snapshot_revision,
            room_participant_status_snapshot_mode: room.participant_status_snapshot_mode,
            room_participant_status_authoritative_scope: room
                .participant_status_authoritative_scope,
            room_media_match_peer_tiers: room.media_match_peer_tiers.clone(),
            room_known_rooms: room.known_rooms.clone(),
            room_playstates: room.playstates.clone(),
            room_playstate_updated_at_seconds: room.playstate_updated_at_seconds.clone(),
            room_playstate_authority_changed_at_seconds: room
                .playstate_authority_changed_at_seconds
                .clone(),
            playback: PlaybackResetProjection::from_session(session),
            playlist: PlaylistResetProjection::from_session(session),
            readiness: ReadinessResetProjection::from_session(session),
            reconnect: ReconnectResetProjection::from_session(session),
            controller: ControllerResetProjection::from_session(session),
            behavior_config: behavior_config.clone(),
            chat_config: chat_config.clone(),
            pending_chat_notifications: pending_chat_notifications.clone(),
            pending_controlled_room_creation_notifications:
                pending_controlled_room_creation_notifications.clone(),
            pending_controller_auth_notifications: pending_controller_auth_notifications.clone(),
            pending_user_change_notifications: pending_user_change_notifications.clone(),
            pending_compatibility_fallbacks: pending_compatibility_fallbacks.clone(),
            playback_barrier: format!("{playback_barrier:#?}"),
        }
    }
}

#[test]
fn reconnect_reset_matches_a_fresh_reference() {
    // Different attempts and generated seeds exercise the same contract over
    // bounded, distinguishable pre-reset states without relying on timing.
    for generated_seed in 1_u64..=24 {
        let attempt = u32::try_from(generated_seed % 7).expect("bounded attempt");
        let mut actual = fully_seeded_session(generated_seed);
        assert_dense_seed(&actual);
        let mut expected = fresh_reference_preserving_only_durable_state(&actual);

        assert_eq!(
            actual.plan_reconnect_retry(attempt),
            expected.plan_reconnect_retry(attempt),
            "generated seed {generated_seed} changed the owning reconnect transition"
        );

        let actual_projection = SessionResetProjection::from_session(&actual);
        let expected_projection = SessionResetProjection::from_session(&expected);
        assert_eq!(
            actual_projection, expected_projection,
            "generated seed {generated_seed} leaked mutable session state across reconnect reset"
        );
    }
}

#[test]
fn reconnect_reset_is_idempotent_for_the_complete_projection() {
    for generated_seed in 1_u64..=24 {
        let mut session = fully_seeded_session(generated_seed);
        let _ = session.plan_reconnect_retry(3);
        let once = SessionResetProjection::from_session(&session);
        let _ = session.plan_reconnect_retry(3);
        let twice = SessionResetProjection::from_session(&session);
        assert_eq!(
            once, twice,
            "generated seed {generated_seed} changed after an identical second reset"
        );
    }
}

#[test]
fn reconnect_reset_rejects_stale_reducer_completions() {
    let mut session = fully_seeded_session(91);
    let _ = session.plan_reconnect_retry(4);

    let retained_local_transaction = session.model.local_pause_change_in_flight();
    let retained_room_transaction = room_pause_sync_in_flight(&session);
    let retained_degraded_health =
        session.local_pause_change_health() != crate::LocalPauseChangeHealth::Healthy;

    let room_follow_up = session.model.apply(ClientEvent::EffectSucceeded(
        ClientEffect::SetPlayerPosition(391.0),
    ));
    let local_follow_up =
        session
            .model
            .apply(ClientEvent::EffectSucceeded(ClientEffect::SetPlayerPaused(
                false,
            )));
    let room_completion =
        session
            .model
            .apply(ClientEvent::EffectSucceeded(ClientEffect::SetPlayerPaused(
                true,
            )));
    let stale_room_failure =
        session
            .model
            .apply(ClientEvent::EffectFailed(ClientEffect::SetPlayerPosition(
                391.0,
            )));
    let stale_local_failure =
        session
            .model
            .apply(ClientEvent::EffectFailed(ClientEffect::SetPlayerPaused(
                false,
            )));

    assert!(
        !retained_local_transaction
            && !retained_room_transaction
            && !retained_degraded_health
            && room_follow_up.is_empty()
            && local_follow_up.is_empty()
            && room_completion.is_empty()
            && stale_room_failure.is_empty()
            && stale_local_failure.is_empty()
            && session.model.playback.local_position.is_none()
            && session.model.playback.local_paused.is_none(),
        "{RESET_DEFECT_ID}; retained_local={retained_local_transaction}; \
         retained_room={retained_room_transaction}; retained_health={retained_degraded_health}; \
         stale_room_follow_up={room_follow_up:?}; stale_local_follow_up={local_follow_up:?}; \
         stale_room_completion={room_completion:?}; stale_room_failure={stale_room_failure:?}; \
         stale_local_failure={stale_local_failure:?}; local_position={:?}; local_paused={:?}",
        session.model.playback.local_position,
        session.model.playback.local_paused,
    );

    assert_eq!(
        session.model.apply(ClientEvent::LocalPauseChangeRequested {
            original_paused: None,
            original_ready: None,
            original_last_paused_on_leave_at_seconds: None,
            planned_paused: Some(true),
            planned_ready: None,
            planned_last_paused_on_leave_at_seconds: None,
            effects: vec![ClientEffect::SetPlayerPaused(true)],
        }),
        vec![ClientEffect::SetPlayerPaused(true)],
        "a post-reconnect transaction must start after stale completions are rejected"
    );
    assert!(session.model.local_pause_change_in_flight());
}

fn fully_seeded_session(seed: u64) -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true,"remoteReadiness":true,"sharedPlaylists":true,"managedRooms":true,"mediaMatch":true,"plexPlaylistUris":true,"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true,"persistentRooms":true},"sorotteReadinessReconnectToken":"reset-token"}}"#,
        )
        .expect("dense seed Hello should apply");

    seed_durable_configuration(&mut session, seed);
    seed_room_state(&mut session, seed);
    seed_playback_state(&mut session, seed);
    seed_playlist_state(&mut session, seed);
    seed_readiness_state(&mut session, seed);
    seed_reconnect_state(&mut session, seed);
    seed_controller_state(&mut session, seed);
    seed_notification_state(&mut session, seed);
    seed_playback_barrier_state(&mut session, seed);
    session
}

fn seed_durable_configuration(session: &mut ClientSession, seed: u64) {
    *session.behavior_config_mut() = crate::SessionBehaviorConfig {
        pause_on_leave: false,
        show_same_room_osd: false,
        show_osd_warnings: false,
        show_noncontroller_osd: true,
        show_different_room_osd: true,
        loop_at_end_of_playlist: true,
        loop_single_files: true,
        only_switch_to_trusted_domains: false,
        trusted_domains: vec![format!("seed-{seed}.example")],
        reconnect_state_restore_auto_correct: false,
        reconnect_state_restore_correction_policy_mode_override: Some(
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion,
        ),
        reconnect_state_restore_position_tolerance_seconds: 1.25 + seed as f64 / 100.0,
        reconnect_state_restore_correction_retry_max_attempts: 11,
        reconnect_state_restore_correction_retry_cooldown_ticks: 12,
        reconnect_state_restore_correction_retry_exponential_backoff: true,
        reconnect_state_restore_correction_retry_max_cooldown_ticks: 13,
        reconnect_state_restore_correction_retry_adaptive_cycle_backoff: true,
        reconnect_state_restore_correction_retry_adaptive_cycle_budget: true,
        reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts: 14,
        reconnect_state_restore_correction_disable_after_mismatch_cycles: 15,
        reconnect_state_restore_correction_disable_after_mismatch_decay_on_success: 16,
        reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles: 17,
    };
    *session.chat_config_mut() = ChatConfig {
        max_chat_message_length: 377,
        apply_server_max_chat_message_length: false,
    };
    session.model.playback.desync_config = crate::DesyncCorrectionConfig {
        rewind_on_desync: false,
        rewind_threshold_seconds: 21.1,
        fastforward_on_desync: false,
        fastforward_threshold_seconds: 22.2,
        fastforward_behind_threshold_seconds: 23.3,
        fastforward_extra_seconds: 24.4,
        fastforward_reset_threshold_seconds: 25.5,
        slow_on_desync: false,
        slowdown_threshold_seconds: 26.6,
        slowdown_rate: 0.77,
        slowdown_reset_threshold_seconds: 27.7,
    };
    session.model.readiness.config = ReadinessAutoplayConfig {
        unpause_action: UnpauseActionMode::Always,
        auto_play_threshold: Some(9),
        autoplay_require_same_filenames: true,
        show_duration_notification: false,
        different_duration_threshold_seconds: 31.1,
        autoplay_delay_seconds: 32.2,
        last_paused_diff_threshold_seconds: 33.3,
    };
    session.model.reconnect.policy = crate::ReconnectPolicyConfig {
        max_retries: 41,
        base_delay_seconds: 4.1,
        max_backoff_exponent: 42,
    };
}

fn seed_room_state(session: &mut ClientSession, seed: u64) {
    session.model.room.name = Some("room1".to_owned());
    session
        .model
        .room
        .domain
        .join_room_with_ready("alice", "room1", Some(true));
    session
        .model
        .room
        .domain
        .join_room_with_ready("bob", "other-room", Some(false));
    session.model.room.users.insert(
        "alice".to_owned(),
        crate::ClientUserView {
            room: Some("room1".to_owned()),
            ready: Some(true),
            file: Some(shared_file("alice", seed)),
            capabilities: Some(peer_capabilities("gui")),
            controller: true,
        },
    );
    session
        .model
        .room
        .participant_status_capabilities
        .extend([("alice".to_owned(), true), ("bob".to_owned(), true)]);
    session
        .model
        .room
        .legacy_list_position_snapshots
        .insert("bob".to_owned(), 17.5 + seed as f64);
    session.model.room.participant_statuses.insert(
        "bob".to_owned(),
        crate::ClientParticipantStatusView::from_wire(ParticipantStatusView::new(
            ParticipantStatusAvailability::AwaitingReport,
        )),
    );
    session.model.room.participant_status_receipts.insert(
        "bob".to_owned(),
        crate::model::ParticipantStatusReceipt::new(18.5 + seed as f64),
    );
    session.model.room.participant_status_snapshot_revision = Some(seed.max(1));
    session.model.room.participant_status_snapshot_mode = ParticipantStatusSnapshotMode::Compact;
    session.model.room.participant_status_authoritative_scope =
        Some(ParticipantPlaybackScope::new(seed.max(1)).with_transport_revision(seed.max(1)));
    session.model.room.users.insert(
        "bob".to_owned(),
        crate::ClientUserView {
            room: Some("other-room".to_owned()),
            ready: Some(false),
            file: Some(shared_file("bob", seed + 1)),
            capabilities: Some(peer_capabilities("cli")),
            controller: false,
        },
    );
    session
        .model
        .room
        .media_match_peer_tiers
        .insert("bob".to_owned(), MediaMatchTier::Strong);
    session
        .model
        .room
        .known_rooms
        .extend(["room1".to_owned(), "other-room".to_owned()]);
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(101.0 + seed as f64),
            paused: Some(false),
            do_seek: Some(true),
            set_by: Some("bob".to_owned()),
        },
    );
    session
        .model
        .room
        .playstate_updated_at_seconds
        .insert("room1".to_owned(), 102.0 + seed as f64);
    session
        .model
        .room
        .playstate_authority_changed_at_seconds
        .insert("room1".to_owned(), 103.0 + seed as f64);
}

fn seed_playback_state(session: &mut ClientSession, seed: u64) {
    let playback = &mut session.model.playback;
    playback.speed_changed = true;
    playback.speed_correction_rate = Some(0.88);
    playback.behind_first_detected_at_seconds = Some(201.0 + seed as f64);
    playback.last_paused_on_leave_at_seconds = Some(202.0 + seed as f64);
    playback.last_advanced_at_seconds = Some(203.0 + seed as f64);
    playback.last_rewound_at_seconds = Some(204.0 + seed as f64);
    playback.local_position = Some(205.0 + seed as f64);
    playback.local_paused = Some(false);
    playback.local_playback_rate = Some(1.11);
    playback.local_paused_for_cache = Some(true);
    playback.local_cache_buffering_percent = Some(66.6);
    playback.pending_cache_room_playstate_resync = true;
    playback.cache_recovery_observation_position = Some(206.0 + seed as f64);
    playback.cache_recovery_waiting_for_post_cache_position = true;
    playback.client_ignoring_on_the_fly = 207;
    playback.server_ignoring_on_the_fly = 208;

    assert_eq!(
        session.model.apply(ClientEvent::LocalPauseChangeRequested {
            original_paused: Some(true),
            original_ready: Some(true),
            original_last_paused_on_leave_at_seconds: Some(209.0 + seed as f64),
            planned_paused: Some(false),
            planned_ready: Some(false),
            planned_last_paused_on_leave_at_seconds: Some(210.0 + seed as f64),
            effects: vec![ClientEffect::SetPlayerPaused(false)],
        }),
        vec![ClientEffect::SetPlayerPaused(false)],
        "local reducer seed should enter an in-flight transaction"
    );
    assert_eq!(
        session.model.apply(ClientEvent::RoomPauseSyncRequested {
            original_position: Some(211.0 + seed as f64),
            target_position: Some(391.0),
            target_paused: Some(true),
            clear_cache_resync_on_success: true,
        }),
        vec![ClientEffect::SetPlayerPosition(391.0)],
        "room reducer seed should enter an independent in-flight transaction"
    );
    session.model.playback.local_pause_change_health =
        crate::LocalPauseChangeHealth::ControlEffectFailedAfterPlayerChange;
}

fn seed_playlist_state(session: &mut ClientSession, seed: u64) {
    let room_playlist = crate::RoomPlaylistView {
        files: vec![
            format!("episode-{seed}-a.mkv"),
            format!("episode-{seed}-b.mkv"),
        ],
        index: Some(1),
        set_by: Some("alice".to_owned()),
        revision: 301 + seed,
    };
    let playlist = &mut session.model.playlist;
    playlist
        .rooms
        .insert("room1".to_owned(), room_playlist.clone());
    playlist.pending = Some(crate::RoomPlaylistView {
        files: vec!["pending.mkv".to_owned()],
        index: Some(0),
        set_by: Some("bob".to_owned()),
        revision: 302 + seed,
    });
    playlist.pending_remote_revision = 303 + seed;
    playlist
        .selection_revisions
        .insert("room1".to_owned(), 304 + seed);
    playlist.pending_selection_revision = 305 + seed;
    playlist
        .pending_local_change_echoes
        .entry("room1".to_owned())
        .or_default()
        .record(306 + seed, &room_playlist.files);
    playlist
        .pending_local_change_echoes
        .entry("invalidated-room".to_owned())
        .or_default()
        .invalidated = true;
    playlist
        .pending_local_index_echoes
        .entry("room1".to_owned())
        .or_default()
        .record(307 + seed, 1);
    playlist
        .pending_local_index_echoes
        .entry("invalidated-room".to_owned())
        .or_default()
        .invalidated = true;
    playlist
        .remote_revisions
        .insert("room1".to_owned(), 306 + seed);
    playlist
        .active_targets_before_index_update
        .insert("room1".to_owned(), format!("episode-{seed}-a.mkv"));
    playlist.undo_snapshots.insert(
        "room1".to_owned(),
        vec![format!("undo-{seed}-a.mkv"), format!("undo-{seed}-b.mkv")],
    );
    playlist.shuffle_nonce = 307 + seed;
    playlist.received_first_index = true;
    playlist.pending_index_reset_pause_before_sync = Some(false);
    playlist.pending_index_reset_refresh_recently_advanced = true;
    playlist.suppress_next_self_index_reset = true;
    playlist.last_seek_position_before_manual_seek = Some(308.0 + seed as f64);
}

fn seed_readiness_state(session: &mut ClientSession, seed: u64) {
    let snapshot = readiness_snapshot(seed);
    let readiness = &mut session.model.readiness;
    readiness.autoplay_enabled = true;
    readiness.autoplay_timer_running = true;
    readiness.autoplay_time_left_seconds = 401.0 + seed as f64;
    readiness.canonical_snapshot = Some(snapshot);
    readiness.canonical_room = Some("room1".to_owned());
    readiness.awaiting_readiness_reconciliation_snapshot = true;
    readiness.pending_intent = Some(crate::PendingReadinessIntent {
        room: "room1".to_owned(),
        operation_id: format!("reset-operation-{seed}"),
        request_nonce: 402 + seed,
        membership_epoch: 403 + seed,
        desired: UserReadinessIntent::NotReady,
        source: UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::KeyboardShortcut,
        },
        target_username: Some("alice".to_owned()),
        expected_user_intent_revision: Some(404 + seed),
        scope_from_rejection_result: true,
        needs_send: false,
    });
    readiness.next_request_nonce = 405 + seed;
    readiness.reconnect_token = Some(format!("readiness-token-{seed}").into());
}

fn seed_reconnect_state(session: &mut ClientSession, seed: u64) {
    let reconnect = &mut session.model.reconnect;
    reconnect.ready_restore_snapshot = Some(false);
    reconnect.ready_restore_intent = Some(true);
    reconnect.file_restore_snapshot = Some(shared_file("snapshot", seed + 10));
    reconnect.file_restore_intent = Some(shared_file("intent", seed + 11));
    reconnect.controller_restore_snapshot = Some(false);
    reconnect.playlist_restore_snapshot = Some(ReconnectPlaylistRestoreIntent {
        files: vec![format!("snapshot-{seed}.mkv")],
        index: Some(0),
    });
    reconnect.playlist_restore_intent = Some(ReconnectPlaylistRestoreIntent {
        files: vec![format!("intent-{seed}.mkv")],
        index: Some(1),
    });
    reconnect.playlist_restore_pending_ack = Some(ReconnectPlaylistRestoreIntent {
        files: vec![format!("ack-{seed}.mkv")],
        index: Some(2),
    });
    reconnect.state_restore_validation_pending = true;
    reconnect.state_restore_validation_retry_attempts = 501;
    reconnect.state_restore_validation_retry_cooldown_ticks = 502;
    reconnect.state_restore_validation_mismatch_notified = true;
    reconnect.state_restore_validation_mismatch_seen_in_cycle = true;
    reconnect.state_restore_correction_consecutive_mismatch_cycles = 503;
    reconnect.state_restore_correction_consecutive_retry_exhaustions = 504;
    reconnect.state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 505;
    reconnect.state_restore_correction_recovery_suppressed_this_cycle = true;
    reconnect.state_restore_correction_recovery_reenable_notification_pending = true;
    reconnect.state_restore_correction_recovery_reenabled_this_cycle = true;
    reconnect.state_restore_correction_metrics = ReconnectStateRestoreCorrectionMetrics {
        validation_cycles_started: 511,
        validation_cycles_completed_without_mismatch: 512,
        validation_cycles_completed_with_successful_correction: 513,
        mismatch_cycles_detected: 514,
        mismatch_notifications_emitted: 515,
        correction_actions_attempted: 516,
        correction_actions_succeeded: 517,
        correction_action_failures: 518,
        correction_retries_scheduled: 519,
        correction_retry_exhaustions: 520,
        correction_disables_after_repeated_mismatches: 521,
        correction_recovery_cooldown_suppressed_cycles: 522,
        correction_recovery_cooldown_reenabled_cycles: 523,
    };
    reconnect.in_progress = true;
    reconnect.connected_intent = true;
}

fn seed_controller_state(session: &mut ClientSession, seed: u64) {
    let controller = &mut session.model.controller;
    controller.controlled_room_switch_intent = Some(format!("+switch-{seed}"));
    controller.pending_local_room_switch_target = Some(format!("+pending-{seed}"));
    controller.reidentify_intent = Some((
        format!("+reidentify-{seed}"),
        format!("reidentify-password-{seed}").into(),
    ));
    controller.last_auth_password_attempt = Some(format!("last-auth-password-{seed}").into());
    controller.room_passwords.insert(
        format!("+durable-{seed}"),
        format!("durable-password-{seed}").into(),
    );
}

fn seed_notification_state(session: &mut ClientSession, seed: u64) {
    session
        .pending_chat_notifications
        .push(ChatNotification::Message {
            username: Some("bob".to_owned()),
            message: format!("chat-{seed}"),
        });
    session.pending_controlled_room_creation_notifications.push(
        ControlledRoomCreationNotification::Created {
            room: format!("+created-{seed}"),
            password: format!("created-password-{seed}").into(),
        },
    );
    session.pending_controller_auth_notifications.push(
        ControllerAuthTransitionNotification::Succeeded {
            username: "alice".to_owned(),
            room: format!("+auth-{seed}"),
            hide_from_osd: true,
        },
    );
    session
        .pending_user_change_notifications
        .push(UserChangeNotification::Playing {
            username: "bob".to_owned(),
            room: "other-room".to_owned(),
            file_name: Some(format!("playing-{seed}.mkv")),
            file_duration: Some(601.0 + seed as f64),
            include_room_addendum: true,
            hide_from_osd: true,
        });
    session.pending_compatibility_fallbacks.push(
        crate::ClientCompatibilityFallback::IgnoredSetCommand {
            command: format!("future-command-{seed}"),
        },
    );
}

fn seed_playback_barrier_state(session: &mut ClientSession, seed: u64) {
    let generation = 700 + seed;
    let revision = 800 + seed;
    let prepare = PrepareMediaPayload::new(
        generation,
        format!("logical-media-{seed}"),
        12.5,
        PlaybackBarrierPolicy::AllEligible,
    )
    .with_request_nonce(900 + seed)
    .with_timeout_ms(10_000)
    .with_deadline(110.0);
    let commit = CommitStartPayload::new(generation, revision, 12.5, 100.0, 115.0);
    let status = PlaybackBarrierStatusPayload {
        media_generation: generation,
        state_revision: Some(revision),
        phase: PlaybackBarrierPhase::Committed,
        policy: PlaybackBarrierPolicy::AllEligible,
        quorum: None,
        deadline: 115.0,
        participants: BTreeMap::<String, PlaybackBarrierParticipantStatus>::new(),
        excluded_legacy_clients: BTreeSet::new(),
    };
    let buffering_policy =
        RoomBufferingPolicyPayload::new(generation, RoomBufferingPolicy::PauseAnyEligible)
            .with_state_revision(revision)
            .with_debounce_ms(750)
            .with_resume_hysteresis_ms(1_500)
            .with_max_pause_ms(30_000);
    let buffering_status = RoomBufferingStatusPayload {
        config: buffering_policy.clone(),
        phase: RoomBufferingPhase::Paused,
        eligible_clients: 2,
        required_buffering_clients: 1,
        buffering_clients: BTreeSet::from(["bob".to_owned()]),
        pause_deadline: Some(130.0),
    };

    session
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(prepare)
                    .with_commit(commit)
                    .with_status(status)
                    .with_buffering_policy(buffering_policy)
                    .with_buffering_status(buffering_status),
            ),
        ))
        .expect("dense playback-barrier seed should apply");
    assert!(
        session
            .playback_barrier_transport_observation(
                generation,
                Some(revision),
                true,
                Some(4.5),
                Some(120.0),
            )
            .is_some(),
        "dense barrier seed should record the last transport observation"
    );
}

fn fresh_reference_preserving_only_durable_state(source: &ClientSession) -> ClientSession {
    let mut reference = ClientSession::default();

    reference.model.connection.username = source.model.connection.username.clone();
    reference.model.room.name = source.model.room.name.clone();
    reference.behavior_config = source.behavior_config.clone();
    reference.chat_config = source.chat_config.clone();
    reference.model.playback.desync_config = source.model.playback.desync_config.clone();

    reference.model.readiness.config = source.model.readiness.config.clone();
    reference.model.readiness.autoplay_enabled = source.model.readiness.autoplay_enabled;
    reference.model.readiness.canonical_snapshot =
        source.model.readiness.canonical_snapshot.clone();
    reference.model.readiness.canonical_room = source.model.readiness.canonical_room.clone();
    reference.model.readiness.pending_intent = source.model.readiness.pending_intent.clone();
    reference.model.readiness.next_request_nonce = source.model.readiness.next_request_nonce;
    reference.model.readiness.reconnect_token = source.model.readiness.reconnect_token.clone();

    reference.model.reconnect.policy = source.model.reconnect.policy.clone();
    reference.model.reconnect.ready_restore_snapshot =
        source.model.reconnect.ready_restore_snapshot;
    reference.model.reconnect.file_restore_snapshot =
        source.model.reconnect.file_restore_snapshot.clone();
    reference.model.reconnect.controller_restore_snapshot =
        source.model.reconnect.controller_restore_snapshot;
    reference.model.reconnect.playlist_restore_snapshot =
        source.model.reconnect.playlist_restore_snapshot.clone();
    reference
        .model
        .reconnect
        .state_restore_correction_consecutive_mismatch_cycles = source
        .model
        .reconnect
        .state_restore_correction_consecutive_mismatch_cycles;
    reference
        .model
        .reconnect
        .state_restore_correction_consecutive_retry_exhaustions = source
        .model
        .reconnect
        .state_restore_correction_consecutive_retry_exhaustions;
    reference
        .model
        .reconnect
        .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = source
        .model
        .reconnect
        .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining;
    reference
        .model
        .reconnect
        .state_restore_correction_recovery_reenable_notification_pending = source
        .model
        .reconnect
        .state_restore_correction_recovery_reenable_notification_pending;
    reference.model.reconnect.state_restore_correction_metrics =
        source.model.reconnect.state_restore_correction_metrics;

    reference.model.controller.last_auth_password_attempt =
        source.model.controller.last_auth_password_attempt.clone();
    reference.model.controller.room_passwords = source.model.controller.room_passwords.clone();
    reference.pending_compatibility_fallbacks = source.pending_compatibility_fallbacks.clone();
    reference
}

fn assert_dense_seed(session: &ClientSession) {
    let projection = SessionResetProjection::from_session(session);
    let fresh = SessionResetProjection::from_session(&ClientSession::default());
    assert_ne!(projection.connection_phase, fresh.connection_phase);
    assert_ne!(projection.room_domain, fresh.room_domain);
    assert_ne!(projection.room_users, fresh.room_users);
    assert_ne!(projection.playback, fresh.playback);
    assert_ne!(projection.playlist, fresh.playlist);
    assert_ne!(projection.readiness, fresh.readiness);
    assert_ne!(projection.reconnect, fresh.reconnect);
    assert_ne!(projection.controller, fresh.controller);
    assert_ne!(projection.behavior_config, fresh.behavior_config);
    assert_ne!(projection.chat_config, fresh.chat_config);
    assert!(!projection.pending_chat_notifications.is_empty());
    assert!(
        !projection
            .pending_controlled_room_creation_notifications
            .is_empty()
    );
    assert!(!projection.pending_controller_auth_notifications.is_empty());
    assert!(!projection.pending_user_change_notifications.is_empty());
    assert!(!projection.pending_compatibility_fallbacks.is_empty());
    assert_ne!(projection.playback_barrier, fresh.playback_barrier);
    assert!(projection.playback.pending_local_pause_change);
    assert!(projection.playback.pending_room_pause_sync);
}

fn room_pause_sync_in_flight(session: &ClientSession) -> bool {
    format!("{:?}", session.model.playback).contains("pending_room_pause_sync: Some(")
}

fn peer_capabilities(ui_mode: &str) -> crate::PeerCapabilities {
    crate::PeerCapabilities {
        shared_playlists: true,
        chat: true,
        feature_list: true,
        readiness: true,
        managed_rooms: true,
        persistent_rooms: true,
        media_match: true,
        plex_playlist_uris: true,
        remote_readiness: true,
        playback_barrier_v1: true,
        readiness_v2: true,
        ui_mode: Some(ui_mode.to_owned()),
    }
}

fn shared_file(label: &str, seed: u64) -> crate::SharedFile {
    crate::SharedFile {
        name: Some(format!("{label}-{seed}.mkv")),
        duration: Some(crate::FileDuration::Float(1_000.0 + seed as f64)),
        size: Some(crate::FileSize::Text(format!("size-{seed}"))),
        media_match: None,
        extra: BTreeMap::from([("futureField".to_owned(), serde_json::json!({"seed": seed}))]),
    }
}

fn readiness_snapshot(seed: u64) -> RoomReadinessSnapshot {
    let revision = 1_000 + seed;
    let membership_epoch = 1_100 + seed;
    let participant = ParticipantReadinessUpdate {
        room_readiness_revision: revision,
        membership_epoch,
        last_technical_report_sequence: 1_200 + seed,
        username: "alice".to_owned(),
        user_intent: UserReadinessIntent::Ready,
        user_intent_revision: 1_300 + seed,
        user_intent_source: ReadinessMutationSource::Initialization,
        last_user_mutation: None,
        terminal_technical_block: None,
        technical_state: TechnicalPlayabilitySummary {
            phase: TechnicalPlayabilityPhase::Playable,
            media_generation: Some(1_400 + seed),
            reason: None,
            recovery: None,
        },
        participation_role: StartParticipationRole::Required,
        room_ready: true,
        start_eligible: true,
        accepted_operation_id: Some(format!("accepted-{seed}")),
    };
    RoomReadinessSnapshot {
        room_readiness_revision: revision,
        media_generation: Some(1_400 + seed),
        start_gate_phase: RoomStartGatePhase::WaitingForIntent {
            media_generation: 1_400 + seed,
        },
        pause_owner: RoomPauseOwner::ReadinessStartGate {
            media_generation: 1_400 + seed,
        },
        mixed_readiness_policy: Default::default(),
        participants: BTreeMap::from([("alice".to_owned(), participant)]),
    }
}
