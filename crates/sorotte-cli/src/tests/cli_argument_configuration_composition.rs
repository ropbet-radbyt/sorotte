use super::*;

#[test]
fn short_attached_password_is_accepted_without_diagnostic_exposure() {
    let overrides = parse_legacy_client_arg_overrides(["-pCLI_PASSWORD_CANARY"]);

    assert!(
        overrides.unknown_options.is_empty(),
        "TC-CLI-005: short-attached password must be accepted without diagnostic exposure"
    );
}

#[test]
fn malformed_final_host_blocks_endpoint_composition() {
    let overrides = parse_legacy_client_arg_overrides([
        "--host",
        "valid.example:8999",
        "--host",
        "invalid.example:notaport",
    ]);

    assert!(
        !overrides.unknown_options.is_empty(),
        "TC-CLI-004: malformed final host must block endpoint composition"
    );
}

#[test]
fn unknown_attached_values_are_secret_safe_in_diagnostics() {
    const SECRET: &str = "CLI_UNKNOWN_OPTION_SECRET_CANARY";
    let overrides = parse_legacy_client_arg_overrides([format!("--api-token={SECRET}")]);
    assert_eq!(overrides.unknown_options.len(), 1);
    assert!(overrides.unknown_options[0].matches_rejected_token(&format!("--api-token={SECRET}")));
    assert!(
        !format!("{:?}", overrides.unknown_options).contains(SECRET),
        "the structural parser issue must not retain the attached value"
    );

    let diagnostic = legacy_unrecognized_arguments_diagnostic_line(&overrides.unknown_options);
    assert!(
        !diagnostic.contains(SECRET),
        "unknown-option diagnostics leaked an attached secret: {diagnostic}"
    );
    assert!(
        diagnostic.contains(sorotte_secret::REDACTED_SECRET),
        "redacted diagnostics must preserve a visible redaction marker: {diagnostic}"
    );

    let unattached = parse_legacy_client_arg_overrides(["--api-token"]);
    assert_eq!(
        legacy_unrecognized_arguments_diagnostic_line(&unattached.unknown_options),
        "error: unrecognized arguments: --api-token"
    );
}

#[test]
fn attached_cli_configuration_values_cross_the_real_parser_boundary() {
    let overrides = parse_legacy_client_arg_overrides([
        "--host=cli.example:4321",
        "--name=cli-user",
        "--room=cli-room",
        "--password=AB-123-456",
    ]);

    assert!(overrides.unknown_options.is_empty());
    assert_eq!(overrides.host.as_deref(), Some("cli.example"));
    assert_eq!(overrides.port, Some(4321));
    assert_eq!(overrides.username.as_deref(), Some("cli-user"));
    assert_eq!(overrides.room.as_deref(), Some("cli-room"));
    assert_eq!(
        overrides
            .controlled_room_password_override
            .as_ref()
            .map(sorotte_secret::SecretValue::expose_secret),
        Some("AB-123-456")
    );
}

#[test]
fn duplicate_host_without_port_clears_the_earlier_cli_port_override() {
    let overrides = parse_legacy_client_arg_overrides([
        "--host",
        "first.example:1111",
        "--host",
        "second.example",
    ]);

    assert_eq!(overrides.host.as_deref(), Some("second.example"));
    assert_eq!(
        overrides.port, None,
        "the later host must reveal the lower-layer port instead of retaining an earlier CLI port"
    );
}

#[test]
fn empty_duplicate_values_clear_the_cli_layer_override() {
    let overrides = parse_legacy_client_arg_overrides([
        "--name",
        "first-user",
        "--name=",
        "--room",
        "first-room",
        "--room=",
        "--password",
        "AB-123-456",
        "--password=",
    ]);

    assert!(overrides.unknown_options.is_empty());
    assert_eq!(overrides.username, None);
    assert_eq!(overrides.room, None);
    assert_eq!(overrides.controlled_room_password_override, None);
}

#[test]
fn missing_required_host_and_name_values_are_invalid_parser_cases() {
    let overrides = parse_legacy_client_arg_overrides(["--host", "--name"]);

    assert!(overrides.unknown_options[0].matches_rejected_token("--host"));
    assert!(overrides.unknown_options[1].matches_rejected_token("--name"));
}

