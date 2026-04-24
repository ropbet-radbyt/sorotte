use super::*;

pub(crate) fn legacy_force_gui_prompt_compatibility_line_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
) -> Option<&'static str> {
    if !overrides.force_gui_prompt_requested {
        return None;
    }
    if overrides.no_gui_requested {
        Some(
            "note: legacy --force-gui-prompt was overridden by --no-gui; continuing in headless mode",
        )
    } else {
        Some(
            "note: legacy --force-gui-prompt requested GUI configuration flow; syncplay-cli has no GUI, so startup is halted. Re-run with --no-gui to continue headless.",
        )
    }
}

pub(crate) fn should_halt_for_stored_force_gui_prompt_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
    settings: &StoredClientSettingsMvp,
) -> bool {
    !overrides.force_gui_prompt_requested
        && settings.force_gui_prompt == Some(true)
        && !overrides.no_gui_requested
}

pub(crate) fn stored_force_gui_prompt_compatibility_line_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
    settings: &StoredClientSettingsMvp,
) -> Option<&'static str> {
    if overrides.force_gui_prompt_requested || settings.force_gui_prompt != Some(true) {
        return None;
    }
    if overrides.no_gui_requested {
        Some(
            "note: stored client_settings.forceGuiPrompt = True was overridden by --no-gui; continuing in headless mode",
        )
    } else {
        Some(
            "note: stored client_settings.forceGuiPrompt = True requested GUI configuration flow; syncplay-cli has no GUI, so startup is halted. Re-run with --no-gui or clear the stored setting to continue headless.",
        )
    }
}
