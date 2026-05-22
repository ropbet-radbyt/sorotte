use super::*;
use std::path::PathBuf;

#[cfg(windows)]
#[test]
#[ignore = "local smoke test; requires standalone mpv binary"]
fn gui_persisted_config_runtime_owner_starts_real_managed_mpv_from_saved_config() {
    let default_mpv = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mpv/mpv.exe");
    let mpv_path = std::env::var_os("SOROTTE_MPV_SMOKE_BIN")
        .map(PathBuf::from)
        .unwrap_or(default_mpv);
    if !mpv_path.is_file() {
        panic!("expected mpv binary at {}", mpv_path.display());
    }

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let config_path =
        std::env::temp_dir().join(format!("sorotte-gui-real-mpv-startup-{unique_suffix}.ini"));
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            player_path: Some(mpv_path.to_string_lossy().into_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("real-mpv startup seed should write sorotte.ini");

    let (mut owner, _session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
            Some(config_path.clone()),
            &|_name| None,
        )
        .with_client_core_chat_session_runtime("smoke-user", "smoke-room")
        .expect("managed-mpv smoke should bootstrap an active session");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("smoke-user".to_owned()),
        room: Some("smoke-room".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("mpv")
    );
    assert!(owner.managed_mpv_process.is_some());
    assert_eq!(owner.player_unavailability_reason, None);

    drop(owner);
    let _ = std::fs::remove_file(config_path);
}
