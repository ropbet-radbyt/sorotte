use super::*;

pub(crate) fn legacy_syncplay_ui_settings_from_stored_settings(
    settings: Option<&StoredClientSettingsMvp>,
) -> LegacySyncplayUiSettings {
    let resolved = settings
        .map(ClientConfig::resolve)
        .map(|resolution| resolution.config)
        .unwrap_or_default();
    let interface = resolved.interface;
    LegacySyncplayUiSettings {
        show_osd: interface.show_osd,
        chat_output_enabled: interface.chat_output_enabled,
        chat_input_enabled: interface.chat_input_enabled,
        chat_input_font_underline: interface.chat_input_font_underline,
        chat_input_font_family: interface.chat_input_font_family,
        chat_input_relative_font_size: interface.chat_input_relative_font_size,
        chat_input_font_weight: interface.chat_input_font_weight,
        chat_input_font_color: interface.chat_input_font_color,
        chat_input_position: interface.chat_input_position,
        chat_direct_input: interface.chat_direct_input,
        chat_output_font_underline: interface.chat_output_font_underline,
        chat_output_font_family: interface.chat_output_font_family,
        chat_output_relative_font_size: interface.chat_output_relative_font_size,
        chat_output_font_weight: interface.chat_output_font_weight,
        chat_output_mode: interface.chat_output_mode,
        chat_max_lines: interface.chat_max_lines,
        chat_top_margin: interface.chat_top_margin,
        chat_left_margin: interface.chat_left_margin,
        chat_bottom_margin: interface.chat_bottom_margin,
        chat_move_osd: interface.chat_move_osd,
        chat_osd_margin: interface.chat_osd_margin,
        notification_timeout_ms: interface.notification_timeout.as_millis(),
        alert_timeout_ms: interface.alert_timeout.as_millis(),
        chat_timeout_ms: interface.chat_timeout.as_millis(),
    }
}

fn legacy_syncplayintf_script_candidate_paths_legacy_compatible() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    if let Ok(executable_path) = std::env::current_exe()
        && let Some(executable_dir) = executable_path.parent()
    {
        paths.push(executable_dir.join("resources/sorotte_syncplayintf.lua"));
    }
    paths.push(manifest_dir.join("../../resources/sorotte_syncplayintf.lua"));
    paths
}

fn find_legacy_syncplayintf_script_path_legacy_compatible() -> Option<PathBuf> {
    legacy_syncplayintf_script_candidate_paths_legacy_compatible()
        .into_iter()
        .find(|candidate| candidate.is_file())
}

pub(crate) fn apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(
    player: &mut MpvAdapter,
    settings: Option<&StoredClientSettingsMvp>,
) -> anyhow::Result<()> {
    let resolved = legacy_syncplay_ui_settings_from_stored_settings(settings);
    player
        .configure_legacy_syncplay_ui_settings(resolved.clone())
        .map_err(|error| anyhow!("failed to configure mpv OSD/chat settings: {error}"))?;

    if player.is_connected() {
        if !player.legacy_syncplayintf_script_loaded() {
            if resolved.uses_syncplayintf_bridge() {
                let script_path = find_legacy_syncplayintf_script_path_legacy_compatible()
                    .ok_or_else(|| anyhow!("Sorotte's bundled mpv bridge could not be found"))?;
                player
                    .load_legacy_syncplayintf_script(&script_path)
                    .map_err(|error| {
                        anyhow!(
                            "failed to load Sorotte mpv bridge from '{}': {error}",
                            script_path.display()
                        )
                    })?;
            } else if !player
                .discover_loaded_legacy_syncplayintf_script()
                .map_err(|error| anyhow!("failed to discover Sorotte's mpv bridge: {error}"))?
            {
                return Ok(());
            }
        }

        let deadline = Instant::now() + std::time::Duration::from_millis(2_500);
        loop {
            match player.apply_pending_legacy_syncplayintf_options() {
                Ok(()) => break,
                Err(error) if Instant::now() >= deadline => {
                    return Err(anyhow!(
                        "Sorotte's mpv bridge did not acknowledge the settings update: {error}"
                    ));
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        }
    }

    Ok(())
}
