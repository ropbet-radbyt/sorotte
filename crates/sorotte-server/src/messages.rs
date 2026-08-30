use super::*;

pub(crate) fn user_joined_message_with_metadata(
    username: &str,
    room_name: &str,
    version: &str,
    features: Option<Value>,
) -> ProtocolMessage {
    let event = json!({
        "joined": true,
        "version": version,
        "features": features.unwrap_or(Value::Null),
    });
    user_event_message(username, room_name, event)
}

pub(crate) fn user_room_update_message(username: &str, room_name: &str) -> ProtocolMessage {
    user_setting_message(username, room_name, None)
}

pub(crate) fn user_file_update_message(
    username: &str,
    room_name: &str,
    file: Value,
) -> ProtocolMessage {
    let mut users = BTreeMap::new();
    users.insert(
        username.to_owned(),
        UserSetPayload::new()
            .with_room(RoomRef::new(room_name))
            .with_file(file),
    );
    ProtocolMessage::set(SetPayload::new().with_user(users))
}

pub(crate) fn user_features_update_message(username: &str, features: Value) -> ProtocolMessage {
    ProtocolMessage::set(SetPayload::new().with_features(json!({
        "username": username,
        "features": features,
    })))
}

pub(crate) fn user_event_message(username: &str, room_name: &str, event: Value) -> ProtocolMessage {
    user_setting_message(username, room_name, Some(event))
}

pub(crate) fn user_setting_message(
    username: &str,
    room_name: &str,
    event: Option<Value>,
) -> ProtocolMessage {
    let mut user_setting = UserSetPayload::new().with_room(RoomRef::new(room_name));
    if let Some(event) = event {
        user_setting = user_setting.with_event(event);
    }
    let mut users = BTreeMap::new();
    users.insert(username.to_owned(), user_setting);
    ProtocolMessage::set(SetPayload::new().with_user(users))
}

pub(crate) fn ready_update_message(
    username: &str,
    is_ready: impl Into<Option<bool>>,
    manually_initiated: bool,
    set_by_username: Option<&str>,
) -> ProtocolMessage {
    let mut payload = ReadyPayload::new(is_ready)
        .with_manually_initiated(manually_initiated)
        .with_username(username);
    if let Some(set_by) = set_by_username {
        payload = payload.with_set_by(set_by);
    }
    ProtocolMessage::set(SetPayload::new().with_ready(payload))
}

pub(crate) fn playback_barrier_set_message(
    extension: PlaybackBarrierSetExtension,
) -> ProtocolMessage {
    ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(extension))
}

pub(crate) fn readiness_set_message(extension: ReadinessSetExtension) -> ProtocolMessage {
    ProtocolMessage::set(SetPayload::new().with_readiness_v2(extension))
}

pub(crate) fn readiness_legacy_chat_message(
    set_by_username: &str,
    username: &str,
    is_ready: bool,
) -> ProtocolMessage {
    let readiness = if is_ready { "ready" } else { "not ready" };
    ProtocolMessage::chat_message(
        set_by_username,
        format!("I have set {username} as {readiness}."),
    )
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StateSyncOptions<'a> {
    pub(crate) set_by: Option<&'a str>,
    pub(crate) server_ignoring_counter: Option<u32>,
    pub(crate) client_latency_calculation: Option<f64>,
    pub(crate) client_ignoring_counter: Option<u32>,
    pub(crate) server_rtt_seconds: f64,
    pub(crate) latency_calculation_seconds: Option<f64>,
    pub(crate) participant_status: Option<ParticipantStatusStateExtension>,
}

