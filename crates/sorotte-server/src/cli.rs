use std::env;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use sorotte_secret::SecretValue;

const DEFAULT_SERVER_PORT: u16 = 8999;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum OptionalCliValue<T> {
    #[default]
    Absent,
    PresentWithoutValue,
    Value(T),
}

impl<T> OptionalCliValue<T> {
    fn into_option(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::PresentWithoutValue => None,
        }
    }

    fn resolve_with_env(self, env_value: impl FnOnce() -> Option<T>) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent => env_value(),
            Self::PresentWithoutValue => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) struct ServerCliOverrides {
    port: OptionalCliValue<u16>,
    password: OptionalCliValue<SecretValue>,
    salt: OptionalCliValue<SecretValue>,
    motd_file: OptionalCliValue<PathBuf>,
    rooms_db_file: OptionalCliValue<PathBuf>,
    permanent_rooms_file: OptionalCliValue<PathBuf>,
    stats_db_file: OptionalCliValue<PathBuf>,
    tls_cert_path: OptionalCliValue<PathBuf>,
    disable_ready: bool,
    disable_chat: bool,
    max_chat_message_length: OptionalCliValue<usize>,
    max_username_length: OptionalCliValue<usize>,
    max_persistent_rooms: OptionalCliValue<usize>,
    max_persistent_rooms_per_identity: OptionalCliValue<usize>,
    persistent_room_creation_cooldown_seconds: OptionalCliValue<u64>,
    persistent_room_inactivity_expiry_seconds: OptionalCliValue<u64>,
    isolate_rooms: bool,
    ipv4_only: bool,
    ipv6_only: bool,
    interface_ipv4: OptionalCliValue<String>,
    interface_ipv6: OptionalCliValue<String>,
}

