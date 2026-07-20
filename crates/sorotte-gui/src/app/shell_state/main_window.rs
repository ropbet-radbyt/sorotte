use super::super::DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD;
use sorotte_client_app::app_boundary::readiness::ParticipantReadinessPresentation;
use std::collections::BTreeMap;

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::app) struct GuiPlaylistDefaultSourceId(Option<GuiMediaSourceProviderId>);

impl GuiPlaylistDefaultSourceId {
    pub(in crate::app) fn automatic() -> Self {
        Self(None)
    }

    pub(in crate::app) fn provider(provider_id: GuiMediaSourceProviderId) -> Self {
        Self(Some(provider_id))
    }

    pub(in crate::app) fn from_action_id(value: &str) -> Self {
        if value == "automatic" {
            Self::automatic()
        } else {
            Self::provider(GuiMediaSourceProviderId::new(value.to_owned()))
        }
    }

    pub(in crate::app) fn as_action_id(&self) -> &str {
        self.0
            .as_ref()
            .map(GuiMediaSourceProviderId::as_str)
            .unwrap_or("automatic")
    }

    pub(in crate::app) fn provider_id(&self) -> Option<&GuiMediaSourceProviderId> {
        self.0.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiPlaylistSourceStatus {
    Available,
    Active,
    Resolving,
    Loading,
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
            Self::Loading => "loading",
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
pub(in crate::app) struct GuiPlaylistDefaultSourceOption {
    pub(in crate::app) source_id: GuiPlaylistDefaultSourceId,
    pub(in crate::app) label: String,
    pub(in crate::app) status: GuiPlaylistSourceStatus,
    pub(in crate::app) detail: Option<String>,
    pub(in crate::app) enabled: bool,
    pub(in crate::app) selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPlaylistDefaultSourceState {
    pub(in crate::app) current_source_id: GuiPlaylistDefaultSourceId,
    pub(in crate::app) current_label: String,
    pub(in crate::app) options: Vec<GuiPlaylistDefaultSourceOption>,
}

impl Default for GuiPlaylistDefaultSourceState {
    fn default() -> Self {
        Self {
            current_source_id: GuiPlaylistDefaultSourceId::automatic(),
            current_label: "Automatic".to_owned(),
            options: default_playlist_source_default_options(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPlaylistResolutionStep {
    pub(in crate::app) provider_id: GuiMediaSourceProviderId,
    pub(in crate::app) label: String,
    pub(in crate::app) status: GuiPlaylistSourceStatus,
    pub(in crate::app) detail: Option<String>,
}

impl std::fmt::Debug for GuiPlaylistResolutionStep {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlaylistResolutionStep")
            .field("provider_id", &self.provider_id)
            .field("label", &self.label)
            .field("status", &self.status)
            .field(
                "detail",
                &self
                    .detail
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .finish()
    }
}

#[derive(Clone)]
pub(in crate::app) struct GuiPlaylistSourceState {
    pub(in crate::app) entry_id: GuiPlaylistEntryId,
    /// The provider selected by an explicit row/default policy. Automatic rows
    /// intentionally have no preferred provider.
    pub(in crate::app) preferred_provider_id: Option<GuiMediaSourceProviderId>,
    /// The provider that produced the current resolution/load attempt.
    pub(in crate::app) resolved_provider_id: Option<GuiMediaSourceProviderId>,
    /// Compatibility/presentation alias for the provider currently shown in
    /// the row. Resolution updates this without changing `policy`.
    pub(in crate::app) current_provider_id: GuiMediaSourceProviderId,
    pub(in crate::app) current_label: String,
    pub(in crate::app) policy: GuiPlaylistSourcePolicy,
    pub(in crate::app) selection_origin: GuiPlaylistSourceSelectionOrigin,
    pub(in crate::app) status: GuiPlaylistSourceStatus,
    pub(in crate::app) detail: Option<String>,
    pub(in crate::app) options: Vec<GuiPlaylistSourceOption>,
    pub(in crate::app) resolution_steps: Vec<GuiPlaylistResolutionStep>,
}

impl PartialEq for GuiPlaylistSourceState {
    fn eq(&self, other: &Self) -> bool {
        self.preferred_provider_id == other.preferred_provider_id
            && self.resolved_provider_id == other.resolved_provider_id
            && self.current_provider_id == other.current_provider_id
            && self.current_label == other.current_label
            && self.policy == other.policy
            && self.selection_origin == other.selection_origin
            && self.status == other.status
            && self.detail == other.detail
            && self.options == other.options
            && self.resolution_steps == other.resolution_steps
    }
}

impl Eq for GuiPlaylistSourceState {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiPlaylistSourcePolicy {
    Automatic,
    ForceLocal,
    PreferMediaMatching,
    ForceMediaMatching,
    ForcePlex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiPlaylistSourceSelectionOrigin {
    Inferred,
    PlaylistDefault,
    UserOverride,
}

struct GuiPlaylistSourceOptionDebug<'a>(&'a GuiPlaylistSourceOption);

impl std::fmt::Debug for GuiPlaylistSourceOptionDebug<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlaylistSourceOption")
            .field("provider_id", &self.0.provider_id)
            .field("label", &self.0.label)
            .field("status", &self.0.status)
            .field(
                "detail",
                &self
                    .0
                    .detail
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("enabled", &self.0.enabled)
            .field("selected", &self.0.selected)
            .finish()
    }
}

impl std::fmt::Debug for GuiPlaylistSourceState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let options = self
            .options
            .iter()
            .map(GuiPlaylistSourceOptionDebug)
            .collect::<Vec<_>>();

        formatter
            .debug_struct("GuiPlaylistSourceState")
            .field("entry_id", &self.entry_id)
            .field("preferred_provider_id", &self.preferred_provider_id)
            .field("resolved_provider_id", &self.resolved_provider_id)
            .field("current_provider_id", &self.current_provider_id)
            .field("current_label", &self.current_label)
            .field("policy", &self.policy)
            .field("selection_origin", &self.selection_origin)
            .field("status", &self.status)
            .field(
                "detail",
                &self
                    .detail
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("options", &options)
            .field("resolution_steps", &self.resolution_steps)
            .finish()
    }
}

impl GuiPlaylistSourceState {
    pub(in crate::app) fn inferred_for_entry(entry: &str) -> Self {
        Self::new(
            Self::inferred_provider_for_entry(entry),
            GuiPlaylistSourcePolicy::Automatic,
            GuiPlaylistSourceSelectionOrigin::Inferred,
        )
    }

    pub(in crate::app) fn inferred_provider_for_entry(entry: &str) -> GuiMediaSourceProviderId {
        if sorotte_plex::is_plex_playlist_uri(entry) {
            GuiMediaSourceProviderId::plex_stream()
        } else {
            GuiMediaSourceProviderId::local()
        }
    }

    pub(in crate::app) fn for_provider(provider_id: GuiMediaSourceProviderId) -> Self {
        let policy = Self::forced_policy_for_provider(&provider_id);
        Self::new(
            provider_id,
            policy,
            GuiPlaylistSourceSelectionOrigin::UserOverride,
        )
    }

    pub(in crate::app) fn for_playlist_default(provider_id: GuiMediaSourceProviderId) -> Self {
        let policy = if provider_id == GuiMediaSourceProviderId::media_matching() {
            GuiPlaylistSourcePolicy::PreferMediaMatching
        } else {
            Self::forced_policy_for_provider(&provider_id)
        };
        Self::new(
            provider_id,
            policy,
            GuiPlaylistSourceSelectionOrigin::PlaylistDefault,
        )
    }

    fn forced_policy_for_provider(
        provider_id: &GuiMediaSourceProviderId,
    ) -> GuiPlaylistSourcePolicy {
        if provider_id == &GuiMediaSourceProviderId::plex_stream() {
            GuiPlaylistSourcePolicy::ForcePlex
        } else if provider_id == &GuiMediaSourceProviderId::media_matching() {
            GuiPlaylistSourcePolicy::ForceMediaMatching
        } else {
            GuiPlaylistSourcePolicy::ForceLocal
        }
    }

    fn new(
        provider_id: GuiMediaSourceProviderId,
        policy: GuiPlaylistSourcePolicy,
        selection_origin: GuiPlaylistSourceSelectionOrigin,
    ) -> Self {
        let current_label = playlist_source_provider_label(&provider_id).to_owned();
        Self {
            entry_id: GuiPlaylistEntryId::next(),
            preferred_provider_id: (!matches!(policy, GuiPlaylistSourcePolicy::Automatic))
                .then_some(provider_id.clone()),
            resolved_provider_id: None,
            current_provider_id: provider_id.clone(),
            current_label,
            policy,
            selection_origin,
            status: GuiPlaylistSourceStatus::Available,
            detail: Some("Waiting for playlist activation.".to_owned()),
            options: default_playlist_source_options(&provider_id),
            resolution_steps: Vec::new(),
        }
    }

    pub(in crate::app) fn preferred_provider_id(&self) -> Option<&GuiMediaSourceProviderId> {
        self.preferred_provider_id.as_ref()
    }

    pub(in crate::app) fn set_resolved_provider(&mut self, provider_id: GuiMediaSourceProviderId) {
        self.resolved_provider_id = Some(provider_id.clone());
        self.current_provider_id = provider_id.clone();
        self.current_label = playlist_source_provider_label(&provider_id).to_owned();
        for option in &mut self.options {
            option.selected = option.provider_id == provider_id;
        }
    }

    pub(in crate::app) fn clear_resolved_provider(&mut self) {
        self.resolved_provider_id = None;
        if self.policy == GuiPlaylistSourcePolicy::Automatic {
            self.current_label = "Automatic".to_owned();
            for option in &mut self.options {
                option.selected = false;
            }
        }
    }
}

fn playlist_source_provider_label(provider_id: &GuiMediaSourceProviderId) -> &'static str {
    if provider_id == &GuiMediaSourceProviderId::plex_stream() {
        "Plex Stream"
    } else if provider_id == &GuiMediaSourceProviderId::media_matching() {
        "Media Matching"
    } else {
        "Local"
    }
}

fn default_playlist_source_default_options() -> Vec<GuiPlaylistDefaultSourceOption> {
    [
        (GuiPlaylistDefaultSourceId::automatic(), "Automatic"),
        (
            GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::local()),
            "Local",
        ),
        (
            GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::media_matching()),
            "Media Matching",
        ),
        (
            GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::plex_stream()),
            "Plex Stream",
        ),
    ]
    .into_iter()
    .map(|(source_id, label)| GuiPlaylistDefaultSourceOption {
        selected: source_id == GuiPlaylistDefaultSourceId::automatic(),
        source_id,
        label: label.to_owned(),
        status: GuiPlaylistSourceStatus::Available,
        detail: None,
        enabled: true,
    })
    .collect()
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

#[derive(Clone, PartialEq, Eq)]
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

impl std::fmt::Debug for MainWindowUserRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowUserRow")
            .field("username", &self.username)
            .field("room_name", &self.room_name)
            .field("is_self", &self.is_self)
            .field("is_ready", &self.is_ready)
            .field("is_controller", &self.is_controller)
            .field("has_file", &self.has_file)
            .field(
                "file_name",
                &self
                    .file_name
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("file_name_label", &sorotte_secret::REDACTED_SECRET)
            .field("file_size_label", &self.file_size_label)
            .field("file_duration_label", &self.file_duration_label)
            .field("file_is_url", &self.file_is_url)
            .field("file_is_trusted", &self.file_is_trusted)
            .field("filename_differs", &self.filename_differs)
            .field("filesize_differs", &self.filesize_differs)
            .field("fileduration_differs", &self.fileduration_differs)
            .field("is_selected", &self.is_selected)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(in crate::app) struct GuiPlaylistEntryId(u64);

impl GuiPlaylistEntryId {
    pub(in crate::app) fn next() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

#[derive(Clone)]
pub(in crate::app) struct MainWindowPlaylistRow {
    pub(in crate::app) entry_id: GuiPlaylistEntryId,
    pub(in crate::app) label: String,
    pub(in crate::app) is_selected: bool,
    pub(in crate::app) source_state: GuiPlaylistSourceState,
}

impl PartialEq for MainWindowPlaylistRow {
    fn eq(&self, other: &Self) -> bool {
        self.entry_id == other.entry_id
            && self.label == other.label
            && self.is_selected == other.is_selected
            && self.source_state == other.source_state
    }
}

impl Eq for MainWindowPlaylistRow {}

impl std::fmt::Debug for MainWindowPlaylistRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowPlaylistRow")
            .field("entry_id", &self.entry_id)
            .field("label", &sorotte_secret::REDACTED_SECRET)
            .field("is_selected", &self.is_selected)
            .field("source_state", &self.source_state)
            .finish()
    }
}

impl MainWindowPlaylistRow {
    pub(in crate::app) fn inferred(label: impl Into<String>, is_selected: bool) -> Self {
        let label = label.into();
        let source_state = GuiPlaylistSourceState::inferred_for_entry(&label);
        Self {
            entry_id: source_state.entry_id,
            source_state,
            label,
            is_selected,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct MainWindowChatRow {
    pub(in crate::app) sender: String,
    pub(in crate::app) message: String,
}

impl std::fmt::Debug for MainWindowChatRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowChatRow")
            .field("sender", &self.sender)
            .field("message", &sorotte_secret::REDACTED_SECRET)
            .finish()
    }
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

#[derive(Clone, PartialEq)]
pub(in crate::app) struct MainWindowShellState {
    pub(in crate::app) room_name: String,
    pub(in crate::app) room_control_status: String,
    pub(in crate::app) shared_playlist_enabled: bool,
    pub(in crate::app) controlled_room_active: bool,
    pub(in crate::app) hide_empty_rooms: bool,
    pub(in crate::app) rooms: Vec<MainWindowRoomRow>,
    pub(in crate::app) users: Vec<MainWindowUserRow>,
    pub(in crate::app) readiness: BTreeMap<String, ParticipantReadinessPresentation>,
    pub(in crate::app) playlist: Vec<MainWindowPlaylistRow>,
    pub(in crate::app) playlist_default_source: GuiPlaylistDefaultSourceState,
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

impl std::fmt::Debug for MainWindowShellState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowShellState")
            .field("room_name", &self.room_name)
            .field("room_control_status", &self.room_control_status)
            .field("shared_playlist_enabled", &self.shared_playlist_enabled)
            .field("controlled_room_active", &self.controlled_room_active)
            .field("hide_empty_rooms", &self.hide_empty_rooms)
            .field("room_count", &self.rooms.len())
            .field("users", &self.users)
            .field("readiness", &self.readiness)
            .field("playlist", &self.playlist)
            .field("active_playlist_index", &self.active_playlist_index)
            .field("chat", &self.chat)
            .field("playback", &self.playback)
            .field("playback_paused", &self.playback_paused)
            .field("autoplay_active", &self.autoplay_active)
            .field("autoplay_threshold", &self.autoplay_threshold)
            .field(
                "autoplay_countdown_seconds",
                &self.autoplay_countdown_seconds,
            )
            .field("user_offset_seconds", &self.user_offset_seconds)
            .field("show_playback_buttons", &self.show_playback_buttons)
            .field("show_autoplay_controls", &self.show_autoplay_controls)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
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

impl std::fmt::Debug for MainWindowRuntimeUserSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowRuntimeUserSnapshot")
            .field("username", &self.username)
            .field("room_name", &self.room_name)
            .field("is_self", &self.is_self)
            .field("is_ready", &self.is_ready)
            .field("is_controller", &self.is_controller)
            .field("has_file", &self.has_file)
            .field(
                "file_name",
                &self
                    .file_name
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("file_size_label", &self.file_size_label)
            .field("file_duration_label", &self.file_duration_label)
            .field("file_is_url", &self.file_is_url)
            .field("file_is_trusted", &self.file_is_trusted)
            .field("filename_differs", &self.filename_differs)
            .field("filesize_differs", &self.filesize_differs)
            .field("fileduration_differs", &self.fileduration_differs)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::app) struct MainWindowRuntimeRoomSnapshot {
    pub(in crate::app) room_name: String,
    pub(in crate::app) is_controlled: bool,
    pub(in crate::app) has_named_users: bool,
}

#[derive(Clone, PartialEq, Eq, Default)]
pub(in crate::app) struct MainWindowRuntimeChatSnapshot {
    pub(in crate::app) sender: String,
    pub(in crate::app) message: String,
}

impl std::fmt::Debug for MainWindowRuntimeChatSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowRuntimeChatSnapshot")
            .field("sender", &self.sender)
            .field("message", &sorotte_secret::REDACTED_SECRET)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::app) struct MainWindowRuntimeSnapshot {
    pub(in crate::app) room_name: String,
    pub(in crate::app) room_control_status: String,
    pub(in crate::app) shared_playlist_enabled: bool,
    pub(in crate::app) controlled_room_active: bool,
    pub(in crate::app) hide_empty_rooms: bool,
    pub(in crate::app) rooms: Vec<MainWindowRuntimeRoomSnapshot>,
    pub(in crate::app) users: Vec<MainWindowRuntimeUserSnapshot>,
    pub(in crate::app) readiness: BTreeMap<String, ParticipantReadinessPresentation>,
    pub(in crate::app) playlist: Vec<String>,
    pub(in crate::app) playlist_entry_ids: Vec<GuiPlaylistEntryId>,
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

impl std::fmt::Debug for MainWindowRuntimeSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindowRuntimeSnapshot")
            .field("room_name", &self.room_name)
            .field("room_control_status", &self.room_control_status)
            .field("shared_playlist_enabled", &self.shared_playlist_enabled)
            .field("controlled_room_active", &self.controlled_room_active)
            .field("hide_empty_rooms", &self.hide_empty_rooms)
            .field("rooms", &self.rooms)
            .field("users", &self.users)
            .field("readiness", &self.readiness)
            .field("playlist_count", &self.playlist.len())
            .field("playlist_entry_id_count", &self.playlist_entry_ids.len())
            .field("playlist_source_states", &self.playlist_source_states)
            .field("active_playlist_index", &self.active_playlist_index)
            .field("chat", &self.chat)
            .field("can_toggle_pause", &self.can_toggle_pause)
            .field("can_seek", &self.can_seek)
            .field("can_undo_seek", &self.can_undo_seek)
            .field("can_set_offset", &self.can_set_offset)
            .field("can_toggle_autoplay", &self.can_toggle_autoplay)
            .field(
                "can_adjust_autoplay_threshold",
                &self.can_adjust_autoplay_threshold,
            )
            .field("can_set_ready", &self.can_set_ready)
            .field("can_set_others_ready", &self.can_set_others_ready)
            .field("can_manage_playlist", &self.can_manage_playlist)
            .field("playback_paused", &self.playback_paused)
            .field("autoplay_active", &self.autoplay_active)
            .field("autoplay_threshold", &self.autoplay_threshold)
            .field(
                "autoplay_countdown_seconds",
                &self.autoplay_countdown_seconds,
            )
            .field("user_offset_seconds", &self.user_offset_seconds)
            .field("show_playback_buttons", &self.show_playback_buttons)
            .field("show_autoplay_controls", &self.show_autoplay_controls)
            .finish()
    }
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
            readiness: BTreeMap::new(),
            playlist: Vec::new(),
            playlist_entry_ids: Vec::new(),
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
    pub(in crate::app) fn matches_shell_state_with_omitted_playlist_metadata(
        &self,
        state: &MainWindowShellState,
    ) -> bool {
        let mut current = Self::from_shell_state(state);
        if self.playlist_entry_ids.is_empty() {
            current.playlist_entry_ids.clear();
        }
        if self.playlist_source_states.is_empty() {
            current.playlist_source_states.clear();
        }
        self == &current
    }

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
            readiness: state.readiness.clone(),
            playlist: state.playlist.iter().map(|row| row.label.clone()).collect(),
            playlist_entry_ids: state.playlist.iter().map(|row| row.entry_id).collect(),
            playlist_source_states: state
                .playlist
                .iter()
                .map(|row| {
                    let mut source_state = row.source_state.clone();
                    source_state.entry_id = row.entry_id;
                    source_state
                })
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
