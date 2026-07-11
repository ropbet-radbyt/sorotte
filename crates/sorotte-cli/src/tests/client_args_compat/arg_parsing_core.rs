use super::*;

#[test]
fn parse_host_and_optional_port_from_host_arg_legacy_compatible_parses_expected_shapes() {
    assert_eq!(
        parse_host_and_optional_port_from_host_arg_legacy_compatible("example.org:8999"),
        ("example.org".to_owned(), Some(8999))
    );
    assert_eq!(
        parse_host_and_optional_port_from_host_arg_legacy_compatible("example.org:notaport"),
        ("example.org".to_owned(), None)
    );
    assert_eq!(
        parse_host_and_optional_port_from_host_arg_legacy_compatible("[2001:db8::1]:8999"),
        ("[2001:db8::1]".to_owned(), Some(8999))
    );
    assert_eq!(
        parse_host_and_optional_port_from_host_arg_legacy_compatible("2001:db8::1"),
        ("[2001:db8::1]".to_owned(), None)
    );
}

#[test]
fn parse_legacy_client_arg_overrides_parses_legacy_client_flags() {
    let overrides = parse_legacy_client_arg_overrides([
        "--no-gui",
        "-a",
        "example.org:12345",
        "-n",
        "alice",
        "-r",
        "room1",
        "-p",
        "AB-123-456",
    ]);

    assert_eq!(
        overrides,
        LegacyClientArgOverrides {
            connect_requested: true,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: true,
            clear_gui_data_requested: false,
            config_path: None,
            config_root: None,
            language: None,
            player_path: None,
            file: None,
            player_args: vec![],
            load_playlist_from_file: None,
            host: Some("example.org".to_owned()),
            port: Some(12345),
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            controlled_room_password_override: Some("AB-123-456".into()),
            show_help: false,
            show_version: false,
            unknown_options: vec![],
        }
    );
    assert!(overrides.should_connect_client());
}

#[test]
fn parse_legacy_client_arg_overrides_handles_optional_room_and_ipv6_host() {
    let overrides = parse_legacy_client_arg_overrides(["-r", "-n", "alice", "--host", "[::1]"]);

    assert_eq!(
        overrides,
        LegacyClientArgOverrides {
            connect_requested: true,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: false,
            clear_gui_data_requested: false,
            config_path: None,
            config_root: None,
            language: None,
            player_path: None,
            file: None,
            player_args: vec![],
            load_playlist_from_file: None,
            host: Some("[::1]".to_owned()),
            port: None,
            username: Some("alice".to_owned()),
            room: None,
            controlled_room_password_override: None,
            show_help: false,
            show_version: false,
            unknown_options: vec![],
        }
    );
}

#[test]
fn parse_legacy_client_arg_overrides_parses_help_and_version_switches() {
    let overrides = parse_legacy_client_arg_overrides(["--help", "-v"]);
    assert_eq!(
        overrides,
        LegacyClientArgOverrides {
            connect_requested: false,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: false,
            clear_gui_data_requested: false,
            config_path: None,
            config_root: None,
            language: None,
            player_path: None,
            file: None,
            player_args: vec![],
            load_playlist_from_file: None,
            host: None,
            port: None,
            username: None,
            room: None,
            controlled_room_password_override: None,
            show_help: true,
            show_version: true,
            unknown_options: vec![],
        }
    );
    assert!(!overrides.should_connect_client());
}

#[test]
fn parse_legacy_client_arg_overrides_stops_parsing_at_double_dash() {
    let overrides = parse_legacy_client_arg_overrides([
        "--no-gui",
        "--",
        "--host",
        "example.org:12345",
        "-n",
        "alice",
    ]);

    assert_eq!(
        overrides,
        LegacyClientArgOverrides {
            connect_requested: true,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: true,
            clear_gui_data_requested: false,
            config_path: None,
            config_root: None,
            language: None,
            player_path: None,
            file: None,
            player_args: vec![
                "--host".to_owned(),
                "example.org:12345".to_owned(),
                "-n".to_owned(),
                "alice".to_owned()
            ],
            load_playlist_from_file: None,
            host: None,
            port: None,
            username: None,
            room: None,
            controlled_room_password_override: None,
            show_help: false,
            show_version: false,
            unknown_options: vec![],
        }
    );
}

