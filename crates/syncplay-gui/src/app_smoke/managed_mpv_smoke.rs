use super::*;

#[cfg(windows)]
#[test]
#[ignore = "local smoke test; requires standalone mpv binary"]
fn gui_persisted_config_runtime_owner_starts_real_managed_mpv_from_saved_config() {
    let default_mpv = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mpv/mpv.exe");
    let mpv_path = std::env::var_os("SYNCPLAY_MPV_SMOKE_BIN")
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
        std::env::temp_dir().join(format!("syncplay-gui-real-mpv-startup-{unique_suffix}.ini"));
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            player_path: Some(mpv_path.to_string_lossy().into_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("real-mpv startup seed should write syncplay.ini");

    let owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(config_path.clone()),
        &|_name| None,
    );
    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("mpv")
    );
    assert!(owner.managed_mpv_process.is_some());
    assert_eq!(owner.player_unavailability_reason, None);

    drop(owner);
    let _ = std::fs::remove_file(config_path);
}
