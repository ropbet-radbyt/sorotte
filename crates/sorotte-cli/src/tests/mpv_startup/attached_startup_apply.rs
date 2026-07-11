use super::*;

#[test]
fn apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_opens_file() {
    #[derive(Default)]
    struct RecordingPlayer {
        opened: Vec<String>,
    }
    impl PlayerAdapter for RecordingPlayer {
        fn name(&self) -> &'static str {
            "recording"
        }
        fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
            self.opened.push(path.to_owned());
            Ok(())
        }
    }

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SOROTTE_MPV_IPC_PATH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    env.set_var(key_client_ipc, r"\\.\pipe\syncplay-test");
    env.remove_var(key_fallback_ipc);

    let overrides = LegacyClientArgOverrides {
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
        file: Some("movie.mkv".to_owned()),
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
    };
    let mut player = RecordingPlayer::default();

    let opened =
        apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
            &mut player,
            &overrides,
        )
        .expect("open_file should succeed");
    assert!(opened);
    assert_eq!(player.opened, vec!["movie.mkv".to_owned()]);

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
    match old_fallback_ipc {
        Some(value) => env.set_var(key_fallback_ipc, value),
        None => env.remove_var(key_fallback_ipc),
    }
}

#[test]
fn apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_applies_runtime_commands_and_supported_player_args()
 {
    #[derive(Default)]
    struct RecordingPlayer {
        events: Vec<String>,
    }
    impl PlayerAdapter for RecordingPlayer {
        fn name(&self) -> &'static str {
            "recording"
        }
        fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
            self.events.push(format!("open:{path}"));
            Ok(())
        }
        fn set_option_string(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
            self.events.push(format!("option:{name}={value}"));
            Ok(())
        }
        fn apply_profile(&mut self, profile: &str) -> Result<(), PlayerError> {
            self.events.push(format!("profile:{profile}"));
            Ok(())
        }
        fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
            self.events.push(format!("pause:{paused}"));
            Ok(())
        }
        fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
            self.events.push(format!("seek:{position_seconds}"));
            Ok(())
        }
        fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
            self.events.push(format!("speed:{rate}"));
            Ok(())
        }
        fn set_volume(&mut self, volume: f64) -> Result<(), PlayerError> {
            self.events.push(format!("volume:{volume}"));
            Ok(())
        }
        fn set_muted(&mut self, muted: bool) -> Result<(), PlayerError> {
            self.events.push(format!("mute:{muted}"));
            Ok(())
        }
        fn set_deinterlace(&mut self, deinterlace: bool) -> Result<(), PlayerError> {
            self.events.push(format!("deinterlace:{deinterlace}"));
            Ok(())
        }
        fn set_keepaspect(&mut self, keepaspect: bool) -> Result<(), PlayerError> {
            self.events.push(format!("keepaspect:{keepaspect}"));
            Ok(())
        }
        fn set_keepaspect_window(&mut self, keepaspect_window: bool) -> Result<(), PlayerError> {
            self.events
                .push(format!("keepaspect-window:{keepaspect_window}"));
            Ok(())
        }
        fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), PlayerError> {
            self.events.push(format!("fullscreen:{fullscreen}"));
            Ok(())
        }
        fn set_ontop(&mut self, ontop: bool) -> Result<(), PlayerError> {
            self.events.push(format!("ontop:{ontop}"));
            Ok(())
        }
        fn set_border(&mut self, border: bool) -> Result<(), PlayerError> {
            self.events.push(format!("border:{border}"));
            Ok(())
        }
        fn set_force_window(&mut self, force_window: bool) -> Result<(), PlayerError> {
            self.events.push(format!("force-window:{force_window}"));
            Ok(())
        }
        fn set_keep_open(&mut self, keep_open: bool) -> Result<(), PlayerError> {
            self.events.push(format!("keep-open:{keep_open}"));
            Ok(())
        }
        fn set_keep_open_pause(&mut self, keep_open_pause: bool) -> Result<(), PlayerError> {
            self.events
                .push(format!("keep-open-pause:{keep_open_pause}"));
            Ok(())
        }
        fn set_cursor_autohide_fs_only(
            &mut self,
            cursor_autohide_fs_only: bool,
        ) -> Result<(), PlayerError> {
            self.events
                .push(format!("cursor-autohide-fs-only:{cursor_autohide_fs_only}"));
            Ok(())
        }
        fn set_stop_screensaver(&mut self, stop_screensaver: bool) -> Result<(), PlayerError> {
            self.events
                .push(format!("stop-screensaver:{stop_screensaver}"));
            Ok(())
        }
        fn set_sub_visibility(&mut self, sub_visibility: bool) -> Result<(), PlayerError> {
            self.events.push(format!("sub-visibility:{sub_visibility}"));
            Ok(())
        }
        fn set_osd_bar(&mut self, osd_bar: bool) -> Result<(), PlayerError> {
            self.events.push(format!("osd-bar:{osd_bar}"));
            Ok(())
        }
        fn set_window_maximized(&mut self, window_maximized: bool) -> Result<(), PlayerError> {
            self.events
                .push(format!("window-maximized:{window_maximized}"));
            Ok(())
        }
        fn set_window_minimized(&mut self, window_minimized: bool) -> Result<(), PlayerError> {
            self.events
                .push(format!("window-minimized:{window_minimized}"));
            Ok(())
        }
    }

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SOROTTE_MPV_IPC_PATH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    env.set_var(key_client_ipc, r"\\.\pipe\syncplay-test");
    env.remove_var(key_fallback_ipc);

    let overrides = LegacyClientArgOverrides {
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
        file: Some("movie.mkv".to_owned()),
        player_args: vec![
            "--profile=fast".to_owned(),
            "--script-opts=osc=no".to_owned(),
            "--fs".to_owned(),
            "--start".to_owned(),
            "12.5".to_owned(),
            "--pause".to_owned(),
            "--speed=1.25".to_owned(),
            "--volume=50".to_owned(),
            "--mute".to_owned(),
            "--deinterlace=no".to_owned(),
            "--deinterlace".to_owned(),
            "--keepaspect=no".to_owned(),
            "--keepaspect".to_owned(),
            "--keepaspect-window=no".to_owned(),
            "--keepaspect-window".to_owned(),
            "--fullscreen=no".to_owned(),
            "--fs".to_owned(),
            "--ontop=no".to_owned(),
            "--ontop".to_owned(),
            "--border=no".to_owned(),
            "--border".to_owned(),
            "--force-window=no".to_owned(),
            "--force-window".to_owned(),
            "--keep-open=no".to_owned(),
            "--keep-open".to_owned(),
            "--keep-open-pause=no".to_owned(),
            "--keep-open-pause".to_owned(),
            "--cursor-autohide-fs-only=no".to_owned(),
            "--cursor-autohide-fs-only".to_owned(),
            "--stop-screensaver=no".to_owned(),
            "--stop-screensaver".to_owned(),
            "--sub-visibility=no".to_owned(),
            "--sub-visibility".to_owned(),
            "--osd-bar=no".to_owned(),
            "--osd-bar".to_owned(),
            "--window-maximized=no".to_owned(),
            "--window-maximized".to_owned(),
            "--window-minimized=no".to_owned(),
            "--window-minimized".to_owned(),
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
    };
    let mut player = RecordingPlayer::default();

    let applied =
        apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
            &mut player,
            &overrides,
        )
        .expect("supported explicit-mpv-IPC startup subset should apply");
    assert!(applied);
    assert_eq!(
        player.events,
        vec![
            "profile:fast".to_owned(),
            "option:script-opts=osc=no".to_owned(),
            "open:movie.mkv".to_owned(),
            "seek:12.5".to_owned(),
            "pause:true".to_owned(),
            "speed:1.25".to_owned(),
            "volume:50".to_owned(),
            "mute:true".to_owned(),
            "deinterlace:true".to_owned(),
            "keepaspect:true".to_owned(),
            "keepaspect-window:true".to_owned(),
            "fullscreen:true".to_owned(),
            "ontop:true".to_owned(),
            "border:true".to_owned(),
            "force-window:true".to_owned(),
            "keep-open:true".to_owned(),
            "keep-open-pause:true".to_owned(),
            "cursor-autohide-fs-only:true".to_owned(),
            "stop-screensaver:true".to_owned(),
            "sub-visibility:true".to_owned(),
            "osd-bar:true".to_owned(),
            "window-maximized:true".to_owned(),
            "window-minimized:true".to_owned(),
        ]
    );

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
    match old_fallback_ipc {
        Some(value) => env.set_var(key_fallback_ipc, value),
        None => env.remove_var(key_fallback_ipc),
    }
}

