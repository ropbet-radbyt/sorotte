use std::{collections::BTreeMap, fmt::Write as _};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::*;

const SCHEMA_VERSION: u32 = 1;
const REPORT_KIND: &str = "sorotte-gui-participant-status-system";
const ACCEPTED_FRESH_STATUS_PREFIXES: &[&str] = &[
    "Player connected",
    "Player starting",
    "No media",
    "Loading",
    "Prebuffering",
    "Ready",
    "Playing",
    "Rebuffering",
    "Seeking",
    "Ended",
    "Playback failed",
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParticipantStatusSystemOptions {
    binary_path: PathBuf,
    mpv_path: PathBuf,
    artifact_dir: PathBuf,
    shared_lifecycle_path: PathBuf,
    host: String,
    port: u16,
    run_id: String,
    observer_username: String,
    reporter_username: String,
    room: String,
    expected_gui_sha256: String,
    expected_mpv_sha256: String,
    timeout: Duration,
}

#[derive(Debug, Serialize)]
struct BinaryEvidence {
    file_name: String,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct ProjectionEvidence {
    reporter_username: String,
    user_row_identity: String,
    participant_index: u32,
    username_bounds: [i32; 4],
    status_automation_id: String,
    status_bounds: [i32; 4],
    status_label: String,
    binding_source: &'static str,
    vertical_gap_px: i32,
    visible: bool,
}

#[derive(Debug, Serialize)]
struct ParticipantStatusSystemReport {
    schema_version: u32,
    kind: &'static str,
    result: &'static str,
    run_id: String,
    endpoint: String,
    room: String,
    observer_username: String,
    reporter_username: String,
    gui_pid: u32,
    gui: BinaryEvidence,
    configured_mpv: BinaryEvidence,
    projection: ProjectionEvidence,
    artifacts: BTreeMap<String, String>,
    assertions: Vec<&'static str>,
}

pub(crate) fn run_participant_status_system_from_args(args: &[String]) -> Result<String, String> {
    let options = parse_participant_status_system_options(args)?;
    run_participant_status_system(options)
}

fn run_participant_status_system(
    options: ParticipantStatusSystemOptions,
) -> Result<String, String> {
    if options.host != "127.0.0.1" {
        return Err(format!(
            "participant-status system proof requires strict IPv4 loopback host, got {:?}",
            options.host
        ));
    }
    fs::create_dir_all(&options.artifact_dir).map_err(|error| {
        format!(
            "failed to create participant-status artifact directory {}: {error}",
            options.artifact_dir.display()
        )
    })?;

    let binary_path = resolve_binary_path(&options.binary_path)?;
    let mpv_path = fs::canonicalize(&options.mpv_path).map_err(|error| {
        format!(
            "failed to resolve participant-status mpv at {}: {error}",
            options.mpv_path.display()
        )
    })?;
    let gui_sha256 = sha256_file(&binary_path)?;
    let mpv_sha256 = sha256_file(&mpv_path)?;
    require_expected_digest("GUI", &gui_sha256, &options.expected_gui_sha256)?;
    require_expected_digest("mpv", &mpv_sha256, &options.expected_mpv_sha256)?;

    if options.shared_lifecycle_path.exists() {
        return Err(format!(
            "shared lifecycle path must be create-new for participant-status proof: {}",
            options.shared_lifecycle_path.display()
        ));
    }

    let config_path = options.artifact_dir.join("participant-status-system.ini");
    let internal_lifecycle_path = options.artifact_dir.join("gui-internal-lifecycle.jsonl");
    let screenshot_path = options.artifact_dir.join("participant-status-system.png");
    let projection_path = options
        .artifact_dir
        .join("participant-status-projection.json");
    let media_search_path = options.artifact_dir.join("media-search");
    let open_media_path = options.artifact_dir.join("unused-open-target.mkv");
    let appdata_root = options.artifact_dir.join("appdata");
    for create_new_path in [&internal_lifecycle_path, &screenshot_path, &projection_path] {
        if create_new_path.exists() {
            return Err(format!(
                "participant-status proof artifact must be create-new: {}",
                create_new_path.display()
            ));
        }
    }
    fs::create_dir_all(&media_search_path)
        .map_err(|error| format!("failed to create isolated media-search directory: {error}"))?;
    fs::create_dir_all(&appdata_root)
        .map_err(|error| format!("failed to create isolated APPDATA directory: {error}"))?;
    fs::write(&open_media_path, b"unused participant status target")
        .map_err(|error| format!("failed to seed isolated open target: {error}"))?;

    let player_path = mpv_path.display().to_string();
    let player_args = vec![
        "--no-config".to_owned(),
        "--force-window=no".to_owned(),
        "--video=no".to_owned(),
        "--audio-display=no".to_owned(),
        "--ao=null".to_owned(),
        "--idle=yes".to_owned(),
    ];
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            host: Some(options.host.clone()),
            port: Some(options.port),
            tls_policy: Some("Plaintext".to_owned()),
            username: Some(options.observer_username.clone()),
            room: Some(options.room.clone()),
            player_path: Some(player_path.clone()),
            per_player_arguments: Some(BTreeMap::from([(player_path, player_args)])),
            shared_playlist_enabled: Some(true),
            check_for_updates_automatically: Some(false),
            show_osd: Some(false),
            chat_input_enabled: Some(false),
            chat_output_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to write participant-status system config {}: {error}",
            config_path.display()
        )
    })?;

    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_path,
        open_media_file_path: &open_media_path,
        public_servers_spec: "[]",
        network_mode: NativeNetworkMode::TcpLoopback {
            bootstrap: NativeTcpBootstrap::SavedConfig,
        },
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let driver = PlatformNativeGuiDriver::default();
    let (mut child, window) = launch_sorotte_gui_with_retry_and_test_overrides(
        &driver,
        &binary_path,
        launch,
        options.timeout,
        GuiLaunchTestOverrides {
            appdata_root: Some(&appdata_root),
            explicit_config_path_with_appdata_root: true,
            lifecycle_observation_path: Some(&internal_lifecycle_path),
            shared_lifecycle_evidence_path: Some(&options.shared_lifecycle_path),
            shared_lifecycle_run_id: Some(&options.run_id),
            shared_lifecycle_emitter: Some("gui-status-observer"),
            ..GuiLaunchTestOverrides::default()
        },
    )?;
    let gui_pid = child.id();

    let outcome = (|| -> Result<ProjectionEvidence, String> {
        let step_timeout = options.timeout.min(Duration::from_secs(20));
        wait_for_any_accessible_name(
            &driver,
            window,
            &["view: setup", "view: room"],
            step_timeout,
        )?;
        if wait_for_accessible_name(
            &driver,
            window,
            "modal: player-setup",
            Duration::from_millis(600),
        )
        .is_ok()
        {
            return Err("exact GUI failed to attach the configured supported mpv".to_owned());
        }
        navigate_to_view_with_wait(
            &driver,
            window,
            ROOM_SURFACE_AUTOMATION_ID,
            "view: room",
            step_timeout,
        )?;
        wait_for_main_window_user_row_name(
            &driver,
            window,
            &options.reporter_username,
            step_timeout,
        )?;
        let projection = wait_for_username_bound_fresh_projection(
            &driver,
            window,
            &options.reporter_username,
            step_timeout,
        )?;
        driver.capture_window_png(window, &screenshot_path)?;
        write_json_create_new(&projection_path, &projection)?;
        driver.close_window(window)?;
        wait_for_process_exit(&mut child, options.timeout)?;
        Ok(projection)
    })();

    let projection = match outcome {
        Ok(projection) => projection,
        Err(error) => {
            capture_native_failure_artifacts_at(
                &driver,
                window,
                &options.artifact_dir,
                "participant-status-system",
                &error,
            );
            if driver.close_window(window).is_err()
                || wait_for_process_exit(&mut child, Duration::from_secs(5)).is_err()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(error);
        }
    };

    let ledger = fs::read_to_string(&options.shared_lifecycle_path).map_err(|error| {
        format!(
            "failed to read exact GUI lifecycle evidence {}: {error}",
            options.shared_lifecycle_path.display()
        )
    })?;
    for required_transition in ["PLAYER-ATTACH-001", "STATUS-FRESH-001"] {
        if !ledger.contains(required_transition) {
            return Err(format!(
                "exact GUI lifecycle evidence omitted required transition {required_transition}"
            ));
        }
    }

    let artifacts = BTreeMap::from([
        (
            "screenshot".to_owned(),
            screenshot_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("participant-status-system.png")
                .to_owned(),
        ),
        (
            "projection".to_owned(),
            projection_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("participant-status-projection.json")
                .to_owned(),
        ),
        (
            "gui_lifecycle".to_owned(),
            options
                .shared_lifecycle_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("gui-product-lifecycle.jsonl")
                .to_owned(),
        ),
    ]);
    let report = ParticipantStatusSystemReport {
        schema_version: SCHEMA_VERSION,
        kind: REPORT_KIND,
        result: "passed",
        run_id: options.run_id,
        endpoint: format!("{}:{}", options.host, options.port),
        room: options.room,
        observer_username: options.observer_username,
        reporter_username: options.reporter_username,
        gui_pid,
        gui: binary_evidence(&binary_path, gui_sha256),
        configured_mpv: binary_evidence(&mpv_path, mpv_sha256),
        projection,
        artifacts,
        assertions: vec![
            "exact-gui-digest-matched",
            "configured-mpv-digest-matched",
            "actual-server-room-visible",
            "named-reporter-row-bound-to-status-node",
            "production-participant-status-fresh",
            "exact-gui-player-attached",
            "exact-gui-projection-ledger-recorded",
            "native-window-captured",
            "graceful-gui-shutdown",
        ],
    };
    serde_json::to_string(&report)
        .map_err(|error| format!("failed to serialize participant-status report: {error}"))
}

