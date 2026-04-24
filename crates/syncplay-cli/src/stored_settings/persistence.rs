use super::*;

#[cfg(test)]
pub(crate) fn parse_syncplay_ini_stored_client_settings_mvp(
    contents: &str,
) -> StoredClientSettingsMvp {
    shared_parse_syncplay_ini_stored_client_settings_mvp(contents)
}

#[cfg(test)]
pub(crate) fn upsert_syncplay_ini_stored_client_settings_mvp(
    existing_contents: &str,
    settings: &StoredClientSettingsMvp,
) -> String {
    shared_upsert_syncplay_ini_stored_client_settings_mvp(existing_contents, settings)
}

pub(crate) fn load_syncplay_cli_stored_settings_mvp_legacy_compatible()
-> anyhow::Result<Option<StoredClientSettingsMvp>> {
    let Some(path) = resolve_syncplay_cli_config_path_legacy_compatible() else {
        return Ok(None);
    };
    shared_load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
}

pub(crate) fn persist_syncplay_cli_stored_settings_mvp_legacy_compatible(
    config: &ClientLoopConfig,
) -> anyhow::Result<()> {
    let Some(path) = resolve_syncplay_cli_config_path_legacy_compatible() else {
        return Ok(());
    };
    let settings = StoredClientSettingsMvp {
        language: None,
        check_for_updates_automatically: None,
        last_checked_for_updates: None,
        host: Some(config.host.clone()),
        port: Some(config.port),
        server_password: None,
        username: Some(config.username.clone()),
        room: Some(config.room.clone()),
        room_list: None,
        player_path: None,
        per_player_arguments: None,
        media_search_directories: None,
        public_servers: None,
        folder_search_first_file_timeout_seconds: None,
        folder_search_timeout_seconds: None,
        folder_search_double_check_interval_seconds: None,
        folder_search_warning_threshold_seconds: None,
        force_gui_prompt: None,
        autoplay_initial_state: Some(config.autoplay_enabled),
        autoplay_require_same_filenames: Some(config.autoplay_require_same_filenames),
        ready_at_start: config.ready_at_start_override,
        shared_playlist_enabled: config.shared_playlists_enabled_override,
        pause_on_leave: config.pause_on_leave_override,
        loop_at_end_of_playlist: config.loop_at_end_of_playlist_override,
        loop_single_files: config.loop_single_files_override,
        only_switch_to_trusted_domains: config.only_switch_to_trusted_domains_override,
        trusted_domains: config.trusted_domains_override.clone(),
        rewind_on_desync: config.rewind_on_desync_override,
        fastforward_on_desync: config.fastforward_on_desync_override,
        slow_on_desync: config.slow_on_desync_override,
        dont_slow_down_with_me: config.dont_slow_down_with_me_override,
        rewind_threshold_seconds: config.rewind_threshold_seconds_override,
        fastforward_threshold_seconds: config.fastforward_threshold_seconds_override,
        slowdown_threshold_seconds: config.slowdown_threshold_seconds_override,
        unpause_action: config.unpause_action_override.clone(),
        autoplay_min_users: config.auto_play_threshold_override.clone(),
        filename_privacy_mode: Some(config.filename_privacy_mode),
        filesize_privacy_mode: Some(config.filesize_privacy_mode),
        show_duration_notification: config.show_duration_notification_override,
        autosave_joins_to_list: None,
        show_osd: None,
        chat_input_enabled: None,
        chat_input_font_underline: None,
        chat_input_font_family: None,
        chat_input_relative_font_size: None,
        chat_input_font_weight: None,
        chat_input_font_color: None,
        chat_input_position: None,
        chat_direct_input: None,
        chat_output_enabled: None,
        chat_output_font_underline: None,
        chat_output_font_family: None,
        chat_output_relative_font_size: None,
        chat_output_font_weight: None,
        chat_output_mode: None,
        chat_move_osd: None,
        chat_max_lines: None,
        chat_top_margin: None,
        chat_left_margin: None,
        chat_bottom_margin: None,
        chat_osd_margin: None,
        notification_timeout_seconds: None,
        alert_timeout_seconds: None,
        chat_timeout_seconds: None,
        show_same_room_osd: config.show_same_room_osd_override,
        show_osd_warnings: config.show_osd_warnings_override,
        show_slowdown_osd: None,
        show_noncontroller_osd: config.show_noncontroller_osd_override,
        show_different_room_osd: config.show_different_room_osd_override,
        show_contact_info: None,
    };
    shared_upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &settings)
}

pub(crate) fn persist_syncplay_cli_language_setting_legacy_compatible(
    language: &str,
) -> anyhow::Result<()> {
    let Some(language) = normalized_legacy_runtime_language_tag_legacy_compatible(language) else {
        return Ok(());
    };
    let Some(path) = resolve_syncplay_cli_config_path_legacy_compatible() else {
        return Ok(());
    };
    shared_update_syncplay_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.language = Some(language.to_owned());
    })
}

pub(crate) fn persist_syncplay_cli_player_path_setting_legacy_compatible(
    player_path: &str,
) -> anyhow::Result<()> {
    let Some(path) = resolve_syncplay_cli_config_path_legacy_compatible() else {
        return Ok(());
    };
    shared_update_syncplay_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.player_path = Some(player_path.to_owned());
    })
}

