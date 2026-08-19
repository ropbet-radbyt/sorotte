use super::*;
use crate::app::shell_state::{
    GuiPlaylistSourcePolicy, MainWindowParticipantStatusPresentation,
    MainWindowParticipantStatusReport, MainWindowUserRow,
};

impl SorotteGuiShellAppState {
    fn seek_preparation_panel(&self) -> Option<GuiWidgetNode> {
        let Some(preparation) = self.seek_preparation.as_ref() else {
            let reason = self.seek_preparation_degraded_reason?;
            let status = match reason {
                GuiSeekPreparationDegradedReason::NonSeekable => {
                    "Could not complete seek: this stream is not seekable."
                }
                GuiSeekPreparationDegradedReason::OutsideLiveWindow => {
                    "Could not complete seek: target is outside the current live window."
                }
                GuiSeekPreparationDegradedReason::TimedOut => "Buffer refill timed out.",
                GuiSeekPreparationDegradedReason::TimelineWindowUnavailable => {
                    "Could not determine a safe seek window for this stream."
                }
                GuiSeekPreparationDegradedReason::TransportFailed => {
                    "Stream transport failed during seek preparation."
                }
                GuiSeekPreparationDegradedReason::ConvergenceDegraded => {
                    "Seek completed, but room convergence degraded."
                }
            };
            return Some(
                GuiWidgetNode::branch(
                    "main-window:seek-preparation",
                    "Playback synchronization",
                    GuiWidgetKind::Panel,
                    vec![
                        GuiWidgetNode::leaf(
                            "main-window:seek-preparation:status",
                            "Status",
                            GuiWidgetKind::Status,
                            Some(status.to_owned()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            "main-window:seek-preparation:guidance",
                            "Next step",
                            GuiWidgetKind::Status,
                            Some("Playback stayed in a safe degraded state. Choose a new seek or retry the stream when ready.".to_owned()),
                            true,
                            false,
                        ),
                    ],
                )
                .with_tooltip(
                    "This status is scoped to the current media and clears on the next media or room transition.",
                ),
            );
        };
        let target = format_seek_preparation_timestamp(preparation.frozen_target_seconds);
        let status = match preparation.phase {
            GuiSeekPreparationPhase::Seeking => format!("Seeking to {target}"),
            GuiSeekPreparationPhase::Fetching => "Fetching stream data".to_owned(),
            GuiSeekPreparationPhase::Refilling => preparation.cache_refill_percent.map_or_else(
                || "Buffer refill in progress".to_owned(),
                |percent| format!("Buffer refill {percent:.0}%"),
            ),
            GuiSeekPreparationPhase::ReadyToJoin => "Ready; joining playback".to_owned(),
            GuiSeekPreparationPhase::CatchingUp => "Catching up to the room".to_owned(),
        };

        let mut details = vec![
            GuiWidgetNode::leaf(
                "main-window:seek-preparation:status",
                "Status",
                GuiWidgetKind::Status,
                Some(status),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "main-window:seek-preparation:target",
                "Seek target",
                GuiWidgetKind::Status,
                Some(target),
                true,
                false,
            ),
        ];
        if let Some(percent) = preparation.cache_refill_percent {
            details.push(
                GuiWidgetNode::leaf(
                    "main-window:seek-preparation:refill",
                    "Cache refill",
                    GuiWidgetKind::Status,
                    Some(format!("{percent:.0}%")),
                    true,
                    false,
                )
                .with_tooltip(
                    "Player-reported cache refill progress. This is not file download progress.",
                ),
            );
        }
        if let Some(seconds) = preparation.buffered_ahead_seconds {
            details.push(GuiWidgetNode::leaf(
                "main-window:seek-preparation:buffered-ahead",
                "Buffered ahead",
                GuiWidgetKind::Status,
                Some(format!("{seconds:.1} s")),
                true,
                false,
            ));
        }
        if let Some(position) = preparation.nearest_safe_buffered_position_seconds {
            details.push(GuiWidgetNode::leaf(
                "main-window:seek-preparation:nearest-buffered",
                "Nearest buffered position",
                GuiWidgetKind::Status,
                Some(format_seek_preparation_timestamp(position)),
                true,
                false,
            ));
        }

        let mut actions = Vec::new();
        if preparation.can_keep_waiting {
            actions.push(
                GuiWidgetNode::leaf(
                    "main-window:seek-preparation:keep-waiting",
                    "Keep waiting",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    true,
                )
                .with_tooltip("Continue waiting for this stream position to refill."),
            );
        }
        if preparation.can_cancel_and_remain {
            actions.push(
                GuiWidgetNode::leaf(
                    "main-window:seek-preparation:cancel",
                    "Cancel and remain here",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    false,
                )
                .with_tooltip(
                    "Cancel this local seek preparation and remain at the current position.",
                ),
            );
        }
        if preparation.can_join_nearest_buffered
            && preparation.nearest_safe_buffered_position_seconds.is_some()
        {
            actions.push(
                GuiWidgetNode::leaf(
                    "main-window:seek-preparation:join-nearest",
                    "Join nearest buffered position",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    false,
                )
                .with_tooltip(
                    "Opt in to the nearest position the player reports as safely buffered.",
                ),
            );
        }
        if !actions.is_empty() {
            details.push(GuiWidgetNode::layout(
                "main-window:seek-preparation:actions",
                "Seek preparation actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 150.0,
                },
                actions,
            ));
        }

        Some(
            GuiWidgetNode::branch(
                "main-window:seek-preparation",
                "Playback synchronization",
                GuiWidgetKind::Panel,
                details,
            )
            .with_tooltip(
                "Sorotte is holding the requested stream position steady while the player refills its cache.",
            ),
        )
    }