#[test]
fn canonical_short_attached_values_and_boolean_clusters_match_argparse_shape() {
    let overrides = parse_legacy_client_arg_overrides([
        "-aexample.org:8999",
        "-nAlice",
        "-rroom",
        "-pCLI_PASSWORD_CANARY",
        "-dg",
    ]);

    assert!(overrides.unknown_options.is_empty());
    assert_eq!(overrides.host.as_deref(), Some("example.org"));
    assert_eq!(overrides.port, Some(8999));
    assert_eq!(overrides.username.as_deref(), Some("Alice"));
    assert_eq!(overrides.room.as_deref(), Some("room"));
    assert_eq!(
        overrides
            .controlled_room_password_override
            .as_ref()
            .map(sorotte_secret::SecretValue::expose_secret),
        Some("CLI_PASSWORD_CANARY")
    );
    assert!(overrides.debug_requested);
    assert!(overrides.force_gui_prompt_requested);

    let reverse_cluster = parse_legacy_client_arg_overrides(["-gd"]);
    assert!(reverse_cluster.debug_requested);
    assert!(reverse_cluster.force_gui_prompt_requested);
    assert!(reverse_cluster.unknown_options.is_empty());
}

#[test]
fn optional_short_values_and_psn_overlap_follow_pinned_argparse_precedence() {
    let flag_looking_values = parse_legacy_client_arg_overrides(["-p", "-d", "-r", "-g"]);
    assert_eq!(flag_looking_values.controlled_room_password_override, None);
    assert_eq!(flag_looking_values.room, None);
    assert!(flag_looking_values.debug_requested);
    assert!(flag_looking_values.force_gui_prompt_requested);
    assert!(flag_looking_values.unknown_options.is_empty());

    let separated_psn = parse_legacy_client_arg_overrides(["-psn", "VALUE"]);
    let equals_psn = parse_legacy_client_arg_overrides(["-psn=VALUE"]);
    for parsed in [&separated_psn, &equals_psn] {
        assert_eq!(parsed.controlled_room_password_override, None);
        assert_eq!(parsed.file, None);
        assert!(parsed.unknown_options.is_empty());
    }

    let password_prefix = parse_legacy_client_arg_overrides(["-psnVALUE"]);
    assert_eq!(
        password_prefix
            .controlled_room_password_override
            .as_ref()
            .map(sorotte_secret::SecretValue::expose_secret),
        Some("snVALUE")
    );
    assert!(password_prefix.unknown_options.is_empty());

    let missing_psn_value = parse_legacy_client_arg_overrides(["-psn"]);
    assert!(missing_psn_value.unknown_options[0].matches_rejected_token("-psn"));
}

fn final_host_argument_error(arguments: &[&str]) -> HostArgumentError {
    let parsed = parse_legacy_client_arg_overrides(arguments);
    assert_eq!(parsed.unknown_options.len(), 1, "arguments={arguments:?}");
    match parsed.unknown_options[0] {
        LegacyClientArgumentIssue::InvalidHost { error, .. } => error,
        ref issue => panic!("expected invalid-host issue for {arguments:?}, got {issue:?}"),
    }
}

#[test]
fn malformed_final_cli_endpoints_fail_closed_with_specific_error_kinds() {
    for (value, expected) in [
        ("example.org:notaport", HostArgumentError::NonNumericPort),
        ("example.org:", HostArgumentError::EmptyPort),
        ("example.org:0", HostArgumentError::PortOutOfRange),
        ("example.org:65536", HostArgumentError::PortOutOfRange),
        (":1234", HostArgumentError::EmptyHost),
        ("[::1]:notaport", HostArgumentError::NonNumericPort),
        ("[::1]:0", HostArgumentError::PortOutOfRange),
        ("[::1]:65536", HostArgumentError::PortOutOfRange),
        ("[::1", HostArgumentError::MalformedBracketedIpv6),
        ("[::1]junk", HostArgumentError::MalformedBracketedIpv6),
    ] {
        assert_eq!(
            final_host_argument_error(&["--host", value]),
            expected,
            "value={value:?}"
        );
    }
}

