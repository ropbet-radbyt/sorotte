#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyConfigurationGetterCompatibilityStatus {
    Supported,
    Ignored,
}

impl LegacyConfigurationGetterCompatibilityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyConfigurationGetterStartupCompatEntry {
    pub input: &'static str,
    pub status: LegacyConfigurationGetterCompatibilityStatus,
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyConfigurationGetterIniCompatEntry {
    pub key: &'static str,
    pub status: LegacyConfigurationGetterCompatibilityStatus,
    pub note: &'static str,
}

pub fn legacy_configuration_getter_startup_compat_entries()
-> &'static [LegacyConfigurationGetterStartupCompatEntry] {
    use LegacyConfigurationGetterCompatibilityStatus::{Ignored, Supported};

    &[
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--no-gui",
            status: Supported,
            note: "starts client mode in syncplay-cli",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--host",
            status: Supported,
            note: "legacy host[:port] parsing supported",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--name",
            status: Supported,
            note: "legacy username override supported",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--debug",
            status: Supported,
            note: "enables syncplay-cli diagnostics output (player telemetry, drift, and reconnect-correction snapshots)",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--force-gui-prompt",
            status: Supported,
            note: "headless compatibility gate: requests GUI startup and halts syncplay-cli unless --no-gui explicitly overrides",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--no-store",
            status: Supported,
            note: "disables stored-settings persistence",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--room",
            status: Supported,
            note: "legacy room / controlled-room password parsing supported",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--password",
            status: Supported,
            note: "controlled-room password override supported",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--player-path",
            status: Supported,
            note: "legacy mpv paths auto-select managed mpv integration with Python-style mpv path resolution; non-mpv values remain supported as launch-only unmanaged fallback and are explicitly ignored by managed mpv or explicit-IPC modes",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "-psn",
            status: Ignored,
            note: "macOS launcher artifact; consumed and ignored",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--language",
            status: Supported,
            note: "supported Python language tags are normalized/persisted and localize the user-facing startup/help and runtime notification surfaces; raw JSON diagnostics and low-level operator-facing technical warnings remain intentionally stable English output",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "file",
            status: Supported,
            note: "startup parsing and routing supported across managed mpv, unmanaged external launch, and explicit-mpv-IPC open-file; non-startup side effects (GUI/relative-config) are tracked separately",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--clear-gui-data",
            status: Supported,
            note: "clears syncplay.ini stored settings and legacy GUI QSettings stores (PlayerList, MediaBrowseDialog, MainWindow, Interface, MoreSettings)",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--version",
            status: Supported,
            note: "prints syncplay-cli version and exits",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "--load-playlist-from-file",
            status: Supported,
            note: "connect-time one-shot playlistChange + playlistIndex(0) after server Hello",
        },
        LegacyConfigurationGetterStartupCompatEntry {
            input: "_args",
            status: Supported,
            note: "launch modes forward arbitrary argv with Python-style file routing; explicit-mpv-IPC applies the runtime property subset plus generic --name=value / --profile attach commands, and only remaining launch-only tokens emit a deterministic attach-mode warning",
        },
    ]
}