    pub(super) fn main_window_summary_projection(&self) -> (Option<GuiWidgetNode>, GuiWidgetNode) {
        let can_edit_room = self.pending_operation.is_none();
        let can_set_local_room = can_edit_room && !self.commands.can_disconnect_session;
        let can_request_runtime_room_change = can_edit_room && self.commands.can_disconnect_session;
        let room_draft = self
            .configuration
            .control_value(SettingId::ConnectionRoom)
            .unwrap_or_default()
            .to_owned();
        let has_room_draft = configured_room_name_text(&room_draft).is_some();
        let has_joined_room = joined_room_name_text(&self.main_window.room_name).is_some();
        let local_ready_available =
            self.main_window.playback.can_set_ready && self.pending_operation.is_none();
        let local_user_ready = self.displayed_local_main_window_user_ready();
        let ready_button = GuiWidgetNode::leaf(
            "main-window:control:set-ready",
            if local_user_ready {
                "Ready"
            } else {
                "Not Ready"
            },
            GuiWidgetKind::Button,
            None,
            local_ready_available,
            false,
        );
        let saved_session_target = self.saved_session_connect_target();
        let connection_status = match self.pending_operation.as_ref().map(|pending| pending.kind) {
            Some(GuiPendingOperationKind::ConnectSavedServer) => "connecting",
            Some(GuiPendingOperationKind::DisconnectSession) => "disconnecting",
            _ if self.commands.can_disconnect_session => "connected",
            _ if saved_session_target.is_some() => "disconnected",
            _ => "not-configured",
        };
        let connection_target = saved_session_target
            .as_ref()
            .map(|target| target.address.clone())
            .unwrap_or_else(|| "(not configured)".to_owned());
        let player_setup_panel = self.player_setup_issue.as_ref().map(|issue| {
            GuiWidgetNode::branch(
                "main-window:player-setup",
                "Playback Recovery",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:player-setup:title",
                        "Title",
                        GuiWidgetKind::Status,
                        self.player_setup_issue_title().map(str::to_owned),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:player-setup:summary",
                        "Summary",
                        GuiWidgetKind::Status,
                        self.player_setup_issue_summary().map(str::to_owned),
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:player-setup:detail",
                        "Detail",
                        GuiWidgetKind::Status,
                        Some(issue.message.clone()),
                        true,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:player-setup:actions",
                        "Playback Recovery Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:autodetect",
                                "Auto-detect mpv",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:choose-path",
                                "Choose mpv.exe",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:retry",
                                self.player_setup_retry_label(),
                                GuiWidgetKind::Button,
                                None,
                                self.player_setup_retry_available(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:player-setup:open-settings",
                                "Open Settings",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none(),
                                false,
                            ),
                        ],
                    ),
                ],
            )
        });

        let connection_status_tooltip = match connection_status {
            "connected" => format!("Connected to {connection_target}."),
            "connecting" => format!("Connecting to {connection_target}."),
            "disconnecting" => "Disconnecting from the current session.".to_owned(),
            "disconnected" => format!("Disconnected. Saved server: {connection_target}."),
            _ => "No server is configured.".to_owned(),
        };
        let room_control_tooltip = self.main_window.room_control_status.clone();
        let room_playback_intent_label = self.main_window.room_playback_intent.status_label();
        let room_playback_state_tooltip = self.main_window.room_playback_intent.detail_tooltip();
        let mut participant_indices: Vec<usize> = self
            .main_window
            .users
            .iter()
            .enumerate()
            .filter(|(_, user)| user.room_name == self.main_window.room_name)
            .map(|(index, _)| index)
            .collect();
        participant_indices.sort_by_key(|index| {
            let user = &self.main_window.users[*index];
            (!user.is_self, *index)
        });
        let mut participant_children = participant_indices
            .into_iter()
            .map(|user_index| {
                let user = &self.main_window.users[user_index];
                let legacy_readiness;
                let readiness = if let Some(readiness) = self
                    .main_window
                    .readiness
                    .get(&user.username)
                {
                    readiness
                } else {
                    legacy_readiness = sorotte_client_app::app_boundary::readiness::ParticipantReadinessPresentation::from_legacy(
                        user.username.clone(),
                        user.is_ready,
                    );
                    &legacy_readiness
                };
                let mut cue_parts = Vec::new();
                if !user.has_file {
                    cue_parts.push("no-file".to_owned());
                }
                if user.filename_differs {
                    cue_parts.push("name-diff".to_owned());
                }
                if user.filesize_differs {
                    cue_parts.push("size-diff".to_owned());
                }
                if user.fileduration_differs {
                    cue_parts.push("duration-diff".to_owned());
                }
                if user.file_is_url && !user.file_is_trusted {
                    cue_parts.push("untrusted-url".to_owned());
                }
                let cue_suffix = if cue_parts.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", cue_parts.join(", "))
                };
                let participant_status_tooltip =
                    participant_status_tooltip(user, readiness, &user.participant_status);
                let mut user_children = vec![GuiWidgetNode::layout(
                    format!("main-window:user:{user_index}:summary"),
                    format!("{} Summary", user.username),
                    GuiLayoutMode::KeyValueGrid {
                        min_pair_width: 220.0,
                    },
                    vec![
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:state"),
                            "State",
                            GuiWidgetKind::Status,
                            Some(format!(
                                "self={}, ready={}, controller={}",
                                bool_label(user.is_self),
                                bool_label(user.is_ready),
                                bool_label(user.is_controller),
                            )),
                            true,
                            user.is_self,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:participant-status"),
                            "Observed playback",
                            GuiWidgetKind::Status,
                            Some(user.participant_status.compact_label()),
                            true,
                            false,
                        )
                        .with_status_tone(participant_status_tone(&user.participant_status))
                        .with_tooltip(participant_status_tooltip.clone()),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-phase"),
                            "Playback phase",
                            GuiWidgetKind::Status,
                            Some(user.participant_status.phase_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-position"),
                            "Media position",
                            GuiWidgetKind::Status,
                            Some(user.participant_status.position_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-offset"),
                            "Room offset",
                            GuiWidgetKind::Status,
                            Some(user.participant_status.offset_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-buffer"),
                            "Buffer",
                            GuiWidgetKind::Status,
                            Some(user.participant_status.buffer_label()),
                            true,
                            false,
                        )
                        .with_tooltip(
                            "Buffered ahead is playable media headroom. Cache refill is mpv's refill-target progress, not total media download progress.",
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-freshness"),
                            "Status heartbeat",
                            GuiWidgetKind::Status,
                            Some(user.participant_status.freshness_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-player"),
                            "Player availability",
                            GuiWidgetKind::Status,
                            Some(member_player_availability_label(&user.participant_status)),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-logical-pause"),
                            "Logical pause",
                            GuiWidgetKind::Status,
                            Some(member_logical_pause_label(&user.participant_status)),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-rate"),
                            "Playback rate",
                            GuiWidgetKind::Status,
                            Some(member_playback_rate_label(&user.participant_status)),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-generation"),
                            "Media generation",
                            GuiWidgetKind::Status,
                            Some(member_generation_label(&user.participant_status)),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:member-revision"),
                            "Room revision",
                            GuiWidgetKind::Status,
                            Some(member_revision_label(&user.participant_status)),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:start-barrier"),
                            "Start barrier participant",
                            GuiWidgetKind::Status,
                            Some(
                                user.start_barrier_status
                                    .clone()
                                    .unwrap_or_else(|| "inactive or unavailable".to_owned()),
                            ),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness"),
                            "Readiness",
                            GuiWidgetKind::Status,
                            Some(readiness.status_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-intent"),
                            "Intent",
                            GuiWidgetKind::Status,
                            Some(readiness.intent_detail_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-technical"),
                            "Technical",
                            GuiWidgetKind::Status,
                            Some(readiness.technical_detail_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-eligibility"),
                            "Start eligibility",
                            GuiWidgetKind::Status,
                            Some(readiness.eligibility_detail_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-participation"),
                            "Start cohort",
                            GuiWidgetKind::Status,
                            Some(readiness.participation_detail_label().to_owned()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-gate"),
                            "Automatic start",
                            GuiWidgetKind::Status,
                            Some(readiness.start_gate_detail_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-revision"),
                            "Readiness revision",
                            GuiWidgetKind::Status,
                            Some(readiness.revision_detail_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:readiness-operation"),
                            "Readiness operation",
                            GuiWidgetKind::Status,
                            Some(readiness.operation_detail_label()),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:size"),
                            "Size",
                            GuiWidgetKind::Status,
                            Some(if user.file_size_label.is_empty() {
                                "(none)".to_owned()
                            } else {
                                user.file_size_label.clone()
                            }),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:duration"),
                            "Duration",
                            GuiWidgetKind::Status,
                            Some(if user.file_duration_label.is_empty() {
                                "(none)".to_owned()
                            } else {
                                user.file_duration_label.clone()
                            }),
                            true,
                            false,
                        ),
                        GuiWidgetNode::leaf(
                            format!("main-window:user:{user_index}:file"),
                            "File",
                            GuiWidgetKind::Status,
                            Some(format!("{}{}", user.file_name_label, cue_suffix)),
                            true,
                            false,
                        ),
                    ],
                )];
                if user.is_self {
                    user_children.push(ready_button.clone());
                }

                let mut user_panel = GuiWidgetNode::branch(
                    format!("main-window:user:{user_index}"),
                    &user.username,
                    GuiWidgetKind::Panel,
                    user_children,
                );
                user_panel.selected = user.is_self;
                user_panel.with_tooltip(participant_status_tooltip)
            })
            .collect::<Vec<_>>();
        if participant_children.is_empty() {
            participant_children.push(GuiWidgetNode::leaf(
                "main-window:participants:empty",
                "Participants",
                GuiWidgetKind::Status,
                Some("No users in this room.".to_owned()),
                true,
                false,
            ));
        }

        let mut session_summary_children = vec![
            GuiWidgetNode::leaf(
                "main-window:connection-status",
                "Status",
                GuiWidgetKind::Status,
                Some(connection_status.to_owned()),
                true,
                false,
            )
            .with_tooltip(connection_status_tooltip),
            GuiWidgetNode::leaf(
                "main-window:connection-target",
                "Server",
                GuiWidgetKind::Status,
                Some(connection_target),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "main-window:room",
                "Room",
                GuiWidgetKind::Status,
                Some(self.main_window.room_name.clone()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "main-window:room-control",
                "Room Control",
                GuiWidgetKind::Status,
                Some(self.main_window.room_control_status.clone()),
                true,
                false,
            )
            .with_tooltip(room_control_tooltip),
            GuiWidgetNode::leaf(
                "main-window:room-playback-state",
                "Room Intent",
                GuiWidgetKind::Status,
                Some(room_playback_intent_label),
                true,
                false,
            )
            .with_tooltip(room_playback_state_tooltip),
            GuiWidgetNode::layout(
                "main-window:room-header:actions",
                "Room Header Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 104.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:connection:connect",
                        self.saved_session_connect_button_label(),
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_connect_saved_server,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:connection:disconnect",
                        "Disconnect",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_disconnect_session,
                        false,
                    ),
                ],
            ),
        ];

        if let Some(header_actions) = session_summary_children
            .iter_mut()
            .find(|node| node.id == "main-window:room-header:actions")
        {
            header_actions.children.insert(
                1,
                GuiWidgetNode::leaf(
                    "main-window:room-actions:toggle",
                    "Change Room",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    self.main_window_room_change_expanded,
                ),
            );
        }

        if self.main_window_room_change_expanded {
            session_summary_children.push(GuiWidgetNode::branch(
                "main-window:room-actions",
                "Room",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "main-window:room-actions:form",
                        "Room Actions Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:room-input",
                            "Room",
                            GuiWidgetKind::TextInput,
                            Some(room_draft),
                            can_edit_room,
                            false,
                        )],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:room-actions:buttons",
                        "Room Action Buttons",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:room:set",
                                "Set Room",
                                GuiWidgetKind::Button,
                                None,
                                can_set_local_room && has_room_draft,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:room:join",
                                "Join Room",
                                GuiWidgetKind::Button,
                                None,
                                can_request_runtime_room_change && has_room_draft,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:room:leave",
                                "Leave Room",
                                GuiWidgetKind::Button,
                                None,
                                can_request_runtime_room_change && has_joined_room,
                                false,
                            ),
                        ],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:controller-actions",
                        "Controller Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:room-actions:create-controlled-room",
                                "Create Controlled Room",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none() && has_joined_room,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:room-actions:identify-controller",
                                "Identify As Controller",
                                GuiWidgetKind::Button,
                                None,
                                self.pending_operation.is_none()
                                    && self.main_window.room_name.as_str().starts_with('+'),
                                false,
                            ),
                        ],
                    ),
                ],
            ));
        }

        session_summary_children.push(GuiWidgetNode::layout(
            "main-window:participants",
            "Participants",
            GuiLayoutMode::Stack,
            participant_children,
        ));

        let session_summary = GuiWidgetNode::branch(
            "main-window:connection",
            "Room",
            GuiWidgetKind::Panel,
            session_summary_children,
        )
        .with_min_content_height(320.0);

        let summary_column = GuiWidgetNode::layout(
            "main-window:summary-column",
            "Summary Column",
            GuiLayoutMode::Stack,
            self.seek_preparation_panel()
                .into_iter()
                .chain([session_summary.clone()])
                .collect(),
        );

        (player_setup_panel, summary_column)
    }

    pub(super) fn playlist_source_button_node(&self, index: usize) -> Option<GuiWidgetNode> {
        let row = self.main_window.playlist.get(index)?;
        let mut source_button = GuiWidgetNode::branch(
            format!("main-window:playlist:{index}:source"),
            row.source_state.current_label.clone(),
            GuiWidgetKind::Button,
            row.source_state
                .options
                .iter()
                .map(|option| {
                    let mut option_node = GuiWidgetNode::leaf(
                        format!(
                            "main-window:playlist:{index}:source:{}",
                            option.provider_id.as_str()
                        ),
                        option.label.clone(),
                        GuiWidgetKind::Button,
                        Some(option.status.label().to_owned()),
                        option.enabled,
                        option.selected,
                    );
                    if let Some(detail) = option.detail.as_ref() {
                        option_node = option_node.with_tooltip(detail.clone());
                    }
                    option_node
                })
                .collect(),
        )
        .with_tooltip(playlist_source_tooltip(&row.source_state));
        source_button.value = Some(row.source_state.status.label().to_owned());
        Some(source_button)
    }
}