impl std::fmt::Debug for ServerCliOverrides {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerCliOverrides")
            .field("port", &self.port)
            .field(
                "password",
                &matches!(self.password, OptionalCliValue::Value(_))
                    .then_some(sorotte_secret::REDACTED_SECRET),
            )
            .field(
                "salt",
                &matches!(self.salt, OptionalCliValue::Value(_))
                    .then_some(sorotte_secret::REDACTED_SECRET),
            )
            .field("disable_ready", &self.disable_ready)
            .field("disable_chat", &self.disable_chat)
            .field("isolate_rooms", &self.isolate_rooms)
            .field("ipv4_only", &self.ipv4_only)
            .field("ipv6_only", &self.ipv6_only)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
    Run(Box<ServerCliOverrides>),
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerBindFamily {
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerBindEndpoint {
    pub(crate) family: ServerBindFamily,
    pub(crate) host: String,
}

#[derive(Clone)]
pub(crate) struct ServerRunConfig {
    pub(crate) bind_endpoints: Vec<ServerBindEndpoint>,
    pub(crate) port: u16,
    pub(crate) server_password_token: Option<SecretValue>,
    pub(crate) room_password_salt: Option<SecretValue>,
    pub(crate) motd_template: Option<String>,
    pub(crate) rooms_db_file: Option<PathBuf>,
    pub(crate) permanent_rooms_file: Option<PathBuf>,
    pub(crate) stats_db_file: Option<PathBuf>,
    pub(crate) tls_cert_path: Option<PathBuf>,
    pub(crate) persistent_rooms_enabled: bool,
    pub(crate) chat_enabled: bool,
    pub(crate) readiness_enabled: bool,
    pub(crate) max_chat_message_length: Option<usize>,
    pub(crate) max_username_length: Option<usize>,
    pub(crate) max_persistent_rooms: Option<usize>,
    pub(crate) max_persistent_rooms_per_identity: Option<usize>,
    pub(crate) persistent_room_creation_cooldown_seconds: Option<u64>,
    pub(crate) persistent_room_inactivity_expiry_seconds: Option<u64>,
    pub(crate) isolate_rooms: bool,
}

impl std::fmt::Debug for ServerRunConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerRunConfig")
            .field("bind_endpoints", &self.bind_endpoints)
            .field("port", &self.port)
            .field(
                "server_password_token",
                &self
                    .server_password_token
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field(
                "room_password_salt",
                &self
                    .room_password_salt
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("rooms_db_file", &self.rooms_db_file)
            .field("permanent_rooms_file", &self.permanent_rooms_file)
            .field("stats_db_file", &self.stats_db_file)
            .field("tls_cert_path", &self.tls_cert_path)
            .field("persistent_rooms_enabled", &self.persistent_rooms_enabled)
            .field("chat_enabled", &self.chat_enabled)
            .field("readiness_enabled", &self.readiness_enabled)
            .field("isolate_rooms", &self.isolate_rooms)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliParseError(String);

impl CliParseError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliParseError {}

fn env_flag_enabled(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

fn env_trimmed(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn env_u16(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
}

fn help_text() -> &'static str {
    concat!(
        "sorotte-server\n\n",
        "Usage:\n",
        "  sorotte-server [options]\n\n",
        "Supported options:\n",
        "  -h, --help                       Show this help text\n",
        "  -V, --version                    Show version\n",
        "  --port [port]                    TCP port to listen on (default: 8999)\n",
        "  --password [password]            Server password\n",
        "  --salt [salt]                    Salt for controlled-room password hashes\n",
        "  --disable-ready                  Disable readiness feature\n",
        "  --disable-chat                   Disable chat feature\n",
        "  --isolate-rooms                  Isolate rooms (room-scoped visibility/listing)\n",
        "  --motd-file [file]               Read MOTD template from file\n",
        "  --rooms-db-file [file]           Enable persistent rooms using SQLite file\n",
        "  --permanent-rooms-file [file]    Load permanent room names (one per line)\n",
        "  --max-chat-message-length [n]    Advertised/applied chat message length cap\n",
        "  --max-username-length [n]        Advertised/applied username length cap\n",
        "  --max-persistent-rooms [n]       Maximum client-created durable rooms (default: 1024)\n",
        "  --max-persistent-rooms-per-identity [n]\n",
        "                                    Durable-room quota per peer IP (default: 64)\n",
        "  --persistent-room-creation-cooldown-seconds [n]\n",
        "                                    Minimum per-IP creation interval (default: 1)\n",
        "  --persistent-room-inactivity-expiry-seconds [n]\n",
        "                                    Expire empty inactive durable rooms (default: 2592000; 0 disables)\n",
        "  --stats-db-file [file]           Enable stats snapshots using SQLite file\n",
        "  --tls [dir]                      TLS bundle directory (atomic current.json/generations or loose PEM files)\n",
        "  --ipv4-only                      Bind only IPv4 listen socket\n",
        "  --ipv6-only                      Bind IPv6 listen socket\n",
        "  --interface-ipv4 [ip]            Bind to specific IPv4 address\n",
        "  --interface-ipv6 [ip]            Bind to specific IPv6 address\n\n",
        "Environment overrides (legacy bootstrap compatibility):\n",
        "  SOROTTE_SERVER_PORT\n",
        "  SOROTTE_PASSWORD\n",
        "  SOROTTE_SERVER_PASSWORD\n",
        "  SOROTTE_SALT\n",
        "  SOROTTE_SERVER_SALT\n",
        "  SOROTTE_SERVER_MOTD_TEMPLATE\n",
        "  SOROTTE_SERVER_ROOMS_DB_FILE\n",
        "  SOROTTE_SERVER_PERMANENT_ROOMS_FILE\n",
        "  SOROTTE_SERVER_STATS_DB_FILE\n",
        "  SOROTTE_SERVER_TLS_CERT_PATH\n",
        "  SOROTTE_SERVER_PERSISTENT_ROOMS\n",
    )
}

fn take_optional_option_value(
    inline_value: Option<String>,
    args: &[String],
    index: &mut usize,
) -> OptionalCliValue<String> {
    if let Some(value) = inline_value {
        return OptionalCliValue::Value(value);
    }

    if let Some(value) = args.get(*index + 1)
        && !value.starts_with('-')
    {
        *index += 1;
        return OptionalCliValue::Value(value.clone());
    }

    OptionalCliValue::PresentWithoutValue
}

fn parse_optional_option_value<T>(
    inline_value: Option<String>,
    args: &[String],
    index: &mut usize,
    parse: impl FnOnce(String) -> Result<T, CliParseError>,
) -> Result<OptionalCliValue<T>, CliParseError> {
    match take_optional_option_value(inline_value, args, index) {
        OptionalCliValue::Value(value) => parse(value).map(OptionalCliValue::Value),
        OptionalCliValue::Absent => Ok(OptionalCliValue::Absent),
        OptionalCliValue::PresentWithoutValue => Ok(OptionalCliValue::PresentWithoutValue),
    }
}

fn split_long_option(arg: &str) -> Option<(&str, Option<String>)> {
    if !arg.starts_with("--") {
        return None;
    }
    match arg.split_once('=') {
        Some((name, value)) => Some((name, Some(value.to_owned()))),
        None => Some((arg, None)),
    }
}

pub(crate) fn parse_server_cli_args<I, S>(args: I) -> Result<CliAction, CliParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut overrides = ServerCliOverrides::default();
    let mut unknown_options: Vec<String> = Vec::new();
    let mut positional_args: Vec<String> = Vec::new();
    let mut index = 0usize;

    while index < args.len() {
        let arg = &args[index];

        if arg == "-h" || arg == "--help" {
            return Ok(CliAction::Help);
        }
        if arg == "-V" || arg == "--version" {
            return Ok(CliAction::Version);
        }
        if arg == "--" {
            positional_args.extend(args[(index + 1)..].iter().cloned());
            break;
        }

        let Some((name, inline_value)) = split_long_option(arg) else {
            if arg.starts_with('-') {
                unknown_options.push(arg.clone());
            } else {
                positional_args.push(arg.clone());
            }
            index += 1;
            continue;
        };

        match name {
            "--port" => {
                overrides.port =
                    parse_optional_option_value(inline_value, &args, &mut index, |value| {
                        value.parse::<u16>().map_err(|_| {
                            CliParseError::new(format!(
                                "--port expects an integer in 0..=65535, got '{value}'"
                            ))
                        })
                    })?;
            }
            "--password" => {
                overrides.password =
                    parse_optional_option_value(inline_value, &args, &mut index, |value| {
                        Ok(SecretValue::from(value))
                    })?;
            }
            "--salt" => {
                overrides.salt =
                    parse_optional_option_value(inline_value, &args, &mut index, |value| {
                        Ok(SecretValue::from(value))
                    })?;
            }
            "--disable-ready" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--disable-ready does not take a value"));
                }
                overrides.disable_ready = true;
            }
            "--disable-chat" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--disable-chat does not take a value"));
                }
                overrides.disable_chat = true;
            }
            "--isolate-rooms" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--isolate-rooms does not take a value"));
                }
                overrides.isolate_rooms = true;
            }
            "--motd-file" => {
                overrides.motd_file =
                    take_optional_option_value(inline_value, &args, &mut index).into_path_value();
            }
            "--rooms-db-file" => {
                overrides.rooms_db_file =
                    take_optional_option_value(inline_value, &args, &mut index).into_path_value();
            }
            "--permanent-rooms-file" => {
                overrides.permanent_rooms_file =
                    take_optional_option_value(inline_value, &args, &mut index).into_path_value();
            }
            "--stats-db-file" => {
                overrides.stats_db_file =
                    take_optional_option_value(inline_value, &args, &mut index).into_path_value();
            }
            "--max-chat-message-length" => {
                overrides.max_chat_message_length = parse_optional_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    |value| {
                        value.parse::<usize>().map_err(|_| {
                            CliParseError::new(format!(
                                "--max-chat-message-length expects a non-negative integer, got '{value}'"
                            ))
                        })
                    },
                )?;
            }
            "--max-username-length" => {
                overrides.max_username_length = parse_optional_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    |value| {
                        value.parse::<usize>().map_err(|_| {
                            CliParseError::new(format!(
                                "--max-username-length expects a non-negative integer, got '{value}'"
                            ))
                        })
                    },
                )?;
            }
            "--max-persistent-rooms" => {
                overrides.max_persistent_rooms = parse_optional_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    |value| {
                        value.parse::<usize>().map_err(|_| {
                            CliParseError::new(format!(
                                "--max-persistent-rooms expects a non-negative integer, got '{value}'"
                            ))
                        })
                    },
                )?;
            }
            "--max-persistent-rooms-per-identity" => {
                overrides.max_persistent_rooms_per_identity = parse_optional_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    |value| {
                        value.parse::<usize>().map_err(|_| {
                            CliParseError::new(format!(
                                "--max-persistent-rooms-per-identity expects a non-negative integer, got '{value}'"
                            ))
                        })
                    },
                )?;
            }
            "--persistent-room-creation-cooldown-seconds" => {
                overrides.persistent_room_creation_cooldown_seconds = parse_optional_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    |value| {
                        value.parse::<u64>().map_err(|_| {
                            CliParseError::new(format!(
                                "--persistent-room-creation-cooldown-seconds expects a non-negative integer, got '{value}'"
                            ))
                        })
                    },
                )?;
            }
            "--persistent-room-inactivity-expiry-seconds" => {
                overrides.persistent_room_inactivity_expiry_seconds = parse_optional_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    |value| {
                        value.parse::<u64>().map_err(|_| {
                            CliParseError::new(format!(
                                "--persistent-room-inactivity-expiry-seconds expects a non-negative integer, got '{value}'"
                            ))
                        })
                    },
                )?;
            }
            "--tls" => {
                overrides.tls_cert_path =
                    take_optional_option_value(inline_value, &args, &mut index).into_path_value();
            }
            "--ipv4-only" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--ipv4-only does not take a value"));
                }
                overrides.ipv4_only = true;
            }
            "--ipv6-only" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--ipv6-only does not take a value"));
                }
                overrides.ipv6_only = true;
            }
            "--interface-ipv4" => {
                overrides.interface_ipv4 =
                    take_optional_option_value(inline_value, &args, &mut index);
            }
            "--interface-ipv6" => {
                overrides.interface_ipv6 =
                    take_optional_option_value(inline_value, &args, &mut index);
            }
            _ => {
                unknown_options.push(arg.clone());
            }
        }

        index += 1;
    }

    if !unknown_options.is_empty() {
        return Err(CliParseError::new(format!(
            "unknown option(s): {} (see --help)",
            unknown_options.join(", ")
        )));
    }

    if !positional_args.is_empty() {
        return Err(CliParseError::new(format!(
            "positional arguments are not supported yet: {}",
            positional_args.join(" ")
        )));
    }

    Ok(CliAction::Run(Box::new(overrides)))
}