pub(crate) fn state_sync_message(
    position: f64,
    paused: bool,
    do_seek: impl Into<Option<bool>>,
    options: StateSyncOptions<'_>,
) -> ProtocolMessage {
    let mut playstate = PlaystatePayload::new()
        .with_position(position)
        .with_paused(paused);
    if let Some(do_seek) = do_seek.into() {
        playstate = playstate.with_do_seek(do_seek);
    } else {
        playstate.extra.insert("doSeek".to_owned(), Value::Null);
    }
    if let Some(set_by) = options.set_by {
        playstate = playstate.with_set_by(set_by);
    }

    let mut ping = PingPayload::new()
        .with_latency_calculation(
            options
                .latency_calculation_seconds
                .unwrap_or_else(current_unix_timestamp_seconds),
        )
        .with_server_rtt(options.server_rtt_seconds);
    if let Some(client_latency_calculation) = options.client_latency_calculation {
        ping = ping.with_client_latency_calculation(client_latency_calculation);
    }
    let mut state = StatePayload::new()
        .with_playstate(playstate)
        .with_ping(ping);
    if options.server_ignoring_counter.is_some() || options.client_ignoring_counter.is_some() {
        let mut ignoring = IgnoringOnTheFlyPayload::new();
        if let Some(server_ignoring_counter) = options.server_ignoring_counter {
            ignoring = ignoring.with_server(server_ignoring_counter);
        }
        if let Some(client_ignoring_counter) = options.client_ignoring_counter {
            ignoring = ignoring.with_client(client_ignoring_counter);
        }
        state = state.with_ignoring_on_the_fly(ignoring);
    }
    if let Some(participant_status) = options.participant_status {
        state = state.with_participant_status_v1(participant_status);
    }
    ProtocolMessage::state(state)
}

pub(crate) fn current_unix_timestamp_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub(crate) fn controller_auth_status_message(
    username: &str,
    room_name: &str,
    success: bool,
) -> ProtocolMessage {
    let auth_status = ControllerAuthPayload::new()
        .with_user(username)
        .with_room(room_name)
        .with_success(success);
    ProtocolMessage::set(SetPayload::new().with_controller_auth(auth_status))
}

pub(crate) fn new_controlled_room_message(room_name: &str, password: &str) -> ProtocolMessage {
    let payload = NewControlledRoomPayload::new()
        .with_room_name(room_name)
        .with_password(password);
    ProtocolMessage::set(SetPayload::new().with_new_controlled_room(payload))
}

#[allow(dead_code)]
pub(crate) fn playlist_snapshot_change_message(
    files: Vec<String>,
    set_by: Option<&str>,
    epoch: u64,
) -> ProtocolMessage {
    let mut playlist_change =
        playlist_change_with_plex_sidecar(files, false).with_playlist_epoch(epoch);
    playlist_change = if let Some(set_by) = set_by {
        playlist_change.with_user(set_by)
    } else {
        playlist_change.with_null_user()
    };
    ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change))
}

pub(crate) fn playlist_snapshot_index_message(
    index: Option<i64>,
    set_by: Option<&str>,
    epoch: u64,
) -> ProtocolMessage {
    let mut playlist_index = PlaylistIndexPayload::from_optional(index).with_playlist_epoch(epoch);
    playlist_index = if let Some(set_by) = set_by {
        playlist_index.with_user(set_by)
    } else {
        playlist_index.with_null_user()
    };
    ProtocolMessage::set(SetPayload::new().with_playlist_index(playlist_index))
}

#[cfg(test)]
pub(crate) fn controlled_room_name_for(room_name: &str, password: &str) -> String {
    RoomPasswordProvider::default().controlled_room_name_for(room_name, password)
}

pub(crate) fn server_feature_list(
    persistent_rooms_enabled: bool,
    isolate_rooms: bool,
    chat_enabled: bool,
    readiness_enabled: bool,
    max_chat_message_length: usize,
    max_username_length: usize,
) -> Value {
    json!({
        "isolateRooms": isolate_rooms,
        "readiness": readiness_enabled,
        "managedRooms": true,
        "persistentRooms": persistent_rooms_enabled,
        "chat": chat_enabled,
        "maxFilenameLength": DEFAULT_MAX_FILENAME_LENGTH,
        "maxRoomNameLength": DEFAULT_MAX_ROOM_NAME_LENGTH,
        "maxChatMessageLength": max_chat_message_length,
        "maxUsernameLength": max_username_length,
        "featureList": true,
        "sharedPlaylists": true,
        "mediaMatch": true,
        SOROTTE_PLEX_PLAYLIST_URIS_FEATURE: true,
        "setOthersReadiness": readiness_enabled,
        SOROTTE_PLAYBACK_BARRIER_V1: true,
        SOROTTE_READINESS_V2: readiness_enabled,
        SOROTTE_PARTICIPANT_STATUS_V1: true,
        "uiMode": LEGACY_UI_MODE_UNKNOWN,
    })
}
