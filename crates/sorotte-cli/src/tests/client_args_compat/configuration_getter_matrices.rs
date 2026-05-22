use super::*;

#[test]
fn legacy_configuration_getter_startup_compat_matrix_covers_python_startup_inputs() {
    let entries = legacy_configuration_getter_startup_compat_entries();
    let expected_inputs = [
        "--no-gui",
        "--host",
        "--name",
        "--debug",
        "--force-gui-prompt",
        "--no-store",
        "--room",
        "--password",
        "--player-path",
        "-psn",
        "--language",
        "file",
        "--clear-gui-data",
        "--version",
        "--load-playlist-from-file",
        "_args",
    ];

    for expected in expected_inputs {
        assert!(
            entries.iter().any(|entry| entry.input == expected),
            "compatibility matrix should include {expected}"
        );
    }

    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        assert!(
            seen.insert(entry.input),
            "compatibility matrix should not duplicate input {}",
            entry.input
        );
    }
}

#[test]
fn legacy_configuration_getter_startup_compat_matrix_classifies_supported_and_ignored_inputs() {
    fn entry_for(input: &str) -> LegacyConfigurationGetterStartupCompatEntry {
        *legacy_configuration_getter_startup_compat_entries()
            .iter()
            .find(|entry| entry.input == input)
            .unwrap_or_else(|| panic!("missing compatibility entry for {input}"))
    }

    assert_eq!(
        entry_for("--load-playlist-from-file").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("--debug").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("--player-path").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert!(
        entry_for("--player-path")
            .note
            .contains("legacy mpv paths auto-select managed mpv integration")
    );
    assert!(
        entry_for("--player-path")
            .note
            .contains("launch-only unmanaged fallback")
    );
    assert_eq!(
        entry_for("file").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("--language").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert!(
        entry_for("--language")
            .note
            .contains("runtime notification surfaces")
    );
    assert_eq!(
        entry_for("_args").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("--clear-gui-data").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("--force-gui-prompt").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert!(
        entry_for("--force-gui-prompt")
            .note
            .contains("halts sorotte-cli unless --no-gui explicitly overrides")
    );
}

#[test]
fn legacy_configuration_getter_ini_compat_matrix_covers_key_python_ini_fields() {
    let entries = legacy_configuration_getter_ini_compat_entries();
    let expected_keys = [
        "server_data.host",
        "server_data.port",
        "server_data.password",
        "client_settings.name",
        "client_settings.room",
        "client_settings.autoplayInitialState",
        "client_settings.readyAtStart",
        "client_settings.sharedPlaylistEnabled",
        "client_settings.pauseOnLeave",
        "client_settings.loopAtEndOfPlaylist",
        "client_settings.loopSingleFiles",
        "client_settings.unpauseAction",
        "client_settings.autoplayMinUsers",
        "client_settings.playerPath",
        "client_settings.perPlayerArguments",
        "client_settings.roomList",
        "client_settings.mediaSearchDirectories",
        "client_settings.publicServers",
        "client_settings.{folderSearchFirstFileTimeout,folderSearchTimeout,folderSearchDoubleCheckInterval,folderSearchWarningThreshold}",
        "client_settings.forceGuiPrompt",
        "client_settings.{slowOnDesync,rewindOnDesync,fastforwardOnDesync}",
        "client_settings.dontSlowDownWithMe",
        "gui.{autosaveJoinsToList,showOSD,showSlowdownOSD,showContactInfo}",
        "gui.{chatMoveOSD,chatMaxLines,chatTopMargin,chatLeftMargin,chatBottomMargin,chatOSDMargin,notificationTimeout,alertTimeout,chatTimeout}",
        "gui.{chatInputEnabled,chatInputFontUnderline,chatInputFontFamily,chatInputRelativeFontSize,chatInputFontWeight,chatInputFontColor,chatInputPosition,chatDirectInput,chatOutputEnabled,chatOutputFontUnderline,chatOutputFontFamily,chatOutputRelativeFontSize,chatOutputFontWeight,chatOutputMode}",
        "gui.showDurationNotification",
        "gui.{showSameRoomOSD,showOSDWarnings,showNonControllerOSD,showDifferentRoomOSD}",
        "general.language",
        "general.{checkForUpdatesAutomatically,lastCheckedForUpdates}",
    ];

    for expected in expected_keys {
        assert!(
            entries.iter().any(|entry| entry.key == expected),
            "ini compatibility matrix should include {expected}"
        );
    }

    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        assert!(
            seen.insert(entry.key),
            "ini compatibility matrix should not duplicate key {}",
            entry.key
        );
    }
}

#[test]
fn legacy_configuration_getter_ini_compat_matrix_classifies_supported_and_ignored_fields() {
    fn entry_for(key: &str) -> LegacyConfigurationGetterIniCompatEntry {
        *legacy_configuration_getter_ini_compat_entries()
            .iter()
            .find(|entry| entry.key == key)
            .unwrap_or_else(|| panic!("missing ini compatibility entry for {key}"))
    }

    assert_eq!(
        entry_for("server_data.password").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.autoplayInitialState").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.readyAtStart").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.sharedPlaylistEnabled").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.unpauseAction").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.playerPath").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.perPlayerArguments").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.roomList").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.mediaSearchDirectories").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.publicServers").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
            entry_for("client_settings.{folderSearchFirstFileTimeout,folderSearchTimeout,folderSearchDoubleCheckInterval,folderSearchWarningThreshold}").status,
            LegacyConfigurationGetterCompatibilityStatus::Supported
        );
    assert_eq!(
        entry_for("client_settings.forceGuiPrompt").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert!(
        entry_for("client_settings.forceGuiPrompt")
            .note
            .contains("headless startup gate")
    );
    assert_eq!(
        entry_for("client_settings.{onlySwitchToTrustedDomains,trustedDomains}").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.{slowOnDesync,rewindOnDesync,fastforwardOnDesync}").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.{slowdownThreshold,rewindThreshold,fastforwardThreshold}")
            .status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("client_settings.dontSlowDownWithMe").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
        entry_for("gui.{autosaveJoinsToList,showOSD,showSlowdownOSD,showContactInfo}").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
    assert_eq!(
            entry_for("gui.{chatMoveOSD,chatMaxLines,chatTopMargin,chatLeftMargin,chatBottomMargin,chatOSDMargin,notificationTimeout,alertTimeout,chatTimeout}").status,
            LegacyConfigurationGetterCompatibilityStatus::Supported
        );
    assert_eq!(
            entry_for("gui.{chatInputEnabled,chatInputFontUnderline,chatInputFontFamily,chatInputRelativeFontSize,chatInputFontWeight,chatInputFontColor,chatInputPosition,chatDirectInput,chatOutputEnabled,chatOutputFontUnderline,chatOutputFontFamily,chatOutputRelativeFontSize,chatOutputFontWeight,chatOutputMode}").status,
            LegacyConfigurationGetterCompatibilityStatus::Supported
        );
    assert_eq!(
        entry_for("gui.* (remaining unenumerated GUI keys / QSettings visual state)").status,
        LegacyConfigurationGetterCompatibilityStatus::Ignored
    );
    assert_eq!(
        entry_for("general.{checkForUpdatesAutomatically,lastCheckedForUpdates}").status,
        LegacyConfigurationGetterCompatibilityStatus::Supported
    );
}
