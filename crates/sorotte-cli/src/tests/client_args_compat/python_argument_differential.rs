use super::*;
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const PINNED_LEGACY_SYNCPLAY_SHA: &str = "d1c5f85af377c960c5a940707c4d01bc84fd9c3f";
const ARGUMENT_PROBE_SCHEMA: &str = "sorotte-pinned-configuration-getter-arguments-v1";
const ENDPOINT_PROBE_SCHEMA: &str = "sorotte-pinned-configuration-getter-endpoint-v1";
const PASSWORD_CANARY: &str = "CLI_DIFFERENTIAL_PASSWORD_CANARY";

fn legacy_root() -> PathBuf {
    std::env::var_os("SYNCPLAY_LEGACY_ROOT").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(".interop-cache/syncplay-legacy")
        },
        PathBuf::from,
    )
}

fn assert_pinned_legacy_root(root: &Path) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git should inspect the pinned legacy checkout");
    assert!(
        output.status.success(),
        "cannot identify pinned legacy checkout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        PINNED_LEGACY_SYNCPLAY_SHA,
        "legacy argument oracle must use the reviewed pinned revision"
    );
}

fn run_python_probe(root: &Path, script_name: &str, request: &[u8]) -> std::process::Output {
    let python = std::env::var_os("SYNCPLAY_PYTHON_BIN").unwrap_or_else(|| "python".into());
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(script_name);
    let mut child = Command::new(python)
        .arg(&script)
        .arg("--legacy-root")
        .arg(root)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("pinned ConfigurationGetter probe should start");
    child
        .stdin
        .take()
        .expect("probe stdin should be piped")
        .write_all(request)
        .expect("probe request should write");
    let output = child
        .wait_with_output()
        .expect("pinned ConfigurationGetter probe should exit");
    assert!(
        output.status.success(),
        "pinned ConfigurationGetter probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn rust_argument_projection(id: &str, arguments: &[String]) -> serde_json::Value {
    let parsed = parse_legacy_client_arg_overrides(arguments);
    if !parsed.unknown_options.is_empty() {
        return serde_json::json!({"id": id, "accepted": false});
    }
    let host = parsed.host.as_ref().map(|host| {
        parsed
            .port
            .map_or_else(|| host.clone(), |port| format!("{host}:{port}"))
    });
    serde_json::json!({
        "id": id,
        "accepted": true,
        "host": host,
        "name": parsed.username,
        "room": parsed.room,
        "password": parsed
            .controlled_room_password_override
            .as_ref()
            .map(|_| sorotte_secret::REDACTED_SECRET),
        "debug": parsed.debug_requested,
        "force_gui_prompt": parsed.force_gui_prompt_requested,
        "file": parsed.file,
        "player_args": parsed.player_args,
    })
}

#[test]
fn cli_short_option_grammar_matches_pinned_python_configuration_getter() {
    let root = legacy_root();
    if !root.join("syncplay/ui/ConfigurationGetter.py").is_file() {
        eprintln!(
            "optional-skip(reason=missing-pinned-legacy-root): {}",
            root.display()
        );
        return;
    }
    assert_pinned_legacy_root(&root);

    let cases = [
        ("password-attached", vec![format!("-p{PASSWORD_CANARY}")]),
        ("password-equals", vec![format!("-p={PASSWORD_CANARY}")]),
        (
            "password-separated",
            vec!["-p".to_owned(), PASSWORD_CANARY.to_owned()],
        ),
        ("password-missing-optional", vec!["-p".to_owned()]),
        ("host-attached", vec!["-aexample.org:8999".to_owned()]),
        ("name-attached", vec!["-nAlice".to_owned()]),
        ("room-attached", vec!["-rroom".to_owned()]),
        ("flags-debug-gui", vec!["-dg".to_owned()]),
        ("flags-gui-debug", vec!["-gd".to_owned()]),
        ("psn-separated", vec!["-psn".to_owned(), "VALUE".to_owned()]),
        ("psn-equals", vec!["-psn=VALUE".to_owned()]),
        ("psn-prefix-is-password", vec!["-psnVALUE".to_owned()]),
        (
            "duplicate-host-final-wins",
            vec!["-afirst:1111".to_owned(), "-asecond:2222".to_owned()],
        ),
        (
            "duplicate-password-final-empty",
            vec![format!("-p{PASSWORD_CANARY}"), "-p=".to_owned()],
        ),
        ("empty-password-equals", vec!["-p=".to_owned()]),
        (
            "empty-password-separated",
            vec!["-p".to_owned(), String::new()],
        ),
        ("empty-room", vec!["-r=".to_owned()]),
        ("missing-host", vec!["-a".to_owned()]),
        ("missing-name", vec!["-n".to_owned()]),
        (
            "optional-password-before-flag",
            vec!["-p".to_owned(), "-d".to_owned()],
        ),
        (
            "optional-room-before-flag",
            vec!["-r".to_owned(), "-g".to_owned()],
        ),
        (
            "required-host-before-flag",
            vec!["-a".to_owned(), "-d".to_owned()],
        ),
        (
            "required-name-before-flag",
            vec!["-n".to_owned(), "-g".to_owned()],
        ),
        (
            "attached-password-starts-dash",
            vec!["-p=-secret".to_owned()],
        ),
        ("attached-host-starts-dash", vec!["-a=-host".to_owned()]),
    ];
    let request_cases = cases
        .iter()
        .map(|(id, arguments)| serde_json::json!({"id": id, "arguments": arguments}))
        .collect::<Vec<_>>();
    let request = serde_json::to_vec(&serde_json::json!({
        "schema": ARGUMENT_PROBE_SCHEMA,
        "cases": request_cases,
    }))
    .expect("argument probe request should serialize");

    let output = run_python_probe(&root, "python_argument_parser_probe.py", &request);
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !String::from_utf8_lossy(stream).contains(PASSWORD_CANARY),
            "pinned differential output exposed the password canary"
        );
    }

    let python_projection: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("probe output should be JSON");
    let rust_projection = cases
        .iter()
        .map(|(id, arguments)| rust_argument_projection(id, arguments))
        .collect::<Vec<_>>();
    assert_eq!(
        python_projection,
        serde_json::json!({
            "schema": ARGUMENT_PROBE_SCHEMA,
            "cases": rust_projection,
        })
    );
}

