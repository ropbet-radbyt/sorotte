use super::*;

use sorotte_client_app::app_boundary::readiness::ParticipantReadinessPresentation;
use sorotte_protocol::{
    MixedReadinessPolicy, ParticipantPlaybackPhase, ParticipantPlaybackScope,
    ParticipantPlayerConnection, ParticipantStatusAvailability, ParticipantStatusCorrelation,
    ParticipantStatusView, ParticipantTimelineKind, RoomStartGatePhase, StartGateDegradedReason,
};

#[test]
fn main_window_contact_info_follows_the_saved_gui_preference() {
    let hidden = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        show_contact_info: Some(false),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        hidden
            .main_window_widget_tree()
            .find("main-window:contact-info")
            .is_none()
    );

    let shown = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        show_contact_info: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let shown_tree = shown.main_window_widget_tree();
    let contact = shown_tree
        .find("main-window:contact-info")
        .expect("saved contact-info preference should project support details");
    assert_eq!(contact.kind, GuiWidgetKind::Status);
    assert_eq!(
        contact.value.as_deref(),
        Some("Report issues: github.com/ropbet-radbyt/sorotte")
    );
}

#[test]
fn gui_shell_app_state_projects_main_window_widget_trees() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        username: Some("Alice".to_owned()),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("hello widget ".to_owned()),
        },
    )));

    let tree = state.main_window_widget_tree();
    assert_eq!(tree.label, "Room");
    assert!(tree.find("main-window:tabs").is_none());
    assert!(tree.find("main-window:tab:overview").is_none());
    let room_panel = tree
        .find("main-window:connection")
        .expect("combined room panel should exist in widget tree");
    assert_eq!(room_panel.kind, GuiWidgetKind::Panel);
    assert_eq!(room_panel.label, "Room");
    assert!(tree.find("main-window:browser").is_some());
    let participants = tree
        .find("main-window:participants")
        .expect("current-room participants should exist in widget tree");
    assert_eq!(
        participants
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main-window:user:0", "main-window:user:1"]
    );
    let local_user_state = tree
        .find("main-window:user:0:state")
        .expect("local user state should exist in widget tree");
    assert_eq!(local_user_state.kind, GuiWidgetKind::Status);
    assert!(local_user_state.selected);
    let selected_remote_user_state = tree
        .find("main-window:user:1:state")
        .expect("remote user state should exist in widget tree");
    assert_eq!(selected_remote_user_state.kind, GuiWidgetKind::Status);
    assert!(!selected_remote_user_state.selected);
    assert!(tree.find("main-window:user:new").is_none());
    assert!(tree.find("main-window:user:1:open").is_none());
    assert!(tree.find("main-window:user:1:ready").is_none());
    let room_toggle = tree
        .find("main-window:room-actions:toggle")
        .expect("room-change toggle should exist in widget tree");
    assert_eq!(room_toggle.kind, GuiWidgetKind::Button);
    assert_eq!(room_toggle.label, "Change Room");
    assert!(!room_toggle.selected);
    assert!(
        tree.find("main-window:room-input").is_none(),
        "room-change form should be collapsed by default"
    );

    assert!(state.apply(GuiShellAction::ToggleMainWindowRoomChange));
    let tree = state.main_window_widget_tree();
    let room_toggle = tree
        .find("main-window:room-actions:toggle")
        .expect("room-change toggle should still exist in widget tree");
    assert_eq!(room_toggle.label, "Change Room");
    assert!(room_toggle.selected);
    let room_input = tree
        .find("main-window:room-input")
        .expect("room input should exist once room change is expanded");
    assert_eq!(room_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(room_input.label, "Room");
    assert_eq!(room_input.value.as_deref(), Some("Lounge"));
    assert!(room_input.enabled);
    assert!(tree.find("main-window:username").is_none());
    let room_control = tree
        .find("main-window:room-control")
        .expect("room-control status should exist in widget tree");
    assert_eq!(room_control.kind, GuiWidgetKind::Status);
    assert_eq!(
        room_control.value.as_deref(),
        Some("Unavailable: no active server session.")
    );

    let playlist = tree
        .find("main-window:playlist:1")
        .expect("selected playlist row should exist in widget tree");
    assert_eq!(playlist.kind, GuiWidgetKind::ListItem);
    assert!(playlist.selected);
    let playlist_add_files = tree
        .find("main-window:playlist:add-files")
        .expect("playlist add-files button should exist in widget tree");
    assert_eq!(playlist_add_files.kind, GuiWidgetKind::Button);
    let playlist_add_url = tree
        .find("main-window:playlist:add-url")
        .expect("playlist add-url button should exist in widget tree");
    assert_eq!(playlist_add_url.kind, GuiWidgetKind::Button);
    let playlist_add_plex = tree
        .find("main-window:playlist:add-plex")
        .expect("playlist add-plex button should exist in widget tree");
    assert_eq!(playlist_add_plex.kind, GuiWidgetKind::Button);
    assert!(
        !playlist_add_plex.enabled,
        "Plex playlist picker should be disabled until a Plex server is selected"
    );
    let playlist_header = tree
        .find("main-window:playlist-header:actions")
        .expect("playlist header actions should exist in widget tree");
    assert_eq!(
        playlist_header
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "main-window:playlist:add-files",
            "main-window:playlist:add-url",
            "main-window:playlist:add-plex",
            "main-window:playlist:more-menu",
        ]
    );
    let playlist_more_menu = tree
        .find("main-window:playlist:more-menu")
        .expect("playlist more menu should exist in widget tree");
    assert!(
        playlist_more_menu.enabled,
        "playlist More menu should remain expandable even when some nested actions are disabled"
    );
    assert!(
        playlist_more_menu
            .children
            .iter()
            .any(|child| child.id == "main-window:playlist:load")
    );
    assert!(
        playlist_more_menu
            .children
            .iter()
            .any(|child| child.id == "main-window:playlist:save")
    );
    assert_eq!(
        playlist_more_menu
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main-window:playlist:load", "main-window:playlist:save"]
    );
    let playlist_row_remove = tree
        .find("main-window:playlist:1:remove")
        .expect("playlist row remove action should exist on the selected row");
    assert_eq!(playlist_row_remove.kind, GuiWidgetKind::Button);
    assert!(
        tree.find("main-window:playlist-selection:actions")
            .is_none()
    );
    assert!(tree.find("main-window:playlist:count").is_none());
    assert!(tree.find("main-window:playlist-empty").is_none());
    assert!(tree.find("main-window:playlist:new").is_none());
    assert!(tree.find("main-window:playlist:add").is_none());
    assert!(tree.find("main-window:user:add").is_none());

    let chat_input = tree
        .find("main-window:chat-input")
        .expect("chat input should exist in widget tree");
    assert_eq!(chat_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(chat_input.value.as_deref(), Some("hello widget "));
    assert_eq!(chat_input.enabled, state.commands.can_send_chat_message);
}

