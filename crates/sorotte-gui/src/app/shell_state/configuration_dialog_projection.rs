use super::*;

impl FirstRunConfigurationDialogState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let config = ClientConfig::resolve(settings).config;
        let startup_entries = legacy_configuration_getter_startup_compat_entries();
        let ignored_startup_exception_count = startup_entries
            .iter()
            .filter(|entry| entry.status == LegacyConfigurationGetterCompatibilityStatus::Ignored)
            .count();

        Self {
            launch_mode: if settings == &StoredClientSettingsMvp::default() {
                GuiLaunchMode::FirstRun
            } else {
                GuiLaunchMode::ExistingConfig
            },
            connection: GuiConnectionSettingsSection {
                host: settings.host.clone(),
                port: settings.port,
                username: settings.username.clone(),
                room: settings.room.clone(),
                server_password_set: settings
                    .server_password
                    .as_ref()
                    .map(|password| password.expose_secret())
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty()),
                player_path: settings.player_path.clone(),
                player_arguments_text: player_arguments_text_for_path(
                    settings.per_player_arguments.as_ref(),
                    settings.player_path.as_deref(),
                ),
                room_history_text: optional_string_list_multiline_text(
                    settings.room_list.as_deref(),
                ),
                public_server_count: settings.public_servers.as_ref().map_or(0, Vec::len),
                room_history_count: settings.room_list.as_ref().map_or(0, Vec::len),
            },
            readiness: GuiReadinessSection {
                ready_at_start: config.readiness.ready_at_start,
                autoplay_enabled: config.readiness.autoplay_initial_state,
                autoplay_require_same_filenames: config.readiness.autoplay_require_same_filenames,
                shared_playlist_enabled: config.playback.shared_playlist_enabled,
                pause_on_leave: config.playback.pause_on_leave,
                loop_at_end_of_playlist: config.playback.loop_at_end_of_playlist,
                loop_single_files: config.playback.loop_single_files,
                unpause_action_label: settings
                    .unpause_action
                    .clone()
                    .map(unpause_action_mode_legacy_name_compatible)
                    .unwrap_or("IfAlreadyReady")
                    .to_owned(),
                autoplay_min_users_label: settings
                    .autoplay_min_users
                    .as_ref()
                    .map(autoplay_threshold_override_legacy_value_compatible)
                    .unwrap_or_else(|| "app-default".to_owned()),
            },
            privacy: GuiPrivacySection {
                filename_privacy_mode_label: privacy_mode_legacy_name_compatible(
                    config.playback.filename_privacy_mode,
                )
                .to_owned(),
                filesize_privacy_mode_label: privacy_mode_legacy_name_compatible(
                    config.playback.filesize_privacy_mode,
                )
                .to_owned(),
                only_switch_to_trusted_domains: config.playback.only_switch_to_trusted_domains,
                trusted_domains_text: optional_string_list_multiline_text(
                    settings.trusted_domains.as_deref(),
                ),
                trusted_domain_count: settings.trusted_domains.as_ref().map_or(0, Vec::len),
            },
            desync: GuiDesyncSection {
                rewind_on_desync: config.synchronization.rewind_on_desync,
                fastforward_on_desync: config.synchronization.fastforward_on_desync,
                slow_on_desync: config.synchronization.slow_on_desync,
                dont_slow_down_with_me: config.synchronization.dont_slow_down_with_me,
                rewind_threshold_seconds: Some(config.synchronization.rewind_threshold.get()),
                fastforward_threshold_seconds: Some(
                    config.synchronization.fastforward_threshold.get(),
                ),
                slowdown_threshold_seconds: Some(config.synchronization.slowdown_threshold.get()),
            },
            media_search: GuiMediaSearchSection {
                media_directories_text: optional_string_list_multiline_text(
                    settings.media_search_directories.as_deref(),
                ),
                media_directory_count: settings
                    .media_search_directories
                    .as_ref()
                    .map_or(0, Vec::len),
                folder_search_first_file_timeout_seconds: settings
                    .folder_search_first_file_timeout_seconds,
                folder_search_timeout_seconds: settings.folder_search_timeout_seconds,
                folder_search_double_check_interval_seconds: settings
                    .folder_search_double_check_interval_seconds,
                folder_search_warning_threshold_seconds: settings
                    .folder_search_warning_threshold_seconds,
            },
            chat: GuiChatSection {
                chat_input_enabled: config.interface.chat_input_enabled,
                chat_output_enabled: config.interface.chat_output_enabled,
                chat_direct_input: config.interface.chat_direct_input,
                chat_move_osd: config.interface.chat_move_osd,
                chat_max_lines: Some(config.interface.chat_max_lines),
                chat_input_position_label: config.interface.chat_input_position.clone(),
                chat_input_font_family: Some(config.interface.chat_input_font_family.clone()),
                chat_input_relative_font_size: Some(config.interface.chat_input_relative_font_size),
                chat_input_font_weight: Some(config.interface.chat_input_font_weight),
                chat_input_font_color: Some(config.interface.chat_input_font_color.clone()),
                chat_output_mode_label: config.interface.chat_output_mode.clone(),
                chat_output_font_family: Some(config.interface.chat_output_font_family.clone()),
                chat_output_relative_font_size: Some(
                    config.interface.chat_output_relative_font_size,
                ),
                chat_output_font_weight: Some(config.interface.chat_output_font_weight),
                chat_top_margin: Some(config.interface.chat_top_margin),
                chat_left_margin: Some(config.interface.chat_left_margin),
                chat_bottom_margin: Some(config.interface.chat_bottom_margin),
                chat_osd_margin: Some(config.interface.chat_osd_margin),
            },
            osd: GuiOsdSection {
                show_osd: config.interface.show_osd,
                show_duration_notification: config.readiness.show_duration_notification,
                show_same_room_osd: config.interface.show_same_room_osd,
                show_osd_warnings: config.interface.show_osd_warnings,
                show_slowdown_osd: config.interface.show_slowdown_osd,
                show_noncontroller_osd: config.interface.show_noncontroller_osd,
                show_different_room_osd: config.interface.show_different_room_osd,
                show_contact_info: config.interface.show_contact_info,
                notification_timeout_seconds: Some(
                    config.interface.notification_timeout.get() as i64
                ),
                alert_timeout_seconds: Some(config.interface.alert_timeout.get() as i64),
                chat_timeout_seconds: Some(config.interface.chat_timeout.get() as i64),
            },
            system: GuiSystemSection {
                language_tag: config.interface.language.clone(),
                check_for_updates_automatically: config.interface.check_for_updates_automatically,
                update_channel_label: config.interface.update_channel.to_ascii_lowercase(),
                autosave_joins_to_list: config.interface.autosave_joins_to_list,
                force_gui_prompt: config.interface.force_gui_prompt,
                compatibility_startup_entry_count: startup_entries.len(),
                ignored_startup_exception_count,
            },
        }
    }

    pub(in crate::app) fn dialog_sections(&self) -> Vec<GuiDialogSection> {
        vec![
            GuiDialogSection {
                title: "Connection",
                controls: vec![
                    GuiDialogControl {
                        label: "Host",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.host.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Port",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_port_text(self.connection.port),
                    },
                    GuiDialogControl {
                        label: "Username",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.username.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Room",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_room_text(self.connection.room.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Server Password",
                        kind: GuiDialogControlKind::PasswordInput,
                        value: if self.connection.server_password_set {
                            "(configured)".to_owned()
                        } else {
                            "(unset)".to_owned()
                        },
                    },
                    GuiDialogControl {
                        label: "Player Path",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.player_path.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Player Arguments",
                        kind: GuiDialogControlKind::TextInput,
                        value: self.connection.player_arguments_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Public Servers",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.public_server_count.to_string(),
                    },
                    GuiDialogControl {
                        label: "Room History",
                        kind: GuiDialogControlKind::TextArea,
                        value: self.connection.room_history_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Room History Count",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.room_history_count.to_string(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Readiness",
                controls: vec![
                    GuiDialogControl {
                        label: "Ready At Start",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.ready_at_start).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Autoplay",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.autoplay_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Require Same Filenames",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.autoplay_require_same_filenames)
                            .to_owned(),
                    },
                    GuiDialogControl {
                        label: "Shared Playlists",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.shared_playlist_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Pause On Leave",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.pause_on_leave).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Loop At End Of Playlist",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.loop_at_end_of_playlist).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Loop Single Files",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.loop_single_files).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Unpause Action",
                        kind: GuiDialogControlKind::Select,
                        value: self.readiness.unpause_action_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Autoplay Min Users",
                        kind: GuiDialogControlKind::Select,
                        value: self.readiness.autoplay_min_users_label.clone(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Privacy",
                controls: vec![
                    GuiDialogControl {
                        label: "Filename Privacy",
                        kind: GuiDialogControlKind::Select,
                        value: self.privacy.filename_privacy_mode_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Filesize Privacy",
                        kind: GuiDialogControlKind::Select,
                        value: self.privacy.filesize_privacy_mode_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Trusted Domains Only",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.privacy.only_switch_to_trusted_domains).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Trusted Domains",
                        kind: GuiDialogControlKind::TextArea,
                        value: self.privacy.trusted_domains_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Trusted Domain Count",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.privacy.trusted_domain_count.to_string(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Desync",
                controls: vec![
                    GuiDialogControl {
                        label: "Rewind On Desync",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.rewind_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Fastforward On Desync",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.fastforward_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Slow On Desync",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.slow_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Dont Slow Down With Me",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.dont_slow_down_with_me).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Rewind Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.rewind_threshold_seconds),
                    },
                    GuiDialogControl {
                        label: "Fastforward Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.fastforward_threshold_seconds),
                    },
                    GuiDialogControl {
                        label: "Slowdown Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.slowdown_threshold_seconds),
                    },
                ],
            },
            GuiDialogSection {
                title: "Media Search",
                controls: vec![
                    GuiDialogControl {
                        label: "Directories",
                        kind: GuiDialogControlKind::TextArea,
                        value: self.media_search.media_directories_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Directory Count",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.media_search.media_directory_count.to_string(),
                    },
                    GuiDialogControl {
                        label: "First File Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search.folder_search_first_file_timeout_seconds,
                        ),
                    },
                    GuiDialogControl {
                        label: "Search Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.media_search.folder_search_timeout_seconds),
                    },
                    GuiDialogControl {
                        label: "Double Check Interval",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search
                                .folder_search_double_check_interval_seconds,
                        ),
                    },
                    GuiDialogControl {
                        label: "Warning Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search.folder_search_warning_threshold_seconds,
                        ),
                    },
                ],
            },
            GuiDialogSection {
                title: "Chat",
                controls: vec![
                    GuiDialogControl {
                        label: "Chat Input",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_input_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Chat Output",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_output_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Direct Input",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_direct_input).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Move OSD",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_move_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Input Position",
                        kind: GuiDialogControlKind::Select,
                        value: self.chat.chat_input_position_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Output Mode",
                        kind: GuiDialogControlKind::Select,
                        value: self.chat.chat_output_mode_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Max Lines",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_max_lines),
                    },
                    GuiDialogControl {
                        label: "Input Font",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_input_font_family.as_deref())
                            .to_owned(),
                    },
                    GuiDialogControl {
                        label: "Input Font Size",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_input_relative_font_size),
                    },
                    GuiDialogControl {
                        label: "Input Font Weight",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_input_font_weight),
                    },
                    GuiDialogControl {
                        label: "Input Color",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_input_font_color.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Output Font",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_output_font_family.as_deref())
                            .to_owned(),
                    },
                    GuiDialogControl {
                        label: "Output Font Size",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_output_relative_font_size),
                    },
                    GuiDialogControl {
                        label: "Output Font Weight",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_output_font_weight),
                    },
                    GuiDialogControl {
                        label: "Top Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_top_margin),
                    },
                    GuiDialogControl {
                        label: "Left Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_left_margin),
                    },
                    GuiDialogControl {
                        label: "Bottom Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_bottom_margin),
                    },
                    GuiDialogControl {
                        label: "OSD Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_osd_margin),
                    },
                ],
            },
            GuiDialogSection {
                title: "OSD",
                controls: vec![
                    GuiDialogControl {
                        label: "Show OSD",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Duration",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_duration_notification).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Same Room",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_same_room_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Warnings",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_osd_warnings).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Slowdown",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_slowdown_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Noncontroller",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_noncontroller_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Different Room",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_different_room_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Contact Info",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_contact_info).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Notification Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.notification_timeout_seconds),
                    },
                    GuiDialogControl {
                        label: "Alert Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.alert_timeout_seconds),
                    },
                    GuiDialogControl {
                        label: "Chat Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.chat_timeout_seconds),
                    },
                ],
            },
            GuiDialogSection {
                title: "System",
                controls: vec![
                    GuiDialogControl {
                        label: "Language",
                        kind: GuiDialogControlKind::Select,
                        value: self.system.language_tag.clone(),
                    },
                    GuiDialogControl {
                        label: "Auto Update",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.check_for_updates_automatically).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Update Channel",
                        kind: GuiDialogControlKind::Select,
                        value: self.system.update_channel_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Autosave Joins To List",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.autosave_joins_to_list).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Force GUI Prompt",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.force_gui_prompt).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Supported Languages",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY.to_owned(),
                    },
                ],
            },
        ]
    }
}