pub(crate) fn persist_syncplay_cli_per_player_arguments_setting_legacy_compatible(
    player_path: &str,
    player_args: &[String],
) -> anyhow::Result<()> {
    let Some(path) = resolve_syncplay_cli_config_path_legacy_compatible() else {
        return Ok(());
    };
    shared_update_syncplay_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        let mut per_player_arguments = settings.per_player_arguments.take().unwrap_or_default();
        if let Some(normalized_player_path) =
            normalize_player_path_for_stored_per_player_arguments_lookup_legacy_compatible(
                player_path,
            )
        {
            let duplicate_keys = per_player_arguments
                .keys()
                .filter(|stored_player_path| stored_player_path.as_str() != player_path)
                .filter_map(|stored_player_path| {
                    let normalized_stored_path =
                        normalize_player_path_for_stored_per_player_arguments_lookup_legacy_compatible(
                            stored_player_path,
                        )?;
                    (normalized_stored_path == normalized_player_path)
                        .then_some(stored_player_path.clone())
                })
                .collect::<Vec<_>>();
            for duplicate_key in duplicate_keys {
                per_player_arguments.remove(&duplicate_key);
            }
        }
        per_player_arguments.insert(player_path.to_owned(), player_args.to_vec());
        settings.per_player_arguments = Some(per_player_arguments);
    })
}

pub(crate) fn clear_syncplay_cli_stored_settings_legacy_compatible() -> anyhow::Result<bool> {
    let Some(path) = resolve_syncplay_cli_config_path_legacy_compatible() else {
        return Ok(false);
    };
    shared_clear_syncplay_ini_stored_client_settings_mvp_at_path(&path)
}

fn legacy_gui_qsettings_store_names_legacy_compatible() -> [&'static str; 5] {
    [
        "PlayerList",
        "MediaBrowseDialog",
        "MainWindow",
        "Interface",
        "MoreSettings",
    ]
}

fn remove_file_if_exists_legacy_compatible(path: &Path, context: &str) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(anyhow!(
            "{context} path is not a file and cannot be cleared: {}",
            path.display()
        ));
    }
    std::fs::remove_file(path)
        .map_err(|error| anyhow!("failed clearing {context} {}: {error}", path.display()))?;
    Ok(true)
}

fn clear_syncplay_cli_gui_qsettings_filesystem_legacy_compatible(
    root: &Path,
) -> anyhow::Result<bool> {
    let mut changed = false;
    let syncplay_dir = root.join("Syncplay");
    for store in legacy_gui_qsettings_store_names_legacy_compatible() {
        for extension in [".conf", ".ini"] {
            let candidate = syncplay_dir.join(format!("{store}{extension}"));
            changed |= remove_file_if_exists_legacy_compatible(&candidate, "legacy GUI QSettings")?;
        }
    }
    Ok(changed)
}

fn clear_windows_registry_key_tree_legacy_compatible(key: &str) -> anyhow::Result<bool> {
    let query_status = Command::new("reg")
        .args(["query", key])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| anyhow!("failed querying registry key {key}: {error}"))?;
    if !query_status.success() {
        return Ok(false);
    }

    let output = Command::new("reg")
        .args(["delete", key, "/f"])
        .output()
        .map_err(|error| anyhow!("failed deleting registry key {key}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(anyhow!("failed deleting registry key {key}"));
        }
        return Err(anyhow!("failed deleting registry key {key}: {stderr}"));
    }
    Ok(true)
}

fn clear_syncplay_cli_gui_qsettings_windows_registry_legacy_compatible() -> anyhow::Result<bool> {
    let mut changed = false;
    for store in legacy_gui_qsettings_store_names_legacy_compatible() {
        let key = format!(r"HKEY_CURRENT_USER\Software\Syncplay\{store}");
        changed |= clear_windows_registry_key_tree_legacy_compatible(&key)?;
    }
    Ok(changed)
}

fn clear_syncplay_cli_gui_qsettings_macos_plists_legacy_compatible(
    preferences_dir: &Path,
) -> anyhow::Result<bool> {
    let mut changed = false;
    for store in legacy_gui_qsettings_store_names_legacy_compatible() {
        for candidate_name in [
            format!("com.Syncplay.{store}.plist"),
            format!("org.Syncplay.{store}.plist"),
            format!("Syncplay.{store}.plist"),
        ] {
            let candidate = preferences_dir.join(candidate_name);
            changed |= remove_file_if_exists_legacy_compatible(&candidate, "legacy GUI QSettings")?;
        }
    }
    Ok(changed)
}

pub(crate) fn clear_syncplay_cli_gui_qsettings_legacy_compatible() -> anyhow::Result<bool> {
    if let Some(root) = syncplay_cli_legacy_gui_qsettings_root_override() {
        return clear_syncplay_cli_gui_qsettings_filesystem_legacy_compatible(&root);
    }

    if cfg!(windows) {
        let mut changed = clear_syncplay_cli_gui_qsettings_windows_registry_legacy_compatible()?;
        if let Some(root) = default_syncplay_cli_config_root_legacy_compatible() {
            changed |= clear_syncplay_cli_gui_qsettings_filesystem_legacy_compatible(&root)?;
        }
        return Ok(changed);
    }

    if cfg!(target_os = "macos") {
        let mut changed = false;
        if let Some(home) = env_trimmed("HOME") {
            let preferences_dir = PathBuf::from(home).join("Library").join("Preferences");
            changed |=
                clear_syncplay_cli_gui_qsettings_macos_plists_legacy_compatible(&preferences_dir)?;
        }
        if let Some(root) = default_syncplay_cli_config_root_legacy_compatible() {
            changed |= clear_syncplay_cli_gui_qsettings_filesystem_legacy_compatible(&root)?;
        }
        return Ok(changed);
    }

    let Some(root) = default_syncplay_cli_config_root_legacy_compatible() else {
        return Ok(false);
    };
    clear_syncplay_cli_gui_qsettings_filesystem_legacy_compatible(&root)
}
