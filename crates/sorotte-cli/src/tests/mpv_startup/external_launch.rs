use super::*;

const EXTERNAL_PROCESS_FIXTURE_TEST: &str =
    "tests::mpv_startup::external_launch::external_player_process_fixture_entrypoint";
const PROCESS_FIXTURE_ROLE: &str = "SOROTTE_TEST_EXTERNAL_PROCESS_FIXTURE_ROLE";
const PROCESS_FIXTURE_ROOT: &str = "SOROTTE_TEST_EXTERNAL_PROCESS_FIXTURE_ROOT";
const STDOUT_SENTINEL: &str = "sorotte-external-child-stdout-must-be-null";
const STDERR_SENTINEL: &str = "sorotte-external-child-stderr-must-be-null";

static PROCESS_FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

struct ProcessFixtureDirectory {
    path: PathBuf,
}

impl ProcessFixtureDirectory {
    fn new(case: &str) -> Self {
        let sequence = PROCESS_FIXTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sorotte-cli-external-process-{}-{sequence}-{case}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("process fixture directory should be created");
        Self { path }
    }

    fn marker(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for ProcessFixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn wait_for_process_marker(path: &Path, description: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {description}: {}",
            path.display()
        );
        std::thread::yield_now();
    }
}

fn exact_fixture_args() -> Vec<String> {
    vec![
        "--exact".to_owned(),
        EXTERNAL_PROCESS_FIXTURE_TEST.to_owned(),
        "--nocapture".to_owned(),
    ]
}

