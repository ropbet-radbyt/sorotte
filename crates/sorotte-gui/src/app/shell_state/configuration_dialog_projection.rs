use super::*;

impl FirstRunConfigurationDialogState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
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
                unpause_action: GuiResolvedSettingValue {
                    stored_override: settings
                        .unpause_action
                        .clone()
                        .map(unpause_action_mode_legacy_name_compatible)
                        .map(str::to_owned),
                    effective: unpause_action_mode_legacy_name_compatible(
                        config.readiness.unpause_action.clone(),
                    )
                    .to_owned(),
                },
                autoplay_min_users: GuiResolvedSettingValue {
                    stored_override: settings
                        .autoplay_min_users
                        .as_ref()
                        .map(autoplay_threshold_override_legacy_value_compatible),
                    effective: autoplay_threshold_override_legacy_value_compatible(
                        &config.readiness.autoplay_min_users,
                    ),
                },
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
                    GuiDialogControl {
                        id: SettingId::ConnectionHost,
                        label: SettingId::ConnectionHost.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.host.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionPort,
                        label: SettingId::ConnectionPort.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_port_text(self.connection.port),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionUsername,
                        label: SettingId::ConnectionUsername.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.username.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionRoom,
                        label: SettingId::ConnectionRoom.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_room_text(self.connection.room.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionServerPassword,
                        label: SettingId::ConnectionServerPassword.label(),
                        kind: GuiDialogControlKind::PasswordInput,
                        value: String::new(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlayerExecutable,
                        label: SettingId::PlayerExecutable.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.player_path.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlayerArguments,
                        label: SettingId::PlayerArguments.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: self.connection.player_arguments_text.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionPublicServerCount,
                        label: SettingId::ConnectionPublicServerCount.label(),
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.public_server_count.to_string(),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionRoomHistory,
                        label: SettingId::ConnectionRoomHistory.label(),
                        kind: GuiDialogControlKind::TextArea,
                        value: self.connection.room_history_text.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::ConnectionRoomHistoryCount,
                        label: SettingId::ConnectionRoomHistoryCount.label(),
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.room_history_count.to_string(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Readiness",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::PlaybackReadyAtStart,
                        label: SettingId::PlaybackReadyAtStart.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.ready_at_start).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackAutoplay,
                        label: SettingId::PlaybackAutoplay.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.autoplay_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackRequireSameFilenames,
                        label: SettingId::PlaybackRequireSameFilenames.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.autoplay_require_same_filenames)
                            .to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackSharedPlaylists,
                        label: SettingId::PlaybackSharedPlaylists.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.shared_playlist_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackPauseOnLeave,
                        label: SettingId::PlaybackPauseOnLeave.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.pause_on_leave).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackLoopPlaylist,
                        label: SettingId::PlaybackLoopPlaylist.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.loop_at_end_of_playlist).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackLoopSingleFiles,
                        label: SettingId::PlaybackLoopSingleFiles.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.loop_single_files).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackUnpauseAction,
                        label: SettingId::PlaybackUnpauseAction.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.readiness.unpause_action.effective.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::PlaybackAutoplayMinUsers,
                        label: SettingId::PlaybackAutoplayMinUsers.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.readiness.autoplay_min_users.effective.clone(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Privacy",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::PrivacyFilename,
                        label: SettingId::PrivacyFilename.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.privacy.filename_privacy_mode_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::PrivacyFilesize,
                        label: SettingId::PrivacyFilesize.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.privacy.filesize_privacy_mode_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::PrivacyTrustedDomainsOnly,
                        label: SettingId::PrivacyTrustedDomainsOnly.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.privacy.only_switch_to_trusted_domains).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::PrivacyTrustedDomains,
                        label: SettingId::PrivacyTrustedDomains.label(),
                        kind: GuiDialogControlKind::TextArea,
                        value: self.privacy.trusted_domains_text.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::PrivacyTrustedDomainCount,
                        label: SettingId::PrivacyTrustedDomainCount.label(),
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.privacy.trusted_domain_count.to_string(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Desync",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::SyncRewindOnDesync,
                        label: SettingId::SyncRewindOnDesync.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.rewind_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::SyncFastforwardOnDesync,
                        label: SettingId::SyncFastforwardOnDesync.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.fastforward_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::SyncSlowOnDesync,
                        label: SettingId::SyncSlowOnDesync.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.slow_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::SyncDontSlowDownWithMe,
                        label: SettingId::SyncDontSlowDownWithMe.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.dont_slow_down_with_me).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::SyncRewindThreshold,
                        label: SettingId::SyncRewindThreshold.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.rewind_threshold_seconds),
                    },
                    GuiDialogControl {
                        id: SettingId::SyncFastforwardThreshold,
                        label: SettingId::SyncFastforwardThreshold.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.fastforward_threshold_seconds),
                    },
                    GuiDialogControl {
                        id: SettingId::SyncSlowdownThreshold,
                        label: SettingId::SyncSlowdownThreshold.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.slowdown_threshold_seconds),
                    },
                ],
            },
            GuiDialogSection {
                title: "Streaming",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::StreamingQuality,
                        label: SettingId::StreamingQuality.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.streaming.quality_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingCustomFormat,
                        label: SettingId::StreamingCustomFormat.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.streaming.custom_format.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingBufferTargetSeconds,
                        label: SettingId::StreamingBufferTargetSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.buffer_target_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingReadAheadSeconds,
                        label: SettingId::StreamingReadAheadSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.read_ahead_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingMemoryCacheMib,
                        label: SettingId::StreamingMemoryCacheMib.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: self.streaming.memory_cache_mebibytes.to_string(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingDiskCache,
                        label: SettingId::StreamingDiskCache.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.streaming.disk_cache_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingRecoveryPolicy,
                        label: SettingId::StreamingRecoveryPolicy.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.streaming.recovery_policy_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingMaximumCatchupRate,
                        label: SettingId::StreamingMaximumCatchupRate.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.maximum_catchup_rate)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingHardSeekThresholdSeconds,
                        label: SettingId::StreamingHardSeekThresholdSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.hard_seek_threshold_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingMaximumHardSeeks,
                        label: SettingId::StreamingMaximumHardSeeks.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: self.streaming.maximum_hard_seeks.to_string(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingStabilityIntervalSeconds,
                        label: SettingId::StreamingStabilityIntervalSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.stability_interval_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingRecoveryRetryBudget,
                        label: SettingId::StreamingRecoveryRetryBudget.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: self.streaming.retry_budget.to_string(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingRecoveryCooldownSeconds,
                        label: SettingId::StreamingRecoveryCooldownSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.recovery_cooldown_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingRoomBufferingPolicy,
                        label: SettingId::StreamingRoomBufferingPolicy.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.streaming.room_buffering_policy_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingRoomQuorumPercent,
                        label: SettingId::StreamingRoomQuorumPercent.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.room_quorum_percent)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingRoomMaximumPauseSeconds,
                        label: SettingId::StreamingRoomMaximumPauseSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.room_maximum_pause_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingStartSynchronization,
                        label: SettingId::StreamingStartSynchronization.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.streaming.start_policy_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingStartQuorumPercent,
                        label: SettingId::StreamingStartQuorumPercent.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.start_quorum_percent)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingStartTimeoutSeconds,
                        label: SettingId::StreamingStartTimeoutSeconds.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(Some(self.streaming.start_timeout_seconds)),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingStartTimeoutAction,
                        label: SettingId::StreamingStartTimeoutAction.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.streaming.start_timeout_action_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingQualityDowngradeSuggestions,
                        label: SettingId::StreamingQualityDowngradeSuggestions.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.streaming.quality_downgrade_suggestions).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::StreamingEffectiveMpvOptions,
                        label: SettingId::StreamingEffectiveMpvOptions.label(),
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.streaming.effective_mpv_options.clone(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Media Search",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::MediaLibraryDirectories,
                        label: SettingId::MediaLibraryDirectories.label(),
                        kind: GuiDialogControlKind::TextArea,
                        value: self.media_search.media_directories_text.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::MediaLibraryDirectoryCount,
                        label: SettingId::MediaLibraryDirectoryCount.label(),
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.media_search.media_directory_count.to_string(),
                    },
                    GuiDialogControl {
                        id: SettingId::MediaLibraryFirstFileTimeout,
                        label: SettingId::MediaLibraryFirstFileTimeout.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search.folder_search_first_file_timeout_seconds,
                        ),
                    },
                    GuiDialogControl {
                        id: SettingId::MediaLibrarySearchTimeout,
                        label: SettingId::MediaLibrarySearchTimeout.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.media_search.folder_search_timeout_seconds),
                    },
                    GuiDialogControl {
                        id: SettingId::MediaLibraryDoubleCheckInterval,
                        label: SettingId::MediaLibraryDoubleCheckInterval.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search
                                .folder_search_double_check_interval_seconds,
                        ),
                    },
                    GuiDialogControl {
                        id: SettingId::MediaLibraryWarningThreshold,
                        label: SettingId::MediaLibraryWarningThreshold.label(),
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
                        id: SettingId::ChatInputEnabled,
                        label: SettingId::ChatInputEnabled.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_input_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatOutputEnabled,
                        label: SettingId::ChatOutputEnabled.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_output_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatDirectInput,
                        label: SettingId::ChatDirectInput.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_direct_input).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatMoveOsd,
                        label: SettingId::ChatMoveOsd.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_move_osd).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatInputPosition,
                        label: SettingId::ChatInputPosition.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.chat.chat_input_position_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatOutputMode,
                        label: SettingId::ChatOutputMode.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.chat.chat_output_mode_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatMaxLines,
                        label: SettingId::ChatMaxLines.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_max_lines),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatInputFont,
                        label: SettingId::ChatInputFont.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_input_font_family.as_deref())
                            .to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatInputFontSize,
                        label: SettingId::ChatInputFontSize.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_input_relative_font_size),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatInputFontWeight,
                        label: SettingId::ChatInputFontWeight.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_input_font_weight),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatInputColor,
                        label: SettingId::ChatInputColor.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_input_font_color.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatOutputFont,
                        label: SettingId::ChatOutputFont.label(),
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_output_font_family.as_deref())
                            .to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatOutputFontSize,
                        label: SettingId::ChatOutputFontSize.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_output_relative_font_size),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatOutputFontWeight,
                        label: SettingId::ChatOutputFontWeight.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_output_font_weight),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatTopMargin,
                        label: SettingId::ChatTopMargin.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_top_margin),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatLeftMargin,
                        label: SettingId::ChatLeftMargin.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_left_margin),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatBottomMargin,
                        label: SettingId::ChatBottomMargin.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_bottom_margin),
                    },
                    GuiDialogControl {
                        id: SettingId::ChatOsdMargin,
                        label: SettingId::ChatOsdMargin.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_osd_margin),
                    },
                ],
            },
            GuiDialogSection {
                title: "OSD",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::OsdShow,
                        label: SettingId::OsdShow.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_osd).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowDuration,
                        label: SettingId::OsdShowDuration.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_duration_notification).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowSameRoom,
                        label: SettingId::OsdShowSameRoom.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_same_room_osd).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowWarnings,
                        label: SettingId::OsdShowWarnings.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_osd_warnings).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowSlowdown,
                        label: SettingId::OsdShowSlowdown.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_slowdown_osd).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowNoncontroller,
                        label: SettingId::OsdShowNoncontroller.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_noncontroller_osd).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowDifferentRoom,
                        label: SettingId::OsdShowDifferentRoom.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_different_room_osd).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdShowContactInfo,
                        label: SettingId::OsdShowContactInfo.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_contact_info).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdNotificationTimeout,
                        label: SettingId::OsdNotificationTimeout.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.notification_timeout_seconds),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdAlertTimeout,
                        label: SettingId::OsdAlertTimeout.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.alert_timeout_seconds),
                    },
                    GuiDialogControl {
                        id: SettingId::OsdChatTimeout,
                        label: SettingId::OsdChatTimeout.label(),
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.chat_timeout_seconds),
                    },
                ],
            },
            GuiDialogSection {
                title: "System",
                controls: vec![
                    GuiDialogControl {
                        id: SettingId::GeneralLanguage,
                        label: SettingId::GeneralLanguage.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.system.language_tag.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::GeneralCheckForUpdatesAutomatically,
                        label: SettingId::GeneralCheckForUpdatesAutomatically.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.check_for_updates_automatically).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::GeneralUpdateChannel,
                        label: SettingId::GeneralUpdateChannel.label(),
                        kind: GuiDialogControlKind::Select,
                        value: self.system.update_channel_label.clone(),
                    },
                    GuiDialogControl {
                        id: SettingId::GeneralAutosaveJoinsToList,
                        label: SettingId::GeneralAutosaveJoinsToList.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.autosave_joins_to_list).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::GeneralForceGuiPrompt,
                        label: SettingId::GeneralForceGuiPrompt.label(),
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.force_gui_prompt).to_owned(),
                    },
                    GuiDialogControl {
                        id: SettingId::DiagnosticsSupportedLanguages,
                        label: SettingId::DiagnosticsSupportedLanguages.label(),
                        kind: GuiDialogControlKind::ReadOnly,
                        value: SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY.to_owned(),
                    },
                ],
            },
        ]
    }
}