#[test]
fn strict_mixed_room_explains_automatic_start_unavailability_in_widget_tree() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("Alice".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut readiness = ParticipantReadinessPresentation::from_legacy("Alice", false);
    readiness.mixed_readiness_policy = Some(MixedReadinessPolicy::RequireAllMembers);
    readiness.start_gate_phase = Some(RoomStartGatePhase::Degraded {
        media_generation: 7,
        reason: StartGateDegradedReason::IncompatibleLegacyParticipant,
    });
    state
        .main_window
        .readiness
        .insert("Alice".to_owned(), readiness);

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:user:0:readiness-participation")
            .and_then(|node| node.value.as_deref()),
        Some(
            "legacy participant; automatic start unavailable until every member supports readiness V2"
        )
    );
    assert_eq!(
        tree.find("main-window:user:0:readiness-gate")
            .and_then(|node| node.value.as_deref()),
        Some("automatic start unavailable: a room member does not support readiness V2")
    );
}

#[test]
fn room_intent_and_member_observation_remain_explicit_and_separate() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("Alice".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.room_playback_intent.position_seconds = Some(755.03);
    state.main_window.room_playback_intent.paused = Some(false);
    state.main_window.room_playback_intent.set_by = Some("server".to_owned());
    state.main_window.room_playback_intent.authority =
        Some("server start barrier, generation 7, revision 19".to_owned());
    state.main_window.room_playback_intent.start_gate = Some("committed by server".to_owned());
    let mut observed = ParticipantStatusView::new(ParticipantStatusAvailability::Delayed);
    observed.correlation = Some(ParticipantStatusCorrelation::Exact);
    observed.playback_scope = Some(ParticipantPlaybackScope::new(7).with_state_revision(19));
    observed.player_connection = Some(ParticipantPlayerConnection::Connected);
    observed.phase = Some(ParticipantPlaybackPhase::Rebuffering);
    observed.timeline_kind = Some(ParticipantTimelineKind::Vod);
    observed.position_seconds = Some(751.2);
    observed.logical_paused = Some(false);
    observed.playback_rate = Some(1.0);
    observed.buffered_ahead_seconds = Some(0.4);
    observed.cache_percent = Some(20.0);
    observed.report_age_ms = Some(4_200);
    observed.sample_age_ms = Some(4_200);
    observed.position_sample_age_ms = Some(4_200);
    observed.room_offset_seconds = Some(-3.83);
    state.main_window.users[0].participant_status = MainWindowParticipantStatusPresentation::Report(
        MainWindowParticipantStatusReport::from_client_view(
            sorotte_client_core::ClientParticipantStatusView::from_wire(observed),
            false,
        ),
    );
    state.main_window.users[0].start_barrier_status = Some("pending".to_owned());

    let tree = state.main_window_widget_tree();
    let room_intent = tree
        .find("main-window:room-playback-state")
        .expect("authoritative room intent should be visible");
    assert_eq!(
        room_intent.value.as_deref(),
        Some("Room intent: PLAYING · 12:35.0 · Start gate: committed by server")
    );
    let room_tooltip = room_intent
        .tooltip
        .as_deref()
        .expect("room intent should explain its authority");
    assert!(room_tooltip.contains("Authoritative room intent: playing"));
    assert!(room_tooltip.contains("Set by: server"));
    assert!(room_tooltip.contains("member playback rows are observed advisory status"));

    let participant_status = tree
        .find("main-window:user:0:participant-status")
        .expect("participant status should be visible");
    assert_eq!(participant_status.status_tone, Some(GuiStatusTone::Warning));
    assert_eq!(
        participant_status.value.as_deref(),
        Some(
            "Rebuffering · 12:31.2 · Offset unavailable · 0.4 s buffered · cache refill 20% · delayed"
        )
    );
    let member_tooltip = participant_status
        .tooltip
        .as_deref()
        .expect("participant status should expose detailed diagnostics");
    for expected in [
        "Room session: present",
        "Sorotte connection: delayed · 4.5 s old",
        "Player: connected",
        "Logical pause: no",
        "Playback rate: 1.00×",
        "Media generation: 7",
        "Room revision: 19",
        "Technical readiness:",
        "Automatic start cohort:",
        "Start barrier participant: pending",
        "not total media download progress",
    ] {
        assert!(member_tooltip.contains(expected), "missing {expected:?}");
    }
    let browser_status = tree
        .find("main-window:user:browser:0:participant-status")
        .expect("room browser participant status should use the same typed projection");
    assert_eq!(browser_status.status_tone, Some(GuiStatusTone::Warning));
    assert!(
        browser_status
            .tooltip
            .as_deref()
            .is_some_and(|tooltip| tooltip.contains("Player: connected"))
    );

    let MainWindowParticipantStatusPresentation::Report(report) =
        &mut state.main_window.users[0].participant_status
    else {
        panic!("test participant should retain a report");
    };
    report.freshness = MainWindowParticipantStatusFreshness::Stale;
    report.report_age_seconds = Some(12.0);
    let stale_tree = state.main_window_widget_tree();
    let stale_status = stale_tree
        .find("main-window:user:0:participant-status")
        .expect("stale participant status should remain projected");
    assert_eq!(
        stale_status.value.as_deref(),
        Some("Status stale · last update 12.0 s ago"),
        "stale summaries must not repeat old position, phase, offset, or buffer evidence"
    );
    assert_eq!(stale_status.status_tone, Some(GuiStatusTone::Danger));
    let stale_tooltip = stale_status
        .tooltip
        .as_deref()
        .expect("stale participant status should retain truthful diagnostics");
    assert!(stale_tooltip.contains("Last reported player: connected"));
    assert!(stale_tooltip.contains("Playback evidence: unavailable (status stale)"));
    assert!(!stale_tooltip.contains("Playback: Rebuffering"));
    assert!(!stale_tooltip.contains("Timestamp: 12:31.2"));
}

