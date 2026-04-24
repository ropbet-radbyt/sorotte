#[test]
fn parse_legacy_explicit_mpv_ipc_startup_player_args_parses_supported_subset_and_last_wins() {
    let args = vec![
        "--fs".to_owned(),
        "--start".to_owned(),
        "12.5".to_owned(),
        "--pause=no".to_owned(),
        "--pause".to_owned(),
        "--speed".to_owned(),
        "1.25".to_owned(),
        "--speed=1.5".to_owned(),
        "--volume=50".to_owned(),
    ];

    let parsed = crate::parse_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(&args);
    assert_eq!(
        parsed,
        crate::LegacyExplicitMpvIpcStartupPlayerArgs {
            paused: Some(true),
            start_position_seconds: Some(12.5),
            playback_rate: Some(1.5),
            muted: None,
            volume: Some(50.0),
            deinterlace: None,
            keepaspect: None,
            keepaspect_window: None,
            fullscreen: Some(true),
            ontop: None,
            border: None,
            force_window: None,
            keep_open: None,
            keep_open_pause: None,
            cursor_autohide_fs_only: None,
            stop_screensaver: None,
            sub_visibility: None,
            osd_bar: None,
            window_maximized: None,
            window_minimized: None,
        }
    );
}

#[test]
fn parse_legacy_explicit_mpv_ipc_startup_player_args_accepts_timecode_start_values() {
    let args = vec![
        "--start".to_owned(),
        "01:02:03.5".to_owned(),
        "--pause=false".to_owned(),
    ];

    let parsed = crate::parse_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(&args);
    assert_eq!(
        parsed,
        crate::LegacyExplicitMpvIpcStartupPlayerArgs {
            paused: Some(false),
            start_position_seconds: Some(3723.5),
            playback_rate: None,
            muted: None,
            volume: None,
            deinterlace: None,
            keepaspect: None,
            keepaspect_window: None,
            fullscreen: None,
            ontop: None,
            border: None,
            force_window: None,
            keep_open: None,
            keep_open_pause: None,
            cursor_autohide_fs_only: None,
            stop_screensaver: None,
            sub_visibility: None,
            osd_bar: None,
            window_maximized: None,
            window_minimized: None,
        }
    );
}

#[test]
fn analyze_legacy_explicit_mpv_ipc_startup_player_args_classifies_token_outcomes() {
    let args = vec![
        "--start=00:01:02".to_owned(),
        "--volume=50".to_owned(),
        "--mute=yes".to_owned(),
        "--deinterlace=yes".to_owned(),
        "--keepaspect=yes".to_owned(),
        "--keepaspect-window=yes".to_owned(),
        "--fs".to_owned(),
        "--ontop=false".to_owned(),
        "--border=yes".to_owned(),
        "--force-window=yes".to_owned(),
        "--keep-open=no".to_owned(),
        "--keep-open-pause=true".to_owned(),
        "--cursor-autohide-fs-only=no".to_owned(),
        "--stop-screensaver=yes".to_owned(),
        "--sub-visibility=no".to_owned(),
        "--osd-bar=yes".to_owned(),
        "--window-maximized=true".to_owned(),
        "--window-minimized=false".to_owned(),
        "--profile=fast".to_owned(),
        "--script-opts=osc=no".to_owned(),
        "--pause=maybe".to_owned(),
        "--speed".to_owned(),
        "fast".to_owned(),
        "--unknown".to_owned(),
    ];

    let analysis =
        crate::analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(&args);
    assert_eq!(
        analysis.parsed,
        crate::LegacyExplicitMpvIpcStartupPlayerArgs {
            paused: None,
            start_position_seconds: Some(62.0),
            playback_rate: None,
            muted: Some(true),
            volume: Some(50.0),
            deinterlace: Some(true),
            keepaspect: Some(true),
            keepaspect_window: Some(true),
            fullscreen: Some(true),
            ontop: Some(false),
            border: Some(true),
            force_window: Some(true),
            keep_open: Some(false),
            keep_open_pause: Some(true),
            cursor_autohide_fs_only: Some(false),
            stop_screensaver: Some(true),
            sub_visibility: Some(false),
            osd_bar: Some(true),
            window_maximized: Some(true),
            window_minimized: Some(false),
        }
    );
    assert_eq!(
        analysis.runtime_commands,
        vec![
            crate::LegacyExplicitMpvIpcStartupPlayerCommand::ApplyProfile {
                profile: "fast".to_owned()
            },
            crate::LegacyExplicitMpvIpcStartupPlayerCommand::SetOptionString {
                name: "script-opts".to_owned(),
                value: "osc=no".to_owned(),
            },
        ]
    );
    assert_eq!(
        analysis.diagnostics,
        crate::LegacyExplicitMpvIpcStartupPlayerArgDiagnostics {
            supported_tokens: vec![
                "--start=00:01:02".to_owned(),
                "--volume=50".to_owned(),
                "--mute=yes".to_owned(),
                "--deinterlace=yes".to_owned(),
                "--keepaspect=yes".to_owned(),
                "--keepaspect-window=yes".to_owned(),
                "--fs".to_owned(),
                "--ontop=false".to_owned(),
                "--border=yes".to_owned(),
                "--force-window=yes".to_owned(),
                "--keep-open=no".to_owned(),
                "--keep-open-pause=true".to_owned(),
                "--cursor-autohide-fs-only=no".to_owned(),
                "--stop-screensaver=yes".to_owned(),
                "--sub-visibility=no".to_owned(),
                "--osd-bar=yes".to_owned(),
                "--window-maximized=true".to_owned(),
                "--window-minimized=false".to_owned(),
                "--profile=fast".to_owned(),
                "--script-opts=osc=no".to_owned(),
            ],
            malformed_tokens: vec!["--pause=maybe".to_owned(), "--speed fast".to_owned()],
            unsupported_tokens: vec!["--unknown".to_owned()],
        }
    );
}

