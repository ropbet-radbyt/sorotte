use super::*;

use syncplay_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsMvp,
    stored_client_settings_runtime_snapshot_legacy_compatible,
};
use syncplay_client_core::UnpauseActionMode;

#[test]
fn gui_client_core_chat_session_runtime_adapter_syncs_runtime_settings_into_session_and_reconnects_with_them()
 {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
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