#[test]
fn participant_status_diagnostic_widgets_project_exact_member_evidence_labels() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("Alice".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let report = |player_connection, logical_paused, playback_rate, playback_scope| {
        let mut status = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
        status.player_connection = player_connection;
        status.phase = Some(ParticipantPlaybackPhase::Playing);
        status.logical_paused = logical_paused;
        status.playback_rate = playback_rate;
        status.playback_scope = playback_scope;
        status.sample_age_ms = Some(0);
        status.report_age_ms = Some(500);
        MainWindowParticipantStatusPresentation::Report(
            MainWindowParticipantStatusReport::from_client_view(
                sorotte_client_core::ClientParticipantStatusView::from_wire(status),
                false,
            ),
        )
    };
    let mut post_construction_scope_mismatch = report(
        Some(ParticipantPlayerConnection::Connected),
        Some(false),
        Some(1.5),
        Some(ParticipantPlaybackScope::new(10).with_state_revision(22)),
    );
    let MainWindowParticipantStatusPresentation::Report(mismatched) =
        &mut post_construction_scope_mismatch
    else {
        unreachable!();
    };
    mismatched.status.correlation = Some(ParticipantStatusCorrelation::Superseded);
    let mut post_construction_stale = report(
        Some(ParticipantPlayerConnection::Connected),
        Some(false),
        Some(1.5),
        Some(ParticipantPlaybackScope::new(10).with_state_revision(22)),
    );
    let MainWindowParticipantStatusPresentation::Report(stale) = &mut post_construction_stale
    else {
        unreachable!();
    };
    stale.freshness = MainWindowParticipantStatusFreshness::Stale;
    let mut post_construction_timeline_mismatch = report(
        Some(ParticipantPlayerConnection::Connected),
        Some(false),
        Some(1.5),
        Some(ParticipantPlaybackScope::new(10).with_state_revision(22)),
    );
    let MainWindowParticipantStatusPresentation::Report(timeline_mismatch) =
        &mut post_construction_timeline_mismatch
    else {
        unreachable!();
    };
    timeline_mismatch.timeline_mismatch = true;
    let mut uncorrelated_local_evidence = report(
        Some(ParticipantPlayerConnection::Connected),
        Some(false),
        Some(1.25),
        Some(ParticipantPlaybackScope::new(7).with_state_revision(19)),
    );
    let MainWindowParticipantStatusPresentation::Report(uncorrelated) =
        &mut uncorrelated_local_evidence
    else {
        unreachable!();
    };
    uncorrelated.status.correlation = Some(ParticipantStatusCorrelation::Uncorrelated);
    let cases = [
        (
            "status unavailable",
            MainWindowParticipantStatusPresentation::Unavailable,
            [
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "legacy client",
            MainWindowParticipantStatusPresentation::LegacyClient,
            [
                "unavailable (legacy client)",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "waiting for first report",
            MainWindowParticipantStatusPresentation::WaitingForFirstReport,
            [
                "waiting for first report",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "report without player evidence",
            report(None, None, None, None),
            [
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "player starting",
            report(
                Some(ParticipantPlayerConnection::Starting),
                None,
                None,
                None,
            ),
            [
                "starting",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "connected with zero-valued scope",
            report(
                Some(ParticipantPlayerConnection::Connected),
                Some(true),
                Some(1.25),
                Some(ParticipantPlaybackScope::new(0).with_state_revision(0)),
            ),
            ["connected", "yes", "1.25×", "0", "0"],
        ),
        (
            "post-construction scope mismatch",
            post_construction_scope_mismatch,
            [
                "connected",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "post-construction stale evidence",
            post_construction_stale,
            [
                "connected",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "post-construction timeline mismatch",
            post_construction_timeline_mismatch,
            [
                "connected",
                "unavailable",
                "unavailable",
                "unavailable",
                "unavailable",
            ],
        ),
        (
            "uncorrelated local evidence",
            uncorrelated_local_evidence,
            ["connected", "no", "1.25×", "7", "19"],
        ),
        (
            "disconnected with revision unavailable",
            report(
                Some(ParticipantPlayerConnection::Disconnected),
                Some(false),
                Some(0.75),
                Some(ParticipantPlaybackScope::new(8)),
            ),
            [
                "disconnected",
                "unavailable",
                "unavailable",
                "8",
                "unavailable",
            ],
        ),
        (
            "player failed",
            report(
                Some(ParticipantPlayerConnection::Failed),
                Some(false),
                Some(1.0),
                Some(ParticipantPlaybackScope::new(9).with_state_revision(21)),
            ),
            ["failed", "unavailable", "unavailable", "9", "21"],
        ),
    ];
    let diagnostic_ids = [
        "main-window:user:0:member-player",
        "main-window:user:0:member-logical-pause",
        "main-window:user:0:member-rate",
        "main-window:user:0:member-generation",
        "main-window:user:0:member-revision",
    ];

    for (case, presentation, expected_values) in cases {
        state.main_window.users[0].participant_status = presentation;
        let tree = state.main_window_widget_tree();
        for (widget_id, expected_value) in diagnostic_ids.iter().zip(expected_values) {
            let node = tree
                .find(widget_id)
                .unwrap_or_else(|| panic!("{case}: missing production widget {widget_id}"));
            assert_eq!(
                node.value.as_deref(),
                Some(expected_value),
                "{case}: {widget_id} must project the exact participant evidence label"
            );
        }
    }
}

#[test]
fn participant_status_tones_are_derived_from_typed_status_not_display_text() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("Alice".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    let report = |player_connection, phase, freshness, timeline_mismatch| {
        let (availability, report_age_ms) = match freshness {
            MainWindowParticipantStatusFreshness::Fresh => {
                (ParticipantStatusAvailability::Fresh, 500)
            }
            MainWindowParticipantStatusFreshness::Delayed => {
                (ParticipantStatusAvailability::Delayed, 4_000)
            }
            MainWindowParticipantStatusFreshness::Stale => {
                (ParticipantStatusAvailability::Stale, 12_000)
            }
            _ => (ParticipantStatusAvailability::Fresh, 500),
        };
        let mut status = ParticipantStatusView::new(availability);
        status.player_connection = player_connection;
        status.phase = phase;
        status.report_age_ms = Some(report_age_ms);
        let mut report = MainWindowParticipantStatusReport::from_client_view(
            sorotte_client_core::ClientParticipantStatusView::from_wire(status),
            timeline_mismatch,
        );
        report.freshness = freshness;
        MainWindowParticipantStatusPresentation::Report(report)
    };
    let cases = [
        (
            MainWindowParticipantStatusPresentation::Unavailable,
            GuiStatusTone::Warning,
        ),
        (
            MainWindowParticipantStatusPresentation::WaitingForFirstReport,
            GuiStatusTone::Warning,
        ),
        (
            MainWindowParticipantStatusPresentation::LegacyClient,
            GuiStatusTone::Muted,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Connected),
                Some(ParticipantPlaybackPhase::Playing),
                MainWindowParticipantStatusFreshness::Fresh,
                false,
            ),
            GuiStatusTone::Success,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Connected),
                Some(ParticipantPlaybackPhase::Playing),
                MainWindowParticipantStatusFreshness::Unknown,
                false,
            ),
            GuiStatusTone::Muted,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Connected),
                Some(ParticipantPlaybackPhase::ReadyPaused),
                MainWindowParticipantStatusFreshness::Fresh,
                false,
            ),
            GuiStatusTone::Muted,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Connected),
                Some(ParticipantPlaybackPhase::ReadyPaused),
                MainWindowParticipantStatusFreshness::Fresh,
                true,
            ),
            GuiStatusTone::Warning,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Unavailable),
                Some(ParticipantPlaybackPhase::Unknown),
                MainWindowParticipantStatusFreshness::Fresh,
                false,
            ),
            GuiStatusTone::Warning,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Disconnected),
                Some(ParticipantPlaybackPhase::Playing),
                MainWindowParticipantStatusFreshness::Fresh,
                false,
            ),
            GuiStatusTone::Warning,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Failed),
                Some(ParticipantPlaybackPhase::Playing),
                MainWindowParticipantStatusFreshness::Fresh,
                false,
            ),
            GuiStatusTone::Danger,
        ),
        (
            report(
                Some(ParticipantPlayerConnection::Connected),
                Some(ParticipantPlaybackPhase::Playing),
                MainWindowParticipantStatusFreshness::Stale,
                false,
            ),
            GuiStatusTone::Danger,
        ),
    ];

    for (status, expected_tone) in cases {
        state.main_window.users[0].participant_status = status;
        let tree = state.main_window_widget_tree();
        for widget_id in [
            "main-window:user:0:participant-status",
            "main-window:user:browser:0:participant-status",
        ] {
            let node = tree
                .find(widget_id)
                .unwrap_or_else(|| panic!("{widget_id} should be projected"));
            assert_eq!(
                node.status_tone,
                Some(expected_tone),
                "{widget_id} should carry the typed presentation tone for {:?}",
                state.main_window.users[0].participant_status
            );
        }
    }
}