impl OptionalCliValue<String> {
    fn into_path_value(self) -> OptionalCliValue<PathBuf> {
        match self {
            Self::Absent => OptionalCliValue::Absent,
            Self::PresentWithoutValue => OptionalCliValue::PresentWithoutValue,
            Self::Value(value) => OptionalCliValue::Value(PathBuf::from(value)),
        }
    }
}

fn parse_ipv4_interface(value: Option<String>) -> Result<String, CliParseError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(Ipv4Addr::UNSPECIFIED.to_string());
    };
    let parsed = value.parse::<Ipv4Addr>().map_err(|_| {
        CliParseError::new(format!(
            "--interface-ipv4 expects an IPv4 address, got '{value}'"
        ))
    })?;
    Ok(parsed.to_string())
}

fn parse_ipv6_interface(value: Option<String>) -> Result<String, CliParseError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(Ipv6Addr::UNSPECIFIED.to_string());
    };
    let parsed = value.parse::<Ipv6Addr>().map_err(|_| {
        CliParseError::new(format!(
            "--interface-ipv6 expects an IPv6 address, got '{value}'"
        ))
    })?;
    Ok(parsed.to_string())
}

fn resolve_bind_endpoints(
    overrides: &ServerCliOverrides,
) -> Result<Vec<ServerBindEndpoint>, CliParseError> {
    if overrides.ipv4_only && overrides.ipv6_only {
        return Err(CliParseError::new(
            "--ipv4-only and --ipv6-only are mutually exclusive",
        ));
    }

    if overrides.ipv4_only {
        return Ok(vec![ServerBindEndpoint {
            family: ServerBindFamily::Ipv4,
            host: parse_ipv4_interface(overrides.interface_ipv4.clone().into_option())?,
        }]);
    }
    if overrides.ipv6_only {
        return Ok(vec![ServerBindEndpoint {
            family: ServerBindFamily::Ipv6,
            host: parse_ipv6_interface(overrides.interface_ipv6.clone().into_option())?,
        }]);
    }

    Ok(vec![
        ServerBindEndpoint {
            family: ServerBindFamily::Ipv6,
            host: parse_ipv6_interface(overrides.interface_ipv6.clone().into_option())?,
        },
        ServerBindEndpoint {
            family: ServerBindFamily::Ipv4,
            host: parse_ipv4_interface(overrides.interface_ipv4.clone().into_option())?,
        },
    ])
}