#[test]
fn final_host_occurrence_controls_failure_and_valid_no_port_inherits_lower_port() {
    let valid_then_invalid = parse_legacy_client_arg_overrides([
        "--host",
        "valid.example:8999",
        "--host",
        "invalid.example:notaport",
    ]);
    assert_eq!(valid_then_invalid.host, None);
    assert_eq!(valid_then_invalid.port, None);
    assert_eq!(valid_then_invalid.unknown_options.len(), 1);

    let invalid_then_valid = parse_legacy_client_arg_overrides([
        "--host",
        "invalid.example:notaport",
        "--host",
        "valid.example:8123",
    ]);
    assert!(invalid_then_valid.unknown_options.is_empty());
    assert_eq!(invalid_then_valid.host.as_deref(), Some("valid.example"));
    assert_eq!(invalid_then_valid.port, Some(8123));

    let mut inherited = build_client_loop_config_from_env();
    inherited.host = "lower.example".to_owned();
    inherited.port = 4567;
    let no_port = parse_legacy_client_arg_overrides(["--host", "valid.example"]);
    apply_legacy_client_arg_overrides(&mut inherited, &no_port);
    assert_eq!(inherited.host, "valid.example");
    assert_eq!(inherited.port, 4567);
    assert_eq!(validate_composed_client_endpoint(&inherited), Ok(()));

    inherited.host.clear();
    assert_eq!(
        validate_composed_client_endpoint(&inherited),
        Err(HostArgumentError::EmptyHost)
    );
    inherited.host = "valid.example".to_owned();
    inherited.port = 0;
    assert_eq!(
        validate_composed_client_endpoint(&inherited),
        Err(HostArgumentError::PortOutOfRange)
    );
}

const GENERATED_COMPOSITION_SEED: u64 = 0xc11a_5e7c_0f1a_2026;
const GENERATED_COMPOSITION_CASES: usize = 256;
const GENERATED_COMPOSITION_PATTERN_COUNT: usize = 16;
const EXPECTED_INVALID_CASES: usize = 48;
const EXPECTED_CLEAR_CASES: usize = 64;
const EXPECTED_DUPLICATE_CASES: usize = 112;
const CONTROLLED_ROOM_HASH: &str = "ABCDEF123456";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ConfigField {
    Host,
    Username,
    Room,
    Password,
    Unknown,
}

#[derive(Clone, Copy)]
enum ArgumentStyle {
    Separated,
    Attached,
}

struct ConfigArgumentOperation {
    field: ConfigField,
    value: Option<String>,
    long_form: bool,
    style: ArgumentStyle,
}

impl ConfigArgumentOperation {
    fn option_name(&self) -> &'static str {
        match (self.field, self.long_form) {
            (ConfigField::Host, true) => "--host",
            (ConfigField::Host, false) => "-a",
            (ConfigField::Username, true) => "--name",
            (ConfigField::Username, false) => "-n",
            (ConfigField::Room, true) => "--room",
            (ConfigField::Room, false) => "-r",
            (ConfigField::Password, true) => "--password",
            (ConfigField::Password, false) => "-p",
            (ConfigField::Unknown, _) => "--api-token",
        }
    }

    fn render_into(&self, arguments: &mut Vec<String>) {
        let option = self.option_name();
        match (&self.value, self.style) {
            (Some(value), ArgumentStyle::Separated) => {
                arguments.push(option.to_owned());
                arguments.push(value.clone());
            }
            (Some(value), ArgumentStyle::Attached) => {
                arguments.push(format!("{option}={value}"));
            }
            (None, _) => arguments.push(option.to_owned()),
        }
    }

    fn is_clear(&self) -> bool {
        !self.is_invalid() && self.value.as_ref().is_none_or(|value| value.is_empty())
    }

    fn is_invalid(&self) -> bool {
        self.field == ConfigField::Unknown
            || (matches!(self.field, ConfigField::Host | ConfigField::Username)
                && self.value.is_none())
    }
}

struct GeneratedEnvironment {
    host: Option<String>,
    port: Option<String>,
    server_password: Option<String>,
    username: Option<String>,
    room: Option<String>,
}

struct GeneratedStoredSettings {
    host: String,
    port: u16,
    server_password: String,
    username: String,
    room: String,
}

impl GeneratedStoredSettings {
    fn as_production_settings(&self) -> StoredClientSettingsMvp {
        StoredClientSettingsMvp {
            host: Some(self.host.clone()),
            port: Some(self.port),
            server_password: Some(self.server_password.clone().into()),
            username: Some(self.username.clone()),
            room: Some(self.room.clone()),
            ..StoredClientSettingsMvp::default()
        }
    }
}