#[test]
fn gui_shell_app_state_displays_plex_playlist_rows_by_media_name() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let media_name = "[EG]Gurren_Lagann_03_BD(720p_10bit)[BB5590A5].mkv";
    let playlist_entry = format_plex_playlist_uri(&PlexPlaylistUri {
        machine_identifier: "3f6ba9fad8b4b33a803f1151b5d49ee1fd83e860".to_owned(),
        rating_key: "2918".to_owned(),
        title: Some("Gurren Lagann Episode 3".to_owned()),
        file_name: Some(media_name.to_owned()),
        duration_millis: Some(1_452_000),
        size_bytes: Some(657_000_000),
        media_type: Some(PlexMediaType::Episode),
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            playlist_entry.clone(),
        ]))
    );

    let tree = state.main_window_widget_tree();
    let playlist_row = tree
        .find("main-window:playlist:0")
        .expect("Plex playlist row should exist");
    assert_eq!(playlist_row.label, media_name);
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec![playlist_entry]
    );
}

#[test]
fn gui_shell_app_state_projects_compact_playback_controls_and_ready_button_text() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.can_toggle_pause = true;
    snapshot.can_seek = true;
    snapshot.can_undo_seek = true;
    snapshot.can_set_offset = true;
    snapshot.can_set_ready = true;
    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, false, false)];

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        snapshot.clone(),
    )));

    let tree = state.main_window_widget_tree();
    assert!(
        tree.find("main-window:controls").is_none(),
        "standalone Controls panel should be folded into the Playlist panel"
    );
    let playlist_playback = tree
        .find("main-window:playlist-playback")
        .expect("playlist playback footer should exist");
    assert_eq!(playlist_playback.label, "Playback");
    let playback_actions = tree
        .find("main-window:controls:playback-actions")
        .expect("compact playback controls should exist in the playlist footer");
    assert_eq!(
        playback_actions.layout_mode,
        Some(GuiLayoutMode::CompactButtonWrap {
            button_width: 40.0,
            button_height: 36.0,
            gap: 8.0,
        })
    );
    assert_eq!(playback_actions.children.len(), 5);
    assert!(
        tree.find("main-window:control:set-offset").is_none(),
        "Set Offset should not be exposed in the consolidated playlist controls"
    );
    assert_eq!(
        tree.find("main-window:control:set-ready")
            .expect("ready button should exist")
            .label,
        "Not Ready"
    );

    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, true, false)];
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        snapshot.clone(),
    )));

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:control:set-ready")
            .expect("ready button should still exist")
            .label,
        "Ready"
    );

    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, false, false)];
    state.pending_local_ready_target = Some(true);
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));

    let tree = state.main_window_widget_tree();
    let ready_button = tree
        .find("main-window:control:set-ready")
        .expect("ready button should exist while readiness is pending");
    assert_eq!(ready_button.label, "Ready");
    assert!(ready_button.enabled);
}