fn rust_endpoint_projection(id: &str, arguments: &[String]) -> serde_json::Value {
    let parsed = parse_legacy_client_arg_overrides(arguments);
    if !parsed.unknown_options.is_empty() {
        return serde_json::json!({"id": id, "accepted": false});
    }
    let mut config = build_client_loop_config_from_env();
    config.host = "lower.example".to_owned();
    config.port = 8999;
    apply_legacy_client_arg_overrides(&mut config, &parsed);
    serde_json::json!({
        "id": id,
        "accepted": validate_composed_client_endpoint(&config).is_ok(),
    })
}

#[test]
fn cli_endpoint_acceptance_is_differentially_bounded_by_pinned_final_validation() {
    let root = legacy_root();
    if !root.join("syncplay/ui/ConfigurationGetter.py").is_file() {
        eprintln!(
            "optional-skip(reason=missing-pinned-legacy-root): {}",
            root.display()
        );
        return;
    }
    assert_pinned_legacy_root(&root);

    let cases = [
        (
            "hostname-nonnumeric-port",
            vec!["--host".to_owned(), "example.org:notaport".to_owned()],
        ),
        (
            "hostname-empty-port",
            vec!["--host".to_owned(), "example.org:".to_owned()],
        ),
        (
            "hostname-zero-port",
            vec!["--host".to_owned(), "example.org:0".to_owned()],
        ),
        (
            "hostname-overflow-port",
            vec!["--host".to_owned(), "example.org:65536".to_owned()],
        ),
        ("empty-host", vec!["--host".to_owned(), ":1234".to_owned()]),
        (
            "bracketed-nonnumeric-port",
            vec!["--host".to_owned(), "[::1]:notaport".to_owned()],
        ),
        (
            "bracketed-zero-port",
            vec!["--host".to_owned(), "[::1]:0".to_owned()],
        ),
        (
            "bracketed-overflow-port",
            vec!["--host".to_owned(), "[::1]:65536".to_owned()],
        ),
        (
            "valid-then-invalid",
            vec![
                "--host".to_owned(),
                "valid.example:8999".to_owned(),
                "--host".to_owned(),
                "invalid.example:notaport".to_owned(),
            ],
        ),
        (
            "invalid-then-valid",
            vec![
                "--host".to_owned(),
                "invalid.example:notaport".to_owned(),
                "--host".to_owned(),
                "valid.example:8999".to_owned(),
            ],
        ),
        (
            "valid-without-port-inherits",
            vec!["--host".to_owned(), "valid.example".to_owned()],
        ),
    ];
    let request = serde_json::to_vec(&serde_json::json!({
        "schema": ENDPOINT_PROBE_SCHEMA,
        "cases": cases
            .iter()
            .map(|(id, arguments)| serde_json::json!({"id": id, "arguments": arguments}))
            .collect::<Vec<_>>(),
    }))
    .expect("endpoint probe request should serialize");
    let output = run_python_probe(&root, "python_endpoint_validation_probe.py", &request);
    let python: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("endpoint probe output should be JSON");
    assert_eq!(python["schema"], ENDPOINT_PROBE_SCHEMA);
    let python_cases = python["cases"]
        .as_array()
        .expect("endpoint probe cases should be an array");
    assert_eq!(python_cases.len(), cases.len());

    for ((id, arguments), python_case) in cases.iter().zip(python_cases) {
        let rust_case = rust_endpoint_projection(id, arguments);
        if *id == "bracketed-nonnumeric-port" {
            assert_eq!(
                python_case,
                &serde_json::json!({"id": id, "accepted": true})
            );
            assert_eq!(rust_case, serde_json::json!({"id": id, "accepted": false}));
        } else {
            assert_eq!(
                python_case, &rust_case,
                "endpoint acceptance drifted for {id}"
            );
        }
    }
}
