#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncplayConfigurationGetterCompatibilityStatus {
    Supported,
    Ignored,
}

impl SyncplayConfigurationGetterCompatibilityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncplayConfigurationGetterStartupCompatEntry {
    pub input: &'static str,
    pub status: SyncplayConfigurationGetterCompatibilityStatus,
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncplayConfigurationGetterIniCompatEntry {
    pub key: &'static str,
    pub status: SyncplayConfigurationGetterCompatibilityStatus,
    pub note: &'static str,
}

pub fn syncplay_configuration_getter_startup_compat_entries()
-> &'static [SyncplayConfigurationGetterStartupCompatEntry] {
    use SyncplayConfigurationGetterCompatibilityStatus::{Ignored, Supported};

    &[
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--no-gui",
            status: Supported,
            note: "starts client mode in sorotte-cli",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--host",
            status: Supported,
            note: "Syncplay host[:port] parsing supported",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--name",
            status: Supported,
            note: "username override supported",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--debug",
            status: Supported,
            note: "enables sorotte-cli diagnostics output (player telemetry, drift, and reconnect-correction snapshots)",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--force-gui-prompt",
            status: Supported,
            note: "headless compatibility gate: requests GUI startup and halts sorotte-cli unless --no-gui explicitly overrides",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--no-store",
            status: Supported,
            note: "disables stored-settings persistence",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--room",
            status: Supported,
            note: "Syncplay room / controlled-room password parsing supported",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--password",
            status: Supported,
            note: "controlled-room password override supported",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--player-path",
            status: Supported,
            note: "mpv paths auto-select managed mpv integration with Python-style mpv path resolution; non-mpv values remain supported as launch-only unmanaged fallback and are explicitly ignored by managed mpv or explicit-IPC modes",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "-psn",
            status: Ignored,
            note: "macOS launcher artifact; consumed and ignored",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--language",
            status: Supported,
            note: "supported Python language tags are normalized/persisted and localize the user-facing startup/help and runtime notification surfaces; raw JSON diagnostics and low-level operator-facing technical warnings remain intentionally stable English output",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "file",
            status: Supported,
            note: "startup parsing and routing supported across managed mpv, unmanaged external launch, and explicit-mpv-IPC open-file; non-startup side effects (GUI/relative-config) are tracked separately",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--clear-gui-data",
            status: Supported,
            note: "clears sorotte.ini stored settings and Syncplay GUI QSettings stores (PlayerList, MediaBrowseDialog, MainWindow, Interface, MoreSettings)",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--version",
            status: Supported,
            note: "prints sorotte-cli version and exits",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "--load-playlist-from-file",
            status: Supported,
            note: "connect-time one-shot playlistChange + playlistIndex(0) after server Hello",
        },
        SyncplayConfigurationGetterStartupCompatEntry {
            input: "_args",
            status: Supported,
            note: "launch modes forward arbitrary argv with Python-style file routing; explicit-mpv-IPC applies the runtime property subset plus generic --name=value / --profile attach commands, and only remaining launch-only tokens emit a deterministic attach-mode warning",
        },
    ]
}