fn run_external_stdio_fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    use std::io::Write as _;
    use std::process::Stdio;

    let fixture = ProcessFixtureDirectory::new("stdio");
    let mut coordinator = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    );
    coordinator
        .args(exact_fixture_args())
        .env(PROCESS_FIXTURE_ROLE, "stdio-coordinator")
        .env(PROCESS_FIXTURE_ROOT, &fixture.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut coordinator = coordinator
        .spawn()
        .expect("stdio coordinator should be spawned");

    wait_for_process_marker(
        &fixture.marker("coordinator-ready"),
        "stdio coordinator launch",
    );
    let mut stdin = coordinator
        .stdin
        .take()
        .expect("coordinator stdin should be piped");
    stdin
        .write_all(b"parent-stdin-token")
        .expect("test token should be written");
    drop(stdin);

    let output = coordinator
        .wait_with_output()
        .expect("stdio coordinator should finish");
    assert!(
        output.status.success(),
        "stdio coordinator failed: status={:?}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let observed_stdin =
        std::fs::read(fixture.marker("leaf-stdin")).expect("leaf stdin record should exist");
    (observed_stdin, output.stdout, output.stderr)
}

#[test]
fn external_player_process_fixture_entrypoint() {
    use std::io::Read as _;

    let Some(role) = std::env::var_os(PROCESS_FIXTURE_ROLE) else {
        return;
    };
    let root = PathBuf::from(
        std::env::var_os(PROCESS_FIXTURE_ROOT)
            .expect("process fixture root should accompany fixture role"),
    );
    match role.to_string_lossy().as_ref() {
        "detached-leaf" => {
            std::fs::write(root.join("leaf-started"), b"started")
                .expect("detached leaf should publish its start barrier");
            wait_for_process_marker(&root.join("release-leaf"), "detached leaf release");
            std::fs::write(root.join("leaf-finished"), b"finished")
                .expect("detached leaf should publish completion");
        }
        "early-exit-leaf" => {
            std::fs::write(root.join("leaf-started"), b"started")
                .expect("early-exit leaf should publish its start barrier");
            std::process::exit(23);
        }
        "stdio-coordinator" => {
            // SAFETY: this exact test is the only test running in the coordinator
            // subprocess, and the mutation is complete before it spawns the leaf.
            unsafe {
                std::env::set_var(PROCESS_FIXTURE_ROLE, "stdio-leaf");
            }
            let spec = LegacyExternalPlayerLaunchSpec {
                program: std::env::current_exe()
                    .expect("current test executable should be available"),
                args: exact_fixture_args(),
            };
            let mut child = crate::spawn_legacy_external_player_from_spec_legacy_compatible(&spec)
                .expect("production external launch should spawn the stdio leaf");
            std::fs::write(root.join("coordinator-ready"), b"ready")
                .expect("coordinator should publish its launch barrier");
            let status = child.wait().expect("stdio leaf should be reaped");
            assert!(status.success(), "stdio leaf should exit successfully");
        }
        "stdio-leaf" => {
            let mut stdin = Vec::new();
            std::io::stdin()
                .read_to_end(&mut stdin)
                .expect("stdio leaf should read stdin to EOF");
            std::fs::write(root.join("leaf-stdin"), stdin)
                .expect("stdio leaf should record inherited stdin");
            println!("{STDOUT_SENTINEL}");
            eprintln!("{STDERR_SENTINEL}");
        }
        unexpected => panic!("unknown external process fixture role: {unexpected}"),
    }
}

#[test]
fn external_spawn_failure_is_contextual_and_redacts_arguments() {
    let fixture = ProcessFixtureDirectory::new("spawn-failure");
    let missing_program = fixture.marker("missing-player");
    let secret = "super-secret-player-token";
    let spec = LegacyExternalPlayerLaunchSpec {
        program: missing_program.clone(),
        args: vec![format!("--password={secret}")],
    };

    let error = crate::spawn_legacy_external_player_from_spec_legacy_compatible(&spec)
        .expect_err("a missing external player must fail to spawn")
        .to_string();

    assert!(
        error.contains(missing_program.to_string_lossy().as_ref()),
        "spawn diagnostics should identify the failed program: {error}"
    );
    assert!(
        error.contains("RedactedCommandArgs"),
        "spawn diagnostics should render arguments through the redaction type: {error}"
    );
    assert!(
        !error.contains(secret),
        "spawn diagnostics must not expose argument values: {error}"
    );
}

#[test]
fn external_launch_returns_ownership_for_caller_to_reap_an_early_exit() {
    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let fixture = ProcessFixtureDirectory::new("early-exit");
    env.set_var(PROCESS_FIXTURE_ROLE, "early-exit-leaf");
    env.set_var(PROCESS_FIXTURE_ROOT, &fixture.path);
    let spec = LegacyExternalPlayerLaunchSpec {
        program: std::env::current_exe().expect("current test executable should be available"),
        args: exact_fixture_args(),
    };

    let mut child = crate::spawn_legacy_external_player_from_spec_legacy_compatible(&spec)
        .expect("production external launch should return the child handle");
    wait_for_process_marker(&fixture.marker("leaf-started"), "early-exit child start");
    let status = child.wait().expect("caller should reap early-exit child");

    assert_eq!(
        status.code(),
        Some(23),
        "external launch must preserve the real child exit status for its owner"
    );
}

#[test]
fn unmanaged_external_launch_hands_process_ownership_to_the_child() {
    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    for key in [
        "SOROTTE_CLIENT_MPV_MANAGED_LAUNCH",
        "SOROTTE_CLIENT_MPV_IPC_PATH",
        "SOROTTE_MPV_IPC_PATH",
    ] {
        env.remove_var(key);
    }
    let fixture = ProcessFixtureDirectory::new("ownership-handoff");
    env.set_var(PROCESS_FIXTURE_ROLE, "detached-leaf");
    env.set_var(PROCESS_FIXTURE_ROOT, &fixture.path);
    let overrides = LegacyClientArgOverrides {
        player_path: Some(
            std::env::current_exe()
                .expect("current test executable should be available")
                .to_string_lossy()
                .into_owned(),
        ),
        player_args: exact_fixture_args(),
        ..Default::default()
    };

    assert!(
        crate::spawn_legacy_external_player_if_requested_legacy_compatible(&overrides)
            .expect("production unmanaged launch should succeed"),
        "the unmanaged external launch path should report that it spawned"
    );
    wait_for_process_marker(&fixture.marker("leaf-started"), "detached child start");
    assert!(
        !fixture.marker("leaf-finished").exists(),
        "dropping the unmanaged Child handle must not terminate the external player"
    );

    std::fs::write(fixture.marker("release-leaf"), b"release")
        .expect("detached child release should be published");
    wait_for_process_marker(
        &fixture.marker("leaf-finished"),
        "detached child completion",
    );
}

#[test]
fn external_launch_nulls_child_stdout_and_stderr() {
    let (_, stdout, stderr) = run_external_stdio_fixture();
    assert!(
        !String::from_utf8_lossy(&stdout).contains(STDOUT_SENTINEL),
        "external player stdout leaked into the launching CLI"
    );
    assert!(
        !String::from_utf8_lossy(&stderr).contains(STDERR_SENTINEL),
        "external player stderr leaked into the launching CLI"
    );
}

#[test]
#[should_panic(expected = "external launch must not inherit the CLI stdin handle")]
fn known_defect_external_launch_inherits_cli_stdin() {
    let (observed_stdin, _, _) = run_external_stdio_fixture();
    assert!(
        observed_stdin.is_empty(),
        "external launch must not inherit the CLI stdin handle"
    );
}

#[test]
fn legacy_external_player_launch_spec_from_overrides_orders_player_args_before_file() {
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
        player_path: Some("C:/players/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--fs".to_owned(), "--volume=50".to_owned()],
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

    let spec = legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides)
        .expect("player-path should produce a launch spec");
    assert_eq!(
        spec,
        LegacyExternalPlayerLaunchSpec {
            program: PathBuf::from("C:/players/mpv.exe"),
            args: vec![
                "--fs".to_owned(),
                "--volume=50".to_owned(),
                "C:/media/movie.mkv".to_owned()
            ],
        }
    );
}

