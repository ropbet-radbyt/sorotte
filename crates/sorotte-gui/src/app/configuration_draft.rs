use sorotte_client_app::app_boundary::{
    language::normalized_legacy_runtime_language_tag_legacy_compatible,
    state::{
        StoredClientSettingsMvp, parse_autoplay_min_users_override_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible,
    },
};
use sorotte_client_core::PrivacyMode;

use super::shell_state::{
    FirstRunConfigurationDialogDraft, FirstRunConfigurationDialogState, GuiDialogControl,
    GuiDialogControlKind, GuiSettingApplyRequirement, SecretDraft, SettingId,
    SynchronizationProfileId,
};
use super::support::{
    bool_label, configured_room_name_text, normalized_editable_text,
    optional_string_list_multiline_text, parse_editable_string_list_text, parse_room_history_text,
    set_player_arguments_text_for_path,
};

#[cfg(test)]
mod tests;

impl FirstRunConfigurationDialogDraft {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let state = FirstRunConfigurationDialogState::from_stored_settings(settings);
        let sections = state.dialog_sections();
        debug_assert_eq!(
            sections
                .iter()
                .map(|section| section.controls.len())
                .sum::<usize>(),
            SettingId::ALL.len(),
            "every setting identity must have exactly one projected control",
        );
        Self {
            launch_mode: state.launch_mode,
            compatibility_startup_entry_count: state.system.compatibility_startup_entry_count,
            ignored_startup_exception_count: state.system.ignored_startup_exception_count,
            sections,
            settings: settings.clone(),
            server_password: SecretDraft::Unchanged,
        }
    }

    pub(super) fn to_stored_settings(&self) -> StoredClientSettingsMvp {
        let mut settings = self.settings.clone();
        match &self.server_password {
            SecretDraft::Unchanged => {}
            SecretDraft::Replace(value) => {
                if !value.expose_secret().trim().is_empty() {
                    settings.server_password = Some(value.clone());
                }
            }
            SecretDraft::Clear => settings.server_password = None,
        }
        settings
    }

    pub(super) fn has_unsaved_changes_against(&self, settings: &StoredClientSettingsMvp) -> bool {
        self.to_stored_settings() != *settings
            || self.sections != Self::from_stored_settings(settings).sections
    }

    pub(super) fn changed_setting_ids_against(
        &self,
        settings: &StoredClientSettingsMvp,
    ) -> Vec<SettingId> {
        let persisted = Self::from_stored_settings(settings);
        let mut changed = SettingId::ALL
            .iter()
            .copied()
            .filter(|id| {
                if *id == SettingId::ConnectionServerPassword {
                    return !matches!(self.server_password, SecretDraft::Unchanged);
                }
                self.control_value(*id) != persisted.control_value(*id)
            })
            .collect::<Vec<_>>();

        // Public-server rows are edited through their own workflow. Count-only controls do not
        // detect a same-length replacement, so lock that typed change to the public-server ID.
        if self.settings.public_servers != settings.public_servers
            && !changed.contains(&SettingId::ConnectionPublicServerCount)
        {
            changed.push(SettingId::ConnectionPublicServerCount);
        }
        changed.sort_unstable();
        changed.dedup();
        changed
    }

    pub(super) fn apply_synchronization_profile(&mut self, profile: SynchronizationProfileId) {
        let mut settings = self.to_stored_settings();
        profile.apply_to(&mut settings);
        let profile_values = Self::from_stored_settings(&settings);

        for id in [
            SettingId::SyncRewindOnDesync,
            SettingId::SyncFastforwardOnDesync,
            SettingId::SyncSlowOnDesync,
            SettingId::SyncDontSlowDownWithMe,
            SettingId::SyncRewindThreshold,
            SettingId::SyncFastforwardThreshold,
            SettingId::SyncSlowdownThreshold,
            SettingId::StreamingBufferTargetSeconds,
            SettingId::StreamingReadAheadSeconds,
            SettingId::StreamingMemoryCacheMib,
            SettingId::StreamingDiskCache,
            SettingId::StreamingRecoveryPolicy,
            SettingId::StreamingMaximumCatchupRate,
            SettingId::StreamingHardSeekThresholdSeconds,
            SettingId::StreamingMaximumHardSeeks,
            SettingId::StreamingStabilityIntervalSeconds,
            SettingId::StreamingRecoveryRetryBudget,
            SettingId::StreamingRecoveryCooldownSeconds,
            SettingId::StreamingRoomBufferingPolicy,
            SettingId::StreamingRoomQuorumPercent,
            SettingId::StreamingRoomMaximumPauseSeconds,
            SettingId::StreamingStartSynchronization,
            SettingId::StreamingStartQuorumPercent,
            SettingId::StreamingStartTimeoutSeconds,
            SettingId::StreamingStartTimeoutAction,
        ] {
            let control = profile_values
                .control(id)
                .expect("every synchronization profile setting must have a control");
            let applied = match control.kind {
                GuiDialogControlKind::Checkbox => self.apply_bool_value(id, control.value == "yes"),
                kind if kind.is_editable() => self.apply_text_value(id, &control.value),
                _ => false,
            };
            debug_assert!(applied, "profile setting {id:?} must be editable");
        }
    }

    pub(super) fn merge_apply_requirement_from_settings(
        baseline: &StoredClientSettingsMvp,
        source: &StoredClientSettingsMvp,
        requirement: GuiSettingApplyRequirement,
    ) -> StoredClientSettingsMvp {
        let mut merged = Self::from_stored_settings(baseline);
        let source = Self::from_stored_settings(source);
        for id in SettingId::ALL
            .iter()
            .copied()
            .filter(|id| id.apply_requirement() == requirement)
        {
            let Some(control) = source.control(id) else {
                continue;
            };
            match control.kind {
                GuiDialogControlKind::Checkbox => {
                    let _ = merged.apply_bool_value(id, control.value == "yes");
                }
                kind if kind.is_editable() => {
                    let _ = merged.apply_text_value(id, &control.value);
                }
                _ => {}
            }
        }
        if requirement == GuiSettingApplyRequirement::Reconnect {
            merged.settings.server_password = source.settings.server_password.clone();
        }
        merged.to_stored_settings()
    }

    pub(super) fn control(&self, id: SettingId) -> Option<&GuiDialogControl> {
        self.sections
            .iter()
            .flat_map(|section| &section.controls)
            .find(|control| control.id == id)
    }

    pub(super) fn control_value(&self, id: SettingId) -> Option<&str> {
        self.control(id).map(|control| control.value.as_str())
    }

    pub(super) fn control_identity(
        &self,
        automation_id: &str,
    ) -> Option<(SettingId, GuiDialogControlKind)> {
        let id = SettingId::from_automation_id(automation_id)?;
        self.control(id).map(|control| (id, control.kind))
    }

    pub(super) fn apply_text_value(&mut self, id: SettingId, value: &str) -> bool {
        let Some(kind) = self.control(id).map(|control| control.kind) else {
            return false;
        };
        if !kind.is_editable() || kind == GuiDialogControlKind::Checkbox {
            return false;
        }
        let Some(control) = self.control_mut(id) else {
            return false;
        };
        control.value = value.to_owned();
        self.apply_text_value_to_settings(id, value);
        self.refresh_derived_controls();
        true
    }

    pub(super) fn apply_bool_value(&mut self, id: SettingId, value: bool) -> bool {
        let Some(kind) = self.control(id).map(|control| control.kind) else {
            return false;
        };
        if kind != GuiDialogControlKind::Checkbox {
            return false;
        }
        let Some(control) = self.control_mut(id) else {
            return false;
        };
        control.value = bool_label(value).to_owned();
        self.apply_bool_value_to_settings(id, value);
        self.refresh_derived_controls();
        true
    }

    pub(super) fn room_history_multiline_text(&self) -> String {
        optional_string_list_multiline_text(self.settings.room_list.as_deref())
    }

    pub(super) fn apply_room_history_multiline_text(&mut self, value: &str) {
        self.settings.room_list = parse_room_history_text(value);
        self.refresh_derived_controls();
    }

    pub(super) fn begin_server_password_change(&mut self) {
        self.server_password = SecretDraft::Replace(String::new().into());
        if let Some(control) = self.control_mut(SettingId::ConnectionServerPassword) {
            control.value.clear();
        }
    }

    pub(super) fn remove_server_password(&mut self) {
        self.server_password = SecretDraft::Clear;
        if let Some(control) = self.control_mut(SettingId::ConnectionServerPassword) {
            control.value.clear();
        }
    }

    pub(super) fn cancel_server_password_change(&mut self) {
        self.server_password = SecretDraft::Unchanged;
        if let Some(control) = self.control_mut(SettingId::ConnectionServerPassword) {
            control.value.clear();
        }
    }

    pub(super) fn server_password_is_configured(&self) -> bool {
        self.to_stored_settings()
            .server_password
            .as_ref()
            .is_some_and(|value| !value.expose_secret().trim().is_empty())
    }

    fn control_mut(&mut self, id: SettingId) -> Option<&mut GuiDialogControl> {
        self.sections
            .iter_mut()
            .find_map(|section| section.control_mut(id))
    }

    fn apply_text_value_to_settings(&mut self, id: SettingId, value: &str) {
        match id {
            SettingId::ConnectionHost => {
                self.settings.host = normalized_editable_text(value);
            }
            SettingId::ConnectionPort => {
                self.settings.port = parse_optional_u16(value);
            }
            SettingId::ConnectionUsername => {
                self.settings.username = normalized_editable_text(value);
            }
            SettingId::ConnectionRoom => {
                self.settings.room = configured_room_name_text(value);
            }
            SettingId::ConnectionServerPassword => {
                self.server_password = SecretDraft::Replace(value.to_owned().into());
            }
            SettingId::PlayerExecutable => {
                self.settings.player_path = normalized_editable_text(value);
            }
            SettingId::PlayerArguments => {
                let player_path = self.settings.player_path.clone();
                set_player_arguments_text_for_path(
                    &mut self.settings.per_player_arguments,
                    player_path.as_deref(),
                    value,
                );
            }
            SettingId::ConnectionRoomHistory => {
                self.settings.room_list = parse_room_history_text(value);
            }
            SettingId::PlaybackUnpauseAction => {
                self.settings.unpause_action = normalized_editable_text(value)
                    .as_deref()
                    .and_then(parse_unpause_action_mode_legacy_compatible);
            }
            SettingId::PlaybackAutoplayMinUsers => {
                self.settings.autoplay_min_users = normalized_editable_text(value)
                    .as_deref()
                    .and_then(|value| {
                        if value.eq_ignore_ascii_case("app-default") {
                            None
                        } else {
                            parse_autoplay_min_users_override_legacy_compatible(value)
                        }
                    });
            }
            SettingId::PrivacyFilename => {
                self.settings.filename_privacy_mode = normalized_editable_text(value)
                    .as_deref()
                    .and_then(PrivacyMode::from_legacy_name);
            }
            SettingId::PrivacyFilesize => {
                self.settings.filesize_privacy_mode = normalized_editable_text(value)
                    .as_deref()
                    .and_then(PrivacyMode::from_legacy_name);
            }
            SettingId::PrivacyTrustedDomains => {
                self.settings.trusted_domains = parse_editable_string_list_text(value);
            }
            SettingId::SyncRewindThreshold => {
                self.settings.rewind_threshold_seconds = parse_optional_nonnegative_f64(value);
            }
            SettingId::SyncFastforwardThreshold => {
                self.settings.fastforward_threshold_seconds = parse_optional_nonnegative_f64(value);
            }
            SettingId::SyncSlowdownThreshold => {
                self.settings.slowdown_threshold_seconds = parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingQuality => {
                self.settings.streaming_quality_preset =
                    normalized_editable_text(value).map(|value| value.to_ascii_lowercase());
            }
            SettingId::StreamingCustomFormat => {
                self.settings.streaming_custom_format = normalized_editable_text(value);
            }
            SettingId::StreamingBufferTargetSeconds => {
                self.settings.streaming_buffer_target_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingReadAheadSeconds => {
                self.settings.streaming_read_ahead_seconds = parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingMemoryCacheMib => {
                self.settings.streaming_memory_cache_mebibytes = parse_optional_u64(value);
            }
            SettingId::StreamingRecoveryPolicy => {
                self.settings.streaming_recovery_policy =
                    normalized_editable_text(value).map(|value| value.to_ascii_lowercase());
            }
            SettingId::StreamingMaximumCatchupRate => {
                self.settings.streaming_max_catchup_rate = parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingHardSeekThresholdSeconds => {
                self.settings.streaming_hard_seek_threshold_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingMaximumHardSeeks => {
                self.settings.streaming_max_hard_seeks_per_episode = parse_optional_u32(value);
            }
            SettingId::StreamingStabilityIntervalSeconds => {
                self.settings.streaming_stability_interval_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingRecoveryRetryBudget => {
                self.settings.streaming_recovery_retry_budget = parse_optional_u32(value);
            }
            SettingId::StreamingRecoveryCooldownSeconds => {
                self.settings.streaming_recovery_cooldown_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingRoomBufferingPolicy => {
                self.settings.streaming_room_buffering_policy =
                    normalized_editable_text(value).map(|value| value.to_ascii_lowercase());
            }
            SettingId::StreamingRoomQuorumPercent => {
                self.settings.streaming_room_quorum_percent = parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingRoomMaximumPauseSeconds => {
                self.settings.streaming_room_max_pause_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingStartSynchronization => {
                self.settings.streaming_start_policy =
                    normalized_editable_text(value).map(|value| value.to_ascii_lowercase());
            }
            SettingId::StreamingStartQuorumPercent => {
                self.settings.streaming_start_quorum_percent =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingStartTimeoutSeconds => {
                self.settings.streaming_start_timeout_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::StreamingStartTimeoutAction => {
                self.settings.streaming_start_timeout_action =
                    normalized_editable_text(value).map(|value| value.to_ascii_lowercase());
            }
            SettingId::MediaLibraryFirstFileTimeout => {
                self.settings.folder_search_first_file_timeout_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::MediaLibraryDirectories => {
                self.settings.media_search_directories = parse_editable_string_list_text(value);
            }
            SettingId::MediaLibrarySearchTimeout => {
                self.settings.folder_search_timeout_seconds = parse_optional_nonnegative_f64(value);
            }
            SettingId::MediaLibraryDoubleCheckInterval => {
                self.settings.folder_search_double_check_interval_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::MediaLibraryWarningThreshold => {
                self.settings.folder_search_warning_threshold_seconds =
                    parse_optional_nonnegative_f64(value);
            }
            SettingId::ChatMaxLines => {
                self.settings.chat_max_lines = parse_optional_positive_i64(value);
            }
            SettingId::ChatInputPosition => {
                self.settings.chat_input_position = normalized_editable_text(value);
            }
            SettingId::ChatInputFont => {
                self.settings.chat_input_font_family = normalized_editable_text(value);
            }
            SettingId::ChatInputFontSize => {
                self.settings.chat_input_relative_font_size = parse_optional_positive_i64(value);
            }
            SettingId::ChatInputFontWeight => {
                self.settings.chat_input_font_weight = parse_optional_nonnegative_i64(value);
            }
            SettingId::ChatInputColor => {
                self.settings.chat_input_font_color = normalized_editable_text(value);
            }
            SettingId::ChatOutputMode => {
                self.settings.chat_output_mode = normalized_editable_text(value);
            }
            SettingId::ChatOutputFont => {
                self.settings.chat_output_font_family = normalized_editable_text(value);
            }
            SettingId::ChatOutputFontSize => {
                self.settings.chat_output_relative_font_size = parse_optional_positive_i64(value);
            }
            SettingId::ChatOutputFontWeight => {
                self.settings.chat_output_font_weight = parse_optional_nonnegative_i64(value);
            }
            SettingId::ChatTopMargin => {
                self.settings.chat_top_margin = parse_optional_nonnegative_i64(value);
            }
            SettingId::ChatLeftMargin => {
                self.settings.chat_left_margin = parse_optional_nonnegative_i64(value);
            }
            SettingId::ChatBottomMargin => {
                self.settings.chat_bottom_margin = parse_optional_nonnegative_i64(value);
            }
            SettingId::ChatOsdMargin => {
                self.settings.chat_osd_margin = parse_optional_nonnegative_i64(value);
            }
            SettingId::OsdNotificationTimeout => {
                self.settings.notification_timeout_seconds = parse_optional_nonnegative_i64(value);
            }
            SettingId::OsdAlertTimeout => {
                self.settings.alert_timeout_seconds = parse_optional_nonnegative_i64(value);
            }
            SettingId::OsdChatTimeout => {
                self.settings.chat_timeout_seconds = parse_optional_nonnegative_i64(value);
            }
            SettingId::GeneralLanguage => {
                self.settings.language = normalized_editable_text(value)
                    .as_deref()
                    .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
                    .map(str::to_owned);
            }
            SettingId::GeneralUpdateChannel => {
                self.settings.update_channel =
                    normalized_editable_text(value).map(|value| value.to_ascii_lowercase());
            }
            _ => {}
        }
    }

    fn apply_bool_value_to_settings(&mut self, id: SettingId, value: bool) {
        match id {
            SettingId::PlaybackReadyAtStart => {
                self.settings.ready_at_start = Some(value);
            }
            SettingId::PlaybackAutoplay => {
                self.settings.autoplay_initial_state = Some(value);
            }
            SettingId::PlaybackRequireSameFilenames => {
                self.settings.autoplay_require_same_filenames = Some(value);
            }
            SettingId::PlaybackSharedPlaylists => {
                self.settings.shared_playlist_enabled = Some(value);
            }
            SettingId::PlaybackPauseOnLeave => {
                self.settings.pause_on_leave = Some(value);
            }
            SettingId::PlaybackLoopPlaylist => {
                self.settings.loop_at_end_of_playlist = Some(value);
            }
            SettingId::PlaybackLoopSingleFiles => {
                self.settings.loop_single_files = Some(value);
            }
            SettingId::PrivacyTrustedDomainsOnly => {
                self.settings.only_switch_to_trusted_domains = Some(value);
            }
            SettingId::SyncRewindOnDesync => {
                self.settings.rewind_on_desync = Some(value);
            }
            SettingId::SyncFastforwardOnDesync => {
                self.settings.fastforward_on_desync = Some(value);
            }
            SettingId::SyncSlowOnDesync => {
                self.settings.slow_on_desync = Some(value);
            }
            SettingId::SyncDontSlowDownWithMe => {
                self.settings.dont_slow_down_with_me = Some(value);
            }
            SettingId::StreamingDiskCache => {
                self.settings.streaming_disk_cache_enabled = Some(value);
            }
            SettingId::StreamingQualityDowngradeSuggestions => {
                self.settings.streaming_quality_downgrade_suggestions = Some(value);
            }
            SettingId::ChatInputEnabled => {
                self.settings.chat_input_enabled = Some(value);
            }
            SettingId::ChatOutputEnabled => {
                self.settings.chat_output_enabled = Some(value);
            }
            SettingId::ChatDirectInput => {
                self.settings.chat_direct_input = Some(value);
            }
            SettingId::ChatMoveOsd => {
                self.settings.chat_move_osd = Some(value);
            }
            SettingId::OsdShow => {
                self.settings.show_osd = Some(value);
            }
            SettingId::OsdShowDuration => {
                self.settings.show_duration_notification = Some(value);
            }
            SettingId::OsdShowSameRoom => {
                self.settings.show_same_room_osd = Some(value);
            }
            SettingId::OsdShowWarnings => {
                self.settings.show_osd_warnings = Some(value);
            }
            SettingId::OsdShowSlowdown => {
                self.settings.show_slowdown_osd = Some(value);
            }
            SettingId::OsdShowNoncontroller => {
                self.settings.show_noncontroller_osd = Some(value);
            }
            SettingId::OsdShowDifferentRoom => {
                self.settings.show_different_room_osd = Some(value);
            }
            SettingId::OsdShowContactInfo => {
                self.settings.show_contact_info = Some(value);
            }
            SettingId::GeneralCheckForUpdatesAutomatically => {
                self.settings.check_for_updates_automatically = Some(value);
            }
            SettingId::GeneralAutosaveJoinsToList => {
                self.settings.autosave_joins_to_list = Some(value);
            }
            SettingId::GeneralForceGuiPrompt => {
                self.settings.force_gui_prompt = Some(value);
            }
            _ => {}
        }
    }

    fn refresh_derived_controls(&mut self) {
        let baseline_settings = self.to_stored_settings();
        let baseline = FirstRunConfigurationDialogState::from_stored_settings(&baseline_settings)
            .dialog_sections();
        for section in &mut self.sections {
            for control in &mut section.controls {
                let Some(baseline_control) = baseline
                    .iter()
                    .flat_map(|candidate| &candidate.controls)
                    .find(|candidate| candidate.id == control.id)
                else {
                    continue;
                };
                if !control.kind.is_editable()
                    || control.kind == GuiDialogControlKind::Checkbox
                    || matches!(
                        control.id,
                        SettingId::ConnectionServerPassword
                            | SettingId::ConnectionRoomHistory
                            | SettingId::PlayerArguments
                            | SettingId::MediaLibraryDirectories
                    )
                {
                    control.value = baseline_control.value.clone();
                }
            }
        }
    }
}

fn parse_optional_u16(value: &str) -> Option<u16> {
    normalized_editable_text(value)?
        .parse::<u16>()
        .ok()
        .filter(|parsed| *parsed > 0)
}

fn parse_optional_u32(value: &str) -> Option<u32> {
    normalized_editable_text(value)?.parse::<u32>().ok()
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    normalized_editable_text(value)?.parse::<u64>().ok()
}

fn parse_optional_nonnegative_f64(value: &str) -> Option<f64> {
    normalized_editable_text(value)?
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite() && *parsed >= 0.0)
}

fn parse_optional_positive_i64(value: &str) -> Option<i64> {
    normalized_editable_text(value)?
        .parse::<i64>()
        .ok()
        .filter(|parsed| *parsed > 0)
}

fn parse_optional_nonnegative_i64(value: &str) -> Option<i64> {
    normalized_editable_text(value)?
        .parse::<i64>()
        .ok()
        .filter(|parsed| *parsed >= 0)
}
