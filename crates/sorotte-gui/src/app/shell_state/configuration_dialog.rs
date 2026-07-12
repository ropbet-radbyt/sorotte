use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
use sorotte_secret::SecretValue;

use super::super::GuiLaunchMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiConnectionSettingsSection {
    pub(in crate::app) host: Option<String>,
    pub(in crate::app) port: Option<u16>,
    pub(in crate::app) username: Option<String>,
    pub(in crate::app) room: Option<String>,
    pub(in crate::app) server_password_set: bool,
    pub(in crate::app) player_path: Option<String>,
    pub(in crate::app) player_arguments_text: String,
    pub(in crate::app) room_history_text: String,
    pub(in crate::app) public_server_count: usize,
    pub(in crate::app) room_history_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiReadinessSection {
    pub(in crate::app) ready_at_start: bool,
    pub(in crate::app) autoplay_enabled: bool,
    pub(in crate::app) autoplay_require_same_filenames: bool,
    pub(in crate::app) shared_playlist_enabled: bool,
    pub(in crate::app) pause_on_leave: bool,
    pub(in crate::app) loop_at_end_of_playlist: bool,
    pub(in crate::app) loop_single_files: bool,
    pub(in crate::app) unpause_action_label: String,
    pub(in crate::app) autoplay_min_users_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiPrivacySection {
    pub(in crate::app) filename_privacy_mode_label: String,
    pub(in crate::app) filesize_privacy_mode_label: String,
    pub(in crate::app) only_switch_to_trusted_domains: bool,
    pub(in crate::app) trusted_domains_text: String,
    pub(in crate::app) trusted_domain_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiDesyncSection {
    pub(in crate::app) rewind_on_desync: bool,
    pub(in crate::app) fastforward_on_desync: bool,
    pub(in crate::app) slow_on_desync: bool,
    pub(in crate::app) dont_slow_down_with_me: bool,
    pub(in crate::app) rewind_threshold_seconds: Option<f64>,
    pub(in crate::app) fastforward_threshold_seconds: Option<f64>,
    pub(in crate::app) slowdown_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiStreamingSection {
    pub(in crate::app) quality_label: String,
    pub(in crate::app) custom_format: Option<String>,
    pub(in crate::app) buffer_target_seconds: f64,
    pub(in crate::app) read_ahead_seconds: f64,
    pub(in crate::app) memory_cache_mebibytes: u64,
    pub(in crate::app) disk_cache_enabled: bool,
    pub(in crate::app) recovery_policy_label: String,
    pub(in crate::app) maximum_catchup_rate: f64,
    pub(in crate::app) hard_seek_threshold_seconds: f64,
    pub(in crate::app) maximum_hard_seeks: u32,
    pub(in crate::app) stability_interval_seconds: f64,
    pub(in crate::app) retry_budget: u32,
    pub(in crate::app) recovery_cooldown_seconds: f64,
    pub(in crate::app) room_buffering_policy_label: String,
    pub(in crate::app) room_quorum_percent: f64,
    pub(in crate::app) room_maximum_pause_seconds: f64,
    pub(in crate::app) start_policy_label: String,
    pub(in crate::app) start_quorum_percent: f64,
    pub(in crate::app) start_timeout_seconds: f64,
    pub(in crate::app) start_timeout_action_label: String,
    pub(in crate::app) quality_downgrade_suggestions: bool,
    pub(in crate::app) effective_mpv_options: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiMediaSearchSection {
    pub(in crate::app) media_directories_text: String,
    pub(in crate::app) media_directory_count: usize,
    pub(in crate::app) folder_search_first_file_timeout_seconds: Option<f64>,
    pub(in crate::app) folder_search_timeout_seconds: Option<f64>,
    pub(in crate::app) folder_search_double_check_interval_seconds: Option<f64>,
    pub(in crate::app) folder_search_warning_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiChatSection {
    pub(in crate::app) chat_input_enabled: bool,
    pub(in crate::app) chat_output_enabled: bool,
    pub(in crate::app) chat_direct_input: bool,
    pub(in crate::app) chat_move_osd: bool,
    pub(in crate::app) chat_max_lines: Option<i64>,
    pub(in crate::app) chat_input_position_label: String,
    pub(in crate::app) chat_input_font_family: Option<String>,
    pub(in crate::app) chat_input_relative_font_size: Option<i64>,
    pub(in crate::app) chat_input_font_weight: Option<i64>,
    pub(in crate::app) chat_input_font_color: Option<String>,
    pub(in crate::app) chat_output_mode_label: String,
    pub(in crate::app) chat_output_font_family: Option<String>,
    pub(in crate::app) chat_output_relative_font_size: Option<i64>,
    pub(in crate::app) chat_output_font_weight: Option<i64>,
    pub(in crate::app) chat_top_margin: Option<i64>,
    pub(in crate::app) chat_left_margin: Option<i64>,
    pub(in crate::app) chat_bottom_margin: Option<i64>,
    pub(in crate::app) chat_osd_margin: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiOsdSection {
    pub(in crate::app) show_osd: bool,
    pub(in crate::app) show_duration_notification: bool,
    pub(in crate::app) show_same_room_osd: bool,
    pub(in crate::app) show_osd_warnings: bool,
    pub(in crate::app) show_slowdown_osd: bool,
    pub(in crate::app) show_noncontroller_osd: bool,
    pub(in crate::app) show_different_room_osd: bool,
    pub(in crate::app) show_contact_info: bool,
    pub(in crate::app) notification_timeout_seconds: Option<i64>,
    pub(in crate::app) alert_timeout_seconds: Option<i64>,
    pub(in crate::app) chat_timeout_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiSystemSection {
    pub(in crate::app) language_tag: String,
    pub(in crate::app) check_for_updates_automatically: bool,
    pub(in crate::app) update_channel_label: String,
    pub(in crate::app) autosave_joins_to_list: bool,
    pub(in crate::app) force_gui_prompt: bool,
    pub(in crate::app) compatibility_startup_entry_count: usize,
    pub(in crate::app) ignored_startup_exception_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiDialogControlKind {
    TextInput,
    TextArea,
    PasswordInput,
    Checkbox,
    Select,
    NumericInput,
    ReadOnly,
}

impl GuiDialogControlKind {
    #[cfg(test)]
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::TextInput => "text",
            Self::TextArea => "textarea",
            Self::PasswordInput => "password",
            Self::Checkbox => "checkbox",
            Self::Select => "select",
            Self::NumericInput => "numeric",
            Self::ReadOnly => "readonly",
        }
    }

    pub(in crate::app) fn is_editable(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiDialogControl {
    pub(in crate::app) label: &'static str,
    pub(in crate::app) kind: GuiDialogControlKind,
    pub(in crate::app) value: String,
}

impl std::fmt::Debug for GuiDialogControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = if self.kind == GuiDialogControlKind::PasswordInput {
            sorotte_secret::REDACTED_SECRET
        } else {
            &self.value
        };
        formatter
            .debug_struct("GuiDialogControl")
            .field("label", &self.label)
            .field("kind", &self.kind)
            .field("value", &value)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum GuiConfigurationTextValue {
    Plain(String),
    Secret(SecretValue),
}

impl GuiConfigurationTextValue {
    pub(in crate::app) fn for_control(
        kind: GuiDialogControlKind,
        value: impl Into<String>,
    ) -> Self {
        let value = value.into();
        if kind == GuiDialogControlKind::PasswordInput {
            Self::Secret(value.into())
        } else {
            Self::Plain(value)
        }
    }

    pub(in crate::app) fn expose_for_ui(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::Secret(value) => value.expose_secret(),
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn as_str(&self) -> &str {
        self.expose_for_ui()
    }

    pub(in crate::app) fn expose_for_config_apply(&self) -> &str {
        self.expose_for_ui()
    }
}

impl std::fmt::Display for GuiConfigurationTextValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain(value) => formatter.write_str(value),
            Self::Secret(_) => formatter.write_str(sorotte_secret::REDACTED_SECRET),
        }
    }
}

impl From<String> for GuiConfigurationTextValue {
    fn from(value: String) -> Self {
        Self::Plain(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiDialogSection {
    pub(in crate::app) title: &'static str,
    pub(in crate::app) controls: Vec<GuiDialogControl>,
}

impl GuiDialogSection {
    pub(in crate::app) fn control_mut(&mut self, label: &str) -> Option<&mut GuiDialogControl> {
        self.controls
            .iter_mut()
            .find(|control| control.label == label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct FirstRunConfigurationDialogState {
    pub(in crate::app) launch_mode: GuiLaunchMode,
    pub(in crate::app) connection: GuiConnectionSettingsSection,
    pub(in crate::app) readiness: GuiReadinessSection,
    pub(in crate::app) privacy: GuiPrivacySection,
    pub(in crate::app) desync: GuiDesyncSection,
    pub(in crate::app) streaming: GuiStreamingSection,
    pub(in crate::app) media_search: GuiMediaSearchSection,
    pub(in crate::app) chat: GuiChatSection,
    pub(in crate::app) osd: GuiOsdSection,
    pub(in crate::app) system: GuiSystemSection,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct FirstRunConfigurationDialogDraft {
    pub(in crate::app) launch_mode: GuiLaunchMode,
    pub(in crate::app) compatibility_startup_entry_count: usize,
    pub(in crate::app) ignored_startup_exception_count: usize,
    pub(in crate::app) sections: Vec<GuiDialogSection>,
    pub(in crate::app) settings: StoredClientSettingsMvp,
}