#[test]
fn apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_skips_without_ipc_env() {
    #[derive(Default)]
    struct RecordingPlayer {
        opened: Vec<String>,
    }
    impl PlayerAdapter for RecordingPlayer {
        fn name(&self) -> &'static str {
            "recording"
        }
        fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
            self.opened.push(path.to_owned());
            Ok(())
        }
    }

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SOROTTE_MPV_IPC_PATH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    env.remove_var(key_client_ipc);
    env.remove_var(key_fallback_ipc);

    let overrides = LegacyClientArgOverrides {
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
        file: Some("movie.mkv".to_owned()),
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
    };
    let mut player = RecordingPlayer::default();

    let opened =
        apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
            &mut player,
            &overrides,
        )
        .expect("helper should skip cleanly");
    assert!(!opened);
    assert!(player.opened.is_empty());

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
    match old_fallback_ipc {
        Some(value) => env.set_var(key_fallback_ipc, value),
        None => env.remove_var(key_fallback_ipc),
    }
}

#[test]
fn apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_propagates_player_errors() {
    struct FailingPlayer;
    impl PlayerAdapter for FailingPlayer {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn open_file(&mut self, _path: &str) -> Result<(), PlayerError> {
            Err(PlayerError::OperationFailed("boom".to_owned()))
        }
    }

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    env.set_var(key_client_ipc, r"\\.\pipe\syncplay-test");
    let overrides = LegacyClientArgOverrides {
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
        file: Some("movie.mkv".to_owned()),
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
    };
    let mut player = FailingPlayer;

    let error = apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
        &mut player,
        &overrides,
    )
    .expect_err("player error should propagate");
    assert!(
        error
            .to_string()
            .contains("failed opening legacy startup file")
    );

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
}