#[test]
fn analyze_legacy_explicit_mpv_ipc_startup_player_args_runtime_commands_last_wins() {
    let args = vec![
        "--profile=fast".to_owned(),
        "--script-opts=osc=no".to_owned(),
        "--profile=slow".to_owned(),
        "--script-opts=osc=yes".to_owned(),
    ];

    let analysis =
        crate::analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(&args);
    assert_eq!(
        analysis.runtime_commands,
        vec![
            crate::LegacyExplicitMpvIpcStartupPlayerCommand::ApplyProfile {
                profile: "slow".to_owned()
            },
            crate::LegacyExplicitMpvIpcStartupPlayerCommand::SetOptionString {
                name: "script-opts".to_owned(),
                value: "osc=yes".to_owned(),
            },
        ]
    );
}

#[test]
fn analyze_legacy_explicit_mpv_ipc_startup_player_args_missing_value_does_not_consume_next_flag() {
    let args = vec!["--start".to_owned(), "--pause".to_owned()];

    let analysis =
        crate::analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(&args);
    assert_eq!(
        analysis.parsed,
        crate::LegacyExplicitMpvIpcStartupPlayerArgs {
            paused: Some(true),
            start_position_seconds: None,
            playback_rate: None,
            muted: None,
            volume: None,
            deinterlace: None,
            keepaspect: None,
            keepaspect_window: None,
            fullscreen: None,
            ontop: None,
            border: None,
            force_window: None,
            keep_open: None,
            keep_open_pause: None,
            cursor_autohide_fs_only: None,
            stop_screensaver: None,
            sub_visibility: None,
            osd_bar: None,
            window_maximized: None,
            window_minimized: None,
        }
    );
    assert_eq!(
        analysis.diagnostics.supported_tokens,
        vec!["--pause".to_owned()]
    );
    assert_eq!(
        analysis.diagnostics.malformed_tokens,
        vec!["--start".to_owned()]
    );
}

#[test]
fn legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_report_summary_and_ignored_groups() {
    let diagnostics = crate::LegacyExplicitMpvIpcStartupPlayerArgDiagnostics {
        supported_tokens: vec![
            "--start=12".to_owned(),
            "--speed=1.25".to_owned(),
            "--volume=50".to_owned(),
            "--mute".to_owned(),
            "--ontop".to_owned(),
            "--profile=fast".to_owned(),
        ],
        malformed_tokens: vec!["--pause=maybe".to_owned()],
        unsupported_tokens: vec!["--untouchable".to_owned()],
    };

    let lines =
        crate::legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_legacy_compatible(
            &diagnostics,
            2,
        );
    assert_eq!(
            lines,
            vec![
                "info: explicit-mpv-IPC startup _args summary: applied=2 ignored=2 (recognized-supported-tokens=6, malformed=1, unsupported=1)".to_owned(),
                "warning: explicit-mpv-IPC malformed _args were ignored: --pause=maybe".to_owned(),
                "warning: explicit-mpv-IPC launch-only _args were ignored in attach mode: --untouchable".to_owned(),
            ]
        );
}