#[test]
fn legacy_external_player_launch_spec_from_overrides_preserves_launch_only_args_for_unmanaged_launch()
 {
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
        player_path: Some("C:/players/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--profile=fast".to_owned(), "--msg-level=all=v".to_owned()],
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

    let spec = legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides)
        .expect("player-path should produce a launch spec");
    assert_eq!(
        spec.args,
        vec![
            "--profile=fast".to_owned(),
            "--msg-level=all=v".to_owned(),
            "C:/media/movie.mkv".to_owned(),
        ]
    );
}

#[test]
fn legacy_external_player_launch_spec_from_overrides_returns_none_without_player_path() {
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
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--fs".to_owned()],
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
    assert!(
        legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides).is_none()
    );
}

#[test]
fn should_skip_legacy_external_player_launch_due_to_mpv_integration_env_respects_mpv_envs() {
    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_managed = "SOROTTE_CLIENT_MPV_MANAGED_LAUNCH";
    let key_client_ipc = "SOROTTE_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SOROTTE_MPV_IPC_PATH";
    let old_managed = std::env::var_os(key_managed);
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    env.remove_var(key_managed);
    env.remove_var(key_client_ipc);
    env.remove_var(key_fallback_ipc);

    assert!(
        !should_skip_legacy_external_player_launch_due_to_mpv_integration_env(),
        "no explicit IPC or managed launch env should allow legacy external spawn path"
    );
    env.set_var(key_client_ipc, r"\\.\pipe\syncplay-test");
    assert!(
        should_skip_legacy_external_player_launch_due_to_mpv_integration_env(),
        "explicit client IPC path should skip unmanaged external spawn"
    );
    env.remove_var(key_client_ipc);
    env.set_var(key_managed, "1");

    assert!(
        should_skip_legacy_external_player_launch_due_to_mpv_integration_env(),
        "managed mpv launch env should skip unmanaged external spawn"
    );

    match old_managed {
        Some(value) => env.set_var(key_managed, value),
        None => env.remove_var(key_managed),
    }
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
fn resolve_managed_mpv_launch_program_legacy_compatible_expands_python_style_directory_inputs() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let portable_dir = std::env::temp_dir().join(format!(
        "sorotte-cli-managed-mpv-resolution-{unique_suffix}-portable-mpv"
    ));
    std::fs::create_dir_all(&portable_dir).expect("portable mpv dir should be created");
    #[cfg(windows)]
    let mpv_executable = portable_dir.join("mpv.exe");
    #[cfg(not(windows))]
    let mpv_executable = portable_dir.join("mpv");
    std::fs::write(&mpv_executable, b"").expect("portable mpv executable should be created");

    assert_eq!(
        crate::resolve_managed_mpv_launch_program_legacy_compatible(&portable_dir),
        mpv_executable
    );
    assert_eq!(
        crate::resolve_managed_mpv_launch_program_legacy_compatible(&portable_dir.join("mpv")),
        mpv_executable
    );

    let _ = std::fs::remove_file(&mpv_executable);
    let _ = std::fs::remove_dir(&portable_dir);
}

