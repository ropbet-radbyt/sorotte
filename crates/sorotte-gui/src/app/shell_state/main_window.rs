use super::super::DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::app) struct GuiMediaSourceProviderId(String);

impl GuiMediaSourceProviderId {
    pub(in crate::app) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(in crate::app) fn local() -> Self {
        Self::new("local")
    }

    pub(in crate::app) fn media_matching() -> Self {
        Self::new("media-matching")
    }

    pub(in crate::app) fn plex_stream() -> Self {
        Self::new("plex-stream")
    }

    pub(in crate::app) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiPlaylistSourceStatus {
    Available,
    Active,
    Resolving,
    Pending,
    Disabled,
    Missing,
    Failed,
}

impl GuiPlaylistSourceStatus {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Active => "active",
            Self::Resolving => "resolving",
            Self::Pending => "pending",
            Self::Disabled => "disabled",
            Self::Missing => "missing",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPlaylistSourceOption {
    pub(in crate::app) provider_id: GuiMediaSourceProviderId,
    pub(in crate::app) label: String,
    pub(in crate::app) status: GuiPlaylistSourceStatus,
    pub(in crate::app) detail: Option<String>,
    pub(in crate::app) enabled: bool,
    pub(in crate::app) selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPlaylistResolutionStep {
    pub(in crate::app) provider_id: GuiMediaSourceProviderId,
    pub(in crate::app) label: String,
    pub(in crate::app) status: GuiPlaylistSourceStatus,
    pub(in crate::app) detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPlaylistSourceState {
    pub(in crate::app) current_provider_id: GuiMediaSourceProviderId,
    pub(in crate::app) current_label: String,
    pub(in crate::app) status: GuiPlaylistSourceStatus,
    pub(in crate::app) detail: Option<String>,
    pub(in crate::app) options: Vec<GuiPlaylistSourceOption>,
    pub(in crate::app) resolution_steps: Vec<GuiPlaylistResolutionStep>,
}

impl GuiPlaylistSourceState {
    pub(in crate::app) fn inferred_for_entry(entry: &str) -> Self {
        let provider_id = if sorotte_plex::is_plex_playlist_uri(entry) {
            GuiMediaSourceProviderId::plex_stream()
        } else {
            GuiMediaSourceProviderId::local()
        };
        let current_label = if provider_id == GuiMediaSourceProviderId::plex_stream() {
            "Plex Stream"
        } else {
            "Local"
        }
        .to_owned();
        Self {
            current_provider_id: provider_id.clone(),
            current_label,
            status: GuiPlaylistSourceStatus::Available,
            detail: Some("Waiting for playlist activation.".to_owned()),
            options: default_playlist_source_options(&provider_id),
            resolution_steps: Vec::new(),
        }
    }
}

fn default_playlist_source_options(
    selected_provider_id: &GuiMediaSourceProviderId,
) -> Vec<GuiPlaylistSourceOption> {
    [
        (GuiMediaSourceProviderId::local(), "Local"),
        (GuiMediaSourceProviderId::media_matching(), "Media Matching"),
        (GuiMediaSourceProviderId::plex_stream(), "Plex Stream"),
    ]
    .into_iter()
    .map(|(provider_id, label)| GuiPlaylistSourceOption {
        selected: &provider_id == selected_provider_id,
        provider_id,
        label: label.to_owned(),
        status: GuiPlaylistSourceStatus::Available,
        detail: None,
        enabled: true,
    })
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MainWindowRoomRow {
    pub(in crate::app) room_name: String,
    pub(in crate::app) is_controlled: bool,
    pub(in crate::app) has_named_users: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MainWindowUserRow {
    pub(in crate::app) username: String,
    pub(in crate::app) room_name: String,
    pub(in crate::app) is_self: bool,
    pub(in crate::app) is_ready: bool,
    pub(in crate::app) is_controller: bool,
    pub(in crate::app) has_file: bool,
    pub(in crate::app) file_name: Option<String>,
    pub(in crate::app) file_name_label: String,
    pub(in crate::app) file_size_label: String,
    pub(in crate::app) file_duration_label: String,
    pub(in crate::app) file_is_url: bool,
    pub(in crate::app) file_is_trusted: bool,
    pub(in crate::app) filename_differs: bool,
    pub(in crate::app) filesize_differs: bool,
    pub(in crate::app) fileduration_differs: bool,
    pub(in crate::app) is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MainWindowPlaylistRow {
    pub(in crate::app) label: String,
    pub(in crate::app) is_selected: bool,
    pub(in crate::app) source_state: GuiPlaylistSourceState,
}

impl MainWindowPlaylistRow {
    pub(in crate::app) fn inferred(label: impl Into<String>, is_selected: bool) -> Self {
        let label = label.into();
        Self {
            source_state: GuiPlaylistSourceState::inferred_for_entry(&label),
            label,
            is_selected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MainWindowChatRow {
    pub(in crate::app) sender: String,
    pub(in crate::app) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct MainWindowPlaybackControls {
    pub(in crate::app) can_toggle_pause: bool,
    pub(in crate::app) can_seek: bool,
    pub(in crate::app) can_undo_seek: bool,
    pub(in crate::app) can_set_offset: bool,
    pub(in crate::app) can_toggle_autoplay: bool,
    pub(in crate::app) can_adjust_autoplay_threshold: bool,
    pub(in crate::app) can_set_ready: bool,
    pub(in crate::app) can_set_others_ready: bool,
    pub(in crate::app) can_manage_playlist: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct MainWindowShellState {
    pub(in crate::app) room_name: String,
    pub(in crate::app) room_control_status: String,
    pub(in crate::app) shared_playlist_enabled: bool,
    pub(in crate::app) controlled_room_active: bool,
    pub(in crate::app) hide_empty_rooms: bool,
    pub(in crate::app) rooms: Vec<MainWindowRoomRow>,
    pub(in crate::app) users: Vec<MainWindowUserRow>,
    pub(in crate::app) playlist: Vec<MainWindowPlaylistRow>,
    pub(in crate::app) active_playlist_index: Option<usize>,
    pub(in crate::app) chat: Vec<MainWindowChatRow>,
    pub(in crate::app) playback: MainWindowPlaybackControls,
    pub(in crate::app) playback_paused: bool,
    pub(in crate::app) autoplay_active: bool,
    pub(in crate::app) autoplay_threshold: usize,
    pub(in crate::app) autoplay_countdown_seconds: Option<u32>,
    pub(in crate::app) user_offset_seconds: f64,
    pub(in crate::app) show_playback_buttons: bool,
    pub(in crate::app) show_autoplay_controls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::app) struct MainWindowRuntimeUserSnapshot {
    pub(in crate::app) username: String,
    pub(in crate::app) room_name: String,
    pub(in crate::app) is_self: bool,
    pub(in crate::app) is_ready: bool,
    pub(in crate::app) is_controller: bool,
    pub(in crate::app) has_file: bool,
    pub(in crate::app) file_name: Option<String>,
    pub(in crate::app) file_size_label: String,
    pub(in crate::app) file_duration_label: String,
    pub(in crate::app) file_is_url: bool,
    pub(in crate::app) file_is_trusted: bool,
    pub(in crate::app) filename_differs: bool,
    pub(in crate::app) filesize_differs: bool,
    pub(in crate::app) fileduration_differs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::app) struct MainWindowRuntimeRoomSnapshot {
    pub(in crate::app) room_name: String,
    pub(in crate::app) is_controlled: bool,
    pub(in crate::app) has_named_users: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::app) struct MainWindowRuntimeChatSnapshot {
    pub(in crate::app) sender: String,
    pub(in crate::app) message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct MainWindowRuntimeSnapshot {
    pub(in crate::app) room_name: String,
    pub(in crate::app) room_control_status: String,
    pub(in crate::app) shared_playlist_enabled: bool,
    pub(in crate::app) controlled_room_active: bool,
    pub(in crate::app) hide_empty_rooms: bool,
    pub(in crate::app) rooms: Vec<MainWindowRuntimeRoomSnapshot>,
    pub(in crate::app) users: Vec<MainWindowRuntimeUserSnapshot>,
    pub(in crate::app) playlist: Vec<String>,
    pub(in crate::app) playlist_source_states: Vec<GuiPlaylistSourceState>,
    pub(in crate::app) active_playlist_index: Option<usize>,
    pub(in crate::app) chat: Vec<MainWindowRuntimeChatSnapshot>,
    pub(in crate::app) can_toggle_pause: bool,
    pub(in crate::app) can_seek: bool,
    pub(in crate::app) can_undo_seek: bool,
    pub(in crate::app) can_set_offset: bool,
    pub(in crate::app) can_toggle_autoplay: bool,
    pub(in crate::app) can_adjust_autoplay_threshold: bool,
    pub(in crate::app) can_set_ready: bool,
    pub(in crate::app) can_set_others_ready: bool,
    pub(in crate::app) can_manage_playlist: bool,
    pub(in crate::app) playback_paused: bool,
    pub(in crate::app) autoplay_active: bool,
    pub(in crate::app) autoplay_threshold: usize,
    pub(in crate::app) autoplay_countdown_seconds: Option<u32>,
    pub(in crate::app) user_offset_seconds: f64,
    pub(in crate::app) show_playback_buttons: bool,
    pub(in crate::app) show_autoplay_controls: bool,
}

impl Default for MainWindowRuntimeSnapshot {
    fn default() -> Self {
        Self {
            room_name: String::new(),
            room_control_status: String::new(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            users: Vec::new(),
            playlist: Vec::new(),
            playlist_source_states: Vec::new(),
            active_playlist_index: None,
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_undo_seek: false,
            can_set_offset: false,
            can_toggle_autoplay: true,
            can_adjust_autoplay_threshold: true,
            can_set_ready: false,
            can_set_others_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            autoplay_threshold: DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD,
            autoplay_countdown_seconds: None,
            user_offset_seconds: 0.0,
            show_playback_buttons: true,
            show_autoplay_controls: true,
        }
    }
}

impl MainWindowRuntimeSnapshot {
    pub(in crate::app) fn from_shell_state(state: &MainWindowShellState) -> Self {
        Self {
            room_name: state.room_name.clone(),
            room_control_status: state.room_control_status.clone(),
            shared_playlist_enabled: state.shared_playlist_enabled,
            controlled_room_active: state.controlled_room_active,
            hide_empty_rooms: state.hide_empty_rooms,
            rooms: state
                .rooms
                .iter()
                .map(|room| MainWindowRuntimeRoomSnapshot {
                    room_name: room.room_name.clone(),
                    is_controlled: room.is_controlled,
                    has_named_users: room.has_named_users,
                })
                .collect(),
            users: state
                .users
                .iter()
                .map(|user| MainWindowRuntimeUserSnapshot {
                    username: user.username.clone(),
                    room_name: user.room_name.clone(),
                    is_self: user.is_self,
                    is_ready: user.is_ready,
                    is_controller: user.is_controller,
                    has_file: user.has_file,
                    file_name: user.file_name.clone(),
                    file_size_label: user.file_size_label.clone(),
                    file_duration_label: user.file_duration_label.clone(),
                    file_is_url: user.file_is_url,
                    file_is_trusted: user.file_is_trusted,
                    filename_differs: user.filename_differs,
                    filesize_differs: user.filesize_differs,
                    fileduration_differs: user.fileduration_differs,
                })
                .collect(),
            playlist: state.playlist.iter().map(|row| row.label.clone()).collect(),
            playlist_source_states: state
                .playlist
                .iter()
                .map(|row| row.source_state.clone())
                .collect(),
            active_playlist_index: state.active_playlist_index,
            chat: state
                .chat
                .iter()
                .map(|row| MainWindowRuntimeChatSnapshot {
                    sender: row.sender.clone(),
                    message: row.message.clone(),
                })
                .collect(),
            can_toggle_pause: state.playback.can_toggle_pause,
            can_seek: state.playback.can_seek,
            can_undo_seek: state.playback.can_undo_seek,
            can_set_offset: state.playback.can_set_offset,
            can_toggle_autoplay: state.playback.can_toggle_autoplay,
            can_adjust_autoplay_threshold: state.playback.can_adjust_autoplay_threshold,
            can_set_ready: state.playback.can_set_ready,
            can_set_others_ready: state.playback.can_set_others_ready,
            can_manage_playlist: state.playback.can_manage_playlist,
            playback_paused: state.playback_paused,
            autoplay_active: state.autoplay_active,
            autoplay_threshold: state.autoplay_threshold,
            autoplay_countdown_seconds: state.autoplay_countdown_seconds,
            user_offset_seconds: state.user_offset_seconds,
            show_playback_buttons: state.show_playback_buttons,
            show_autoplay_controls: state.show_autoplay_controls,
        }
    }
}
