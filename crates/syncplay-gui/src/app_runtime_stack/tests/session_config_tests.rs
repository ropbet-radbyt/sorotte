use super::*;

use syncplay_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsMvp,
    stored_client_settings_runtime_snapshot_legacy_compatible,
};
use syncplay_client_core::{
    DesyncCorrectionConfig, ReadinessAutoplayConfig, SYNCPLAY_COMPAT_VERSION_LEGACY,
    SYNCPLAY_WIRE_VERSION_LEGACY, SessionBehaviorConfig, UnpauseActionMode,
};
use syncplay_protocol::{ProtocolMessage, decode_message_line};

#[test]
fn gui_client_core_chat_session_runtime_adapter_syncs_runtime_settings_into_session_and_reconnects_with_them()
 {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            autoplay_initial_state: Some(true),
            autoplay_require_same_filenames: Some(true),
            pause_on_leave: Some(false),
            loop_at_end_of_playlist: Some(true),
            loop_single_files: Some(true),
            only_switch_to_trusted_domains: Some(false),
            trusted_domains: Some(vec!["*.example.com/videos".to_owned()]),
            rewind_on_desync: Some(false),
            fastforward_on_desync: Some(false),
            slow_on_desync: Some(false),
            dont_slow_down_with_me: Some(true),
            rewind_threshold_seconds: Some(1.5),
            fastforward_threshold_seconds: Some(4.0),
            slowdown_threshold_seconds: Some(0.75),
            unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
            show_duration_notification: Some(false),
            show_same_room_osd: Some(false),
            show_osd_warnings: Some(false),
            show_noncontroller_osd: Some(true),
            show_different_room_osd: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
        "alice",
        "room1",
        Some("ab-123-456".to_owned()),
    )
    .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the session");

    assert!(adapter.dont_slow_down_with_me);
    assert!(adapter.runtime.session().autoplay_enabled());
    assert!(!adapter.runtime.session().behavior_config().pause_on_leave);
    assert!(
        adapter
            .runtime
            .session()
            .behavior_config()
            .loop_at_end_of_playlist
    );
    assert!(
        adapter
            .runtime
            .session()
            .behavior_config()
            .loop_single_files
    );
    assert!(
        !adapter
            .runtime
            .session()
            .behavior_config()
            .only_switch_to_trusted_domains
    );
    assert_eq!(
        adapter.runtime.session().behavior_config().trusted_domains,
        vec!["*.example.com/videos".to_owned()]
    );
    assert!(!adapter.runtime.session().desync_config().rewind_on_desync);
    assert!(
        !adapter
            .runtime
            .session()
            .desync_config()
            .fastforward_on_desync
    );
    assert!(!adapter.runtime.session().desync_config().slow_on_desync);
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .rewind_threshold_seconds,
        1.5
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .fastforward_threshold_seconds,
        4.0
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .slowdown_threshold_seconds,
        0.75
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .unpause_action,
        UnpauseActionMode::IfMinUsersReady
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .auto_play_threshold,
        Some(3)
    );
    assert!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .autoplay_require_same_filenames
    );
    assert!(
        !adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .show_duration_notification
    );

    GuiSessionRuntimeAdapter::connect_public_server(
        &mut adapter,
        Some(("Primary".to_owned(), "syncplay.pl:8999".to_owned())),
    )
    .expect("connect request should reset the runtime for reconnect");

    assert!(adapter.dont_slow_down_with_me);
    assert!(adapter.runtime.session().autoplay_enabled());
    assert!(
        adapter
            .runtime
            .session()
            .behavior_config()
            .loop_single_files
    );
    assert!(!adapter.runtime.session().desync_config().rewind_on_desync);
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .unpause_action,
        UnpauseActionMode::IfMinUsersReady
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .auto_play_threshold,
        Some(3)
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_cached_username_when_runtime_settings_blank()
{
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            room: Some("room1".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the startup hello");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&startup_lines[0]).expect("startup hello should decode")
    else {
        panic!("startup protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.username, "");
    assert_eq!(hello.hello.room.name, "room1");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_updates_dont_slow_down_with_me_without_reconnect() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let disabled_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            dont_slow_down_with_me: Some(false),
            ..StoredClientSettingsMvp::default()
        });
    let enabled_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            dont_slow_down_with_me: Some(true),
            ..StoredClientSettingsMvp::default()
        });

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &disabled_settings)
        .expect("initial runtime settings should sync");
    assert!(
        !adapter.dont_slow_down_with_me,
        "initial sync should keep dontSlowDownWithMe disabled"
    );

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &enabled_settings)
        .expect("steady-state runtime sync should update dontSlowDownWithMe");
    assert!(
        adapter.dont_slow_down_with_me,
        "steady-state sync should update dontSlowDownWithMe without requiring reconnect"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_startup_hello_includes_password_and_full_features()
{
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("bob".to_owned()),
            room: Some("room2".to_owned()),
            server_password: Some("secret-pass".to_owned()),
            shared_playlist_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the startup hello");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&startup_lines[0]).expect("startup hello should decode")
    else {
        panic!("startup protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.username, "bob");
    assert_eq!(hello.hello.room.name, "room2");
    assert_eq!(hello.hello.version, SYNCPLAY_WIRE_VERSION_LEGACY);
    assert_eq!(
        hello.hello.realversion.as_deref(),
        Some(SYNCPLAY_COMPAT_VERSION_LEGACY)
    );
    assert_eq!(
        hello
            .hello
            .extra
            .get("password")
            .and_then(serde_json::Value::as_str),
        Some("secret-pass")
    );

    let features = hello
        .hello
        .features
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .expect("startup hello should advertise a feature map");
    assert_eq!(
        features
            .get("sharedPlaylists")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        features.get("chat").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features.get("uiMode").and_then(serde_json::Value::as_str),
        Some("GUI")
    );
    assert_eq!(
        features
            .get("featureList")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("readiness")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("managedRooms")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("persistentRooms")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("setOthersReadiness")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_uses_updated_runtime_identity() {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("bob".to_owned()),
            room: Some("room2".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the reconnect hello");
    adapter.reset_session_for_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&reconnect_lines[0]).expect("reconnect hello should decode")
    else {
        panic!("reconnect protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.username, "bob");
    assert_eq!(hello.hello.room.name, "room2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_preserves_current_room_over_local_file_name()
 {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room2"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
        )
        .expect("local user update should apply");

    adapter.reset_session_for_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&reconnect_lines[0]).expect("reconnect hello should decode")
    else {
        panic!("reconnect protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.room.name, "room2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_preserves_pending_room_switch() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    GuiSessionRuntimeAdapter::set_room(&mut adapter, "room2".to_owned())
        .expect("room change should queue");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("queued room change should encode");
    let set_message = outbound_lines
        .iter()
        .find_map(|line| {
            match decode_message_line(line)
                .expect("queued room change protocol lines should decode")
            {
                ProtocolMessage::Set(set_message) => Some(set_message),
                _ => None,
            }
        })
        .expect("queued room change protocol lines should include a Set message");
    assert_eq!(
        set_message.set.room.as_ref().map(|room| room.name.as_str()),
        Some("room2")
    );

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    let hello = reconnect_lines
        .iter()
        .find_map(|line| {
            match decode_message_line(line).expect("reconnect protocol lines should decode") {
                ProtocolMessage::Hello(hello) => Some(hello),
                _ => None,
            }
        })
        .expect("reconnect protocol lines should include a Hello message");
    assert_eq!(hello.hello.room.name, "room2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_follows_server_authoritative_room_after_mismatched_room_response()
 {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    GuiSessionRuntimeAdapter::set_room(&mut adapter, "room2".to_owned())
        .expect("room change should queue");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("queued room change should encode");

    adapter
        .apply_message_json(r#"{"Set":{"room":{"name":"room3"}}}"#)
        .expect("authoritative room response should apply");

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    let hello = reconnect_lines
        .iter()
        .find_map(|line| {
            match decode_message_line(line).expect("reconnect protocol lines should decode") {
                ProtocolMessage::Hello(hello) => Some(hello),
                _ => None,
            }
        })
        .expect("reconnect protocol lines should include a Hello message");
    assert_eq!(
        hello.hello.room.name, "room3",
        "future reconnects should follow the server-authoritative room once a room response arrives"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_preserves_ready_at_start_across_reconnect_before_first_hello()
 {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            ready_at_start: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the session");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("reconnect hello should apply");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("ready-at-start lines should encode after reconnect hello");
    assert!(
        outbound_lines
            .iter()
            .any(|line| line.contains(r#""Set":{"ready":{"isReady":true"#)),
        "pre-Hello reconnects should preserve the ready-at-start dispatch after the eventual server hello"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_preserves_whitespace_room_names() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"   "},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&reconnect_lines[0]).expect("reconnect hello should decode")
    else {
        panic!("reconnect protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.room.name, "   ");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_text_backed_runtime_settings_to_defaults() {
    let configured_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            trusted_domains: Some(vec!["*.example.com/videos".to_owned()]),
            rewind_threshold_seconds: Some(1.5),
            fastforward_threshold_seconds: Some(4.0),
            slowdown_threshold_seconds: Some(0.75),
            unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
            show_duration_notification: Some(false),
            ..StoredClientSettingsMvp::default()
        });
    let cleared_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &configured_settings)
        .expect("configured runtime settings should sync");
    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &cleared_settings)
        .expect("cleared runtime settings should sync");

    assert_eq!(
        adapter.runtime.session().behavior_config().trusted_domains,
        SessionBehaviorConfig::default().trusted_domains
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .rewind_threshold_seconds,
        DesyncCorrectionConfig::default().rewind_threshold_seconds
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .fastforward_threshold_seconds,
        DesyncCorrectionConfig::default().fastforward_threshold_seconds
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .slowdown_threshold_seconds,
        DesyncCorrectionConfig::default().slowdown_threshold_seconds
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .unpause_action,
        ReadinessAutoplayConfig::default().unpause_action
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .auto_play_threshold,
        ReadinessAutoplayConfig::default().auto_play_threshold
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .show_duration_notification,
        ReadinessAutoplayConfig::default().show_duration_notification
    );
}