fn format_seek_preparation_timestamp(seconds: f64) -> String {
    let total_seconds = seconds.max(0.0).round() as u64;
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn member_player_availability_label(status: &MainWindowParticipantStatusPresentation) -> String {
    status.connection_label()
}

fn member_report_evidence_is_unavailable(status: &MainWindowParticipantStatusReport) -> bool {
    status.freshness == MainWindowParticipantStatusFreshness::Stale
        || status.timeline_mismatch
        || status.status.correlation
            == Some(sorotte_protocol::ParticipantStatusCorrelation::Superseded)
}

fn member_logical_pause_label(status: &MainWindowParticipantStatusPresentation) -> String {
    let MainWindowParticipantStatusPresentation::Report(status) = status else {
        return "unavailable".to_owned();
    };
    if member_report_evidence_is_unavailable(status) {
        return "unavailable".to_owned();
    }
    status
        .status
        .logical_paused
        .map(|paused| if paused { "yes" } else { "no" }.to_owned())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn member_playback_rate_label(status: &MainWindowParticipantStatusPresentation) -> String {
    let MainWindowParticipantStatusPresentation::Report(status) = status else {
        return "unavailable".to_owned();
    };
    if member_report_evidence_is_unavailable(status) {
        return "unavailable".to_owned();
    }
    status
        .status
        .playback_rate
        .map(|rate| format!("{rate:.2}×"))
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn member_generation_label(status: &MainWindowParticipantStatusPresentation) -> String {
    let MainWindowParticipantStatusPresentation::Report(status) = status else {
        return "unavailable".to_owned();
    };
    if member_report_evidence_is_unavailable(status) {
        return "unavailable".to_owned();
    }
    status
        .status
        .playback_scope
        .map(|scope| scope.media_generation)
        .map(|generation| generation.to_string())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn member_revision_label(status: &MainWindowParticipantStatusPresentation) -> String {
    let MainWindowParticipantStatusPresentation::Report(status) = status else {
        return "unavailable".to_owned();
    };
    if member_report_evidence_is_unavailable(status) {
        return "unavailable".to_owned();
    }
    status
        .status
        .playback_scope
        .and_then(|scope| scope.state_revision)
        .map(|revision| revision.to_string())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn participant_status_tooltip(
    user: &MainWindowUserRow,
    readiness: &sorotte_client_app::app_boundary::readiness::ParticipantReadinessPresentation,
    status: &MainWindowParticipantStatusPresentation,
) -> String {
    let mut details = vec![
        "Room session: present".to_owned(),
        status.detail_label(),
        format!("Readiness: {}", readiness.status_label()),
        format!(
            "Technical readiness: {}",
            readiness.technical_detail_label()
        ),
        format!(
            "Automatic start cohort: {}",
            readiness.participation_detail_label()
        ),
        format!(
            "Start barrier participant: {}",
            user.start_barrier_status
                .as_deref()
                .unwrap_or("inactive or unavailable")
        ),
    ];
    if matches!(status, MainWindowParticipantStatusPresentation::Report(report) if report.status.cache_percent.is_some())
    {
        details.push(
            "Cache refill is player refill-target progress, not total media download progress."
                .to_owned(),
        );
    }
    details.join("\n")
}

fn playlist_source_tooltip(source_state: &GuiPlaylistSourceState) -> String {
    let mut lines = vec![format!("Current source: {}", source_state.current_label)];
    if source_state.policy == GuiPlaylistSourcePolicy::Automatic {
        lines.push("Selection policy: Automatic".to_owned());
    } else {
        lines.push(format!(
            "Selected provider: {}",
            source_state
                .preferred_provider_id
                .as_ref()
                .unwrap_or(&source_state.current_provider_id)
                .as_str()
        ));
    }
    if let Some(provider_id) = source_state.resolved_provider_id.as_ref() {
        lines.push(format!("Resolved provider: {}", provider_id.as_str()));
    }
    lines.push(format!("Status: {}", source_state.status.label()));
    if let Some(detail) = source_state.detail.as_deref() {
        lines.push(detail.to_owned());
    }
    if !source_state.resolution_steps.is_empty() {
        lines.push("Resolution steps:".to_owned());
        lines.extend(source_state.resolution_steps.iter().map(|step| {
            let mut line = format!("- {}: {}", step.label, step.status.label());
            if let Some(detail) = step.detail.as_deref() {
                line.push_str(" - ");
                line.push_str(detail);
            }
            line
        }));
    }
    lines.join("\n")
}