fn read_motd_template_file(path: &PathBuf) -> Result<String, anyhow::Error> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read MOTD file '{}'", path.display()))?;
    let mut contents = String::from_utf8(bytes)
        .map_err(|source| anyhow!("failed to read MOTD file '{}': {source}", path.display()))?;
    if contents.starts_with('\u{feff}') {
        contents.remove(0);
    }
    Ok(contents)
}

pub(crate) fn resolve_run_config(
    overrides: ServerCliOverrides,
) -> Result<ServerRunConfig, anyhow::Error> {
    let bind_endpoints = resolve_bind_endpoints(&overrides)?;
    let port = overrides
        .port
        .resolve_with_env(|| env_u16("SOROTTE_SERVER_PORT"))
        .unwrap_or(DEFAULT_SERVER_PORT);
    let room_password_salt = overrides.salt.resolve_with_env(|| {
        env_trimmed("SOROTTE_SALT")
            .or_else(|| env_trimmed("SOROTTE_SERVER_SALT"))
            .map(SecretValue::from)
    });
    let server_password_token = overrides.password.resolve_with_env(|| {
        env_trimmed("SOROTTE_PASSWORD")
            .or_else(|| env_trimmed("SOROTTE_SERVER_PASSWORD"))
            .map(SecretValue::from)
    });

    let motd_template = match overrides.motd_file {
        OptionalCliValue::Value(motd_file) => Some(read_motd_template_file(&motd_file)?),
        OptionalCliValue::PresentWithoutValue => None,
        OptionalCliValue::Absent => env_trimmed("SOROTTE_SERVER_MOTD_TEMPLATE"),
    };

    let rooms_db_file = overrides
        .rooms_db_file
        .resolve_with_env(|| env_trimmed("SOROTTE_SERVER_ROOMS_DB_FILE").map(PathBuf::from));
    let permanent_rooms_file = overrides
        .permanent_rooms_file
        .resolve_with_env(|| env_trimmed("SOROTTE_SERVER_PERMANENT_ROOMS_FILE").map(PathBuf::from));
    let stats_db_file = overrides
        .stats_db_file
        .resolve_with_env(|| env_trimmed("SOROTTE_SERVER_STATS_DB_FILE").map(PathBuf::from));
    let tls_cert_path = overrides
        .tls_cert_path
        .resolve_with_env(|| env_trimmed("SOROTTE_SERVER_TLS_CERT_PATH").map(PathBuf::from));

    let persistent_rooms_enabled = env_flag_enabled("SOROTTE_SERVER_PERSISTENT_ROOMS")
        || rooms_db_file.is_some()
        || permanent_rooms_file.is_some();

    Ok(ServerRunConfig {
        bind_endpoints,
        port,
        server_password_token,
        room_password_salt,
        motd_template,
        rooms_db_file,
        permanent_rooms_file,
        stats_db_file,
        tls_cert_path,
        persistent_rooms_enabled,
        chat_enabled: !overrides.disable_chat,
        readiness_enabled: !overrides.disable_ready,
        max_chat_message_length: overrides.max_chat_message_length.into_option(),
        max_username_length: overrides.max_username_length.into_option(),
        max_persistent_rooms: overrides.max_persistent_rooms.into_option(),
        max_persistent_rooms_per_identity: overrides
            .max_persistent_rooms_per_identity
            .into_option(),
        persistent_room_creation_cooldown_seconds: overrides
            .persistent_room_creation_cooldown_seconds
            .into_option(),
        persistent_room_inactivity_expiry_seconds: overrides
            .persistent_room_inactivity_expiry_seconds
            .into_option(),
        isolate_rooms: overrides.isolate_rooms,
    })
}