#[test]
fn legacy_player_path_requests_managed_mpv_legacy_compatible_matches_python_style_mpv_paths() {
    assert!(crate::legacy_player_path_requests_managed_mpv_legacy_compatible("mpv"));
    assert!(crate::legacy_player_path_requests_managed_mpv_legacy_compatible("C:/players/mpv.exe"));
    assert!(
        crate::legacy_player_path_requests_managed_mpv_legacy_compatible(r"C:\players\MPV.COM")
    );
    assert!(crate::legacy_player_path_requests_managed_mpv_legacy_compatible("/usr/bin/mpv"));
    assert!(
        !crate::legacy_player_path_requests_managed_mpv_legacy_compatible("C:/players/vlc.exe")
    );
    assert!(
        !crate::legacy_player_path_requests_managed_mpv_legacy_compatible("C:/players/mpvnet.exe")
    );

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let portable_dir = std::env::temp_dir().join(format!(
        "sorotte-cli-mpv-path-resolution-{unique_suffix}-portable-mpv"
    ));
    std::fs::create_dir_all(&portable_dir).expect("portable mpv dir should be created");
    #[cfg(windows)]
    let mpv_executable = portable_dir.join("mpv.exe");
    #[cfg(not(windows))]
    let mpv_executable = portable_dir.join("mpv");
    std::fs::write(&mpv_executable, b"").expect("portable mpv executable should be created");

    assert!(
        crate::legacy_player_path_requests_managed_mpv_legacy_compatible(
            portable_dir.to_string_lossy().as_ref()
        )
    );
    let unresolved_prefix = portable_dir.join("mpv");
    assert!(
        crate::legacy_player_path_requests_managed_mpv_legacy_compatible(
            unresolved_prefix.to_string_lossy().as_ref()
        )
    );

    let _ = std::fs::remove_file(&mpv_executable);
    let _ = std::fs::remove_dir(&portable_dir);
}

#[test]
fn legacy_player_path_compatibility_warning_line_legacy_compatible_distinguishes_mpv_and_non_mpv_values()
 {
    let mpv_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/mpv.exe".to_owned()),
        ..Default::default()
    };
    let vlc_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/vlc.exe".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        crate::legacy_player_path_compatibility_warning_line_legacy_compatible(&mpv_overrides),
        Some(
            "warning: legacy --player-path selects managed mpv integration for Python-style mpv paths; non-mpv values remain launch-only unmanaged fallback"
        )
    );
    assert_eq!(
        crate::legacy_player_path_compatibility_warning_line_legacy_compatible(&vlc_overrides),
        Some(
            "warning: legacy non-mpv --player-path is launch-only unmanaged fallback; it is not adapter-integrated and is ignored when managed mpv or explicit-mpv-IPC is active"
        )
    );
}

#[test]
fn legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible_only_warns_for_non_mpv_values()
 {
    let mpv_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/mpv.exe".to_owned()),
        ..Default::default()
    };
    let vlc_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/vlc.exe".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        crate::legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible(
            &mpv_overrides
        ),
        None
    );
    assert_eq!(
        crate::legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible(
            &vlc_overrides
        ),
        Some(
            "warning: legacy non-mpv --player-path was ignored because managed mpv or explicit-mpv-IPC integration is active"
        )
    );
}
