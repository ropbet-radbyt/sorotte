#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::app) enum MenuSectionId {
    File,
    Playback,
    Advanced,
    Window,
    Help,
}

impl MenuSectionId {
    #[cfg(test)]
    pub(in crate::app) const ALL: [Self; 5] = [
        Self::File,
        Self::Playback,
        Self::Advanced,
        Self::Window,
        Self::Help,
    ];

    pub(in crate::app) const fn automation_id(self) -> &'static str {
        match self {
            Self::File => "menu.section.file",
            Self::Playback => "menu.section.playback",
            Self::Advanced => "menu.section.advanced",
            Self::Window => "menu.section.window",
            Self::Help => "menu.section.help",
        }
    }

    pub(in crate::app) const fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Playback => "Playback",
            Self::Advanced => "Advanced",
            Self::Window => "Window",
            Self::Help => "Help",
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn from_automation_id(automation_id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section_id| section_id.automation_id() == automation_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::app) enum MenuActionId {
    OpenMedia,
    OpenMediaSearch,
    OpenPublicServerBrowser,
    Exit,
    Play,
    Pause,
    TogglePause,
    Seek,
    UndoSeek,
    SharedPlaylist,
    CreateControlledRoom,
    IdentifyAsController,
    TrustedDomains,
    SetOffset,
    TlsCertificates,
    TogglePlaybackButtons,
    ToggleAutoplayControls,
    ToggleHideEmptyRooms,
    About,
    Help,
    CheckForUpdates,
}

impl MenuActionId {
    pub(in crate::app) const ALL: [Self; 21] = [
        Self::OpenMedia,
        Self::OpenMediaSearch,
        Self::OpenPublicServerBrowser,
        Self::Exit,
        Self::Play,
        Self::Pause,
        Self::TogglePause,
        Self::Seek,
        Self::UndoSeek,
        Self::SharedPlaylist,
        Self::CreateControlledRoom,
        Self::IdentifyAsController,
        Self::TrustedDomains,
        Self::SetOffset,
        Self::TlsCertificates,
        Self::TogglePlaybackButtons,
        Self::ToggleAutoplayControls,
        Self::ToggleHideEmptyRooms,
        Self::About,
        Self::Help,
        Self::CheckForUpdates,
    ];

    pub(in crate::app) const fn automation_id(self) -> &'static str {
        match self {
            Self::OpenMedia => "menu.open_media",
            Self::OpenMediaSearch => "menu.open_media_search",
            Self::OpenPublicServerBrowser => "menu.open_public_server_browser",
            Self::Exit => "menu.exit",
            Self::Play => "menu.play",
            Self::Pause => "menu.pause",
            Self::TogglePause => "menu.toggle_pause",
            Self::Seek => "menu.seek",
            Self::UndoSeek => "menu.undo_seek",
            Self::SharedPlaylist => "menu.shared_playlist",
            Self::CreateControlledRoom => "menu.create_controlled_room",
            Self::IdentifyAsController => "menu.identify_as_controller",
            Self::TrustedDomains => "menu.trusted_domains",
            Self::SetOffset => "menu.set_offset",
            Self::TlsCertificates => "menu.tls_certificates",
            Self::TogglePlaybackButtons => "menu.toggle_playback_buttons",
            Self::ToggleAutoplayControls => "menu.toggle_autoplay_controls",
            Self::ToggleHideEmptyRooms => "menu.toggle_hide_empty_rooms",
            Self::About => "menu.about",
            Self::Help => "menu.help",
            Self::CheckForUpdates => "menu.check_for_updates",
        }
    }

    pub(in crate::app) const fn label(self) -> &'static str {
        match self {
            Self::OpenMedia => "Open Media File",
            Self::OpenMediaSearch => "Open Media Search",
            Self::OpenPublicServerBrowser => "Open Public Server Browser",
            Self::Exit => "Exit",
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::TogglePause => "Toggle Pause",
            Self::Seek => "Seek",
            Self::UndoSeek => "Undo Seek",
            Self::SharedPlaylist => "Shared Playlist",
            Self::CreateControlledRoom => "Create Controlled Room",
            Self::IdentifyAsController => "Identify As Controller",
            Self::TrustedDomains => "Trusted Domains",
            Self::SetOffset => "Set Offset",
            Self::TlsCertificates => "TLS Certificates",
            Self::TogglePlaybackButtons => "Playback Buttons",
            Self::ToggleAutoplayControls => "Autoplay",
            Self::ToggleHideEmptyRooms => "Hide Empty Rooms",
            Self::About => "About",
            Self::Help => "Manual / Command Help",
            Self::CheckForUpdates => "Check for Updates",
        }
    }

    pub(in crate::app) const fn is_checkable(self) -> bool {
        matches!(
            self,
            Self::TogglePlaybackButtons | Self::ToggleAutoplayControls | Self::ToggleHideEmptyRooms
        )
    }

    pub(in crate::app) fn from_automation_id(automation_id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action_id| action_id.automation_id() == automation_id)
    }

    pub(in crate::app) const fn help_url() -> &'static str {
        "https://github.com/ropbet-radbyt/sorotte/blob/main/docs/CLIENT.md"
    }
}