struct GeneratedCompositionCase {
    id: String,
    environment: GeneratedEnvironment,
    stored: GeneratedStoredSettings,
    operations: Vec<ConfigArgumentOperation>,
    secret_markers: Vec<String>,
}

impl GeneratedCompositionCase {
    fn arguments(&self) -> Vec<String> {
        let mut arguments = Vec::new();
        for operation in &self.operations {
            operation.render_into(&mut arguments);
        }
        arguments
    }

    fn invalid_count(&self) -> usize {
        self.operations
            .iter()
            .filter(|operation| operation.is_invalid())
            .count()
    }

    fn has_clear(&self) -> bool {
        self.operations
            .iter()
            .any(ConfigArgumentOperation::is_clear)
    }

    fn has_duplicate(&self) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        self.operations
            .iter()
            .filter(|operation| operation.field != ConfigField::Unknown)
            .any(|operation| !seen.insert(operation.field))
    }
}

struct DeterministicCompositionGenerator {
    state: u64,
}

impl DeterministicCompositionGenerator {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(2_862_933_555_777_941_757)
            .wrapping_add(3_037_000_493);
        self.state
    }

    fn bounded(&mut self, upper_bound: u64) -> u64 {
        self.next_u64() % upper_bound
    }

    fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

fn separated(
    field: ConfigField,
    value: Option<String>,
    long_form: bool,
) -> ConfigArgumentOperation {
    ConfigArgumentOperation {
        field,
        value,
        long_form,
        style: ArgumentStyle::Separated,
    }
}

fn attached(field: ConfigField, value: String, long_form: bool) -> ConfigArgumentOperation {
    ConfigArgumentOperation {
        field,
        value: Some(value),
        long_form,
        style: ArgumentStyle::Attached,
    }
}

fn generated_controlled_room(prefix: &str, index: usize, password: &str) -> String {
    format!("+{prefix}-{index}:{CONTROLLED_ROOM_HASH}:{password}")
}

