use super::*;

struct StartupConfigRoot(PathBuf);

impl StartupConfigRoot {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-startup-args-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for StartupConfigRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn startup_with_arguments(extra: &[&str], no_store: bool, explicit_player: bool) -> Vec<String> {
    let mut args = Vec::new();
    if explicit_player {
        args.extend(["--player-path", "C:/players/mpv.exe"]);
    }
    if no_store {
        args.push("--no-store");
    }
    args.push("--");
    args.extend_from_slice(extra);
    let mut overrides = parse_syncplay_client_arg_overrides(args);
    assert!(overrides.unknown_options.is_empty());
    let submitted = overrides.player_args.clone();
    if let Some(settings) = load_sorotte_cli_stored_settings().unwrap() {
        apply_stored_startup_player_defaults_if_arg_absent(&mut overrides, &settings);
    }
    crate::persist_explicit_syncplay_client_arg_settings(&overrides, &submitted);
    overrides.player_args
}

#[test]
fn repeated_startup_persists_only_explicit_arguments_without_accumulation() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let root = StartupConfigRoot::new();
    env.set_var(
        "SOROTTE_CLIENT_CONFIG_PATH",
        root.path().join("sorotte.ini"),
    );
    for _ in 0..3 {
        assert_eq!(
            startup_with_arguments(&["--sub-file=subtitle.srt"], false, true),
            ["--sub-file=subtitle.srt"]
        );
    }
    assert_eq!(
        startup_with_arguments(&[], false, true),
        ["--sub-file=subtitle.srt"]
    );
    // Distinct explicit input replaces persisted input, while this launch still
    // appends the previous saved defaults in their established order.
    assert_eq!(
        startup_with_arguments(&["--profile=cinema"], false, false),
        ["--profile=cinema", "--sub-file=subtitle.srt"]
    );
    assert_eq!(
        startup_with_arguments(&[], false, false),
        ["--profile=cinema"]
    );
    assert_eq!(
        startup_with_arguments(&["--profile=cinema"], false, false),
        ["--profile=cinema"]
    );
}

#[test]
fn startup_argument_roundtrip_preserves_explicit_repetition_and_option_value_pairs() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let root = StartupConfigRoot::new();
    env.set_var(
        "SOROTTE_CLIENT_CONFIG_PATH",
        root.path().join("sorotte.ini"),
    );
    let args = [
        "--script",
        "a.lua",
        "--script",
        "b.lua",
        "--profile=cinema",
        "--profile=cinema",
    ];
    for _ in 0..3 {
        assert_eq!(startup_with_arguments(&args, false, true), args);
    }
    assert_eq!(startup_with_arguments(&[], false, true), args);
}

#[test]
fn no_store_startup_arguments_leave_existing_defaults_unchanged() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let root = StartupConfigRoot::new();
    let path = root.path().join("sorotte.ini");
    env.set_var("SOROTTE_CLIENT_CONFIG_PATH", &path);
    assert_eq!(
        startup_with_arguments(&["--profile=default"], false, true),
        ["--profile=default"]
    );
    let before = std::fs::read(&path).unwrap();
    for _ in 0..3 {
        assert_eq!(
            startup_with_arguments(&["--sub-file=temporary.srt"], true, true),
            ["--sub-file=temporary.srt", "--profile=default"]
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    assert_eq!(
        startup_with_arguments(&[], false, true),
        ["--profile=default"]
    );
}
