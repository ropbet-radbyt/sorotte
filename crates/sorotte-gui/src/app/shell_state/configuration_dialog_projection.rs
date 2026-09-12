use super::*;

impl FirstRunConfigurationDialogState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettings) -> Self {
        let config = ClientConfig::resolve(settings).config;
        let advanced_player_arguments = settings
            .player_path
            .as_deref()
            .and_then(|path| settings.per_player_arguments.as_ref()?.get(path))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let effective_mpv_options = config
            .playback
            .streaming
            .effective_mpv_options(advanced_player_arguments)
            .into_iter()
            .map(|option| {
                if option.overridden_by_advanced_arguments {
                    format!(
                        "{}={} (advanced override)",
                        option.name, option.effective_value
                    )
                } else {
                    format!("{}={}", option.name, option.effective_value)
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        let startup_entries = syncplay_configuration_getter_startup_compat_entries();
        let ignored_startup_exception_count = startup_entries
            .iter()
            .filter(|entry| entry.status == SyncplayConfigurationGetterCompatibilityStatus::Ignored)
            .count();

        Self {
            launch_mode: if settings == &StoredClientSettings::default() {
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
                unpause_action: GuiResolvedSettingValue {
                    stored_override: settings
                        .unpause_action
                        .clone()
                        .map(unpause_action_mode_syncplay_name)
                        .map(str::to_owned),
                    effective: unpause_action_mode_syncplay_name(
                        config.readiness.unpause_action.clone(),
                    )
                    .to_owned(),
                },
                autoplay_min_users: GuiResolvedSettingValue {
                    stored_override: settings
                        .autoplay_min_users
                        .as_ref()
                        .map(autoplay_threshold_override_setting_value),
                    effective: autoplay_threshold_override_setting_value(
                        &config.readiness.autoplay_min_users,
                    ),
                },
            },
            privacy: GuiPrivacySection {
                filename_privacy_mode_label: privacy_mode_syncplay_name(
                    config.playback.filename_privacy_mode,
                )
                .to_owned(),
                filesize_privacy_mode_label: privacy_mode_syncplay_name(
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
            streaming: GuiStreamingSection {
                quality_label: config.playback.streaming.quality.config_value().to_owned(),
                custom_format: config.playback.streaming.custom_format.clone(),
                buffer_target_seconds: config.playback.streaming.buffering.target.get(),
                read_ahead_seconds: config.playback.streaming.buffering.read_ahead.get(),
                memory_cache_mebibytes: config.playback.streaming.buffering.memory_cache_mebibytes,
                disk_cache_enabled: config.playback.streaming.buffering.disk_cache_enabled,
                recovery_policy_label: config
                    .playback
                    .streaming
                    .recovery
                    .policy
                    .config_value()
                    .to_owned(),
                maximum_catchup_rate: config.playback.streaming.recovery.max_catchup_rate.get(),
                hard_seek_threshold_seconds: config
                    .playback
                    .streaming
                    .recovery
                    .hard_seek_threshold
                    .get(),
                maximum_hard_seeks: config
                    .playback
                    .streaming
                    .recovery
                    .max_hard_seeks_per_episode,
                stability_interval_seconds: config
                    .playback
                    .streaming
                    .recovery
                    .stability_interval
                    .get(),
                retry_budget: config.playback.streaming.recovery.retry_budget,
                recovery_cooldown_seconds: config.playback.streaming.recovery.cooldown.get(),
                room_buffering_policy_label: config
                    .playback
                    .streaming
                    .room_buffering
                    .policy
                    .config_value()
                    .to_owned(),
                room_quorum_percent: config.playback.streaming.room_buffering.quorum.get(),
                room_maximum_pause_seconds: config
                    .playback
                    .streaming
                    .room_buffering
                    .maximum_pause
                    .get(),
                start_policy_label: config
                    .playback
                    .streaming
                    .start_synchronization
                    .policy
                    .config_value()
                    .to_owned(),
                start_quorum_percent: config.playback.streaming.start_synchronization.quorum.get(),
                start_timeout_seconds: config
                    .playback
                    .streaming
                    .start_synchronization
                    .timeout
                    .get(),
                start_timeout_action_label: config
                    .playback
                    .streaming
                    .start_synchronization
                    .timeout_action
                    .config_value()
                    .to_owned(),
                quality_downgrade_suggestions: config
                    .playback
                    .streaming
                    .quality_downgrade_suggestions,
                effective_mpv_options,
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
                    GuiDialogControl::new(
                        SettingId::ConnectionHost,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.connection.host.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionPort,
                        GuiDialogControlKind::NumericInput,
                        optional_port_text(self.connection.port),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionUsername,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.connection.username.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionRoom,
                        GuiDialogControlKind::TextInput,
                        optional_room_text(self.connection.room.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionServerPassword,
                        GuiDialogControlKind::PasswordInput,
                        String::new(),
                    ),
                    GuiDialogControl::new(
                        SettingId::PlayerExecutable,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.connection.player_path.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::PlayerArguments,
                        GuiDialogControlKind::TextInput,
                        self.connection.player_arguments_text.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionPublicServerCount,
                        GuiDialogControlKind::ReadOnly,
                        self.connection.public_server_count.to_string(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionRoomHistory,
                        GuiDialogControlKind::TextArea,
                        self.connection.room_history_text.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ConnectionRoomHistoryCount,
                        GuiDialogControlKind::ReadOnly,
                        self.connection.room_history_count.to_string(),
                    ),
                ],
            },
            GuiDialogSection {
                title: "Readiness",
                controls: vec![
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackReadyAtStart,
                        self.readiness.ready_at_start,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackAutoplay,
                        self.readiness.autoplay_enabled,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackRequireSameFilenames,
                        self.readiness.autoplay_require_same_filenames,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackSharedPlaylists,
                        self.readiness.shared_playlist_enabled,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackPauseOnLeave,
                        self.readiness.pause_on_leave,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackLoopPlaylist,
                        self.readiness.loop_at_end_of_playlist,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PlaybackLoopSingleFiles,
                        self.readiness.loop_single_files,
                    ),
                    GuiDialogControl::new(
                        SettingId::PlaybackUnpauseAction,
                        GuiDialogControlKind::Select,
                        self.readiness.unpause_action.effective.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::PlaybackAutoplayMinUsers,
                        GuiDialogControlKind::Select,
                        self.readiness.autoplay_min_users.effective.clone(),
                    ),
                ],
            },
            GuiDialogSection {
                title: "Privacy",
                controls: vec![
                    GuiDialogControl::new(
                        SettingId::PrivacyFilename,
                        GuiDialogControlKind::Select,
                        self.privacy.filename_privacy_mode_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::PrivacyFilesize,
                        GuiDialogControlKind::Select,
                        self.privacy.filesize_privacy_mode_label.clone(),
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::PrivacyTrustedDomainsOnly,
                        self.privacy.only_switch_to_trusted_domains,
                    ),
                    GuiDialogControl::new(
                        SettingId::PrivacyTrustedDomains,
                        GuiDialogControlKind::TextArea,
                        self.privacy.trusted_domains_text.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::PrivacyTrustedDomainCount,
                        GuiDialogControlKind::ReadOnly,
                        self.privacy.trusted_domain_count.to_string(),
                    ),
                ],
            },
            GuiDialogSection {
                title: "Desync",
                controls: vec![
                    GuiDialogControl::checkbox(
                        SettingId::SyncRewindOnDesync,
                        self.desync.rewind_on_desync,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::SyncFastforwardOnDesync,
                        self.desync.fastforward_on_desync,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::SyncSlowOnDesync,
                        self.desync.slow_on_desync,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::SyncDontSlowDownWithMe,
                        self.desync.dont_slow_down_with_me,
                    ),
                    GuiDialogControl::new(
                        SettingId::SyncRewindThreshold,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(self.desync.rewind_threshold_seconds),
                    ),
                    GuiDialogControl::new(
                        SettingId::SyncFastforwardThreshold,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(self.desync.fastforward_threshold_seconds),
                    ),
                    GuiDialogControl::new(
                        SettingId::SyncSlowdownThreshold,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(self.desync.slowdown_threshold_seconds),
                    ),
                ],
            },
            GuiDialogSection {
                title: "Streaming",
                controls: vec![
                    GuiDialogControl::new(
                        SettingId::StreamingQuality,
                        GuiDialogControlKind::Select,
                        self.streaming.quality_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingCustomFormat,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.streaming.custom_format.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingBufferTargetSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.buffer_target_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingReadAheadSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.read_ahead_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingMemoryCacheMib,
                        GuiDialogControlKind::NumericInput,
                        self.streaming.memory_cache_mebibytes.to_string(),
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::StreamingDiskCache,
                        self.streaming.disk_cache_enabled,
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingRecoveryPolicy,
                        GuiDialogControlKind::Select,
                        self.streaming.recovery_policy_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingMaximumCatchupRate,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.maximum_catchup_rate)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingHardSeekThresholdSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.hard_seek_threshold_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingMaximumHardSeeks,
                        GuiDialogControlKind::NumericInput,
                        self.streaming.maximum_hard_seeks.to_string(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingStabilityIntervalSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.stability_interval_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingRecoveryRetryBudget,
                        GuiDialogControlKind::NumericInput,
                        self.streaming.retry_budget.to_string(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingRecoveryCooldownSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.recovery_cooldown_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingRoomBufferingPolicy,
                        GuiDialogControlKind::Select,
                        self.streaming.room_buffering_policy_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingRoomQuorumPercent,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.room_quorum_percent)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingRoomMaximumPauseSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.room_maximum_pause_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingStartSynchronization,
                        GuiDialogControlKind::Select,
                        self.streaming.start_policy_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingStartQuorumPercent,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.start_quorum_percent)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingStartTimeoutSeconds,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(Some(self.streaming.start_timeout_seconds)),
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingStartTimeoutAction,
                        GuiDialogControlKind::Select,
                        self.streaming.start_timeout_action_label.clone(),
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::StreamingQualityDowngradeSuggestions,
                        self.streaming.quality_downgrade_suggestions,
                    ),
                    GuiDialogControl::new(
                        SettingId::StreamingEffectiveMpvOptions,
                        GuiDialogControlKind::ReadOnly,
                        self.streaming.effective_mpv_options.clone(),
                    ),
                ],
            },
            GuiDialogSection {
                title: "Media Search",
                controls: vec![
                    GuiDialogControl::new(
                        SettingId::MediaLibraryDirectories,
                        GuiDialogControlKind::TextArea,
                        self.media_search.media_directories_text.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::MediaLibraryDirectoryCount,
                        GuiDialogControlKind::ReadOnly,
                        self.media_search.media_directory_count.to_string(),
                    ),
                    GuiDialogControl::new(
                        SettingId::MediaLibraryFirstFileTimeout,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(
                            self.media_search.folder_search_first_file_timeout_seconds,
                        ),
                    ),
                    GuiDialogControl::new(
                        SettingId::MediaLibrarySearchTimeout,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(self.media_search.folder_search_timeout_seconds),
                    ),
                    GuiDialogControl::new(
                        SettingId::MediaLibraryDoubleCheckInterval,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(
                            self.media_search
                                .folder_search_double_check_interval_seconds,
                        ),
                    ),
                    GuiDialogControl::new(
                        SettingId::MediaLibraryWarningThreshold,
                        GuiDialogControlKind::NumericInput,
                        optional_f64_text(
                            self.media_search.folder_search_warning_threshold_seconds,
                        ),
                    ),
                ],
            },
            GuiDialogSection {
                title: "Chat",
                controls: vec![
                    GuiDialogControl::checkbox(
                        SettingId::ChatInputEnabled,
                        self.chat.chat_input_enabled,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::ChatOutputEnabled,
                        self.chat.chat_output_enabled,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::ChatDirectInput,
                        self.chat.chat_direct_input,
                    ),
                    GuiDialogControl::checkbox(SettingId::ChatMoveOsd, self.chat.chat_move_osd),
                    GuiDialogControl::new(
                        SettingId::ChatInputPosition,
                        GuiDialogControlKind::Select,
                        self.chat.chat_input_position_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatOutputMode,
                        GuiDialogControlKind::Select,
                        self.chat.chat_output_mode_label.clone(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatMaxLines,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_max_lines),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatInputFont,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.chat.chat_input_font_family.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatInputFontSize,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_input_relative_font_size),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatInputFontWeight,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_input_font_weight),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatInputColor,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.chat.chat_input_font_color.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatOutputFont,
                        GuiDialogControlKind::TextInput,
                        optional_text(self.chat.chat_output_font_family.as_deref()).to_owned(),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatOutputFontSize,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_output_relative_font_size),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatOutputFontWeight,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_output_font_weight),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatTopMargin,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_top_margin),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatLeftMargin,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_left_margin),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatBottomMargin,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_bottom_margin),
                    ),
                    GuiDialogControl::new(
                        SettingId::ChatOsdMargin,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.chat.chat_osd_margin),
                    ),
                ],
            },
            GuiDialogSection {
                title: "OSD",
                controls: vec![
                    GuiDialogControl::checkbox(SettingId::OsdShow, self.osd.show_osd),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowDuration,
                        self.osd.show_duration_notification,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowSameRoom,
                        self.osd.show_same_room_osd,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowWarnings,
                        self.osd.show_osd_warnings,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowSlowdown,
                        self.osd.show_slowdown_osd,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowNoncontroller,
                        self.osd.show_noncontroller_osd,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowDifferentRoom,
                        self.osd.show_different_room_osd,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::OsdShowContactInfo,
                        self.osd.show_contact_info,
                    ),
                    GuiDialogControl::new(
                        SettingId::OsdNotificationTimeout,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.osd.notification_timeout_seconds),
                    ),
                    GuiDialogControl::new(
                        SettingId::OsdAlertTimeout,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.osd.alert_timeout_seconds),
                    ),
                    GuiDialogControl::new(
                        SettingId::OsdChatTimeout,
                        GuiDialogControlKind::NumericInput,
                        optional_i64_text(self.osd.chat_timeout_seconds),
                    ),
                ],
            },
            GuiDialogSection {
                title: "System",
                controls: vec![
                    GuiDialogControl::new(
                        SettingId::GeneralLanguage,
                        GuiDialogControlKind::Select,
                        self.system.language_tag.clone(),
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::GeneralCheckForUpdatesAutomatically,
                        self.system.check_for_updates_automatically,
                    ),
                    GuiDialogControl::new(
                        SettingId::GeneralUpdateChannel,
                        GuiDialogControlKind::Select,
                        self.system.update_channel_label.clone(),
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::GeneralAutosaveJoinsToList,
                        self.system.autosave_joins_to_list,
                    ),
                    GuiDialogControl::checkbox(
                        SettingId::GeneralForceGuiPrompt,
                        self.system.force_gui_prompt,
                    ),
                    GuiDialogControl::new(
                        SettingId::DiagnosticsSupportedLanguages,
                        GuiDialogControlKind::ReadOnly,
                        SUPPORTED_RUNTIME_LANGUAGE_TAGS_DISPLAY.to_owned(),
                    ),
                ],
            },
        ]
    }
}