#[test]
fn gui_shell_app_state_disables_playback_controls_when_playlist_is_empty() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.commands.can_toggle_pause = true;
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.can_toggle_pause = true;
    snapshot.can_seek = true;
    snapshot.can_undo_seek = true;
    snapshot.can_set_offset = true;
    snapshot.can_toggle_autoplay = true;
    snapshot.can_adjust_autoplay_threshold = true;
    snapshot.can_set_ready = true;
    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, false, false)];
    snapshot.playlist = Vec::new();
    snapshot.playlist_entry_ids.clear();
    snapshot.playlist_source_states.clear();

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        snapshot.clone(),
    )));

    let tree = state.main_window_widget_tree();
    for id in [
        "main-window:control:play",
        "main-window:control:pause",
        "main-window:control:toggle-pause",
        "main-window:control:seek",
        "main-window:control:undo-seek",
    ] {
        assert!(
            !tree
                .find(id)
                .unwrap_or_else(|| panic!("{id} should exist"))
                .enabled,
            "{id} should be disabled while the shared playlist is empty"
        );
    }
    assert!(
        tree.find("main-window:control:set-ready")
            .expect("ready button should exist")
            .enabled,
        "Ready should stay available even while the shared playlist is empty"
    );

    snapshot.playlist = vec!["episode1.mkv".to_owned()];
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));
    state.commands.can_toggle_pause = true;

    let tree = state.main_window_widget_tree();
    assert!(tree.find("main-window:control:play").unwrap().enabled);
    assert!(
        tree.find("main-window:control:toggle-pause")
            .unwrap()
            .enabled
    );
    assert!(tree.find("main-window:control:set-ready").unwrap().enabled);
    let autoplay = tree
        .find("main-window:control:autoplay-toggle")
        .expect("autoplay control should remain keyboard and accessibility reachable");
    assert!(autoplay.enabled);
    assert_eq!(autoplay.value.as_deref(), Some("no"));
    assert!(
        tree.find("main-window:control:autoplay-threshold-up")
            .is_some_and(|node| node.enabled)
    );
}