fn wait_for_username_bound_fresh_projection<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    reporter_username: &str,
    timeout: Duration,
) -> Result<ProjectionEvidence, String> {
    let deadline = Instant::now() + timeout;
    let mut last_snapshot = "none".to_owned();
    loop {
        if let Ok(nodes) = driver.accessibility_nodes(window) {
            let matching_users = nodes
                .iter()
                .filter(|node| {
                    node.name == reporter_username && !node.offscreen && node.bounds.is_some()
                })
                .collect::<Vec<_>>();
            let mut candidates = Vec::new();
            for status_node in &nodes {
                let Some(participant_index) =
                    parse_main_window_participant_status_index(&status_node.automation_id)
                else {
                    continue;
                };
                let Some(status_bounds) = status_node.bounds else {
                    continue;
                };
                if status_node.offscreen || !accepted_fresh_status_label(&status_node.name) {
                    continue;
                }
                for username_node in &matching_users {
                    let Some(username_bounds) = username_node.bounds else {
                        continue;
                    };
                    let vertical_gap_px = status_bounds[1] - username_bounds[3];
                    let horizontal_overlap = status_bounds[2].min(username_bounds[2])
                        - status_bounds[0].max(username_bounds[0]);
                    if !(0..=96).contains(&vertical_gap_px) || horizontal_overlap <= 0 {
                        continue;
                    }
                    candidates.push((
                        status_bounds[1],
                        vertical_gap_px,
                        participant_index,
                        username_bounds,
                        status_bounds,
                        status_node,
                    ));
                }
            }
            candidates.sort_by_key(|candidate| (candidate.0, candidate.1));
            if let Some((
                _,
                vertical_gap_px,
                participant_index,
                username_bounds,
                status_bounds,
                status_node,
            )) = candidates.first()
            {
                return Ok(ProjectionEvidence {
                    reporter_username: reporter_username.to_owned(),
                    user_row_identity: format!("main-window:user:{participant_index}"),
                    participant_index: *participant_index,
                    username_bounds: *username_bounds,
                    status_automation_id: status_node.automation_id.clone(),
                    status_bounds: *status_bounds,
                    status_label: status_node.name.clone(),
                    binding_source: "uia-spatial-row+status-index",
                    vertical_gap_px: *vertical_gap_px,
                    visible: true,
                });
            }
            last_snapshot = nodes
                .iter()
                .filter(|node| {
                    node.name == reporter_username
                        || parse_main_window_participant_status_index(&node.automation_id).is_some()
                })
                .map(|node| {
                    format!(
                        "{}={:?} bounds={:?} offscreen={}",
                        node.automation_id, node.name, node.bounds, node.offscreen
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for a fresh participant-status projection bound to reporter {reporter_username:?}; last snapshot: {last_snapshot}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn parse_main_window_participant_status_index(automation_id: &str) -> Option<u32> {
    let suffix = automation_id.strip_prefix("main-window:user:")?;
    let index = suffix.strip_suffix(":participant-status")?;
    if index.contains(':') {
        return None;
    }
    index.parse().ok()
}

fn accepted_fresh_status_label(label: &str) -> bool {
    if label.len() > 192 || !label.ends_with(" · fresh") || label.contains(['\r', '\n']) {
        return false;
    }
    ACCEPTED_FRESH_STATUS_PREFIXES.iter().any(|prefix| {
        label == format!("{prefix} · fresh") || label.starts_with(&format!("{prefix} · "))
    })
}

fn binary_evidence(path: &Path, sha256: String) -> BinaryEvidence {
    BinaryEvidence {
        file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("binary")
            .to_owned(),
        sha256,
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read binary {}: {error}", path.display()))?;
    let digest = Sha256::digest(bytes);
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        write!(&mut rendered, "{byte:02x}")
            .map_err(|error| format!("failed to render SHA-256: {error}"))?;
    }
    Ok(rendered)
}

fn require_expected_digest(label: &str, actual: &str, expected: &str) -> Result<(), String> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "{label} digest {actual} did not match immutable expected digest {expected}"
        ))
    }
}

fn write_json_create_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut payload = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    payload.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    file.write_all(&payload)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn parse_participant_status_system_options(
    args: &[String],
) -> Result<ParticipantStatusSystemOptions, String> {
    let mut binary_path = None;
    let mut mpv_path = None;
    let mut artifact_dir = None;
    let mut shared_lifecycle_path = None;
    let mut host = "127.0.0.1".to_owned();
    let mut port = None;
    let mut run_id = None;
    let mut observer_username = None;
    let mut reporter_username = None;
    let mut room = None;
    let mut expected_gui_sha256 = None;
    let mut expected_mpv_sha256 = None;
    let mut timeout = Duration::from_secs(30);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let mut take_value = |name: &str| -> Result<String, String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match arg.as_str() {
            "--participant-status-system" | "--json" => {}
            "--binary" => binary_path = Some(PathBuf::from(take_value("--binary")?)),
            "--mpv" => mpv_path = Some(PathBuf::from(take_value("--mpv")?)),
            "--artifact-dir" => artifact_dir = Some(PathBuf::from(take_value("--artifact-dir")?)),
            "--shared-lifecycle" => {
                shared_lifecycle_path = Some(PathBuf::from(take_value("--shared-lifecycle")?))
            }
            "--host" => host = take_value("--host")?,
            "--port" => {
                port = Some(
                    take_value("--port")?
                        .parse::<u16>()
                        .map_err(|error| format!("invalid --port: {error}"))?,
                )
            }
            "--run-id" => run_id = Some(take_value("--run-id")?),
            "--observer-username" => observer_username = Some(take_value("--observer-username")?),
            "--reporter-username" => reporter_username = Some(take_value("--reporter-username")?),
            "--room" => room = Some(take_value("--room")?),
            "--expected-gui-sha256" => {
                expected_gui_sha256 = Some(take_value("--expected-gui-sha256")?)
            }
            "--expected-mpv-sha256" => {
                expected_mpv_sha256 = Some(take_value("--expected-mpv-sha256")?)
            }
            "--timeout-ms" => {
                let millis = take_value("--timeout-ms")?
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --timeout-ms: {error}"))?;
                if !(1_000..=120_000).contains(&millis) {
                    return Err("--timeout-ms must be between 1000 and 120000".to_owned());
                }
                timeout = Duration::from_millis(millis);
            }
            _ => return Err(format!("unknown participant-status system option {arg:?}")),
        }
        index += 1;
    }

    let port = port.ok_or_else(|| "--participant-status-system requires --port".to_owned())?;
    if port == 0 {
        return Err("--port must be nonzero".to_owned());
    }
    let run_id = require_safe_token("--run-id", run_id)?;
    let observer_username = require_safe_token("--observer-username", observer_username)?;
    let reporter_username = require_safe_token("--reporter-username", reporter_username)?;
    let room = require_safe_token("--room", room)?;
    if observer_username == reporter_username {
        return Err("observer and reporter usernames must be distinct".to_owned());
    }
    let expected_gui_sha256 = require_digest("--expected-gui-sha256", expected_gui_sha256)?;
    let expected_mpv_sha256 = require_digest("--expected-mpv-sha256", expected_mpv_sha256)?;

    Ok(ParticipantStatusSystemOptions {
        binary_path: binary_path
            .ok_or_else(|| "--participant-status-system requires --binary PATH".to_owned())?,
        mpv_path: mpv_path
            .ok_or_else(|| "--participant-status-system requires --mpv PATH".to_owned())?,
        artifact_dir: artifact_dir
            .ok_or_else(|| "--participant-status-system requires --artifact-dir PATH".to_owned())?,
        shared_lifecycle_path: shared_lifecycle_path.ok_or_else(|| {
            "--participant-status-system requires --shared-lifecycle PATH".to_owned()
        })?,
        host,
        port,
        run_id,
        observer_username,
        reporter_username,
        room,
        expected_gui_sha256,
        expected_mpv_sha256,
        timeout,
    })
}

fn require_safe_token(name: &str, value: Option<String>) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("--participant-status-system requires {name}"))?;
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "{name} must be a 1-64 character privacy-safe token"
        ));
    }
    Ok(value)
}

