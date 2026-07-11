use super::*;

impl MenuDialogShellState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let config = ClientConfig::resolve(settings).config;
        let shared_playlist_enabled = config.playback.shared_playlist_enabled;
        let chat_enabled =
            config.interface.chat_input_enabled || config.interface.chat_output_enabled;

        Self {
            sections: vec![
                MenuSectionShellState {
                    title: "File",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Open Media File",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Open Media Search",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Open Public Server Browser",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Exit",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Playback",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Play",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Pause",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Toggle Pause",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Seek",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Undo Seek",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Shared Playlist",
                            enabled: false,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Advanced",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Create Controlled Room",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Identify As Controller",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Trusted Domains",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Set Offset",
                            enabled: false,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "TLS Certificates",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Window",
                    actions: vec![
                        MenuActionShellItem {
                            label: "Show Chat",
                            enabled: chat_enabled,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Show Playlist",
                            enabled: shared_playlist_enabled,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Show Users",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Playback Buttons",
                            enabled: true,
                            is_selected: true,
                        },
                        MenuActionShellItem {
                            label: "Autoplay",
                            enabled: true,
                            is_selected: true,
                        },
                        MenuActionShellItem {
                            label: "Hide Empty Rooms",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
                MenuSectionShellState {
                    title: "Help",
                    actions: vec![
                        MenuActionShellItem {
                            label: "About",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Manual / Command Help",
                            enabled: true,
                            is_selected: false,
                        },
                        MenuActionShellItem {
                            label: "Check for Updates",
                            enabled: true,
                            is_selected: false,
                        },
                    ],
                },
            ],
            // The menu models whether the persisted checkbox explicitly requests a prompt;
            // the live session uses the resolved playback policy.
            tls_prompt_expected: settings.only_switch_to_trusted_domains == Some(true),
            update_notice_expected: false,
            about_dialog_available: true,
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec!["[Menus & Dialogs]".to_owned()];

        for section in &self.sections {
            lines.push(format!("{}:", section.title));
            for action in &section.actions {
                lines.push(format!(
                    "- {} [enabled={}, selected={}]",
                    action.label,
                    bool_label(action.enabled),
                    bool_label(action.is_selected),
                ));
            }
        }

        lines.push(format!(
            "Dialog Prompts: tls_certificate={}, update_notice={}, about={}",
            bool_label(self.tls_prompt_expected),
            bool_label(self.update_notice_expected),
            bool_label(self.about_dialog_available),
        ));

        lines
    }
}