#[test]
fn gui_shell_app_state_projects_player_setup_into_main_window_widgets() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        player_path: Some("C:/missing/mpv.exe".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::ExitedAfterLaunch,
                    message: "GUI-owned mpv exited with exit code 1.".to_owned(),
                    retry_available: true,
                }),
            },
        ))
    );

    let main_window = state.main_window_widget_tree();
    assert!(main_window.find("main-window:player-setup").is_some());
    assert!(
        main_window
            .find("main-window:player-setup:retry")
            .expect("retry button should exist")
            .enabled
    );
    assert!(
        main_window
            .find("main-window:player-setup:open-settings")
            .expect("open-settings button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert_eq!(
        shell
            .find("shell:open-modal")
            .and_then(|node| node.value.as_deref()),
        Some("player-setup")
    );
    assert!(
        shell
            .find("shell:modal:close")
            .expect("player setup modal close button should exist")
            .enabled
    );
}

#[test]
fn gui_shell_app_state_projects_runtime_room_control_status_into_main_window_widget_tree() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.room_name = "+room1".to_owned();
    snapshot.controlled_room_active = true;
    snapshot.room_control_status = "Not granted by server: room controls are locked.".to_owned();

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:room-control")
            .and_then(|node| node.value.as_deref()),
        Some("Not granted by server: room controls are locked.")
    );
}

