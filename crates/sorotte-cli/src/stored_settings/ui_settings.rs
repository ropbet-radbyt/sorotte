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
    vec![
        manifest_dir.join("../../resources/syncplayintf.lua"),
        manifest_dir
            .join("../../.interop-cache/syncplay-legacy/syncplay/resources/syncplayintf.lua"),
        manifest_dir.join("../../../syncplay/syncplay/resources/syncplayintf.lua"),
    ]
}

pub(crate) const LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER: &str =
    "-- sorotte-chat-input-bridge";

fn legacy_syncplayintf_chat_input_bridge_source_legacy_compatible() -> &'static str {
    r#"
-- sorotte-chat-input-bridge
local syncplay_rust_original_handle_enter = handle_enter
function handle_enter()
    if repl_active and line ~= '' then
        local syncplay_rust_chat_line = line
        if opts['backslashSubstituteCharacter'] ~= nil then
            syncplay_rust_chat_line = string.gsub(syncplay_rust_chat_line, opts['backslashSubstituteCharacter'], "\\")
        end
        mp.commandv("script-message", "syncplayintf-chat", syncplay_rust_chat_line)
    end
    syncplay_rust_original_handle_enter()
end
"#
}

pub(crate) fn legacy_syncplayintf_script_source_with_chat_input_bridge_legacy_compatible(
    source: &str,
) -> String {
    if source.contains(LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER) {
        return source.to_owned();
    }

    let mut patched = source.to_owned();
    if !patched.ends_with('\n') {
        patched.push('\n');
    }
    patched.push_str(legacy_syncplayintf_chat_input_bridge_source_legacy_compatible());
    patched
}

fn prepare_legacy_syncplayintf_script_path_legacy_compatible(
    source_path: &Path,
) -> anyhow::Result<PathBuf> {
    let source = fs::read_to_string(source_path).map_err(|error| {
        anyhow!(
            "failed to read syncplayintf.lua from '{}': {error}",
            source_path.display()
        )
    })?;
    if source.contains(LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER) {
        return Ok(source_path.to_path_buf());
    }

    let patched =
        legacy_syncplayintf_script_source_with_chat_input_bridge_legacy_compatible(&source);
    let target_path =
        std::env::temp_dir().join(format!("sorotte-syncplayintf-{}.lua", std::process::id()));
    fs::write(&target_path, patched).map_err(|error| {
        anyhow!(
            "failed to write patched syncplayintf.lua to '{}': {error}",
            target_path.display()
        )
    })?;
    Ok(target_path)
}

fn find_legacy_syncplayintf_script_path_legacy_compatible() -> Option<PathBuf> {
    let source_path = legacy_syncplayintf_script_candidate_paths_legacy_compatible()
        .into_iter()
        .find(|candidate| candidate.is_file())?;
    Some(
        prepare_legacy_syncplayintf_script_path_legacy_compatible(&source_path)
            .unwrap_or(source_path),
    )
}

pub(crate) fn apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(
    player: &mut MpvAdapter,
    settings: Option<&StoredClientSettingsMvp>,
) -> anyhow::Result<()> {
    let resolved = legacy_syncplay_ui_settings_from_stored_settings(settings);
    player
        .configure_legacy_syncplay_ui_settings(resolved.clone())
        .map_err(|error| anyhow!("failed to configure mpv OSD/chat settings: {error}"))?;

    if (resolved.chat_output_enabled || resolved.chat_input_enabled)
        && let Some(script_path) = find_legacy_syncplayintf_script_path_legacy_compatible()
        && let Err(error) = player.load_legacy_syncplayintf_script(&script_path)
    {
        eprintln!(
            "warning: failed to load legacy mpv syncplayintf.lua from '{}' via JSON IPC: {}",
            script_path.display(),
            error
        );
    }

    Ok(())
}
