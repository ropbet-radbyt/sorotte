use super::*;
use crate::app::runtime_owner::{
    GuiPlexOperationContext, GuiPlexStreamResolveFailure, GuiPlexStreamResolveFailureDisposition,
    GuiPlexStreamResolveOutcome, GuiPlexStreamResolveWorkerResult,
};
use crate::app::shell_state::GuiPlaylistSourceStatus;

const PERMANENT_MISS_INVARIANT: &str =
    "TC-GUI-003: permanent Plex ambiguity must warn once without automatic retry";

fn ambiguous_part_error() -> GuiPlexStreamResolveFailure {
    let error = sorotte_plex::PlexError::InvalidResponse(
        "ambiguous playable parts for Plex metadata 599092; candidates=2".to_owned(),
    );
    assert!(error.is_ambiguous_playable_parts());
    GuiPlexStreamResolveFailure::from_plex_error("episode.mkv", error)
}

fn automatic_plex_state() -> SorotteGuiShellAppState {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    state
}

fn queue_automatic_plex_attempt(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    state: &SorotteGuiShellAppState,
) -> (String, GuiPlexOperationContext) {
    let (_watch_sync_tx, watch_sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(watch_sync_rx);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .take()
        .expect("automatic fallback should retain its Plex worker trigger");
    let operation_context = owner
        .plex_stream_resolve_context
        .take()
        .expect("automatic fallback should retain its Plex operation context");
    owner.plex_sync_rx = None;
    (trigger_key, operation_context)
}

fn finish_automatic_plex_ambiguity(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    state: &SorotteGuiShellAppState,
    trigger_key: String,
    operation_context: GuiPlexOperationContext,
) {
    owner.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
        operation_context,
        trigger_key,
        result: Ok(GuiPlexStreamResolveOutcome {
            stream_target: Err(ambiguous_part_error()),
            cache: sorotte_plex::PlexMatchCache::default(),
        }),
        staged_cache_write: None,
    });
    owner.last_attached_media_resolution_trigger = None;
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
}

fn warning_and_chat_messages(owner: &GuiPersistedConfigRuntimeOwner) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut chats = Vec::new();
    for batch in &owner.pending_stream_feedback {
        for action in batch {
            match action {
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message,
                } => warnings.push(message.clone()),
                GuiShellAction::AnnounceSystemChatEvent(message) => chats.push(message.clone()),
                _ => {}
            }
        }
    }
    (warnings, chats)
}

#[test]
fn permanent_plex_ambiguity_warns_once_without_automatic_retry() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let state = automatic_plex_state();

    let (first_trigger, first_context) = queue_automatic_plex_attempt(&mut owner, &state);
    finish_automatic_plex_ambiguity(&mut owner, &state, first_trigger, first_context);

    let miss = owner
        .plex_miss_state
        .as_ref()
        .expect("permanent ambiguity should retain terminal context");
    assert_eq!(
        miss.disposition,
        GuiPlexStreamResolveFailureDisposition::PermanentForContext
    );
    assert!(miss.next_retry_at.is_none());
    assert!(!miss.retry_in_flight);
    let (_, projected_source) = owner
        .playlist_resolution_source_state_for_projection(&state)
        .expect("terminal ambiguity should project source status");
    assert_eq!(projected_source.status, GuiPlaylistSourceStatus::Failed);
    assert!(
        projected_source
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("multiple indistinguishable playable parts"))
    );

    if let Some(miss) = owner.plex_miss_state.as_mut() {
        miss.next_retry_at = Some(std::time::Instant::now());
    }
    if owner.active_plex_miss_retry_due(&state) {
        owner.last_attached_media_resolution_trigger = None;
        let (second_trigger, second_context) = queue_automatic_plex_attempt(&mut owner, &state);
        finish_automatic_plex_ambiguity(&mut owner, &state, second_trigger, second_context);
    }

    let attempt_count = owner
        .plex_miss_state
        .as_ref()
        .map_or(0, |miss| miss.attempt_count);
    let (warning_messages, chat_messages) = warning_and_chat_messages(&owner);
    let warnings_repeat_identically =
        warning_messages.len() > 1 && warning_messages.windows(2).all(|pair| pair[0] == pair[1]);
    let chats_repeat_identically =
        chat_messages.len() > 1 && chat_messages.windows(2).all(|pair| pair[0] == pair[1]);

    assert!(
        attempt_count <= 1 && warning_messages.len() == 1 && chat_messages.len() == 1,
        "{PERMANENT_MISS_INVARIANT}: attempts={attempt_count}, \
warnings={}, chats={}, warnings_repeat_identically={warnings_repeat_identically}, \
chats_repeat_identically={chats_repeat_identically}, warning_messages={warning_messages:?}",
        warning_messages.len(),
        chat_messages.len(),
    );
}
