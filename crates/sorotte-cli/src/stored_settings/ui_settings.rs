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

pub(crate) fn apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(
    player: &mut MpvAdapter,
    settings: Option<&StoredClientSettingsMvp>,
) -> SorotteBridgeHealth {
    let resolved = legacy_syncplay_ui_settings_from_stored_settings(settings);
    if let Err(error) = player.configure_legacy_syncplay_ui_settings(resolved) {
        return player.mark_sorotte_bridge_degraded(
            SorotteBridgeFailureKind::IpcCommand,
            format!("failed to configure mpv OSD/chat settings: {error}"),
        );
    }

    if player.is_connected() {
        return player.configure_bundled_sorotte_bridge();
    }

    player.sorotte_bridge_health()
}