#[test]
fn parse_legacy_client_arg_overrides_stops_parsing_at_double_dash_preserves_launch_only_args_verbatim()
 {
    let overrides = parse_legacy_client_arg_overrides([
        "--no-gui",
        "--",
        "--profile=fast",
        "--msg-level=all=v",
    ]);

    assert!(overrides.connect_requested);
    assert_eq!(overrides.file, None);
    assert_eq!(
        overrides.player_args,
        vec!["--profile=fast".to_owned(), "--msg-level=all=v".to_owned(),]
    );
    assert!(overrides.unknown_options.is_empty());
}

#[test]
fn parse_legacy_client_arg_overrides_collects_unknown_flags() {
    let overrides = parse_legacy_client_arg_overrides(["--no-gui", "--wat", "-x", "value"]);
    assert_eq!(
        overrides,
        LegacyClientArgOverrides {
            connect_requested: true,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: true,
            clear_gui_data_requested: false,
            config_path: None,
            config_root: None,
            language: None,
            player_path: None,
            file: Some("value".to_owned()),
            player_args: vec![],
            load_playlist_from_file: None,
            host: None,
            port: None,
            username: None,
            room: None,
            controlled_room_password_override: None,
            show_help: false,
            show_version: false,
            unknown_options: vec!["--wat".to_owned(), "-x".to_owned()],
        }
    );
}

#[test]
fn parse_legacy_client_arg_overrides_ignores_legacy_psn_arg() {
    let overrides = parse_legacy_client_arg_overrides(["-psn", "0_12345", "--no-gui"]);
    assert_eq!(
        overrides,
        LegacyClientArgOverrides {
            connect_requested: true,
            no_store: false,
            debug_requested: false,
            force_gui_prompt_requested: false,
            no_gui_requested: true,
            clear_gui_data_requested: false,
            config_path: None,
            config_root: None,
            language: None,
            player_path: None,
            file: None,
            player_args: vec![],
            load_playlist_from_file: None,
            host: None,
            port: None,
            username: None,
            room: None,
            controlled_room_password_override: None,
            show_help: false,
            show_version: false,
            unknown_options: vec![],
        }
    );
}

#[test]
fn parse_legacy_client_arg_overrides_parses_no_store_flag() {
    let overrides = parse_legacy_client_arg_overrides(["--no-gui", "--no-store"]);
    assert!(overrides.connect_requested);
    assert!(overrides.no_gui_requested);
    assert!(overrides.no_store);
    assert!(overrides.unknown_options.is_empty());
}

#[test]
fn parse_legacy_client_arg_overrides_parses_legacy_compatibility_flags_without_error() {
    let overrides = parse_legacy_client_arg_overrides([
        "--debug",
        "--force-gui-prompt",
        "--clear-gui-data",
        "--config-path",
        "custom/sorotte.ini",
        "--config-root",
        "portable-config",
        "--language",
        "fr",
        "--load-playlist-from-file",
        "playlist.txt",
    ]);
    assert!(overrides.debug_requested);
    assert!(overrides.force_gui_prompt_requested);
    assert!(overrides.clear_gui_data_requested);
    assert_eq!(overrides.config_path.as_deref(), Some("custom/sorotte.ini"));
    assert_eq!(overrides.config_root.as_deref(), Some("portable-config"));
    assert_eq!(overrides.language.as_deref(), Some("fr"));
    assert_eq!(
        overrides.load_playlist_from_file.as_deref(),
        Some("playlist.txt")
    );
    assert!(overrides.unknown_options.is_empty());
    assert!(!overrides.should_connect_client());
}
