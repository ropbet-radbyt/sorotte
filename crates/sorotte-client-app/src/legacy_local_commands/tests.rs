use super::{
    LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
    LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
    PlannedLocalRuntimeAction, controlled_room_base_name_legacy_compatible,
    local_command_help_footer_lines_legacy_compatible, local_command_help_lines_legacy_compatible,
    local_input_error_output_line_legacy_compatible,
    localized_current_offset_message_legacy_compatible,
    localized_local_input_error_message_legacy_compatible,
    localized_unknown_command_message_legacy_compatible, parse_local_input_chat_message,
    parse_local_input_command, plan_local_input_command_legacy_compatible,
    plan_local_input_dispatch_legacy_compatible,
    plan_local_offset_runtime_dispatch_legacy_compatible,
    plan_local_playlist_delete_runtime_dispatch_legacy_compatible,
    plan_local_playlist_select_runtime_dispatch_legacy_compatible,
    plan_local_runtime_dispatch_legacy_compatible, playlist_index_in_bounds_legacy_compatible,
    playlist_listing_message_legacy_compatible,
    playlist_listing_message_localized_legacy_compatible,
    render_local_input_display_lines_legacy_compatible,
    resolved_local_user_offset_seconds_legacy_compatible,
};

#[test]
fn parse_local_input_chat_message_recognizes_legacy_aliases() {
    assert_eq!(
        parse_local_input_chat_message("chat hello everyone"),
        Some("hello everyone".to_owned())
    );
    assert_eq!(parse_local_input_chat_message("ch"), Some(String::new()));
    assert_eq!(parse_local_input_chat_message(" hello everyone"), None);
    assert_eq!(parse_local_input_chat_message("/chat hello everyone"), None);
}