fn generated_composition_cases() -> Vec<GeneratedCompositionCase> {
    let mut generator = DeterministicCompositionGenerator::new(GENERATED_COMPOSITION_SEED);
    let mut cases = Vec::with_capacity(GENERATED_COMPOSITION_CASES);
    for index in 0..GENERATED_COMPOSITION_CASES {
        let token = generator.next_u64();
        let env_password = format!("ENV_SERVER_SECRET_{index}_{token:016x}");
        let stored_password = format!("STORED_SERVER_SECRET_{index}_{:016x}", generator.next_u64());
        let env_room_password = format!("EV-{:03}-{:03}", index % 1_000, generator.bounded(1_000));
        let stored_room_password =
            format!("ST-{:03}-{:03}", index % 1_000, generator.bounded(1_000));
        let cli_room_password = format!("CR-{:03}-{:03}", index % 1_000, generator.bounded(1_000));
        let cli_explicit_password =
            format!("CP-{:03}-{:03}", index % 1_000, generator.bounded(1_000));
        let unknown_secret = format!("CLI_UNKNOWN_SECRET_{index}_{:016x}", generator.next_u64());

        let environment = GeneratedEnvironment {
            host: generator
                .coin()
                .then(|| format!("env-{index}-{token:016x}.example")),
            port: generator
                .coin()
                .then(|| (10_000 + generator.bounded(20_000)).to_string()),
            server_password: generator.coin().then(|| env_password.clone()),
            username: generator
                .coin()
                .then(|| format!("env-user-{index}-{token:016x}")),
            room: generator.coin().then(|| {
                if generator.coin() {
                    generated_controlled_room("env-room", index, &env_room_password)
                } else {
                    format!("env-room-{index}-{token:016x}")
                }
            }),
        };
        let stored = GeneratedStoredSettings {
            host: format!("stored-{index}-{token:016x}.example"),
            port: (30_000 + generator.bounded(20_000)) as u16,
            server_password: stored_password.clone(),
            username: format!("stored-user-{index}-{token:016x}"),
            room: if generator.coin() {
                generated_controlled_room("stored-room", index, &stored_room_password)
            } else {
                format!("stored-room-{index}-{token:016x}")
            },
        };

        let cli_host_first = format!(
            "cli-first-{index}.example:{}",
            50_000 + generator.bounded(10_000)
        );
        let cli_host_last = format!("cli-last-{index}.example");
        let cli_username_first = format!("cli-user-first-{index}-{token:016x}");
        let cli_username_last = format!("cli-user-last-{index}-{token:016x}");
        let cli_room_first = format!("cli-room-first-{index}-{token:016x}");
        let cli_room_last = format!("cli-room-last-{index}-{token:016x}");
        let controlled_cli_room = generated_controlled_room("cli-room", index, &cli_room_password);
        let operations = match index % GENERATED_COMPOSITION_PATTERN_COUNT {
            0 => Vec::new(),
            1 => vec![
                separated(ConfigField::Host, Some(cli_host_first.clone()), true),
                separated(
                    ConfigField::Username,
                    Some(cli_username_first.clone()),
                    true,
                ),
                separated(ConfigField::Room, Some(cli_room_first.clone()), true),
                separated(
                    ConfigField::Password,
                    Some(cli_explicit_password.clone()),
                    true,
                ),
            ],
            2 => vec![
                attached(ConfigField::Host, cli_host_first.clone(), false),
                attached(ConfigField::Username, cli_username_first.clone(), false),
                attached(ConfigField::Room, cli_room_first.clone(), false),
                attached(ConfigField::Password, cli_explicit_password.clone(), false),
            ],
            3 => vec![
                separated(ConfigField::Host, Some(cli_host_first.clone()), true),
                attached(ConfigField::Host, cli_host_last.clone(), true),
            ],
            4 => vec![
                separated(ConfigField::Host, Some(cli_host_first.clone()), false),
                attached(ConfigField::Host, String::new(), true),
            ],
            5 => vec![
                separated(
                    ConfigField::Username,
                    Some(cli_username_first.clone()),
                    false,
                ),
                attached(ConfigField::Username, String::new(), true),
            ],
            6 => vec![
                attached(ConfigField::Room, cli_room_first.clone(), true),
                separated(ConfigField::Room, None, false),
            ],
            7 => vec![
                separated(
                    ConfigField::Password,
                    Some(cli_explicit_password.clone()),
                    true,
                ),
                attached(ConfigField::Password, String::new(), false),
            ],
            8 => vec![
                attached(ConfigField::Room, controlled_cli_room.clone(), true),
                separated(
                    ConfigField::Password,
                    Some(cli_explicit_password.clone()),
                    false,
                ),
            ],
            9 => vec![separated(ConfigField::Host, None, generator.coin())],
            10 => vec![separated(ConfigField::Username, None, generator.coin())],
            11 => vec![attached(ConfigField::Unknown, unknown_secret.clone(), true)],
            12 => vec![
                attached(ConfigField::Host, cli_host_last.clone(), true),
                separated(ConfigField::Host, Some(cli_host_first.clone()), false),
                attached(ConfigField::Username, cli_username_first.clone(), true),
                separated(
                    ConfigField::Username,
                    Some(cli_username_last.clone()),
                    false,
                ),
            ],
            13 => vec![separated(
                ConfigField::Room,
                Some(controlled_cli_room.clone()),
                false,
            )],
            14 => vec![attached(
                ConfigField::Username,
                cli_username_last.clone(),
                true,
            )],
            15 => vec![
                attached(ConfigField::Host, cli_host_first, true),
                attached(ConfigField::Username, cli_username_first, true),
                attached(ConfigField::Room, cli_room_first, true),
                separated(ConfigField::Room, Some(cli_room_last), false),
                attached(ConfigField::Password, cli_explicit_password.clone(), true),
            ],
            _ => unreachable!("pattern count is closed"),
        };

        let mut secret_markers = vec![
            stored_password,
            env_password,
            env_room_password,
            stored_room_password,
            cli_room_password,
            cli_explicit_password,
            unknown_secret,
        ];
        secret_markers.retain(|marker| !marker.is_empty());
        cases.push(GeneratedCompositionCase {
            id: format!("case-{index:03}"),
            environment,
            stored,
            operations,
            secret_markers,
        });
    }
    assert_eq!(cases.len(), GENERATED_COMPOSITION_CASES);
    cases
}

#[derive(Debug, PartialEq, Eq)]
struct ConfigurationProjection {
    host: String,
    port: u16,
    server_password: Option<sorotte_secret::SecretValue>,
    username: String,
    room: String,
    controlled_room_password: Option<sorotte_secret::SecretValue>,
}