impl super::MenuActionShellItem {
    pub(in crate::app) const fn new(id: MenuActionId, enabled: bool, is_checked: bool) -> Self {
        Self {
            id,
            label: id.label(),
            enabled,
            is_selected: false,
            is_checked,
        }
    }
}

impl super::MenuDialogShellState {
    pub(in crate::app) fn action(&self, id: MenuActionId) -> Option<&super::MenuActionShellItem> {
        self.sections
            .iter()
            .flat_map(|section| &section.actions)
            .find(|action| action.id == id)
    }

    pub(in crate::app) fn action_mut(
        &mut self,
        id: MenuActionId,
    ) -> Option<&mut super::MenuActionShellItem> {
        self.sections
            .iter_mut()
            .flat_map(|section| &mut section.actions)
            .find(|action| action.id == id)
    }

    pub(in crate::app) fn action_index(&self, id: MenuActionId) -> Option<(usize, usize)> {
        self.sections
            .iter()
            .enumerate()
            .find_map(|(section_index, section)| {
                section
                    .actions
                    .iter()
                    .position(|action| action.id == id)
                    .map(|action_index| (section_index, action_index))
            })
    }

    pub(in crate::app) fn action_id_at(
        &self,
        section_index: usize,
        action_index: usize,
    ) -> Option<MenuActionId> {
        self.sections
            .get(section_index)?
            .actions
            .get(action_index)
            .map(|action| action.id)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

    use super::*;

    #[test]
    fn menu_section_ids_have_unique_stable_automation_ids_and_labels() {
        let automation_ids = MenuSectionId::ALL
            .into_iter()
            .map(MenuSectionId::automation_id)
            .collect::<HashSet<_>>();
        let labels = MenuSectionId::ALL
            .into_iter()
            .map(MenuSectionId::label)
            .collect::<HashSet<_>>();

        assert_eq!(automation_ids.len(), MenuSectionId::ALL.len());
        assert_eq!(labels.len(), MenuSectionId::ALL.len());
        assert!(
            automation_ids
                .iter()
                .all(|id| id.starts_with("menu.section."))
        );
        for section_id in MenuSectionId::ALL {
            assert_eq!(
                MenuSectionId::from_automation_id(section_id.automation_id()),
                Some(section_id)
            );
        }
    }

    #[test]
    fn menu_action_ids_have_unique_stable_automation_ids() {
        let automation_ids = MenuActionId::ALL
            .into_iter()
            .map(MenuActionId::automation_id)
            .collect::<HashSet<_>>();

        assert_eq!(automation_ids.len(), MenuActionId::ALL.len());
        assert!(automation_ids.iter().all(|id| id.starts_with("menu.")));
        for action_id in MenuActionId::ALL {
            assert_eq!(
                MenuActionId::from_automation_id(action_id.automation_id()),
                Some(action_id)
            );
        }
        assert_eq!(
            MenuActionId::ALL
                .into_iter()
                .filter(|action_id| action_id.is_checkable())
                .collect::<Vec<_>>(),
            vec![
                MenuActionId::TogglePlaybackButtons,
                MenuActionId::ToggleAutoplayControls,
                MenuActionId::ToggleHideEmptyRooms,
            ]
        );
    }

    #[test]
    fn menu_action_ids_cover_every_presented_menu_action() {
        let menus = super::super::MenuDialogShellState::from_stored_settings(
            &StoredClientSettingsMvp::default(),
        );
        assert_eq!(
            menus
                .sections
                .iter()
                .map(|section| section.id)
                .collect::<Vec<_>>(),
            MenuSectionId::ALL
        );
        assert!(
            menus
                .sections
                .iter()
                .all(|section| section.title == section.id.label())
        );
        let presented = menus
            .sections
            .iter()
            .flat_map(|section| &section.actions)
            .map(|action| action.id)
            .collect::<HashSet<_>>();
        let declared = MenuActionId::ALL.into_iter().collect::<HashSet<_>>();

        assert_eq!(presented, declared);
    }
}