#[test]
fn parse_local_input_command_parses_common_toggle_and_room_commands() {
    assert_eq!(
        parse_local_input_command("toggle"),
        Some(LocalInputCommand::ToggleReady)
    );
    assert_eq!(
        parse_local_input_command("room room2"),
        Some(LocalInputCommand::SetRoom("room2".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("room "),
        Some(LocalInputCommand::SetRoomWithLegacyFallback)
    );
}

#[test]
fn parse_local_input_command_parses_seek_and_offset_variants() {
    assert_eq!(
        parse_local_input_command("s+0:10"),
        Some(LocalInputCommand::SeekRelative(10.0))
    );
    assert_eq!(
        parse_local_input_command("offset /0:30"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::RelativeFromCurrentPositionMinus(30.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_rejects_slash_and_tab_variants() {
    assert_eq!(
        parse_local_input_command("/queue episode1.mkv"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("queue\tepisode1.mkv"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(parse_local_input_command(" hello everyone"), None);
}

#[test]
fn controlled_room_base_name_legacy_compatible_strips_managed_suffix() {
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+base-room:ABCDEF123456"),
        "base-room".to_owned()
    );
    assert_eq!(
        controlled_room_base_name_legacy_compatible("+room:SHORT"),
        "+room:SHORT".to_owned()
    );
}

#[test]
fn plan_local_input_command_legacy_compatible_resolves_special_room_flows() {
    let context = LocalInputCommandPlanningContext {
        current_room: Some("+watch-party:ABCDEF123456"),
        configured_room: "fallback-room",
    };

    let created = plan_local_input_command_legacy_compatible(
        LocalInputCommand::CreateControlledRoom(None),
        &context,
    );
    let PlannedLocalInputCommand::RequestControllerAuth { room, password } = created else {
        panic!("expected controller auth request");
    };
    assert_eq!(room, "watch-party");
    assert_eq!(password.expose_secret().len(), 10);

    let auth = plan_local_input_command_legacy_compatible(
        LocalInputCommand::AuthController("pw".into()),
        &context,
    );
    assert_eq!(
        auth,
        PlannedLocalInputCommand::RequestControllerAuth {
            room: "+watch-party:ABCDEF123456".to_owned(),
            password: "pw".into(),
        }
    );

    assert_eq!(
        plan_local_input_command_legacy_compatible(
            LocalInputCommand::SetRoomWithLegacyFallback,
            &LocalInputCommandPlanningContext {
                current_room: None,
                configured_room: "fallback-room",
            },
        ),
        PlannedLocalInputCommand::SetRoomWithLegacyFallback("fallback-room".to_owned())
    );
}

#[test]
fn local_controller_auth_command_debug_redacts_password() {
    const MARKER: &str = "local-command-secret-canary-c84d";
    let input = LocalInputCommand::AuthController(MARKER.into());
    let planned = PlannedLocalRuntimeAction::RequestControllerAuth {
        room: "+room:ABCDEF123456".to_owned(),
        password: MARKER.into(),
    };

    for debug in [format!("{input:?}"), format!("{planned:?}")] {
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(MARKER));
    }
}

#[test]
fn planned_local_input_command_uses_shared_playlists_matches_playlist_commands() {
    assert!(PlannedLocalInputCommand::ShowPlaylist.uses_shared_playlists());
    assert!(PlannedLocalInputCommand::SelectPlaylistIndex(1).uses_shared_playlists());
    assert!(!PlannedLocalInputCommand::ToggleReady.uses_shared_playlists());
    assert_eq!(
        plan_local_input_command_legacy_compatible(
            LocalInputCommand::ShowQueueMissingFileError,
            &LocalInputCommandPlanningContext {
                current_room: None,
                configured_room: "fallback-room",
            },
        ),
        PlannedLocalInputCommand::ShowError(LocalInputCommandErrorKind::QueueMissingFile)
    );
}

#[test]
fn resolved_local_user_offset_seconds_legacy_compatible_applies_all_modes() {
    assert_eq!(
        resolved_local_user_offset_seconds_legacy_compatible(
            5.0,
            100.0,
            &LocalOffsetCommand::Absolute(12.0),
        ),
        12.0
    );
    assert_eq!(
        resolved_local_user_offset_seconds_legacy_compatible(
            5.0,
            100.0,
            &LocalOffsetCommand::Relative(3.0),
        ),
        8.0
    );
    assert_eq!(
        resolved_local_user_offset_seconds_legacy_compatible(
            5.0,
            100.0,
            &LocalOffsetCommand::RelativeFromCurrentPositionMinus(90.0),
        ),
        15.0
    );
}

#[test]
fn playlist_index_in_bounds_legacy_compatible_matches_current_room_playlist() {
    let mut session = sorotte_client_core::ClientSession::default();
    assert!(!playlist_index_in_bounds_legacy_compatible(&session, 0));

    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should set the current room");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");

    assert!(playlist_index_in_bounds_legacy_compatible(&session, 0));
    assert!(playlist_index_in_bounds_legacy_compatible(&session, 1));
    assert!(!playlist_index_in_bounds_legacy_compatible(&session, 2));
    assert!(!playlist_index_in_bounds_legacy_compatible(&session, -1));
}

#[test]
fn playlist_listing_message_legacy_compatible_formats_entries_and_empty_states() {
    let mut session = sorotte_client_core::ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    assert_eq!(
        playlist_listing_message_legacy_compatible(&session),
        "Playlist is currently empty."
    );

    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist index should apply");

    assert_eq!(
        playlist_listing_message_legacy_compatible(&session),
        "\t1: episode1.mkv\n *\t2: episode2.mkv"
    );
}

#[test]
fn playlist_listing_message_localized_legacy_compatible_uses_localized_empty_message() {
    let mut session = sorotte_client_core::ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    assert_eq!(
        playlist_listing_message_localized_legacy_compatible(&session, Some("fr")),
        "La playlist est actuellement vide."
    );
    assert_eq!(
        playlist_listing_message_localized_legacy_compatible(&session, Some("de")),
        "Playlist ist derzeit leer."
    );
}

#[test]
fn localized_local_input_error_message_legacy_compatible_localizes_known_messages() {
    assert_eq!(
        localized_local_input_error_message_legacy_compatible(
            LocalInputCommandErrorKind::PlaylistInvalidIndex,
            Some("es"),
        ),
        "Indice de lista de reproduccion no valido"
    );
    assert_eq!(
        localized_local_input_error_message_legacy_compatible(
            LocalInputCommandErrorKind::QueueMissingFile,
            Some("de"),
        ),
        "Keine Datei/URL angegeben"
    );
    assert_eq!(
        localized_local_input_error_message_legacy_compatible(
            LocalInputCommandErrorKind::QueueMissingFile,
            None,
        ),
        "No file/url given"
    );
}

#[test]
fn local_input_error_output_line_legacy_compatible_formats_prefix_and_message() {
    assert_eq!(
        local_input_error_output_line_legacy_compatible(
            LocalInputCommandErrorKind::PlaylistInvalidIndex,
            Some("de"),
        ),
        "FEHLER:\tUngueltiger Playlist-Index"
    );
    assert_eq!(
        local_input_error_output_line_legacy_compatible(
            LocalInputCommandErrorKind::QueueMissingFile,
            None,
        ),
        "ERROR:\tNo file/url given"
    );
}

#[test]
fn local_command_help_lines_legacy_compatible_include_expected_entries() {
    let lines = local_command_help_lines_legacy_compatible(None);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Available commands:")
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("\tql - show the current playlist"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("\tqd [index] - delete the given entry"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("\tundoplaylist - undo last playlist change"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("\to[+-]duration - offset local playback"))
    );
}

#[test]
fn local_command_help_lines_legacy_compatible_localize_heading_and_body() {
    let lines = local_command_help_lines_legacy_compatible(Some("fr"));
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Commandes disponibles:")
    );
    assert!(lines.iter().any(|line| line == "\th - cette aide"));
    assert!(
        lines
            .iter()
            .any(|line| line == "\tql - afficher la playlist actuelle")
    );
}

#[test]
fn local_command_help_footer_lines_legacy_compatible_include_expected_entries() {
    let lines = local_command_help_footer_lines_legacy_compatible(Some("de"), "1.7.5");
    assert_eq!(lines[0], "Sorotte-Version: 1.7.5");
    assert_eq!(lines[1], "Mehr Informationen unter: https://syncplay.pl/");
}

#[test]
fn localized_unknown_command_message_legacy_compatible_uses_selected_language() {
    assert_eq!(
        localized_unknown_command_message_legacy_compatible(Some("es")),
        "Comando no reconocido"
    );
    assert_eq!(
        localized_unknown_command_message_legacy_compatible(None),
        "Unrecognized command"
    );
}

#[test]
fn localized_current_offset_message_legacy_compatible_localizes_user_visible_message() {
    assert_eq!(
        localized_current_offset_message_legacy_compatible(2.5, Some("pt_BR")),
        "Deslocamento atual: 2.5 segundos"
    );
    assert_eq!(
        localized_current_offset_message_legacy_compatible(-1.0, None),
        "Current offset: -1 seconds"
    );
}

#[test]
fn plan_local_offset_runtime_dispatch_legacy_compatible_emits_seek_and_status_line() {
    let dispatch = plan_local_offset_runtime_dispatch_legacy_compatible(
        5.0,
        100.0,
        &LocalOffsetCommand::Relative(3.0),
        Some("es"),
    );
    assert_eq!(dispatch.updated_user_offset_seconds, Some(8.0));
    assert_eq!(
        dispatch.line_to_emit.as_deref(),
        Some("Desfase actual: 8 segundos")
    );
    assert_eq!(
        dispatch.action,
        Some(PlannedLocalRuntimeAction::SeekToPosition(108.0))
    );
}

#[test]
fn plan_local_input_dispatch_legacy_compatible_maps_and_suppresses_commands() {
    assert_eq!(
        plan_local_input_dispatch_legacy_compatible(PlannedLocalInputCommand::ShowHelp, true,),
        PlannedLocalInputDispatch::EmitHelp
    );
    assert_eq!(
        plan_local_input_dispatch_legacy_compatible(PlannedLocalInputCommand::ShowPlaylist, false,),
        PlannedLocalInputDispatch::Suppressed
    );
    assert_eq!(
        plan_local_input_dispatch_legacy_compatible(
            PlannedLocalInputCommand::SendChat("hello".to_owned()),
            true,
        ),
        PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SendChat("hello".to_owned()))
    );
    assert_eq!(
        plan_local_input_dispatch_legacy_compatible(
            PlannedLocalInputCommand::SetRoomWithLegacyFallback("room2".to_owned()),
            true,
        ),
        PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(
            "room2".to_owned()
        ))
    );
}

#[test]
fn render_local_input_display_lines_legacy_compatible_renders_unknown_help_and_playlist() {
    let mut session = sorotte_client_core::ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    let unknown_lines = render_local_input_display_lines_legacy_compatible(
        &PlannedLocalInputDispatch::EmitUnknownCommandHelp,
        &session,
        Some("es"),
        "1.7.5",
    )
    .expect("unknown command should render lines");
    assert_eq!(
        unknown_lines.first().map(String::as_str),
        Some("Comando no reconocido")
    );
    assert!(
        unknown_lines
            .iter()
            .any(|line| line == "Comandos disponibles:")
    );
    assert!(
        unknown_lines
            .iter()
            .any(|line| line == "Version de Sorotte: 1.7.5")
    );

    let playlist_lines = render_local_input_display_lines_legacy_compatible(
        &PlannedLocalInputDispatch::EmitPlaylist,
        &session,
        Some("de"),
        "1.7.5",
    )
    .expect("playlist should render lines");
    assert_eq!(
        playlist_lines,
        vec!["Playlist ist derzeit leer.".to_owned()]
    );
}

#[test]
fn plan_local_playlist_runtime_dispatch_legacy_compatible_handles_valid_and_invalid_indices() {
    let mut session = sorotte_client_core::ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should set the current room");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist change should apply");

    let invalid_dispatch =
        plan_local_playlist_select_runtime_dispatch_legacy_compatible(&session, 5, Some("fr"));
    assert_eq!(
        invalid_dispatch.line_to_emit.as_deref(),
        Some("ERREUR:\tIndice de playlist non valide")
    );
    assert_eq!(invalid_dispatch.action, None);

    let valid_dispatch =
        plan_local_playlist_delete_runtime_dispatch_legacy_compatible(&session, 1, Some("fr"));
    assert_eq!(valid_dispatch.line_to_emit, None);
    assert_eq!(
        valid_dispatch.action,
        Some(PlannedLocalRuntimeAction::DeletePlaylistIndex(1))
    );
}

#[test]
fn plan_local_runtime_dispatch_legacy_compatible_promotes_special_cases() {
    let offset_dispatch = plan_local_runtime_dispatch_legacy_compatible(
        &sorotte_client_core::ClientSession::default(),
        5.0,
        PlannedLocalRuntimeAction::SetUserOffset(LocalOffsetCommand::Relative(3.0)),
        Some("en"),
    );
    assert_eq!(offset_dispatch.updated_user_offset_seconds, Some(8.0));
    assert_eq!(
        offset_dispatch.action,
        Some(PlannedLocalRuntimeAction::SeekToPosition(8.0))
    );

    let mut session = sorotte_client_core::ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    let playlist_dispatch = plan_local_runtime_dispatch_legacy_compatible(
        &session,
        0.0,
        PlannedLocalRuntimeAction::SetPlaylistIndex(3),
        Some("fr"),
    );
    assert_eq!(playlist_dispatch.action, None);
    assert_eq!(
        playlist_dispatch.line_to_emit.as_deref(),
        Some("ERREUR:\tIndice de playlist non valide")
    );
}

#[test]
fn plan_local_runtime_dispatch_legacy_compatible_passthroughs_simple_actions() {
    let dispatch = plan_local_runtime_dispatch_legacy_compatible(
        &sorotte_client_core::ClientSession::default(),
        0.0,
        PlannedLocalRuntimeAction::TogglePause,
        Some("de"),
    );
    assert_eq!(dispatch.line_to_emit, None);
    assert_eq!(dispatch.updated_user_offset_seconds, None);
    assert_eq!(
        dispatch.action,
        Some(PlannedLocalRuntimeAction::TogglePause)
    );
}