fn require_digest(name: &str, value: Option<String>) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("--participant-status-system requires {name}"))?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{name} must be exactly 64 hexadecimal characters"));
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_args() -> Vec<String> {
        vec![
            "--participant-status-system",
            "--binary",
            "gui.exe",
            "--mpv",
            "mpv.exe",
            "--artifact-dir",
            "artifacts",
            "--shared-lifecycle",
            "gui.jsonl",
            "--port",
            "8999",
            "--run-id",
            "status-run-1",
            "--observer-username",
            "status-observer",
            "--reporter-username",
            "status-reporter",
            "--room",
            "status-room",
            "--expected-gui-sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--expected-mpv-sha256",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn participant_status_options_require_closed_complete_identity() {
        let parsed = parse_participant_status_system_options(&complete_args()).unwrap();
        assert_eq!(parsed.port, 8999);
        assert_eq!(parsed.reporter_username, "status-reporter");
        assert_eq!(parsed.timeout, Duration::from_secs(30));
    }

    #[test]
    fn participant_status_options_reject_unknown_or_unsafe_values() {
        let mut unknown = complete_args();
        unknown.push("--surprise".to_owned());
        assert!(parse_participant_status_system_options(&unknown).is_err());

        let mut unsafe_room = complete_args();
        let room_index = unsafe_room.iter().position(|arg| arg == "--room").unwrap() + 1;
        unsafe_room[room_index] = "not safe / room".to_owned();
        assert!(parse_participant_status_system_options(&unsafe_room).is_err());
    }

    #[test]
    fn participant_row_index_parser_rejects_nested_or_nonnumeric_ids() {
        assert_eq!(
            parse_main_window_participant_status_index("main-window:user:17:participant-status"),
            Some(17)
        );
        assert_eq!(
            parse_main_window_participant_status_index(
                "main-window:user:browser:17:participant-status"
            ),
            None
        );
        assert_eq!(
            parse_main_window_participant_status_index("main-window:user:new"),
            None
        );
    }

    #[test]
    fn fresh_status_label_accepts_compact_detail_but_rejects_waiting_or_stale() {
        assert!(accepted_fresh_status_label(
            "Ready · paused · 00:00.0 · Offset unavailable · fresh"
        ));
        assert!(accepted_fresh_status_label(
            "Playing · 00:12.4 · +0.1s · fresh"
        ));
        assert!(!accepted_fresh_status_label(
            "Waiting for first status report"
        ));
        assert!(!accepted_fresh_status_label("Playing · 00:12.4 · stale"));
    }
}