impl ConfigurationProjection {
    fn from_production(config: &ClientLoopConfig) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            server_password: config.server_password.clone(),
            username: config.username.clone(),
            room: config.room.clone(),
            controlled_room_password: config.controlled_room_password_override.clone(),
        }
    }
}

fn independent_controlled_room_normalization(
    room: &str,
) -> (String, Option<sorotte_secret::SecretValue>) {
    let Some((normalized_room, password)) = room.rsplit_once(':') else {
        return (room.to_owned(), None);
    };
    let bytes = password.as_bytes();
    let password_shape_matches = bytes.len() == 10
        && bytes[0].is_ascii_uppercase()
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b'-'
        && bytes[3..6].iter().all(u8::is_ascii_digit)
        && bytes[6] == b'-'
        && bytes[7..10].iter().all(u8::is_ascii_digit);
    if normalized_room.starts_with('+') && password_shape_matches {
        (normalized_room.to_owned(), Some(password.to_owned().into()))
    } else {
        (room.to_owned(), None)
    }
}

fn independent_host_and_port(value: &str) -> (String, Option<u16>) {
    value.rsplit_once(':').map_or_else(
        || (value.to_owned(), None),
        |(host, port)| {
            let parsed_port = port.parse::<u16>().ok().filter(|port| *port > 0);
            if parsed_port.is_some() {
                (host.to_owned(), parsed_port)
            } else {
                (value.to_owned(), None)
            }
        },
    )
}

struct IndependentCliOverrides {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    room: Option<String>,
    password: Option<sorotte_secret::SecretValue>,
    invalid_options: Vec<String>,
}

fn independently_model_cli_operations(
    operations: &[ConfigArgumentOperation],
) -> IndependentCliOverrides {
    let mut modeled = IndependentCliOverrides {
        host: None,
        port: None,
        username: None,
        room: None,
        password: None,
        invalid_options: Vec::new(),
    };
    for operation in operations {
        if operation.is_invalid() {
            let option = operation.option_name();
            modeled.invalid_options.push(match &operation.value {
                Some(value) => format!("{option}={value}"),
                None => option.to_owned(),
            });
            continue;
        }
        match operation.field {
            ConfigField::Host => {
                modeled.host = None;
                modeled.port = None;
                if let Some(value) = operation.value.as_deref().filter(|value| !value.is_empty()) {
                    let (host, port) = independent_host_and_port(value);
                    modeled.host = Some(host);
                    modeled.port = port;
                }
            }
            ConfigField::Username => {
                modeled.username = operation
                    .value
                    .as_ref()
                    .filter(|value| !value.is_empty())
                    .cloned();
            }
            ConfigField::Room => {
                modeled.room = operation
                    .value
                    .as_ref()
                    .filter(|value| !value.is_empty())
                    .cloned();
            }
            ConfigField::Password => {
                modeled.password = operation
                    .value
                    .as_ref()
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .map(Into::into);
            }
            ConfigField::Unknown => unreachable!("invalid branch handles unknown operations"),
        }
    }
    modeled
}

fn independent_lower_layer_projection(
    environment: &GeneratedEnvironment,
    stored: &GeneratedStoredSettings,
) -> ConfigurationProjection {
    let host = environment
        .host
        .clone()
        .unwrap_or_else(|| stored.host.clone());
    let port = environment
        .port
        .as_deref()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(stored.port);
    let server_password = environment
        .server_password
        .clone()
        .unwrap_or_else(|| stored.server_password.clone());
    let username = environment
        .username
        .clone()
        .unwrap_or_else(|| stored.username.clone());
    let room_input = environment.room.as_deref().unwrap_or(stored.room.as_str());
    let (room, controlled_room_password) = independent_controlled_room_normalization(room_input);
    ConfigurationProjection {
        host,
        port,
        server_password: Some(server_password.into()),
        username,
        room,
        controlled_room_password,
    }
}