#[test]
fn apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_redacts_player_arg_errors() {
    const PLAYER_ARG_ERROR_CANARY: &str = "PLAYER_ARG_ERROR_SIGNED_URL_CANARY";

    struct FailingPlayer;
    impl PlayerAdapter for FailingPlayer {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn set_option_string(&mut self, _name: &str, _value: &str) -> Result<(), PlayerError> {
            Err(PlayerError::OperationFailed("boom".to_owned()))
        }
    }

    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let old_client_ipc = std::env::var_os(key_client_ipc);
    env.set_var(key_client_ipc, r"\\.\pipe\syncplay-test");
    let overrides = LegacyClientArgOverrides {
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
        player_args: vec![format!(
            "--script-opts=https://media.example/video?Signature={PLAYER_ARG_ERROR_CANARY}"
        )],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };
    let mut player = FailingPlayer;

    let error = apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible(
        &mut player,
        &overrides,
    )
    .expect_err("player error should propagate");
    let rendered = error.to_string();
    assert!(rendered.contains("failed applying legacy explicit-mpv-IPC startup option"));
    assert!(!rendered.contains(PLAYER_ARG_ERROR_CANARY));
    assert!(!rendered.contains("script-opts"));
    assert!(!rendered.contains("?Signature="));

    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
}