pub fn legacy_configuration_getter_ini_compat_entries()
-> &'static [LegacyConfigurationGetterIniCompatEntry] {
    use LegacyConfigurationGetterCompatibilityStatus::{Ignored, Supported};

    &[
        LegacyConfigurationGetterIniCompatEntry {
            key: "server_data.host",
            status: Supported,
            note: "loaded/persisted into client host",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "server_data.port",
            status: Supported,
            note: "loaded/persisted into client port",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "server_data.password",
            status: Supported,
            note: "loaded from syncplay.ini/env into outbound client Hello password field (parse/upsert preservation also supported)",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.name",
            status: Supported,
            note: "loaded/persisted into username",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.room",
            status: Supported,
            note: "loaded/persisted into room (controlled-room suffix normalization preserved)",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.autoplayInitialState",
            status: Supported,
            note: "loaded/persisted into autoplay enabled default",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.autoplayRequireSameFilenames",
            status: Supported,
            note: "loaded/persisted into readiness autoplay config",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.readyAtStart",
            status: Supported,
            note: "loaded/persisted into connect-time readiness auto-ready behavior",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.sharedPlaylistEnabled",
            status: Supported,
            note: "loaded/persisted into CLI shared-playlist feature advertisement and outbound playlist action gating; syncplay-gui owns interactive playlist workflows",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.pauseOnLeave",
            status: Supported,
            note: "loaded/persisted into client behavior config",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.loopAtEndOfPlaylist",
            status: Supported,
            note: "loaded/persisted into client behavior config",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.loopSingleFiles",
            status: Supported,
            note: "loaded/persisted into client behavior config",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.unpauseAction",
            status: Supported,
            note: "loaded/persisted into readiness autoplay config",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.autoplayMinUsers",
            status: Supported,
            note: "loaded/persisted into readiness autoplay threshold",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.filenamePrivacyMode",
            status: Supported,
            note: "loaded/persisted into filename privacy mode",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.filesizePrivacyMode",
            status: Supported,
            note: "loaded/persisted into filesize privacy mode",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.playerPath",
            status: Supported,
            note: "loaded/persisted into the legacy player startup path default, including Python-style managed mpv path resolution and launch routing",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.perPlayerArguments",
            status: Supported,
            note: "Python-serialized dict is loaded/persisted for startup player-arg defaults keyed by playerPath across stored config, CLI overrides, managed launch, and explicit-mpv-IPC attach mode",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.roomList",
            status: Supported,
            note: "loaded/persisted for CLI room fallback when client_settings.room is absent; syncplay-gui owns interactive room selection",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.{slowdownThreshold,rewindThreshold,fastforwardThreshold}",
            status: Supported,
            note: "loaded/persisted into desync correction threshold tuning",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.{slowOnDesync,rewindOnDesync,fastforwardOnDesync}",
            status: Supported,
            note: "loaded/persisted into desync correction feature toggles",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.dontSlowDownWithMe",
            status: Supported,
            note: "loaded/persisted into CLI desync fast-forward gating runtime flag",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.mediaSearchDirectories",
            status: Supported,
            note: "loaded/persisted and used for CLI startup-file fallback search when the requested media file is missing; syncplay-gui owns interactive media search",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.publicServers",
            status: Supported,
            note: "loaded/persisted for CLI server fallback when server_data.host/server_data.port are absent; syncplay-gui owns public-server browsing",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.{folderSearchFirstFileTimeout,folderSearchTimeout,folderSearchDoubleCheckInterval,folderSearchWarningThreshold}",
            status: Supported,
            note: "loaded/persisted and applied to CLI startup-file fallback search timing/warning behavior; syncplay-gui owns interactive media search timing",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.forceGuiPrompt",
            status: Supported,
            note: "loaded/persisted as the same headless startup gate as legacy --force-gui-prompt; True halts syncplay-cli unless --no-gui explicitly overrides",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "client_settings.{onlySwitchToTrustedDomains,trustedDomains}",
            status: Supported,
            note: "loaded/persisted into trusted-domain playlist URL policy",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "gui.{autosaveJoinsToList,showOSD,showSlowdownOSD,showContactInfo}",
            status: Supported,
            note: "syncplay.ini parse/upsert preservation supported; these GUI-only behavior toggles are storage-compatible only and intentionally have no syncplay-cli runtime effect",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "gui.{chatMoveOSD,chatMaxLines,chatTopMargin,chatLeftMargin,chatBottomMargin,chatOSDMargin,notificationTimeout,alertTimeout,chatTimeout}",
            status: Supported,
            note: "syncplay.ini parse/upsert preservation supported; these GUI chat layout/timeout settings are storage-compatible only and intentionally have no syncplay-cli runtime effect",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "gui.{chatInputEnabled,chatInputFontUnderline,chatInputFontFamily,chatInputRelativeFontSize,chatInputFontWeight,chatInputFontColor,chatInputPosition,chatDirectInput,chatOutputEnabled,chatOutputFontUnderline,chatOutputFontFamily,chatOutputRelativeFontSize,chatOutputFontWeight,chatOutputMode}",
            status: Supported,
            note: "syncplay.ini parse/upsert preservation supported; these GUI chat input/output presentation settings are storage-compatible only and intentionally have no syncplay-cli runtime effect",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "gui.showDurationNotification",
            status: Supported,
            note: "loaded/persisted into readiness notification behavior",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "gui.{showSameRoomOSD,showOSDWarnings,showNonControllerOSD,showDifferentRoomOSD}",
            status: Supported,
            note: "loaded/persisted into OSD visibility behavior toggles",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "gui.* (remaining unenumerated GUI keys / QSettings visual state)",
            status: Ignored,
            note: "remaining GUI-only keys not explicitly enumerated above and non-INI GUI QSettings visual state are not implemented in syncplay-cli",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "general.language",
            status: Supported,
            note: "supported Python language tags are normalized, persisted, and loaded; startup/help and user-facing runtime text use the selected locale, while raw JSON diagnostics and low-level operator-facing technical warnings remain intentionally stable English",
        },
        LegacyConfigurationGetterIniCompatEntry {
            key: "general.{checkForUpdatesAutomatically,lastCheckedForUpdates}",
            status: Supported,
            note: "stored automatic-update cadence and last-checked timestamp are honored headlessly; syncplay-gui owns interactive update checks and dialogs",
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        LegacyConfigurationGetterCompatibilityStatus,
        legacy_configuration_getter_ini_compat_entries,
        legacy_configuration_getter_startup_compat_entries,
    };

    #[test]
    fn startup_entries_include_psn_ignore_only_exception() {
        let psn = legacy_configuration_getter_startup_compat_entries()
            .iter()
            .find(|entry| entry.input == "-psn")
            .expect("missing -psn startup compatibility entry");

        assert_eq!(
            psn.status,
            LegacyConfigurationGetterCompatibilityStatus::Ignored
        );
    }

    #[test]
    fn compatibility_tables_do_not_duplicate_keys_or_inputs() {
        let mut startup_inputs = BTreeSet::new();
        for entry in legacy_configuration_getter_startup_compat_entries() {
            assert!(startup_inputs.insert(entry.input));
        }

        let mut ini_keys = BTreeSet::new();
        for entry in legacy_configuration_getter_ini_compat_entries() {
            assert!(ini_keys.insert(entry.key));
        }
    }
}
