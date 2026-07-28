use super::{
    GuiAttachedPlayerRuntimeAction, GuiClientCoreChatSessionRuntimeAdapter,
    GuiSessionRuntimeAdapter,
};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiInteractionRuntimeSnapshot, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot, MenuActionId,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, SorotteGuiShellAppState,
};
use sorotte_client_app::app_boundary::state::{
    StoredClientSettingsMvp, stored_client_settings_runtime_snapshot_legacy_compatible,
};
use sorotte_client_core::{
    ConnectionPhase, CoordinatorPlayerCommand, LogicalMediaId, MediaLoadIntent, MediaTransportKind,
    ReconnectTransitionNotification,
};
use sorotte_player_api::{
    PlayerMediaGeneration, PlayerObservationTimestamp, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};

fn sync_adapter_to_saved_session_settings(
    adapter: &mut GuiClientCoreChatSessionRuntimeAdapter,
    state: &SorotteGuiShellAppState,
) {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&state.saved_configuration);
    GuiSessionRuntimeAdapter::sync_runtime_settings(adapter, &runtime_settings)
        .expect("saved settings should initialize the active test session");
}

mod chat_projection_tests;
mod controller_autoplay_tests;
mod playback_barrier_integration_tests;
mod playlist_tests;
mod public_server_tests;
mod session_config_tests;
mod session_transition_tests;

#[cfg(windows)]
#[test]
fn missing_media_index_does_not_follow_descendant_junctions() {
    use std::sync::atomic::AtomicBool;

    let fixture = crate::app::testing::support::test_temp_root("media-index-descendant-junction");
    let configured_root = fixture.join("configured-root");
    let outside_root = fixture.join("outside-root");
    std::fs::create_dir_all(&configured_root).expect("configured media root should be created");
    std::fs::create_dir_all(&outside_root).expect("outside directory should be created");
    std::fs::write(outside_root.join("must-not-be-indexed.mkv"), b"outside")
        .expect("outside canary should be written");

    let junction = configured_root.join("outside-link");
    let status = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside_root)
        .status()
        .expect("junction creation command should launch");
    assert!(status.success(), "junction creation should succeed");

    let cancel = AtomicBool::new(false);
    let mut progress = |_: usize, _: usize| {};
    let index =
        GuiClientCoreChatSessionRuntimeAdapter::build_missing_media_file_name_index_for_path_with_progress_and_workers(
            &configured_root,
            None,
            &cancel,
            1,
            &mut progress,
        )
        .expect("media index build should complete");

    std::fs::remove_dir(&junction).expect("junction should be removed without following it");
    std::fs::remove_dir_all(&fixture).expect("fixture should be removed");

    assert!(
        !index.contains_key("must-not-be-indexed.mkv"),
        "descendant junction targets must not be traversed by media indexing: {index:?}"
    );
}

#[cfg(windows)]
#[test]
fn missing_media_index_accepts_an_explicit_root_junction() {
    use std::sync::atomic::AtomicBool;

    let fixture =
        crate::app::testing::support::test_temp_root("media-index-explicit-root-junction");
    let target_root = fixture.join("target-root");
    std::fs::create_dir_all(&target_root).expect("target media root should be created");
    std::fs::write(target_root.join("inside.mkv"), b"inside")
        .expect("inside media file should be written");

    let configured_root = fixture.join("configured-root");
    let status = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&configured_root)
        .arg(&target_root)
        .status()
        .expect("junction creation command should launch");
    assert!(status.success(), "junction creation should succeed");

    let cancel = AtomicBool::new(false);
    let mut progress = |_: usize, _: usize| {};
    let index =
        GuiClientCoreChatSessionRuntimeAdapter::build_missing_media_file_name_index_for_path_with_progress_and_workers(
            &configured_root,
            None,
            &cancel,
            1,
            &mut progress,
        )
        .expect("explicit root junction should be indexed");

    std::fs::remove_dir(&configured_root).expect("junction should be removed without following it");
    std::fs::remove_dir_all(&fixture).expect("fixture should be removed");

    assert_eq!(
        index.get("inside.mkv"),
        Some(&vec!["inside.mkv".to_owned()]),
        "an explicitly configured junction root should remain supported"
    );
}