fn independently_apply_cli_overrides(
    mut projection: ConfigurationProjection,
    overrides: &IndependentCliOverrides,
) -> ConfigurationProjection {
    if let Some(host) = overrides.host.as_ref() {
        projection.host.clone_from(host);
    }
    if let Some(port) = overrides.port {
        projection.port = port;
    }
    if let Some(username) = overrides.username.as_ref() {
        projection.username.clone_from(username);
    }
    if let Some(room) = overrides.room.as_ref() {
        let (room, password) = independent_controlled_room_normalization(room);
        projection.room = room;
        projection.controlled_room_password = password;
    }
    if let Some(password) = overrides.password.as_ref() {
        projection.controlled_room_password = Some(password.clone());
    }
    projection
}

fn install_generated_environment(env: &TestEnvGuard<'_>, environment: &GeneratedEnvironment) {
    fn set_optional(env: &TestEnvGuard<'_>, key: &str, value: Option<&str>) {
        if let Some(value) = value {
            env.set_var(key, value);
        } else {
            env.remove_var(key);
        }
    }

    set_optional(env, "SOROTTE_CLIENT_HOST", environment.host.as_deref());
    set_optional(env, "SOROTTE_CLIENT_PORT", environment.port.as_deref());
    set_optional(
        env,
        "SOROTTE_CLIENT_SERVER_PASSWORD",
        environment.server_password.as_deref(),
    );
    set_optional(
        env,
        "SOROTTE_CLIENT_USERNAME",
        environment.username.as_deref(),
    );
    env.remove_var("SOROTTE_CLIENT_NAME");
    set_optional(env, "SOROTTE_CLIENT_ROOM", environment.room.as_deref());
}

#[test]
fn generated_cli_configuration_composition_matches_independent_precedence_oracle() {
    let cases = generated_composition_cases();
    assert_eq!(
        cases.iter().filter(|case| case.invalid_count() > 0).count(),
        EXPECTED_INVALID_CASES
    );
    assert_eq!(
        cases.iter().filter(|case| case.has_clear()).count(),
        EXPECTED_CLEAR_CASES
    );
    assert_eq!(
        cases.iter().filter(|case| case.has_duplicate()).count(),
        EXPECTED_DUPLICATE_CASES
    );

    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let mut processed = 0usize;
    let mut valid = 0usize;
    let mut invalid = 0usize;
    for case in &cases {
        install_generated_environment(&env, &case.environment);
        let mut production = build_client_loop_config_from_env();
        apply_stored_client_settings_mvp_if_env_absent(
            &mut production,
            &case.stored.as_production_settings(),
        );

        let arguments = case.arguments();
        let parsed = parse_legacy_client_arg_overrides(&arguments);
        let modeled_cli = independently_model_cli_operations(&case.operations);
        assert_eq!(
            parsed.unknown_options.len(),
            modeled_cli.invalid_options.len(),
            "{} parser invalid-option accounting differs",
            case.id
        );
        for (actual, expected) in parsed
            .unknown_options
            .iter()
            .zip(&modeled_cli.invalid_options)
        {
            assert!(
                actual.matches_rejected_token(expected),
                "{} parser invalid-option identity or order differs",
                case.id
            );
        }

        if modeled_cli.invalid_options.is_empty() {
            valid += 1;
            apply_legacy_client_arg_overrides(&mut production, &parsed);
            let expected = independently_apply_cli_overrides(
                independent_lower_layer_projection(&case.environment, &case.stored),
                &modeled_cli,
            );
            assert_eq!(
                ConfigurationProjection::from_production(&production),
                expected,
                "{} precedence or clear/replace projection differs",
                case.id
            );
        } else {
            invalid += 1;
            let diagnostic = legacy_unrecognized_arguments_diagnostic_line(&parsed.unknown_options);
            assert!(
                diagnostic.starts_with("error: unrecognized arguments: "),
                "{} invalid parser case lost its bounded diagnostic",
                case.id
            );
            for secret in &case.secret_markers {
                assert!(
                    !diagnostic.contains(secret),
                    "{} invalid parser diagnostic leaked a secret marker",
                    case.id
                );
            }
        }

        let debug = format!("{production:?}\n{parsed:?}");
        for secret in &case.secret_markers {
            assert!(
                !debug.contains(secret),
                "{} configuration debug leaked a secret marker",
                case.id
            );
        }
        processed += 1;
    }
    assert_eq!(processed, GENERATED_COMPOSITION_CASES);
    assert_eq!(valid, GENERATED_COMPOSITION_CASES - EXPECTED_INVALID_CASES);
    assert_eq!(invalid, EXPECTED_INVALID_CASES);
}
