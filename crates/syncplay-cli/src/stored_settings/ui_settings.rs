use super::*;

fn timeout_ms_from_stored_client_setting_legacy_compatible(
    value: Option<i64>,
    default_ms: u64,
) -> u64 {
    value
        .and_then(|seconds| {
            let seconds = u64::try_from(seconds).ok()?;
            seconds.checked_mul(1_000)
        })
        .unwrap_or(default_ms)
}

pub(crate) fn legacy_syncplay_ui_settings_from_stored_settings(
    settings: Option<&StoredClientSettingsMvp>,
) -> LegacySyncplayUiSettings {
    let mut resolved = LegacySyncplayUiSettings::default();
    let Some(settings) = settings else {
        return resolved;
    };

    if let Some(show_osd) = settings.show_osd {
        resolved.show_osd = show_osd;
    }
    if let Some(chat_output_enabled) = settings.chat_output_enabled {
        resolved.chat_output_enabled = chat_output_enabled;
    }
    if let Some(chat_input_enabled) = settings.chat_input_enabled {
        resolved.chat_input_enabled = chat_input_enabled;
    }
    if let Some(chat_input_font_underline) = settings.chat_input_font_underline {
        resolved.chat_input_font_underline = chat_input_font_underline;
    }
    if let Some(chat_input_font_family) = settings
        .chat_input_font_family
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_input_font_family = chat_input_font_family.to_owned();
    }
    if let Some(chat_input_relative_font_size) = settings
        .chat_input_relative_font_size
        .filter(|value| *value > 0)
    {
        resolved.chat_input_relative_font_size = chat_input_relative_font_size;
    }
    if let Some(chat_input_font_weight) = settings.chat_input_font_weight {
        resolved.chat_input_font_weight = chat_input_font_weight;
    }
    if let Some(chat_input_font_color) = settings
        .chat_input_font_color
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_input_font_color = chat_input_font_color.to_owned();
    }
    if let Some(chat_input_position) = settings
        .chat_input_position
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_input_position = chat_input_position.to_owned();
    }
    if let Some(chat_direct_input) = settings.chat_direct_input {
        resolved.chat_direct_input = chat_direct_input;
    }
    if let Some(chat_output_font_underline) = settings.chat_output_font_underline {
        resolved.chat_output_font_underline = chat_output_font_underline;
    }
    if let Some(chat_output_font_family) = settings
        .chat_output_font_family
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_output_font_family = chat_output_font_family.to_owned();
    }
    if let Some(chat_output_relative_font_size) = settings
        .chat_output_relative_font_size
        .filter(|value| *value > 0)
    {
        resolved.chat_output_relative_font_size = chat_output_relative_font_size;
    }
    if let Some(chat_output_font_weight) = settings.chat_output_font_weight {
        resolved.chat_output_font_weight = chat_output_font_weight;
    }
    if let Some(chat_output_mode) = settings
        .chat_output_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_output_mode = chat_output_mode.to_owned();
    }
    if let Some(chat_max_lines) = settings.chat_max_lines.filter(|value| *value > 0) {
        resolved.chat_max_lines = chat_max_lines;
    }
    if let Some(chat_top_margin) = settings.chat_top_margin.filter(|value| *value >= 0) {
        resolved.chat_top_margin = chat_top_margin;
    }
    if let Some(chat_left_margin) = settings.chat_left_margin.filter(|value| *value >= 0) {
        resolved.chat_left_margin = chat_left_margin;
    }
    if let Some(chat_bottom_margin) = settings.chat_bottom_margin.filter(|value| *value >= 0) {
        resolved.chat_bottom_margin = chat_bottom_margin;
    }
    if let Some(chat_move_osd) = settings.chat_move_osd {
        resolved.chat_move_osd = chat_move_osd;
    }
    if let Some(chat_osd_margin) = settings.chat_osd_margin.filter(|value| *value >= 0) {
        resolved.chat_osd_margin = chat_osd_margin;
    }
    resolved.notification_timeout_ms = timeout_ms_from_stored_client_setting_legacy_compatible(
        settings.notification_timeout_seconds,
        resolved.notification_timeout_ms,
    );
    resolved.alert_timeout_ms = timeout_ms_from_stored_client_setting_legacy_compatible(
        settings.alert_timeout_seconds,
        resolved.alert_timeout_ms,
    );
    resolved.chat_timeout_ms = timeout_ms_from_stored_client_setting_legacy_compatible(
        settings.chat_timeout_seconds,
        resolved.chat_timeout_ms,
    );
    resolved
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
    "-- syncplay-rust-chat-input-bridge";

fn legacy_syncplayintf_chat_input_bridge_source_legacy_compatible() -> &'static str {
    r#"
-- syncplay-rust-chat-input-bridge
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
    let target_path = std::env::temp_dir().join(format!(
        "syncplay-rust-syncplayintf-{}.lua",
        std::process::id()
    ));
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