pub(crate) fn print_help() {
    println!("{}", help_text());
}

pub(crate) fn print_version() {
    println!("sorotte-server {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _guard: MutexGuard<'static, ()>,
        previous: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let guard = ENV_LOCK
                .lock()
                .expect("env test lock should not be poisoned");
            let previous = vars
                .iter()
                .map(|(name, _)| (*name, env::var(name).ok()))
                .collect();
            for (name, value) in vars {
                match value {
                    Some(value) => {
                        // SAFETY: process environment mutation is serialized by ENV_LOCK.
                        unsafe { env::set_var(name, value) };
                    }
                    None => {
                        // SAFETY: process environment mutation is serialized by ENV_LOCK.
                        unsafe { env::remove_var(name) };
                    }
                }
            }
            Self {
                _guard: guard,
                previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in self.previous.iter().rev() {
                match value {
                    Some(value) => {
                        // SAFETY: process environment mutation is serialized by ENV_LOCK.
                        unsafe { env::set_var(name, value) };
                    }
                    None => {
                        // SAFETY: process environment mutation is serialized by ENV_LOCK.
                        unsafe { env::remove_var(name) };
                    }
                }
            }
        }
    }

    #[test]
    fn help_flag_short_circuits_parsing() {
        assert_eq!(parse_server_cli_args(["--help"]).unwrap(), CliAction::Help);
    }

    #[test]
    fn parses_supported_server_flags() {
        let action = parse_server_cli_args([
            "--port",
            "9000",
            "--password",
            "secret",
            "--salt=pepper",
            "--disable-chat",
            "--disable-ready",
            "--rooms-db-file",
            "rooms.sqlite3",
            "--permanent-rooms-file=permanent.txt",
            "--max-chat-message-length",
            "42",
            "--max-username-length=12",
            "--max-persistent-rooms=64",
            "--max-persistent-rooms-per-identity=8",
            "--persistent-room-creation-cooldown-seconds=3",
            "--persistent-room-inactivity-expiry-seconds=600",
            "--stats-db-file",
            "stats.sqlite3",
            "--tls",
            "certs",
            "--ipv6-only",
            "--interface-ipv6",
            "::1",
        ])
        .unwrap();

        let CliAction::Run(overrides) = action else {
            panic!("expected run action");
        };
        let overrides = *overrides;
        assert_eq!(overrides.port, OptionalCliValue::Value(9000));
        assert_eq!(
            overrides.password,
            OptionalCliValue::Value(SecretValue::new("secret"))
        );
        assert_eq!(
            overrides.salt,
            OptionalCliValue::Value(SecretValue::new("pepper"))
        );
        assert!(overrides.disable_chat);
        assert!(overrides.disable_ready);
        assert!(!overrides.isolate_rooms);
        assert_eq!(
            overrides.max_chat_message_length,
            OptionalCliValue::Value(42)
        );
        assert_eq!(overrides.max_username_length, OptionalCliValue::Value(12));
        assert_eq!(overrides.max_persistent_rooms, OptionalCliValue::Value(64));
        assert_eq!(
            overrides.max_persistent_rooms_per_identity,
            OptionalCliValue::Value(8)
        );
        assert_eq!(
            overrides.persistent_room_creation_cooldown_seconds,
            OptionalCliValue::Value(3)
        );
        assert_eq!(
            overrides.persistent_room_inactivity_expiry_seconds,
            OptionalCliValue::Value(600)
        );
        assert_eq!(
            overrides.rooms_db_file,
            OptionalCliValue::Value(PathBuf::from("rooms.sqlite3"))
        );
        assert_eq!(
            overrides.permanent_rooms_file,
            OptionalCliValue::Value(PathBuf::from("permanent.txt"))
        );
        assert_eq!(
            overrides.stats_db_file,
            OptionalCliValue::Value(PathBuf::from("stats.sqlite3"))
        );
        assert_eq!(
            overrides.tls_cert_path,
            OptionalCliValue::Value(PathBuf::from("certs"))
        );
        assert!(overrides.ipv6_only);
        assert_eq!(
            overrides.interface_ipv6,
            OptionalCliValue::Value("::1".to_owned())
        );
    }

    #[test]
    fn parses_supported_server_flags_in_isolate_mode_without_persistence() {
        let action = parse_server_cli_args([
            "--port",
            "9000",
            "--password",
            "secret",
            "--salt=pepper",
            "--disable-chat",
            "--disable-ready",
            "--isolate-rooms",
            "--max-chat-message-length",
            "42",
            "--max-username-length=12",
            "--stats-db-file",
            "stats.sqlite3",
            "--tls",
            "certs",
            "--ipv6-only",
            "--interface-ipv6",
            "::1",
        ])
        .unwrap();

        let CliAction::Run(overrides) = action else {
            panic!("expected run action");
        };
        let overrides = *overrides;
        assert_eq!(overrides.port, OptionalCliValue::Value(9000));
        assert_eq!(
            overrides.password,
            OptionalCliValue::Value(SecretValue::new("secret"))
        );
        assert_eq!(
            overrides.salt,
            OptionalCliValue::Value(SecretValue::new("pepper"))
        );
        assert!(overrides.disable_chat);
        assert!(overrides.disable_ready);
        assert!(overrides.isolate_rooms);
        assert_eq!(
            overrides.max_chat_message_length,
            OptionalCliValue::Value(42)
        );
        assert_eq!(overrides.max_username_length, OptionalCliValue::Value(12));
        assert_eq!(overrides.rooms_db_file, OptionalCliValue::Absent);
        assert_eq!(overrides.permanent_rooms_file, OptionalCliValue::Absent);
        assert_eq!(
            overrides.stats_db_file,
            OptionalCliValue::Value(PathBuf::from("stats.sqlite3"))
        );
        assert_eq!(
            overrides.tls_cert_path,
            OptionalCliValue::Value(PathBuf::from("certs"))
        );
        assert!(overrides.ipv6_only);
        assert_eq!(
            overrides.interface_ipv6,
            OptionalCliValue::Value("::1".to_owned())
        );
    }

    #[test]
    fn python_style_optional_value_flags_accept_missing_values() {
        let action = parse_server_cli_args([
            "--port",
            "--password",
            "--salt",
            "--motd-file",
            "--rooms-db-file",
            "--permanent-rooms-file",
            "--max-chat-message-length",
            "--max-username-length",
            "--max-persistent-rooms",
            "--max-persistent-rooms-per-identity",
            "--persistent-room-creation-cooldown-seconds",
            "--persistent-room-inactivity-expiry-seconds",
            "--stats-db-file",
            "--tls",
            "--interface-ipv4",
            "--interface-ipv6",
        ])
        .unwrap();

        let CliAction::Run(overrides) = action else {
            panic!("expected run action");
        };
        let overrides = *overrides;
        assert_eq!(overrides.port, OptionalCliValue::PresentWithoutValue);
        assert_eq!(overrides.password, OptionalCliValue::PresentWithoutValue);
        assert_eq!(overrides.salt, OptionalCliValue::PresentWithoutValue);
        assert_eq!(overrides.motd_file, OptionalCliValue::PresentWithoutValue);
        assert_eq!(
            overrides.rooms_db_file,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.permanent_rooms_file,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.max_chat_message_length,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.max_username_length,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.max_persistent_rooms,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.max_persistent_rooms_per_identity,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.persistent_room_creation_cooldown_seconds,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.persistent_room_inactivity_expiry_seconds,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.stats_db_file,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.tls_cert_path,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.interface_ipv4,
            OptionalCliValue::PresentWithoutValue
        );
        assert_eq!(
            overrides.interface_ipv6,
            OptionalCliValue::PresentWithoutValue
        );
    }

    #[test]
    fn unknown_flags_are_reported() {
        let error = parse_server_cli_args(["--definitely-not-a-real-flag"]).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("--definitely-not-a-real-flag"));
    }

    #[test]
    fn isolate_mode_accepts_persistent_room_configuration() {
        let overrides = ServerCliOverrides {
            isolate_rooms: true,
            rooms_db_file: OptionalCliValue::Value(PathBuf::from("rooms.sqlite3")),
            ..ServerCliOverrides::default()
        };
        let config = resolve_run_config(overrides).unwrap();
        assert!(config.isolate_rooms);
        assert!(config.persistent_rooms_enabled);
        assert_eq!(config.rooms_db_file, Some(PathBuf::from("rooms.sqlite3")));
    }

    #[test]
    fn bind_mode_conflict_is_rejected() {
        let overrides = ServerCliOverrides {
            ipv4_only: true,
            ipv6_only: true,
            ..ServerCliOverrides::default()
        };
        let error = resolve_bind_endpoints(&overrides).unwrap_err();
        assert!(error.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn default_bind_mode_uses_python_ipv6_then_ipv4_order() {
        let overrides = ServerCliOverrides::default();
        let endpoints = resolve_bind_endpoints(&overrides).unwrap();
        assert_eq!(
            endpoints,
            vec![
                ServerBindEndpoint {
                    family: ServerBindFamily::Ipv6,
                    host: "::".to_owned(),
                },
                ServerBindEndpoint {
                    family: ServerBindFamily::Ipv4,
                    host: "0.0.0.0".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn default_bind_mode_accepts_both_interface_families() {
        let overrides = ServerCliOverrides {
            interface_ipv4: OptionalCliValue::Value("127.0.0.1".to_owned()),
            interface_ipv6: OptionalCliValue::Value("::1".to_owned()),
            ..ServerCliOverrides::default()
        };
        let endpoints = resolve_bind_endpoints(&overrides).unwrap();
        assert_eq!(
            endpoints,
            vec![
                ServerBindEndpoint {
                    family: ServerBindFamily::Ipv6,
                    host: "::1".to_owned(),
                },
                ServerBindEndpoint {
                    family: ServerBindFamily::Ipv4,
                    host: "127.0.0.1".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn legacy_env_password_and_salt_have_precedence_over_rust_aliases() {
        let _env = EnvGuard::set(&[
            ("SOROTTE_PASSWORD", Some("legacy-password")),
            ("SOROTTE_SERVER_PASSWORD", Some("rust-password")),
            ("SOROTTE_SALT", Some("legacy-salt")),
            ("SOROTTE_SERVER_SALT", Some("rust-salt")),
        ]);

        let config = resolve_run_config(ServerCliOverrides::default()).unwrap();
        assert_eq!(
            config
                .server_password_token
                .as_ref()
                .map(SecretValue::expose_secret),
            Some("legacy-password")
        );
        assert_eq!(
            config
                .room_password_salt
                .as_ref()
                .map(SecretValue::expose_secret),
            Some("legacy-salt")
        );
    }

    #[test]
    fn present_without_value_suppresses_environment_defaults() {
        let _env = EnvGuard::set(&[
            ("SOROTTE_SERVER_PORT", Some("9001")),
            ("SOROTTE_PASSWORD", Some("legacy-password")),
            ("SOROTTE_SALT", Some("legacy-salt")),
            ("SOROTTE_SERVER_MOTD_TEMPLATE", Some("motd")),
            ("SOROTTE_SERVER_ROOMS_DB_FILE", Some("rooms.sqlite3")),
            ("SOROTTE_SERVER_PERMANENT_ROOMS_FILE", Some("permanent.txt")),
            ("SOROTTE_SERVER_STATS_DB_FILE", Some("stats.sqlite3")),
            ("SOROTTE_SERVER_TLS_CERT_PATH", Some("certs")),
        ]);
        let action = parse_server_cli_args([
            "--port",
            "--password",
            "--salt",
            "--motd-file",
            "--rooms-db-file",
            "--permanent-rooms-file",
            "--stats-db-file",
            "--tls",
        ])
        .unwrap();
        let CliAction::Run(overrides) = action else {
            panic!("expected run action");
        };

        let config = resolve_run_config(*overrides).unwrap();
        assert_eq!(config.port, DEFAULT_SERVER_PORT);
        assert_eq!(config.server_password_token, None);
        assert_eq!(config.room_password_salt, None);
        assert_eq!(config.motd_template, None);
        assert_eq!(config.rooms_db_file, None);
        assert_eq!(config.permanent_rooms_file, None);
        assert_eq!(config.stats_db_file, None);
        assert_eq!(config.tls_cert_path, None);
    }

    #[test]
    fn server_cli_configuration_debug_redacts_password_and_salt() {
        let password = "server-cli-password-canary";
        let salt = "server-cli-salt-canary";
        let overrides = ServerCliOverrides {
            password: OptionalCliValue::Value(password.into()),
            salt: OptionalCliValue::Value(salt.into()),
            ..ServerCliOverrides::default()
        };

        let overrides_debug = format!("{overrides:?}");
        assert!(overrides_debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!overrides_debug.contains(password));
        assert!(!overrides_debug.contains(salt));

        for field_debug in [
            format!("{:?}", overrides.password),
            format!("{:?}", overrides.salt),
        ] {
            assert!(field_debug.contains(sorotte_secret::REDACTED_SECRET));
            assert!(!field_debug.contains(password));
            assert!(!field_debug.contains(salt));
        }

        let config = resolve_run_config(overrides).expect("server configuration should resolve");
        let config_debug = format!("{config:?}");
        assert!(config_debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!config_debug.contains(password));
        assert!(!config_debug.contains(salt));

        for field_debug in [
            format!("{:?}", config.server_password_token),
            format!("{:?}", config.room_password_salt),
        ] {
            assert!(field_debug.contains(sorotte_secret::REDACTED_SECRET));
            assert!(!field_debug.contains(password));
            assert!(!field_debug.contains(salt));
        }
    }
}
