use super::*;

impl Default for MenuDialogShellState {
    fn default() -> Self {
        Self {
            sections: vec![
                MenuSectionShellState {
                    id: MenuSectionId::File,
                    title: MenuSectionId::File.label(),
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
                    id: MenuSectionId::Playback,
                    title: MenuSectionId::Playback.label(),
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
                    id: MenuSectionId::Advanced,
                    title: MenuSectionId::Advanced.label(),
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::CreateControlledRoom, false, false),
                        MenuActionShellItem::new(MenuActionId::IdentifyAsController, false, false),
                        MenuActionShellItem::new(MenuActionId::TrustedDomains, true, false),
                        MenuActionShellItem::new(MenuActionId::SetOffset, false, false),
                    ],
                },
                MenuSectionShellState {
                    id: MenuSectionId::Window,
                    title: MenuSectionId::Window.label(),
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::TogglePlaybackButtons, true, true),
                        MenuActionShellItem::new(MenuActionId::ToggleAutoplayControls, true, true),
                        MenuActionShellItem::new(MenuActionId::ToggleHideEmptyRooms, true, false),
                    ],
                },
                MenuSectionShellState {
                    id: MenuSectionId::Help,
                    title: MenuSectionId::Help.label(),
                    actions: vec![
                        MenuActionShellItem::new(MenuActionId::About, true, false),
                        MenuActionShellItem::new(MenuActionId::Help, true, false),
                        MenuActionShellItem::new(MenuActionId::CheckForUpdates, true, false),
                    ],
                },
            ],
            about_dialog_available: true,
        }
    }
}

impl MenuDialogShellState {
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
            "Dialogs: about={}",
            bool_label(self.about_dialog_available),
        ));

        lines
    }
}