pub fn syncplay_configuration_getter_ini_compat_entries()
-> &'static [SyncplayConfigurationGetterIniCompatEntry] {
    use SyncplayConfigurationGetterCompatibilityStatus::{Ignored, Supported};

    &[
        SyncplayConfigurationGetterIniCompatEntry {
            key: "server_data.host",
            status: Supported,
            note: "loaded/persisted into client host",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "server_data.port",
            status: Supported,
            note: "loaded/persisted into client port",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "server_data.password",
            status: Supported,
            note: "loaded from sorotte.ini/env into outbound client Hello password field (parse/upsert preservation also supported)",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.name",
            status: Supported,
            note: "loaded/persisted into username",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.room",
            status: Supported,
            note: "loaded/persisted into room (controlled-room suffix normalization preserved)",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.autoplayInitialState",
            status: Supported,
            note: "loaded/persisted into autoplay enabled default",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.autoplayRequireSameFilenames",
            status: Supported,
            note: "loaded/persisted into readiness autoplay config",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.readyAtStart",
            status: Supported,
            note: "loaded/persisted into connect-time readiness auto-ready behavior",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.sharedPlaylistEnabled",
            status: Supported,
            note: "loaded/persisted into CLI shared-playlist feature advertisement and outbound playlist action gating; sorotte-gui owns interactive playlist workflows",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.pauseOnLeave",
            status: Supported,
            note: "loaded/persisted into client behavior config",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.loopAtEndOfPlaylist",
            status: Supported,
            note: "loaded/persisted into client behavior config",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.loopSingleFiles",
            status: Supported,
            note: "loaded/persisted into client behavior config",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.unpauseAction",
            status: Supported,
            note: "loaded/persisted into readiness autoplay config",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.autoplayMinUsers",
            status: Supported,
            note: "loaded/persisted into readiness autoplay threshold",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.filenamePrivacyMode",
            status: Supported,
            note: "loaded/persisted into filename privacy mode",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.filesizePrivacyMode",
            status: Supported,
            note: "loaded/persisted into filesize privacy mode",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.playerPath",
            status: Supported,
            note: "loaded/persisted into the player startup path default, including Python-style managed mpv path resolution and launch routing",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.perPlayerArguments",
            status: Supported,
            note: "Python-serialized dict is loaded/persisted for startup player-arg defaults keyed by playerPath across stored config, CLI overrides, managed launch, and explicit-mpv-IPC attach mode",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.roomList",
            status: Supported,
            note: "loaded/persisted for CLI room fallback when client_settings.room is absent; sorotte-gui owns interactive room selection",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.{slowdownThreshold,rewindThreshold,fastforwardThreshold}",
            status: Supported,
            note: "loaded/persisted into desync correction threshold tuning",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.{slowOnDesync,rewindOnDesync,fastforwardOnDesync}",
            status: Supported,
            note: "loaded/persisted into desync correction feature toggles",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.dontSlowDownWithMe",
            status: Supported,
            note: "loaded/persisted into CLI desync fast-forward gating runtime flag",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.mediaSearchDirectories",
            status: Supported,
            note: "loaded/persisted and used for CLI startup-file fallback search when the requested media file is missing; sorotte-gui owns interactive media search",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.publicServers",
            status: Supported,
            note: "loaded/persisted for CLI server fallback when server_data.host/server_data.port are absent; sorotte-gui owns public-server browsing",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.{folderSearchFirstFileTimeout,folderSearchTimeout,folderSearchDoubleCheckInterval,folderSearchWarningThreshold}",
            status: Supported,
            note: "loaded/persisted and applied to CLI startup-file fallback search timing/warning behavior; sorotte-gui owns interactive media search timing",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.forceGuiPrompt",
            status: Supported,
            note: "loaded/persisted as the same headless startup gate as --force-gui-prompt; True halts sorotte-cli unless --no-gui explicitly overrides",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "client_settings.{onlySwitchToTrustedDomains,trustedDomains}",
            status: Supported,
            note: "loaded/persisted into trusted-domain playlist URL policy",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "gui.{autosaveJoinsToList,showOSD,showSlowdownOSD,showContactInfo}",
            status: Supported,
            note: "sorotte.ini parse/upsert preservation supported; these GUI-only behavior toggles are storage-compatible only and intentionally have no sorotte-cli runtime effect",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "gui.{chatMoveOSD,chatMaxLines,chatTopMargin,chatLeftMargin,chatBottomMargin,chatOSDMargin,notificationTimeout,alertTimeout,chatTimeout}",
            status: Supported,
            note: "sorotte.ini parse/upsert preservation supported; these GUI chat layout/timeout settings are storage-compatible only and intentionally have no sorotte-cli runtime effect",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "gui.{chatInputEnabled,chatInputFontUnderline,chatInputFontFamily,chatInputRelativeFontSize,chatInputFontWeight,chatInputFontColor,chatInputPosition,chatDirectInput,chatOutputEnabled,chatOutputFontUnderline,chatOutputFontFamily,chatOutputRelativeFontSize,chatOutputFontWeight,chatOutputMode}",
            status: Supported,
            note: "sorotte.ini parse/upsert preservation supported; these GUI chat input/output presentation settings are storage-compatible only and intentionally have no sorotte-cli runtime effect",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "gui.showDurationNotification",
            status: Supported,
            note: "loaded/persisted into readiness notification behavior",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "gui.{showSameRoomOSD,showOSDWarnings,showNonControllerOSD,showDifferentRoomOSD}",
            status: Supported,
            note: "loaded/persisted into OSD visibility behavior toggles",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "gui.* (remaining unenumerated GUI keys / QSettings visual state)",
            status: Ignored,
            note: "remaining GUI-only keys not explicitly enumerated above and non-INI GUI QSettings visual state are not implemented in sorotte-cli",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "general.language",
            status: Supported,
            note: "supported Python language tags are normalized, persisted, and loaded; startup/help and user-facing runtime text use the selected locale, while raw JSON diagnostics and low-level operator-facing technical warnings remain intentionally stable English",
        },
        SyncplayConfigurationGetterIniCompatEntry {
            key: "general.{checkForUpdatesAutomatically,lastCheckedForUpdates}",
            status: Supported,
            note: "stored automatic-update cadence and last-checked timestamp are honored headlessly; sorotte-gui owns interactive update checks and dialogs",
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        SyncplayConfigurationGetterCompatibilityStatus,
        syncplay_configuration_getter_ini_compat_entries,
        syncplay_configuration_getter_startup_compat_entries,
    };

    #[test]
    fn startup_entries_include_psn_ignore_only_exception() {
        let psn = syncplay_configuration_getter_startup_compat_entries()
            .iter()
            .find(|entry| entry.input == "-psn")
            .expect("missing -psn startup compatibility entry");

        assert_eq!(
            psn.status,
            SyncplayConfigurationGetterCompatibilityStatus::Ignored
        );
    }

    #[test]
    fn compatibility_tables_do_not_duplicate_keys_or_inputs() {
        let mut startup_inputs = BTreeSet::new();
        for entry in syncplay_configuration_getter_startup_compat_entries() {
            assert!(startup_inputs.insert(entry.input));
        }

        let mut ini_keys = BTreeSet::new();
        for entry in syncplay_configuration_getter_ini_compat_entries() {
            assert!(ini_keys.insert(entry.key));
        }
    }
}
