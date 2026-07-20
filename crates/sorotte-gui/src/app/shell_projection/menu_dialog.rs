use super::*;

impl MenuDialogShellState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        Self {
            sections: vec![
                MenuSectionShellState {
                    title: "File",
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::OpenMedia, false, false),
                        MenuActionShellItem::new(MenuActionId::OpenMediaSearch, true, false),
                        MenuActionShellItem::new(
                            MenuActionId::OpenPublicServerBrowser,
                            true,
                            false,
                        ),
                        MenuActionShellItem::new(MenuActionId::Exit, true, false),
                    ],
                },
                MenuSectionShellState {
                    title: "Playback",
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::Play, false, false),
                        MenuActionShellItem::new(MenuActionId::Pause, false, false),
                        MenuActionShellItem::new(MenuActionId::TogglePause, false, false),
                        MenuActionShellItem::new(MenuActionId::Seek, false, false),
                        MenuActionShellItem::new(MenuActionId::UndoSeek, false, false),
                        MenuActionShellItem::new(MenuActionId::SharedPlaylist, false, false),
                    ],
                },
                MenuSectionShellState {
                    title: "Advanced",
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::CreateControlledRoom, false, false),
                        MenuActionShellItem::new(MenuActionId::IdentifyAsController, false, false),
                        MenuActionShellItem::new(MenuActionId::TrustedDomains, true, false),
                        MenuActionShellItem::new(MenuActionId::SetOffset, false, false),
                        MenuActionShellItem::new(MenuActionId::TlsCertificates, true, false),
                    ],
                },
                MenuSectionShellState {
                    title: "Window",
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::TogglePlaybackButtons, true, true),
                        MenuActionShellItem::new(MenuActionId::ToggleAutoplayControls, true, true),
                        MenuActionShellItem::new(MenuActionId::ToggleHideEmptyRooms, true, false),
                    ],
                },
                MenuSectionShellState {
                    title: "Help",
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::About, true, false),
                        MenuActionShellItem::new(MenuActionId::Help, true, false),
                        MenuActionShellItem::new(MenuActionId::CheckForUpdates, true, false),
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
