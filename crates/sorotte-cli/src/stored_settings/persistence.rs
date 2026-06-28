use super::*;

#[cfg(test)]
pub(crate) fn parse_sorotte_ini_stored_client_settings_mvp(
    contents: &str,
) -> StoredClientSettingsMvp {
    shared_parse_sorotte_ini_stored_client_settings_mvp(contents)
}

#[cfg(test)]
pub(crate) fn upsert_sorotte_ini_stored_client_settings_mvp(
    existing_contents: &str,
    settings: &StoredClientSettingsMvp,
) -> String {
    shared_upsert_sorotte_ini_stored_client_settings_mvp(existing_contents, settings)
}

pub(crate) fn load_sorotte_cli_stored_settings_mvp_legacy_compatible()
-> anyhow::Result<Option<StoredClientSettingsMvp>> {
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(None);
    };
    shared_load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
}

pub(crate) fn persist_sorotte_cli_stored_settings_mvp_legacy_compatible(
    config: &ClientLoopConfig,
) -> anyhow::Result<()> {
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(());
    };
    let settings = StoredClientSettingsMvp {
        language: None,
        check_for_updates_automatically: None,
        update_channel: None,
        last_checked_for_updates: None,
        stream_support_plugin_enabled: None,
        media_matching_plugin_enabled: None,
        plex_plugin_enabled: None,
        host: Some(config.host.clone()),
        port: Some(config.port),
        server_password: None,
        username: Some(config.username.clone()),
        room: Some(config.room.clone()),
        room_list: None,
        player_path: None,
        per_player_arguments: None,
        media_search_directories: None,
        media_match_fingerprinting_enabled: None,
        media_match_background_warmup_enabled: None,
        media_match_wire_sharing_enabled: None,
        media_match_runtime_tolerance_enabled: None,
        media_match_autoplay_policy: None,
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
        plex_sync_enabled: None,
        plex_streaming_enabled: None,
        plex_user_token: None,
        plex_selected_server_id: None,
        plex_selected_server_url: None,
        plex_selected_server_token: None,
    };
    shared_upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &settings)
}

pub(crate) fn persist_sorotte_cli_language_setting_legacy_compatible(
    language: &str,
) -> anyhow::Result<()> {
    let Some(language) = normalized_legacy_runtime_language_tag_legacy_compatible(language) else {
        return Ok(());
    };
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(());
    };
    shared_update_sorotte_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.language = Some(language.to_owned());
    })
}

pub(crate) fn persist_sorotte_cli_player_path_setting_legacy_compatible(
    player_path: &str,
) -> anyhow::Result<()> {
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(());
    };
    shared_update_sorotte_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.player_path = Some(player_path.to_owned());
    })
}

pub(crate) fn persist_sorotte_cli_per_player_arguments_setting_legacy_compatible(
    player_path: &str,
    player_args: &[String],
) -> anyhow::Result<()> {
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(());
    };
    shared_update_sorotte_ini_stored_client_settings_mvp_at_path(&path, |settings| {
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

pub(crate) fn clear_sorotte_cli_stored_settings_legacy_compatible() -> anyhow::Result<bool> {
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(false);
    };
    shared_clear_sorotte_ini_stored_client_settings_mvp_at_path(&path)
}

fn sorotte_gui_state_store_names() -> [&'static str; 5] {
    [
        "PlayerList",
        "MediaBrowseDialog",
        "MainWindow",
        "Interface",
        "MoreSettings",
    ]
}

fn remove_file_if_exists(path: &Path, context: &str) -> anyhow::Result<bool> {
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

fn clear_sorotte_cli_gui_state_filesystem(root: &Path) -> anyhow::Result<bool> {
    let mut changed = false;
    for store in sorotte_gui_state_store_names() {
        let candidate = root.join(format!("{store}.ini"));
        changed |= remove_file_if_exists(&candidate, "Sorotte GUI state")?;
    }
    Ok(changed)
}

pub(crate) fn clear_sorotte_cli_gui_state() -> anyhow::Result<bool> {
    if let Some(root) = sorotte_cli_gui_state_root_override() {
        return clear_sorotte_cli_gui_state_filesystem(&root);
    }

    let Some(root) = resolve_sorotte_cli_storage_root() else {
        return Ok(false);
    };
    clear_sorotte_cli_gui_state_filesystem(&root)
}
