use super::*;

#[test]
fn parse_legacy_client_arg_overrides_parses_player_path_and_positional_file_and_player_args() {
    let overrides = parse_legacy_client_arg_overrides([
        "--no-gui",
        "--player-path",
        "/tmp/mpv",
        "movie.mkv",
        "--",
        "--fs",
        "--volume=50",
    ]);

    assert!(overrides.connect_requested);
    assert_eq!(overrides.player_path.as_deref(), Some("/tmp/mpv"));
    assert_eq!(overrides.file.as_deref(), Some("movie.mkv"));
    assert_eq!(
        overrides.player_args,
        vec!["--fs".to_owned(), "--volume=50".to_owned()]
    );
    assert!(overrides.should_connect_client());
    assert!(overrides.unknown_options.is_empty());
}

#[test]
fn parse_legacy_client_arg_overrides_double_dash_assigns_first_trailing_positional_to_file() {
    let overrides =
        parse_legacy_client_arg_overrides(["--no-gui", "--", "movie.mkv", "--fs", "--speed=1.1"]);

    assert!(overrides.connect_requested);
    assert_eq!(overrides.file.as_deref(), Some("movie.mkv"));
    assert_eq!(
        overrides.player_args,
        vec!["--fs".to_owned(), "--speed=1.1".to_owned()]
    );
    assert!(overrides.unknown_options.is_empty());
}

#[test]
fn parse_legacy_client_arg_overrides_double_dash_promotes_double_dash_prefixed_file_to_player_args()
{
    let overrides = parse_legacy_client_arg_overrides(["--no-gui", "--", "--start=12", "--pause"]);

    assert!(overrides.connect_requested);
    assert_eq!(overrides.file, None);
    assert_eq!(
        overrides.player_args,
        vec!["--start=12".to_owned(), "--pause".to_owned()]
    );
    assert!(overrides.unknown_options.is_empty());
}