#[test]
fn gui_shell_app_state_projects_stream_seek_refill_without_changing_local_file_surface() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state
            .main_window_widget_tree()
            .find("main-window:seek-preparation")
            .is_none(),
        "ordinary local-file playback must not gain a stream refill panel"
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot {
                preparation: Some(GuiSeekPreparationState {
                    phase: GuiSeekPreparationPhase::Refilling,
                    frozen_target_seconds: 135.0,
                    cache_refill_percent: Some(64.6),
                    buffered_ahead_seconds: Some(12.3),
                    nearest_safe_buffered_position_seconds: Some(128.0),
                    can_keep_waiting: true,
                    can_cancel_and_remain: false,
                    can_join_nearest_buffered: true,
                }),
                degraded_reason: None,
            },
        ))
    );

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:seek-preparation:status")
            .and_then(|node| node.value.as_deref()),
        Some("Buffer refill 65%")
    );
    assert_eq!(
        tree.find("main-window:seek-preparation:target")
            .and_then(|node| node.value.as_deref()),
        Some("02:15")
    );
    assert_eq!(
        tree.find("main-window:seek-preparation:buffered-ahead")
            .and_then(|node| node.value.as_deref()),
        Some("12.3 s")
    );
    assert!(
        tree.find("main-window:seek-preparation:refill")
            .and_then(|node| node.tooltip.as_deref())
            .is_some_and(|tooltip| tooltip.contains("not file download progress"))
    );
    assert!(
        tree.find("main-window:seek-preparation:cancel").is_none(),
        "cancel must remain hidden after the core says the primary seek cannot be revoked"
    );
    assert!(
        tree.find("main-window:seek-preparation:join-nearest")
            .is_some_and(|node| node.enabled)
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot {
                preparation: None,
                degraded_reason: Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded),
            },
        ))
    );
    let convergence_degraded = state.main_window_widget_tree();
    assert_eq!(
        convergence_degraded
            .find("main-window:seek-preparation:status")
            .and_then(|node| node.value.as_deref()),
        Some("Seek completed, but room convergence degraded.")
    );
    assert!(
        convergence_degraded
            .find("main-window:seek-preparation:keep-waiting")
            .is_none()
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot {
                preparation: None,
                degraded_reason: Some(GuiSeekPreparationDegradedReason::TimedOut),
            },
        ))
    );
    let degraded = state.main_window_widget_tree();
    assert_eq!(
        degraded
            .find("main-window:seek-preparation:status")
            .and_then(|node| node.value.as_deref()),
        Some("Buffer refill timed out.")
    );
    assert!(
        degraded
            .find("main-window:seek-preparation:keep-waiting")
            .is_none()
    );
    assert!(
        degraded
            .find("main-window:seek-preparation:join-nearest")
            .is_none()
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot::default(),
        ))
    );
    assert!(
        state
            .main_window_widget_tree()
            .find("main-window:seek-preparation")
            .is_none()
    );
}
