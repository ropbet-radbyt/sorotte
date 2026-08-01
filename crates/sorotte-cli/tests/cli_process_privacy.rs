use std::{
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sorotte-cli"))
        .args(arguments)
        .output()
        .expect("sorotte-cli process should start")
}

fn combined_output(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_canary_absent(output: &Output, canary: &str) {
    let combined = combined_output(output);
    assert!(
        !combined.contains(canary),
        "CLI process output exposed a secret canary: {combined}"
    );
}

fn unique_absent_config_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sorotte-cli-{label}-{}-{nonce}",
        std::process::id()
    ));
    assert!(!path.exists(), "unique test config root must begin absent");
    path
}

#[test]
fn accepted_password_spellings_never_reach_process_output() {
    const CANARY: &str = "CLI_PASSWORD_CANARY_8C746A";
    for argument in [
        format!("-p{CANARY}"),
        format!("-p={CANARY}"),
        format!("--password={CANARY}"),
    ] {
        let output = run_cli(&[&argument, "--version"]);
        assert!(
            output.status.success(),
            "accepted password spelling {argument:?} failed: {}",
            combined_output(&output)
        );
        assert_canary_absent(&output, CANARY);
    }
}

#[test]
fn rejected_unknown_attached_value_never_reaches_process_output_or_error_debug() {
    const CANARY: &str = "CLI_UNKNOWN_CANARY_26B41D";
    let argument = format!("--unknown={CANARY}");
    let output = run_cli(&[&argument]);

    assert!(
        !output.status.success(),
        "unknown option unexpectedly succeeded: {}",
        combined_output(&output)
    );
    assert_canary_absent(&output, CANARY);
    assert!(
        combined_output(&output).contains(sorotte_secret::REDACTED_SECRET),
        "unknown attached value should retain a visible redaction marker"
    );
}

#[test]
fn invalid_cli_endpoint_exits_before_settings_player_or_network_side_effects() {
    let malformed_root = unique_absent_config_root("malformed-endpoint");
    let malformed = Command::new(env!("CARGO_BIN_EXE_sorotte-cli"))
        .args([
            "--config-root",
            malformed_root
                .to_str()
                .expect("temporary config path should be UTF-8"),
            "--host",
            ":1234",
            "--player-path",
            "definitely-not-a-player",
            "--no-gui",
        ])
        .output()
        .expect("malformed-endpoint CLI process should start");
    assert!(!malformed.status.success());
    assert!(combined_output(&malformed).contains("host is empty"));
    assert!(
        !malformed_root.exists(),
        "malformed CLI endpoint must fail before config persistence"
    );
    assert!(!combined_output(&malformed).contains("failed to launch"));
}
